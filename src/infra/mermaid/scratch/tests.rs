use super::*;
use crate::infra::mermaid::render_with_scratch;
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const FIXTURE: &str = "infra::mermaid::scratch::tests::child_fixture";
const MODE: &str = "MMV_SCRATCH_TEST_MODE";
const TEST_ROOT: &str = "MMV_SCRATCH_TEST_ROOT";

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mmv-scratch-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
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

// Own exactly the child we started, including on an assertion failure.
struct TestChild(Child);
impl TestChild {
    fn wait(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "fixture did not exit within 15 seconds"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", FIXTURE, "--nocapture", "--"])
        .env(MODE, mode);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[test]
fn child_fixture() {
    let Some(mode) = std::env::var_os(MODE) else {
        return;
    };
    match mode.to_str().unwrap() {
        "hold" => {
            let root = PathBuf::from(std::env::var_os(TEST_ROOT).unwrap());
            let scratch = Scratch::new().unwrap();
            assert_eq!(scratch.path().parent().unwrap(), root);
            fs::write(scratch.path().join("input.mmd"), "private diagram").unwrap();
            fs::write(root.join("ready"), scratch.path().to_str().unwrap()).unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        "cleanup" => {
            crate::infra::cleanup_mermaid_scratch();
        }
        "success" | "invalid" | "missing" | "failure" => {
            if mode == "failure" {
                std::process::exit(3);
            }
            if mode != "missing" {
                let args: Vec<_> = std::env::args_os().collect();
                let output = &args[args.iter().position(|arg| arg == "-o").unwrap() + 1];
                fs::write(
                    output,
                    if mode == "success" {
                        b"\x89PNG\r\n\x1a\nfixture".as_slice()
                    } else {
                        b"not PNG"
                    },
                )
                .unwrap();
            }
        }
        _ => panic!("unknown fixture mode"),
    }
}

#[test]
fn rendering_removes_real_files_on_success_and_all_failure_paths() {
    let directory = TempDir::new();
    let root = directory.0.join("scratch");
    for mode in ["success", "failure", "invalid", "missing", "unavailable"] {
        let scratch = Scratch::in_root(&root).unwrap();
        let path = scratch.path().to_owned();
        let mut command = if mode == "unavailable" {
            Command::new("mmv-no-such-mmdc")
        } else {
            fixture(mode)
        };
        let result = render_with_scratch("flowchart TD\nA --> B", &mut command, scratch);
        match mode {
            "success" => assert_eq!(result.unwrap(), b"\x89PNG\r\n\x1a\nfixture"),
            "failure" => assert!(result.unwrap_err().contains("mermaid-cli exited")),
            "invalid" => assert!(result.unwrap_err().contains("did not produce a PNG")),
            "missing" => assert!(result.unwrap_err().contains("reading generated PNG")),
            _ => assert!(result.unwrap_err().contains("starting mermaid-cli")),
        }
        assert!(!path.exists(), "left temporary files after {mode}");
    }
}

#[test]
fn next_startup_cleanup_preserves_a_live_process_then_collects_its_crash_files() {
    let directory = TempDir::new();
    let user = crate::infra::security::UserSecurity::current().unwrap();
    let root = user_root(&directory.0, &user.sid_text);
    // Override only each dedicated child's TEMP, never process-global test state.
    let startup = |mode| {
        let mut command = fixture(mode);
        command
            .env("TEMP", &directory.0)
            .env("TMP", &directory.0)
            .env("TMPDIR", &directory.0)
            .env(TEST_ROOT, &root);
        TestChild(command.spawn().unwrap())
    };
    let mut child = startup("hold");
    let deadline = Instant::now() + Duration::from_secs(15);
    let ready = root.join("ready");
    while !ready.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "holder exited before readiness"
        );
        assert!(Instant::now() < deadline, "holder did not become ready");
        std::thread::sleep(Duration::from_millis(10));
    }
    let path = PathBuf::from(fs::read_to_string(ready).unwrap());
    let mut next_startup = startup("cleanup");
    assert!(next_startup.wait().success());
    assert_eq!(
        fs::read(path.join("input.mmd")).unwrap(),
        b"private diagram"
    );
    assert!(child.0.try_wait().unwrap().is_none());

    // Terminate only our retained Child handle, not a PID/name discovered by scanning.
    child.0.kill().unwrap();
    assert!(!child.wait().success());
    assert!(path.exists(), "abnormal termination unexpectedly ran Drop");
    let mut next_startup = startup("cleanup");
    assert!(next_startup.wait().success());
    assert!(!path.exists());
}

fn abandoned(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join(LEASE), MARKER).unwrap();
    fs::write(path.join("input.mmd"), "diagram").unwrap();
    path
}

