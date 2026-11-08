#[cfg(windows)]
mod process;

use std::{
    fs::{self, File},
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use crate::app::{DiagramRequest, DiagramResult};

const TIMEOUT: Duration = Duration::from_secs(45);

pub fn generate_diagram(request: DiagramRequest) -> DiagramResult {
    let png = mermaid_command().and_then(|mut command| render(&request.source, &mut command));
    DiagramResult { request, png }
}

fn mermaid_command() -> Result<Command, String> {
    #[cfg(windows)]
    {
        // npm's mmdc.cmd is a shell script. Invoke its Node entry point directly:
        // neither Markdown content nor file names should pass through cmd.exe.
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            let executable = dir.join("mmdc.exe");
            if executable.is_file() {
                return Ok(Command::new(executable));
            }
            if dir.join("mmdc.cmd").is_file() {
                let cli = dir.join("node_modules/@mermaid-js/mermaid-cli/src/cli.js");
                if !cli.is_file() {
                    return Err(format!("mmdc npm entry point not found: {}", cli.display()));
                }
                let local_node = dir.join("node.exe");
                let mut command = Command::new(if local_node.is_file() {
                    local_node
                } else {
                    PathBuf::from("node.exe")
                });
                command.arg(cli);
                return Ok(command);
            }
        }
        Err("mmdc not found on PATH (expected an npm installation or mmdc.exe)".into())
    }
    #[cfg(not(windows))]
    {
        Ok(Command::new("mmdc"))
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "mmv-mermaid-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("creating Mermaid temporary directory: {error}")),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!(
                "removing Mermaid temporary directory {}: {error}",
                self.0.display()
            );
        }
    }
}

fn render(source: &str, command: &mut Command) -> Result<Vec<u8>, String> {
    let scratch = Scratch::new()?;
    let input = scratch.0.join("input.mmd");
    let output = scratch.0.join("output.png");
    let config = scratch.0.join("config.json");
    let stderr = scratch.0.join("stderr.txt");
    fs::write(&input, source).map_err(|error| format!("writing Mermaid source: {error}"))?;
    fs::write(
        &config,
        r#"{"fontFamily":"Yu Gothic UI, sans-serif","securityLevel":"strict"}"#,
    )
    .map_err(|error| format!("writing Mermaid config: {error}"))?;
    let error_file = File::create(&stderr).map_err(|error| error.to_string())?;
    command
        .args([
            "-i".as_ref(),
            input.as_os_str(),
            "-o".as_ref(),
            output.as_os_str(),
        ])
        .arg("-c")
        .arg(&config)
        .args(["-s", "2", "-b", "white"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(error_file));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("starting mermaid-cli: {error}"))?;
    #[cfg(windows)]
    let job = match process::ChildJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50))
            }
            result => {
                #[cfg(windows)]
                drop(job);
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(error) => format!("waiting for mermaid-cli: {error}"),
                    _ => format!("mermaid-cli timed out after {} seconds", TIMEOUT.as_secs()),
                });
            }
        }
    };
    if !status.success() {
        let detail = fs::read_to_string(&stderr).unwrap_or_default();
        return Err(format!(
            "mermaid-cli exited with {status}: {}",
            detail.trim()
        ));
    }
    let png = fs::read(&output).map_err(|error| format!("reading generated PNG: {error}"))?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("mermaid-cli did not produce a PNG".into());
    }
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_executable_reports_a_startup_error() {
        let error = render(
            "flowchart TD\nA --> B",
            &mut Command::new("mmv-no-such-mmdc"),
        )
        .unwrap_err();
        assert!(error.contains("starting mermaid-cli"), "{error}");
    }

    #[test]
    #[ignore = "requires Node.js, mermaid-cli and its browser; run explicitly"]
    fn real_cli_generates_three_diagram_types_and_reports_syntax_errors() {
        for source in [
            "flowchart TD\nA[文書を開く] --> B[本文を読む]",
            "classDiagram\nDocument --> Diagram : 図を含む",
            "erDiagram\nDOCUMENT ||--o{ DIAGRAM : \"図を含む\"",
        ] {
            let png = render(source, &mut mermaid_command().unwrap()).unwrap();
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(png.len() > 1000);
        }
        let error = render("flowchart TD\nA --> [", &mut mermaid_command().unwrap()).unwrap_err();
        assert!(error.contains("mermaid-cli exited"), "{error}");
        assert!(error.contains("Parse error"), "{error}");
    }
}
