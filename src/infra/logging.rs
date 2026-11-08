use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::app::DiagramResult;

const MAX_BYTES: u64 = 5 * 1024 * 1024;
const MAX_FIELD_BYTES: usize = 2048;
const SUPPRESSION_WINDOW: Duration = Duration::from_secs(60);
const MAX_RECENT: usize = 128;

#[derive(Default)]
struct Logger {
    recent: VecDeque<(u64, Instant)>,
}

impl Logger {
    fn record(
        &mut self,
        path: &Path,
        start: usize,
        source: &str,
        error: &str,
        directory: Option<&Path>,
        stderr: &mut impl Write,
    ) {
        // CLI stderr can echo diagram source or arbitrary data. Preserve its exit
        // status as the reason, but never copy that untrusted detail into logs.
        let status = error.strip_prefix("mermaid-cli exited with ").map(|rest| {
            let mut parts = rest.splitn(3, ": ");
            let label = parts.next().unwrap_or("failure");
            let code = parts.next().unwrap_or("");
            if code.parse::<i64>().is_ok() {
                format!("{}: {}", field(label), code)
            } else {
                field(label)
            }
        });
        let mut reason = status.unwrap_or_else(|| field(error));
        if !source.is_empty() {
            reason = reason.replace(&field(source), "[diagram source omitted]");
        }
        if error.starts_with("mermaid-cli exited with ") {
            reason.insert_str(0, "mermaid-cli exited with ");
            if error.contains("Parse error") || error.contains("Syntax error") {
                reason.push_str(" (syntax/parse error)");
            }
        }
        let mut hash = DefaultHasher::new();
        path.hash(&mut hash);
        start.hash(&mut hash);
        reason.hash(&mut hash);
        let key = hash.finish();
        let now = Instant::now();
        self.recent
            .retain(|(_, time)| now.duration_since(*time) < SUPPRESSION_WINDOW);
        if self.recent.iter().any(|(seen, _)| *seen == key) {
            return;
        }
        if self.recent.len() == MAX_RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back((key, now));
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let entry = format!(
            "unix_seconds={timestamp} file={} utf8_byte_offset={} reason={}\n",
            field(&path.to_string_lossy()),
            start,
            reason
        );
        let outcome = directory
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))
            .and_then(|dir| append(dir, entry.as_bytes()));
        if let Err(error) = outcome {
            let _ = writeln!(stderr, "Mermaid log write failed: {error}; {entry}");
        }
    }
}

fn field(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        let escaped = ch.escape_debug().to_string();
        if output.len() + escaped.len() > MAX_FIELD_BYTES {
            output.push_str("...");
            break;
        }
        output.push_str(&escaped);
    }
    output
}

fn append(dir: &Path, entry: &[u8]) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let current = dir.join("viewer.log");
    let size = match fs::metadata(&current) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    if size + entry.len() as u64 > MAX_BYTES {
        let oldest = dir.join("viewer.log.3");
        match fs::remove_file(oldest) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        for number in (1..=2).rev() {
            move_if_exists(
                &dir.join(format!("viewer.log.{number}")),
                &dir.join(format!("viewer.log.{}", number + 1)),
            )?;
        }
        move_if_exists(&current, &dir.join("viewer.log.1"))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(current)?
        .write_all(entry)
}

fn move_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(error) if error.kind() == io::ErrorKind::NotFound && !from.exists() => Ok(()),
        result => result,
    }
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

