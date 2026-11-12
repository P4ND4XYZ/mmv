//! GUI lifetime ownership and CLI startup arbitration are separate mutexes.
use super::{
    deadline::Deadline,
    ipc::{PipeClient, retryable},
    security::{Handle, Namespace, UserSecurity},
};
use crate::app::ExternalOperation;
use std::{io, marker::PhantomData, os::windows::ffi::OsStrExt as _, rc::Rc, time::Instant};
use windows_sys::Win32::{
    Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Security::Authorization::SE_KERNEL_OBJECT,
    System::Threading::{
        CREATE_NO_WINDOW, CreateMutexW, CreateProcessW, PROCESS_INFORMATION, ReleaseMutex,
        STARTUPINFOW, WaitForSingleObject,
    },
};

/// Must be dropped on the acquiring thread. Abandonment is recovered by the next
/// caller; no process is terminated and no persistent lock file is used.
pub struct InstanceGuard {
    handle: Handle,
    _thread: PhantomData<Rc<()>>,
}
impl InstanceGuard {
    fn try_acquire(namespace: &Namespace, role: &str) -> io::Result<Option<Self>> {
        let security = UserSecurity::current()?;
        let attributes = security.attributes();
        let handle =
            Handle::new(unsafe { CreateMutexW(&attributes, 0, namespace.mutex(role).as_ptr()) })?;
        // CreateMutex also opens pre-existing objects: never trust those silently.
        security.verify(handle.0, SE_KERNEL_OBJECT)?;
        match unsafe { WaitForSingleObject(handle.0, 0) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Some(Self {
                handle,
                _thread: PhantomData,
            })),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }
    /// The internal GUI entry point must retain this guard for its full lifetime,
    /// and refuse to create a window if another GUI already owns it.
    pub fn claim_gui(namespace: &Namespace) -> io::Result<Option<Self>> {
        Self::try_acquire(namespace, "live")
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle.0);
        }
    }
}
/// Called by CLI threads, not recursively from the GUI lifetime-owner thread.
pub fn gui_is_alive(namespace: &Namespace) -> io::Result<bool> {
    Ok(InstanceGuard::claim_gui(namespace)?.is_none())
}

pub const INTERNAL_GUI_ARGUMENT: &str = "--mmv-internal-gui";
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

