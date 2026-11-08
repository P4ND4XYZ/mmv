//! Disposable Windows feasibility probe, not a supported CLI or IPC protocol.
//! Run via scripts/probe-phase-one.ps1. No production modules are used.
#[cfg(windows)]
mod probe {
    use std::{
        ffi::c_void,
        fs::{File, OpenOptions},
        io::{BufRead, BufReader, Write},
        os::windows::io::FromRawHandle,
        process::{Command, Stdio},
        sync::mpsc,
        time::{Duration, Instant},
    };

    use gpui_kit::{AppContext as _, WindowOptions, assets::Assets};

    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMutexW(attributes: Handle, owner: i32, name: *const u16) -> Handle;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn ReleaseMutex(handle: Handle) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateNamedPipeW(
            name: *const u16,
            access: u32,
            mode: u32,
            instances: u32,
            out_size: u32,
            in_size: u32,
            timeout: u32,
            security: Handle,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: Handle) -> i32;
        fn GetLastError() -> u32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    struct Lock(Handle);
    impl Lock {
        fn acquire(name: &str, timeout: u32) -> Result<Self, String> {
            // Default DACL and a per-run random namespace are used only for this
            // disposable probe. Product access control is intentionally not implemented.
            let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 0, wide(name).as_ptr()) };
            if handle.is_null() {
                return Err("CreateMutexW failed".into());
            }
            let result = unsafe { WaitForSingleObject(handle, timeout) };
            if result == 0 || result == 0x80 {
                Ok(Self(handle))
            } else {
                unsafe {
                    CloseHandle(handle);
                }
                Err("mutex owned or timed out".into())
            }
        }
    }
    impl Drop for Lock {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }

    fn connect(name: &str) -> std::io::Result<File> {
        OpenOptions::new().read(true).write(true).open(name)
    }

    fn cli(token: &str, expected: &str, id: &str, path: &str) -> Result<(), String> {
        let pipe = format!(r"\\.\pipe\mmv-phase-one-{token}");
        let startup = Lock::acquire(&format!(r"Local\mmv-phase-one-start-{token}"), 10000)?;
        let mut stream = match connect(&pipe) {
            Ok(stream) => stream,
            Err(_) => {
                if let Ok(live) = Lock::acquire(&format!(r"Local\mmv-phase-one-live-{token}"), 0) {
                    drop(live);
                    Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                        .args(["gui", token, expected])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .map_err(|e| e.to_string())?;
                }
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if let Ok(stream) = connect(&pipe) {
                        break stream;
                    }
                    if Instant::now() >= deadline {
                        return Err("GUI pipe startup timeout".into());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        // Serialize only startup/connect. Replies belong to each connection.
        drop(startup);
        writeln!(stream, "{id}|{path}").map_err(|e| e.to_string())?;
        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .map_err(|e| e.to_string())?;
        if !response.starts_with(&format!("{id}|")) {
            return Err("wrong response ID".into());
        }
        print!("{response}");
        Ok(())
    }

    enum Event {
        Loaded {
            id: String,
            content: Result<String, String>,
            reply: mpsc::Sender<String>,
        },
        Finished,
    }

    fn gui(token: &str, expected: usize) -> Result<(), String> {
        let _live = Lock::acquire(&format!(r"Local\mmv-phase-one-live-{token}"), 0)?;
        let pipe = format!(r"\\.\pipe\mmv-phase-one-{token}");
        let (tx, rx) = mpsc::channel();
        // Blocking Pipe and file I/O stay off the UI thread.
        let server = std::thread::spawn(move || {
            for _ in 0..expected {
                let raw = unsafe {
                    CreateNamedPipeW(
                        wide(&pipe).as_ptr(),
                        3,
                        0,
                        1,
                        4096,
                        4096,
                        0,
                        std::ptr::null_mut(),
                    )
                };
                assert_ne!(raw as isize, -1);
                let mut stream = unsafe { File::from_raw_handle(raw) };
                let connected = unsafe { ConnectNamedPipe(raw, std::ptr::null_mut()) };
                assert!(connected != 0 || unsafe { GetLastError() } == 535);
                let mut request = String::new();
                BufReader::new(&mut stream).read_line(&mut request).unwrap();
                let (id, path) = request.trim_end().split_once('|').unwrap();
                let content = std::fs::read_to_string(path).map_err(|e| e.to_string());
                let (reply, result) = mpsc::channel();
                tx.send(Event::Loaded {
                    id: id.into(),
                    content,
                    reply,
                })
                .unwrap();
                let response = result.recv_timeout(Duration::from_secs(10)).unwrap();
                writeln!(stream, "{response}").unwrap();
                // Keep the instance alive until the client consumed its response.
                let mut ack = String::new();
                let _ = BufReader::new(&mut stream).read_line(&mut ack);
            }
            tx.send(Event::Finished).unwrap();
        });
        gpui_kit::application().with_assets(Assets).run(move |cx| {
            gpui_kit::init(cx);
            let (_, text) = gpui_kit::open_window(
                WindowOptions {
                    focus: false,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("MMV isolated phase-one probe");
                    cx.new(|cx| {
                        gpui_kit::component::text::TextViewState::markdown("Probe waiting", cx)
                    })
                },
            )
            .expect("probe window");
            cx.spawn(async move |cx| {
                loop {
                    while let Ok(event) = rx.try_recv() {
                        match event {
                            Event::Loaded { id, content, reply } => {
                                let ok = content.is_ok();
                                cx.update(|cx| {
                                    if let Ok(content) = content {
                                        text.update(cx, |text, cx| text.set_text(&content, cx));
                                    }
                                    reply
                                        .send(format!(
                                            "{id}|{}|{}|{}",
                                            if ok { "ok" } else { "error" },
                                            std::process::id(),
                                            cx.windows().len()
                                        ))
                                        .unwrap();
                                });
                            }
                            Event::Finished => {
                                // Give the external observer time to count the real window.
                                cx.background_executor()
                                    .timer(Duration::from_millis(500))
                                    .await;
                                cx.update(|cx| cx.quit());
                                return;
                            }
                        }
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(10))
                        .await;
                }
            })
            .detach();
            // Deliberately do not call activate or change the external foreground window.
        });
        server.join().map_err(|_| "pipe server failed".to_owned())?;
        Ok(())
    }

    pub fn run() -> Result<(), String> {
        let args = std::env::args().collect::<Vec<_>>();
        match args.get(1).map(String::as_str) {
            Some("gui") if args.len() == 4 => {
                gui(&args[2], args[3].parse().map_err(|_| "expected count")?)
            }
            Some("open") if args.len() == 6 => cli(&args[2], &args[3], &args[4], &args[5]),
            _ => Err("use probe-phase-one.ps1".into()),
        }
    }
}

fn main() {
    #[cfg(windows)]
    if let Err(error) = probe::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    eprintln!("Windows only");
}