pub fn log_config_failure(error: &super::config::ConfigError) {
    let _guard = LOGGER
        .get_or_init(|| Mutex::new(Logger::default()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("my-markdown-viewer").join("logs"));
    record_config_failure(error, directory.as_deref(), &mut io::stderr().lock());
}

fn record_config_failure(
    error: &super::config::ConfigError,
    directory: Option<&Path>,
    stderr: &mut impl Write,
) {
    let entry = format!(
        "unix_seconds={} config={} reason={}\n",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        error
            .path
            .as_ref()
            .map(|path| field(&path.to_string_lossy()))
            .unwrap_or_else(|| "[unavailable]".into()),
        field(&error.cause)
    );
    let outcome = directory
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable"))
        .and_then(|dir| append(dir, entry.as_bytes()));
    if let Err(error) = outcome {
        let _ = writeln!(stderr, "Config log write failed: {error}; {entry}");
    }
}

pub fn log_diagram_failure(result: &DiagramResult) {
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("my-markdown-viewer").join("logs"));
    let mut logger = LOGGER
        .get_or_init(|| Mutex::new(Logger::default()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Err(error) = &result.png {
        logger.record(
            &result.request.path,
            result.request.start,
            &result.request.source,
            error,
            directory.as_deref(),
            &mut io::stderr().lock(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "mmv-log-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn config_error_uses_shared_log_and_stderr_fallback() {
        let dir = TempDir::new();
        let error = crate::infra::config::ConfigError {
            path: Some(PathBuf::from("日本語/init.lua")),
            cause: "invalid color".into(),
        };
        let mut stderr = Vec::new();
        record_config_failure(&error, Some(&dir.0), &mut stderr);
        let log = fs::read_to_string(dir.0.join("viewer.log")).unwrap();
        assert!(log.contains("日本語/init.lua") && log.contains("invalid color"));
        assert!(stderr.is_empty());
        record_config_failure(&error, None, &mut stderr);
        let output = String::from_utf8(stderr).unwrap();
        assert!(output.contains("日本語/init.lua") && output.contains("invalid color"));
    }

    #[test]
    fn rotates_real_files_and_caps_untrusted_details() {
        let dir = TempDir::new();
        let current = dir.0.join("viewer.log");
        let file = fs::File::create(&current).unwrap();
        file.set_len(MAX_BYTES).unwrap();
        drop(file);
        for n in 1..=3 {
            fs::write(dir.0.join(format!("viewer.log.{n}")), format!("old{n}")).unwrap();
        }
        let mut logger = Logger::default();
        let mut fallback = Vec::new();
        let error = format!(
            "mermaid-cli exited with exit code: 1: {}",
            "x".repeat(6 * 1024 * 1024)
        );
        logger.record(
            Path::new("日本語.md"),
            42,
            "secret",
            &error,
            Some(&dir.0),
            &mut fallback,
        );
        assert!(fallback.is_empty());
        let log = fs::read_to_string(&current).unwrap();
        assert!(
            log.contains("日本語.md")
                && log.contains("utf8_byte_offset=42")
                && log.contains("mermaid-cli exited with exit code")
        );
        assert!(!log.contains("secret"));
        assert!(log.len() < 5000);
        assert_eq!(
            fs::metadata(dir.0.join("viewer.log.1")).unwrap().len(),
            MAX_BYTES
        );
        assert_eq!(
            fs::read_to_string(dir.0.join("viewer.log.2")).unwrap(),
            "old1"
        );
        assert_eq!(
            fs::read_to_string(dir.0.join("viewer.log.3")).unwrap(),
            "old2"
        );
    }

    #[test]
    fn failed_storage_falls_back_and_suppresses_repeats() {
        let dir = TempDir::new();
        let blocked = dir.0.join("file");
        fs::write(&blocked, "not a directory").unwrap();
        let mut logger = Logger::default();
        let mut stderr = Vec::new();
        for _ in 0..1000 {
            logger.record(
                Path::new("document.md"),
                17,
                "secret",
                "starting mermaid-cli: missing",
                Some(&blocked),
                &mut stderr,
            );
        }
        let output = String::from_utf8(stderr).unwrap();
        assert_eq!(output.matches("Mermaid log write failed").count(), 1);
        assert!(
            output.contains("document.md")
                && output.contains("utf8_byte_offset=17")
                && output.contains("starting mermaid-cli: missing")
        );
        assert!(logger.recent.len() <= MAX_RECENT);
    }

    #[test]
    fn suppression_cache_is_bounded_and_entries_expire() {
        let mut logger = Logger::default();
        let mut stderr = Vec::new();
        for offset in 0..MAX_RECENT + 10 {
            logger.record(
                Path::new("x.md"),
                offset,
                "source",
                "failure",
                None,
                &mut stderr,
            );
        }
        assert_eq!(logger.recent.len(), MAX_RECENT);
        for (_, time) in &mut logger.recent {
            *time = time.checked_sub(SUPPRESSION_WINDOW).unwrap();
        }
        logger.record(
            Path::new("x.md"),
            MAX_RECENT + 9,
            "source",
            "failure",
            None,
            &mut stderr,
        );
        assert_eq!(logger.recent.len(), 1);
        assert_eq!(
            String::from_utf8(stderr)
                .unwrap()
                .matches("Mermaid log write failed")
                .count(),
            MAX_RECENT + 11
        );
    }

    #[test]
    fn real_stderr_receives_failed_storage_fallback() {
        const CHILD: &str = "MMV_LOG_STDERR_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            Logger::default().record(
                Path::new("fixture.md"),
                8,
                "source",
                "mmdc missing",
                None,
                &mut io::stderr().lock(),
            );
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::logging::tests::real_stderr_receives_failed_storage_fallback",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("Mermaid log write failed")
                && stderr.contains("fixture.md")
                && stderr.contains("mmdc missing")
        );
    }

    #[test]
    fn rotation_failure_falls_back_without_growing_current_file() {
        let dir = TempDir::new();
        let file = fs::File::create(dir.0.join("viewer.log")).unwrap();
        file.set_len(MAX_BYTES).unwrap();
        drop(file);
        fs::create_dir(dir.0.join("viewer.log.3")).unwrap();
        let mut fallback = Vec::new();
        Logger::default().record(
            Path::new("x.md"),
            0,
            "source",
            "failure",
            Some(&dir.0),
            &mut fallback,
        );
        assert!(
            String::from_utf8(fallback)
                .unwrap()
                .contains("Mermaid log write failed")
        );
        assert_eq!(
            fs::metadata(dir.0.join("viewer.log")).unwrap().len(),
            MAX_BYTES
        );
    }

    #[test]
    fn large_reason_and_source_are_bounded_and_source_is_omitted() {
        let dir = TempDir::new();
        let mut logger = Logger::default();
        let mut stderr = Vec::new();
        logger.record(
            Path::new("x.md"),
            0,
            "secret diagram",
            &format!("secret diagram{}", "z".repeat(6 * 1024 * 1024)),
            Some(&dir.0),
            &mut stderr,
        );
        let log = fs::read_to_string(dir.0.join("viewer.log")).unwrap();
        assert!(!log.contains("secret diagram"));
        assert!(log.len() < 5000);
        assert!(stderr.is_empty());
    }
}