#[test]
fn shared_temp_base_keeps_user_roots_separate() {
    let directory = TempDir::new();
    let first = user_root(&directory.0, "S-1-5-21-1001");
    let second = user_root(&directory.0, "S-1-5-21-1002");
    assert_eq!(
        first,
        directory
            .0
            .join("my-markdown-viewer-mermaid-v1-S-1-5-21-1001")
    );
    assert_ne!(first, second);
    assert_eq!(first.parent(), second.parent());
    let first_scratch = abandoned(&first, "stopped");
    let second_scratch = abandoned(&second, "stopped");
    cleanup_in(&first);
    assert!(!first_scratch.exists());
    assert_eq!(
        fs::read(second_scratch.join("input.mmd")).unwrap(),
        b"diagram"
    );
    assert_eq!(fs::read(second_scratch.join(LEASE)).unwrap(), MARKER);
}

#[test]
fn cleanup_ignores_unmarked_invalid_and_legacy_directories() {
    let directory = TempDir::new();
    let root = directory.0.join("scratch");
    let managed = abandoned(&root, "managed");
    let invalid = abandoned(&root, "invalid");
    fs::write(invalid.join(LEASE), vec![b'x'; MARKER.len()]).unwrap();
    let unmarked = root.join("unmarked");
    fs::create_dir(&unmarked).unwrap();
    fs::write(unmarked.join("keep"), "unrelated").unwrap();
    let legacy = directory.0.join("mmv-mermaid-123-0");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("keep"), "legacy").unwrap();
    cleanup_in(&root);
    assert!(!managed.exists());
    assert_eq!(fs::read(invalid.join("input.mmd")).unwrap(), b"diagram");
    assert_eq!(fs::read(unmarked.join("keep")).unwrap(), b"unrelated");
    assert_eq!(fs::read(legacy.join("keep")).unwrap(), b"legacy");
    cleanup_in(&directory.0.join("absent"));
    let file = directory.0.join("not-a-directory");
    fs::write(&file, "keep").unwrap();
    cleanup_in(&file); // A failed scan must not fail GUI startup.
    assert_eq!(fs::read(file).unwrap(), b"keep");
}

#[cfg(windows)]
#[test]
fn failed_deletion_keeps_marker_for_retry() {
    let directory = TempDir::new();
    let root = directory.0.join("scratch");
    let path = abandoned(&root, "blocked");
    let input = path.join("input.mmd");
    use std::os::windows::fs::OpenOptionsExt as _;
    let held = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&input)
        .unwrap();
    cleanup_in(&root);
    assert_eq!(fs::read(path.join(LEASE)).unwrap(), MARKER);
    drop(held);
    cleanup_in(&root);
    assert!(!path.exists());
}

#[cfg(windows)]
#[test]
fn cleanup_never_follows_junctions_at_root_entry_or_inside_scratch() {
    let directory = TempDir::new();
    let outside = directory.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let protected = abandoned(&outside, "protected");
    let root = directory.0.join("scratch");
    fs::create_dir(&root).unwrap();
    let root_link = directory.0.join("root-link");
    junction(&root_link, &outside);
    cleanup_in(&root_link);
    assert!(protected.join("input.mmd").exists());
    junction(&root.join("linked-entry"), &protected);
    let managed = abandoned(&root, "managed");
    junction(&managed.join("nested-link"), &protected);
    fs::create_dir(managed.join("subdir")).unwrap();
    junction(&managed.join("subdir").join("deeper-link"), &protected);
    cleanup_in(&root);
    assert!(!managed.exists());
    assert!(root.join("linked-entry").exists());
    assert_eq!(fs::read(protected.join("input.mmd")).unwrap(), b"diagram");
    assert_eq!(fs::read(protected.join(LEASE)).unwrap(), MARKER);
    assert!(Scratch::in_root(&root_link).is_err());
}

#[cfg(windows)]
fn junction(link: &Path, target: &Path) {
    // All paths are generated test paths, never document content or user input.
    let mut command = Command::new("cmd.exe");
    command
        .args(["/d", "/c", "mklink", "/J"])
        .arg(link)
        .arg(target);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = TestChild(command.spawn().unwrap());
    assert!(child.wait().success(), "creating test junction");
}
