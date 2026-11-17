//! GUI lifetime and startup arbitration use distinct file locks.
use super::{deadline::Deadline, ipc::PipeClient, lock, paths::Namespace};
use crate::app::ExternalOperation;
use std::{io, time::Instant};

pub struct InstanceGuard {
    _file: lock::SecureFile,
}
impl InstanceGuard {
    fn try_acquire(namespace: &Namespace, role: &str) -> io::Result<Option<Self>> {
        let file = lock::open(namespace, role)?;
        match file.file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }
    pub fn claim_gui(namespace: &Namespace) -> io::Result<Option<Self>> {
        Self::try_acquire(namespace, "live")
    }
}
pub fn gui_is_alive(namespace: &Namespace) -> io::Result<bool> {
    Ok(InstanceGuard::claim_gui(namespace)?.is_none())
}

pub const INTERNAL_GUI_ARGUMENT: &str = "--mmv-internal-gui";

#[cfg(windows)]
use super::security::Handle;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt as _;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW,
};
#[cfg(windows)]
fn gui_command() -> io::Result<(Vec<u16>, Vec<u16>)> {
    let executable: Vec<u16> = std::env::current_exe()?.as_os_str().encode_wide().collect();
    // Windows filenames cannot contain quotes. Preserve UTF-16 without a lossy
    // conversion and quote argv[0]; the sole other argument is a fixed constant.
    let command = std::iter::once(b'"' as u16)
        .chain(executable.iter().copied())
        .chain(format!("\" {INTERNAL_GUI_ARGUMENT}").encode_utf16())
        .chain(Some(0))
        .collect();
    Ok((executable.into_iter().chain(Some(0)).collect(), command))
}
/// Start the same executable without a shell, console or ANY inherited handles.
/// Stdio::null alone does not exclude other inheritable handles: retaining a
/// caller's redirected pipe write-end prevents EOF even after its CLI exits.
#[cfg(windows)]
fn spawn_gui() -> io::Result<()> {
    let (executable, mut command) = gui_command()?;
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // No STARTF_USESTDHANDLES, no inherited handles; GPUI separately uses focus:false.
    if unsafe {
        CreateProcessW(
            executable.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // The GUI lives independently; neither handle is needed for arbitration.
    let _process = Handle::new(process.hProcess)?;
    let _thread = Handle::new(process.hThread)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn spawn_gui() -> io::Result<()> {
    use std::process::{Command, Stdio};
    // std Files are CLOEXEC; no lock descriptor survives exec.
    Command::new(std::env::current_exe()?)
        .arg(INTERNAL_GUI_ARGUMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

pub async fn connect_or_start(
    namespace: &Namespace,
    operation: &ExternalOperation,
) -> io::Result<PipeClient> {
    connect_with(namespace, operation, spawn_gui).await
}
async fn connect_with(
    namespace: &Namespace,
    operation: &ExternalOperation,
    spawn: impl FnOnce() -> io::Result<()>,
) -> io::Result<PipeClient> {
    let connection = Deadline::connection(Instant::now());
    if gui_is_alive(namespace)? {
        return PipeClient::connect(namespace, connection).await;
    }
    if !matches!(operation, ExternalOperation::Open { .. }) {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "GUI is not running",
        ));
    }
    let deadline = Deadline::startup(Instant::now());
    let startup = loop {
        if deadline.expired(Instant::now()) {
            return Err(Deadline::timeout());
        }
        if let Some(guard) = InstanceGuard::try_acquire(namespace, "startup")? {
            break guard;
        }
        deadline.pause().await?;
    };
    // Another CLI may have started the GUI while we waited for startup.
    // TCP reachability is not a substitute for the GUI's lifetime lock.
    if !gui_is_alive(namespace)? {
        if deadline.expired(Instant::now()) {
            return Err(Deadline::timeout());
        }
        spawn()?;
    }
    let client = PipeClient::connect(namespace, deadline).await;
    drop(startup);
    client
}

#[cfg(test)]
mod tests {
    use super::super::ipc::PipeServer;
    use super::*;
    use std::{
        process::{Child, Command, Stdio},
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    fn connect(
        namespace: &Namespace,
        operation: &ExternalOperation,
        spawn: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<PipeClient> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(connect_with(namespace, operation, spawn))
    }
    struct Fixture(Namespace);
    impl Fixture {
        fn new() -> Self {
            Self(Namespace::isolated())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0.directory);
        }
    }

    #[test]
    fn file_lock_lifetime_stale_files_and_error_classification() {
        let fixture = Fixture::new();
        let n = &fixture.0;
        let guard = InstanceGuard::claim_gui(n).unwrap().unwrap();
        assert!(gui_is_alive(n).unwrap());
        assert!(InstanceGuard::claim_gui(n).unwrap().is_none());
        drop(guard);
        assert!(n.directory.join("live.lock").exists());
        assert!(!gui_is_alive(n).unwrap());
        // A directory is not a stale lock file or a not-running signal.
        std::fs::remove_file(n.directory.join("live.lock")).unwrap();
        std::fs::create_dir(n.directory.join("live.lock")).unwrap();
        assert!(gui_is_alive(n).is_err());
        assert!(
            connect(n, &ExternalOperation::Top, || panic!(
                "must not spawn on IO failure"
            ))
            .is_err()
        );
    }

    #[test]
    fn simultaneous_open_converges_and_rechecks_liveness() {
        let fixture = Fixture::new();
        let namespace = &fixture.0;
        let barrier = Arc::new(Barrier::new(5));
        let spawned = Arc::new(AtomicUsize::new(0));
        let (finish, finish_rx) = mpsc::channel();
        let finish_rx = Arc::new(std::sync::Mutex::new(finish_rx));
        let (owner_tx, owner_rx) = mpsc::channel();
        let mut callers = Vec::new();
        for _ in 0..4 {
            let n = namespace.clone();
            let barrier = barrier.clone();
            let spawned = spawned.clone();
            let owner_tx = owner_tx.clone();
            let finish_rx = finish_rx.clone();
            callers.push(thread::spawn(move || {
                barrier.wait();
                connect(
                    &n,
                    &ExternalOperation::Open {
                        path: std::env::temp_dir().join("test.md"),
                        line: None,
                    },
                    || {
                        spawned.fetch_add(1, Ordering::SeqCst);
                        let (ready, rx) = mpsc::channel();
                        let other = n.clone();
                        let owner = thread::spawn(move || {
                            let _live = InstanceGuard::claim_gui(&other).unwrap().unwrap();
                            let _server = PipeServer::bind(&other).unwrap();
                            ready.send(()).unwrap();
                            finish_rx
                                .lock()
                                .unwrap()
                                .recv_timeout(Duration::from_secs(10))
                                .unwrap();
                        });
                        owner_tx.send(owner).unwrap();
                        rx.recv_timeout(Duration::from_secs(3)).unwrap();
                        Ok(())
                    },
                )
                .unwrap();
            }));
        }
        barrier.wait();
        for caller in callers {
            caller.join().unwrap();
        }
        assert_eq!(spawned.load(Ordering::SeqCst), 1);
        finish.send(()).unwrap();
        owner_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn live_but_unreachable_or_changed_port_never_spawns() {
        let fixture = Fixture::new();
        let n = &fixture.0;
        let _live = InstanceGuard::claim_gui(n).unwrap().unwrap();
        let mut other_port = Namespace::isolated();
        other_port.directory = n.directory.clone();
        let start = Instant::now();
        let error = connect(
            &other_port,
            &ExternalOperation::Open {
                path: std::env::temp_dir().join("test.md"),
                line: None,
            },
            || panic!("live GUI must not spawn"),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() >= Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_secs(8));
    }

    #[test]
    fn missing_gui_non_open_never_spawns() {
        let fixture = Fixture::new();
        for operation in [
            ExternalOperation::Top,
            ExternalOperation::Bottom,
            ExternalOperation::Back,
            ExternalOperation::Forward,
            ExternalOperation::Scroll(crate::app::ScrollDirection::Down),
        ] {
            let error = connect(&fixture.0, &operation, || panic!("must not launch"))
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::NotConnected);
        }
    }

    #[test]
    fn absent_live_non_open_ignores_tcp_listener() {
        let fixture = Fixture::new();
        let listener = std::net::TcpListener::bind(fixture.0.address()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let result = connect(&fixture.0, &ExternalOperation::Top, || {
            panic!("non-Open must not launch")
        });
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::NotConnected);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(!fixture.0.directory.join("startup.lock").exists());
    }

    #[test]
    fn absent_live_open_reaches_spawn_without_tcp_attempts() {
        let fixture = Fixture::new();
        let n = &fixture.0;
        let listener = std::net::TcpListener::bind(n.address()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut spawned = false;
        let result = connect(
            n,
            &ExternalOperation::Open {
                path: std::env::temp_dir().join("test.md"),
                line: None,
            },
            || {
                spawned = true;
                assert!(InstanceGuard::try_acquire(n, "startup").unwrap().is_none());
                assert!(!gui_is_alive(n).unwrap());
                assert_eq!(
                    listener.accept().unwrap_err().kind(),
                    io::ErrorKind::WouldBlock
                );
                Err(io::Error::other("fixture stops before spawning"))
            },
        );
        assert!(
            spawned,
            "TCP-only listener must not bypass startup arbitration"
        );
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::Other);
        assert!(InstanceGuard::try_acquire(n, "startup").unwrap().is_some());
    }

    #[test]
    fn invalid_locks_fail_closed_before_tcp() {
        for role in ["live", "startup"] {
            let fixture = Fixture::new();
            let n = &fixture.0;
            drop(lock::open(n, "live").unwrap());
            if role == "live" {
                std::fs::remove_file(n.directory.join("live.lock")).unwrap();
            }
            std::fs::create_dir(n.directory.join(format!("{role}.lock"))).unwrap();
            let listener = std::net::TcpListener::bind(n.address()).unwrap();
            listener.set_nonblocking(true).unwrap();
            let result = connect(
                n,
                &ExternalOperation::Open {
                    path: std::env::temp_dir().join("test.md"),
                    line: None,
                },
                || panic!("invalid lock must not launch"),
            );
            assert!(result.is_err(), "{role} failure was bypassed by TCP");
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    fn startup_wait_rechecks_live_before_tcp() {
        let fixture = Fixture::new();
        let n = &fixture.0;
        let startup = InstanceGuard::try_acquire(n, "startup").unwrap().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let operation = ExternalOperation::Open {
                path: std::env::temp_dir().join("test.md"),
                line: None,
            };
            let connecting = connect_with(n, &operation, || panic!("must recheck live"));
            tokio::pin!(connecting);
            // The first live probe creates live.lock; with startup held, the
            // connection future then yields while waiting for arbitration.
            Deadline::after(Duration::from_secs(2))
                .until(async {
                    tokio::select! {
                        biased;
                        _ = &mut connecting => panic!("startup lock was bypassed"),
                        _ = async {
                            while !n.directory.join("live.lock").exists() {
                                tokio::task::yield_now().await;
                            }
                        } => Ok(()),
                    }
                })
                .await
                .unwrap();
            std::fs::remove_file(n.directory.join("live.lock")).unwrap();
            std::fs::create_dir(n.directory.join("live.lock")).unwrap();
            let listener = std::net::TcpListener::bind(n.address()).unwrap();
            listener.set_nonblocking(true).unwrap();
            drop(startup);
            let result = Deadline::after(Duration::from_secs(2))
                .until(connecting)
                .await;
            let error = result
                .err()
                .expect("startup recheck must reject a TCP-only listener");
            assert_ne!(
                error.kind(),
                io::ErrorKind::TimedOut,
                "live recheck must fail immediately"
            );
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            assert!(InstanceGuard::try_acquire(n, "startup").unwrap().is_some());
        });
    }

    #[test]
    fn isolated_new_and_existing_gui_connection_measurements() {
        let fixture = Fixture::new();
        let n = &fixture.0;
        let mut owner = None;
        let start = Instant::now();
        let client = connect(
            n,
            &ExternalOperation::Open {
                path: std::env::temp_dir().join("test.md"),
                line: None,
            },
            || {
                let live = InstanceGuard::claim_gui(n)?.unwrap();
                let server = PipeServer::bind(n)?;
                owner = Some((server, live));
                Ok(())
            },
        )
        .unwrap();
        eprintln!("isolated new GUI-equivalent connect: {:?}", start.elapsed());
        assert!(owner.is_some());
        assert!(gui_is_alive(n).unwrap());
        assert!(InstanceGuard::try_acquire(n, "startup").unwrap().is_some());
        drop(client);
        let start = Instant::now();
        let client = connect(n, &ExternalOperation::Top, || panic!("must not launch")).unwrap();
        eprintln!(
            "isolated existing GUI-equivalent connect: {:?}",
            start.elapsed()
        );
        drop(client);
        drop(owner);
        assert!(!gui_is_alive(n).unwrap());
    }

    struct TestChild(Child);
    impl Drop for TestChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    // Test executable helper, never enabled by production arguments.
    #[test]
    fn isolated_lock_child() {
        let Some(directory) = std::env::var_os("MMV_TEST_INSTANCE_CHILD") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("mmv-test-")
        );
        let n = Namespace { directory, port: 1 };
        if std::env::var_os("MMV_TEST_INSTANCE_IDLE").is_some() {
            std::fs::write(n.directory.join("idle.ready"), b"ready").unwrap();
            thread::sleep(Duration::from_secs(15));
            return;
        }
        let _live = InstanceGuard::claim_gui(&n).unwrap().unwrap();
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn crash_releases_lock_and_child_does_not_inherit_ownership() {
        let fixture = Fixture::new();
        let n = &fixture.0;
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "infra::instance::tests::isolated_lock_child"])
            .env("MMV_TEST_INSTANCE_CHILD", &n.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = TestChild(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !gui_is_alive(n).unwrap() {
            assert!(Instant::now() < deadline, "child did not acquire lock");
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before acquiring"
            );
            thread::sleep(Duration::from_millis(5));
        }
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let parent = InstanceGuard::claim_gui(n).unwrap().unwrap();
        // Spawn while holding the lock; child must not keep it alive after Drop.
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "infra::instance::tests::isolated_lock_child"])
            .env("MMV_TEST_INSTANCE_CHILD", &n.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command.env("MMV_TEST_INSTANCE_IDLE", "1");
        let mut other = TestChild(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !n.directory.join("idle.ready").exists() {
            assert!(Instant::now() < deadline, "idle child did not start");
            assert!(other.0.try_wait().unwrap().is_none());
            thread::sleep(Duration::from_millis(5));
        }
        drop(parent);
        assert!(other.0.try_wait().unwrap().is_none());
        assert!(
            !gui_is_alive(n).unwrap(),
            "live lock inherited by unrelated running child"
        );
        let _ = other.0.kill();
        let _ = other.0.wait();
        assert!(!gui_is_alive(n).unwrap());
    }

    fn process_command(n: &Namespace, mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infra::instance::tests::isolated_startup_process_child",
            ])
            .env("MMV_TEST_STARTUP_DIRECTORY", &n.directory)
            .env("MMV_TEST_STARTUP_PORT", n.port.to_string())
            .env("MMV_TEST_STARTUP_MODE", mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }

    #[test]
    fn isolated_startup_process_child() {
        let Some(directory) = std::env::var_os("MMV_TEST_STARTUP_DIRECTORY") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("mmv-test-")
        );
        let n = Namespace {
            directory,
            port: std::env::var("MMV_TEST_STARTUP_PORT")
                .unwrap()
                .parse()
                .unwrap(),
        };
        let limit = Instant::now() + Duration::from_secs(20);
        if std::env::var("MMV_TEST_STARTUP_MODE").unwrap() == "gui" {
            let _live = InstanceGuard::claim_gui(&n).unwrap().unwrap();
            let _server = PipeServer::bind(&n).unwrap();
            std::fs::write(
                n.directory.join(format!("gui-{}", std::process::id())),
                b"ready",
            )
            .unwrap();
            while !n.directory.join("stop").exists() {
                assert!(
                    Instant::now() < limit,
                    "process fixture did not receive shutdown"
                );
                thread::sleep(Duration::from_millis(5));
            }
        } else {
            let mut owner = None;
            connect(
                &n,
                &ExternalOperation::Open {
                    path: std::env::temp_dir().join("process.md"),
                    line: None,
                },
                || {
                    owner = Some(TestChild(process_command(&n, "gui").spawn()?));
                    Ok(())
                },
            )
            .unwrap();
            std::fs::write(
                n.directory.join(format!("caller-{}", std::process::id())),
                b"connected",
            )
            .unwrap();
            if let Some(mut child) = owner {
                loop {
                    if let Some(status) = child.0.try_wait().unwrap() {
                        assert!(status.success());
                        break;
                    }
                    assert!(Instant::now() < limit, "spawned GUI fixture did not exit");
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn independent_cli_processes_start_only_one_service_process() {
        let fixture = Fixture::new();
        let n = &fixture.0;
        drop(lock::open(n, "live").unwrap());
        let mut children = Vec::new();
        // Declare after children so shutdown is signalled before kill/wait during unwind.
        struct Stop(std::path::PathBuf);
        impl Drop for Stop {
            fn drop(&mut self) {
                let _ = std::fs::write(self.0.join("stop"), b"stop");
            }
        }
        let stop = Stop(n.directory.clone());
        for _ in 0..4 {
            children.push(TestChild(process_command(n, "caller").spawn().unwrap()));
        }
        let limit = Instant::now() + Duration::from_secs(10);
        loop {
            let names: Vec<_> = std::fs::read_dir(&n.directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            if names
                .iter()
                .filter(|name| name.starts_with("caller-"))
                .count()
                == 4
            {
                assert_eq!(
                    names.iter().filter(|name| name.starts_with("gui-")).count(),
                    1
                );
                break;
            }
            assert!(Instant::now() < limit, "CLI processes did not converge");
            thread::sleep(Duration::from_millis(5));
        }
        drop(stop);
        for child in &mut children {
            loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                assert!(Instant::now() < limit, "child did not stop");
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(!gui_is_alive(n).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn gui_command_is_current_executable_only() {
        let (executable, command) = gui_command().unwrap();
        let expected = std::env::current_exe().unwrap();
        assert_eq!(
            executable,
            expected
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            String::from_utf16(&command[..command.len() - 1]).unwrap(),
            format!("\"{}\" {INTERNAL_GUI_ARGUMENT}", expected.display())
        );
    }
}