pub async fn connect_or_start(
    namespace: &Namespace,
    operation: &ExternalOperation,
) -> io::Result<PipeClient> {
    connect_with(namespace, operation, spawn_gui).await
}
// Run exactly one negotiation at a time on this current_thread runtime: Windows
// mutex ownership is thread-owned (including across awaits), not task-owned.
async fn connect_with(
    namespace: &Namespace,
    operation: &ExternalOperation,
    spawn: impl FnOnce() -> io::Result<()>,
) -> io::Result<PipeClient> {
    let start = Instant::now();
    match PipeClient::try_connect(namespace) {
        Ok(client) => return Ok(client),
        Err(error) if retryable(&error) => {}
        Err(error) => return Err(error),
    }
    if gui_is_alive(namespace)? {
        return PipeClient::connect(namespace, Deadline::connection(start)).await;
    }
    if !matches!(operation, ExternalOperation::Open(_)) {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "GUI is not running",
        ));
    }
    let deadline = Deadline::startup(start);
    let startup = loop {
        if deadline.expired(Instant::now()) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "GUI startup deadline exceeded",
            ));
        }
        if let Some(guard) = InstanceGuard::try_acquire(namespace, "startup")? {
            break guard;
        }
        deadline.pause().await?;
    };
    // Another CLI may have started a GUI while we waited for arbitration.
    // Recheck both its ownership and endpoint before exercising the startup right.
    match PipeClient::try_connect(namespace) {
        Ok(client) => return Ok(client),
        Err(error) if retryable(&error) => {}
        Err(error) => return Err(error),
    }
    if !gui_is_alive(namespace)? {
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
        os::windows::process::CommandExt,
        process::{Child, Command, Stdio},
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    fn connect_with(
        namespace: &Namespace,
        operation: &ExternalOperation,
        spawn: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<PipeClient> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(super::connect_with(namespace, operation, spawn))
    }

    #[test]
    fn real_mutex_lifetime_is_thread_owned_and_released() {
        let namespace = Namespace::isolated();
        let (ready, ready_rx) = mpsc::channel();
        let (done, done_rx) = mpsc::channel();
        let other = namespace.clone();
        let owner = thread::spawn(move || {
            let guard = InstanceGuard::claim_gui(&other).unwrap().unwrap();
            ready.send(()).unwrap();
            done_rx.recv().unwrap();
            drop(guard);
        });
        ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(gui_is_alive(&namespace).unwrap());
        assert!(InstanceGuard::claim_gui(&namespace).unwrap().is_none());
        done.send(()).unwrap();
        owner.join().unwrap();
        assert!(!gui_is_alive(&namespace).unwrap());
    }

    #[test]
    fn simultaneous_open_converges_under_real_startup_mutex_and_rechecks_liveness() {
        let namespace = Namespace::isolated();
        let barrier = Arc::new(Barrier::new(5));
        let spawned = Arc::new(AtomicUsize::new(0));
        let (finish, finish_rx) = mpsc::channel();
        let finish_rx = Arc::new(std::sync::Mutex::new(finish_rx));
        let (owner_tx, owner_rx) = mpsc::channel();
        let mut callers = Vec::new();
        for _ in 0..4 {
            let namespace = namespace.clone();
            let barrier = barrier.clone();
            let spawned = spawned.clone();
            let owner_tx = owner_tx.clone();
            let finish_rx = finish_rx.clone();
            callers.push(thread::spawn(move || {
                barrier.wait();
                connect_with(
                    &namespace,
                    &ExternalOperation::Open(r"C:\test.md".into()),
                    || {
                        spawned.fetch_add(1, Ordering::SeqCst);
                        let (ready, rx) = mpsc::channel();
                        let other = namespace.clone();
                        let owner = thread::spawn(move || {
                            let _live = InstanceGuard::claim_gui(&other).unwrap().unwrap();
                            let _server = PipeServer::bind(&other).unwrap();
                            ready.send(()).unwrap();
                            finish_rx.lock().unwrap().recv().unwrap();
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
    fn live_but_unconnectable_gui_never_spawns_another_process() {
        let namespace = Namespace::isolated();
        let other = namespace.clone();
        let (ready, ready_rx) = mpsc::channel();
        let (finish, finish_rx) = mpsc::channel();
        let owner = thread::spawn(move || {
            let _live = InstanceGuard::claim_gui(&other).unwrap().unwrap();
            ready.send(()).unwrap();
            finish_rx.recv().unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let start = Instant::now();
        let error = connect_with(
            &namespace,
            &ExternalOperation::Open(r"C:\test.md".into()),
            || panic!("live GUI must not spawn"),
        )
        .err()
        .unwrap();
        finish.send(()).unwrap();
        owner.join().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() >= Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_secs(8));
    }

    // Invoked in a separate test executable process only with an isolated name.
    #[test]
    fn isolated_mutex_child() {
        let Ok(stem) = std::env::var("MMV_TEST_INSTANCE_CHILD") else {
            return;
        };
        assert!(stem.starts_with("mmv-test-"));
        let namespace = Namespace { stem };
        let _live = InstanceGuard::claim_gui(&namespace).unwrap().unwrap();
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn child_exit_releases_real_mutex_and_abandoned_ownership_is_recoverable() {
        struct TestChild(Child);
        impl Drop for TestChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let namespace = Namespace::isolated();
        let mut child = TestChild(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "infra::instance::tests::isolated_mutex_child"])
                .env("MMV_TEST_INSTANCE_CHILD", &namespace.stem)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !gui_is_alive(&namespace).unwrap() {
            assert!(
                Instant::now() < deadline,
                "child did not acquire lifetime mutex"
            );
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before acquiring"
            );
            thread::sleep(Duration::from_millis(5));
        }
        // Keep the named mutex object alive even after its owning process exits.
        let user = UserSecurity::current().unwrap();
        let retained = Handle::new(unsafe {
            CreateMutexW(&user.attributes(), 0, namespace.mutex("live").as_ptr())
        })
        .unwrap();
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(retained.0, 0) },
            WAIT_ABANDONED
        );
        assert_ne!(unsafe { ReleaseMutex(retained.0) }, 0);
        let recovered = InstanceGuard::claim_gui(&namespace).unwrap().unwrap();
        drop(recovered);
        assert!(!gui_is_alive(&namespace).unwrap());
    }

    #[test]
    fn missing_gui_navigation_never_spawns_and_command_is_current_executable_only() {
        let namespace = Namespace::isolated();
        let error = connect_with(&namespace, &ExternalOperation::Back, || {
            panic!("must not launch")
        })
        .err()
        .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::NotConnected);
        let (executable, command) = gui_command().unwrap();
        let expected = std::env::current_exe().unwrap();
        let units: Vec<u16> = expected.as_os_str().encode_wide().chain(Some(0)).collect();
        assert_eq!(executable, units);
        assert_eq!(
            String::from_utf16(&command[..command.len() - 1]).unwrap(),
            format!("\"{}\" {INTERNAL_GUI_ARGUMENT}", expected.display())
        );
    }
}
