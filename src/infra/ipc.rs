//! Current-user local Named Pipes. No app/UI state is accessed here.
mod io;
mod protocol;
#[cfg(test)]
mod tests;

use super::{
    deadline::Deadline,
    security::{Handle, Namespace, UserSecurity, invalid},
};
use io::Stop;
pub use protocol::{Request, Response};
use std::{
    io as stdio, ptr,
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY, GENERIC_READ,
        GENERIC_WRITE,
    },
    Security::Authorization::SE_FILE_OBJECT,
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, READ_CONTROL, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    },
    System::Pipes::{
        CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    },
};

/// Consumed on send; a late completion after disconnect simply returns an error.
/// The sender does not retain any kernel handle or connection worker.
pub struct Reply {
    id: crate::app::ExternalRequestId,
    sender: mpsc::SyncSender<Response>,
    alive: Arc<std::sync::atomic::AtomicBool>,
}
impl Reply {
    pub fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn send(self, response: Response) -> stdio::Result<()> {
        if response.id != self.id {
            return Err(invalid("reply request ID mismatch"));
        }
        self.sender
            .try_send(response)
            .map_err(|_| stdio::Error::new(stdio::ErrorKind::BrokenPipe, "reply receiver gone"))
    }
}
pub struct Incoming {
    pub request: Request,
    pub reply: Reply,
}

pub struct PipeServer {
    pub incoming: mpsc::Receiver<stdio::Result<Incoming>>,
    stop: Arc<Stop>,
    worker: Option<JoinHandle<()>>,
    #[cfg(test)]
    active: Arc<std::sync::atomic::AtomicUsize>,
}
impl PipeServer {
    /// Starts a dedicated accept worker; all Pipe I/O and completion waits stay
    /// outside the caller/UI thread. Queue and concurrent workers are bounded.
    pub fn bind(namespace: &Namespace) -> stdio::Result<Self> {
        let first = listener(namespace, true)?;
        let namespace = namespace.clone();
        let stop = Arc::new(Stop::new()?);
        let worker_stop = stop.clone();
        let (sender, incoming) = mpsc::sync_channel(128);
        #[cfg(test)]
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        #[cfg(test)]
        let worker_active = active.clone();
        let worker = thread::Builder::new()
            .name("mmv-pipe-accept".into())
            .spawn(move || {
                let mut next = first;
                let mut connections: Vec<JoinHandle<()>> = Vec::new();
                while !worker_stop.signalled() {
                    // Reap completed tasks rather than retaining per-client resources.
                    let mut index = 0;
                    while index < connections.len() {
                        if connections[index].is_finished() {
                            let _ = connections.swap_remove(index).join();
                        } else {
                            index += 1;
                        }
                    }
                    match accept_next(&next, &worker_stop, Deadline::startup(Instant::now())) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(_) if worker_stop.signalled() => break,
                        Err(error) => {
                            let _ = sender.try_send(Err(error));
                            break;
                        }
                    }
                    let pipe = next;
                    // Create the next accept instance before dispatching this client.
                    // A silent/slow client cannot hold up the entire accept loop.
                    next = match listener(&namespace, false) {
                        Ok(next) => next,
                        Err(error) => {
                            let _ = sender.try_send(Err(error));
                            break;
                        }
                    };
                    if connections.len() >= 128 {
                        drop(pipe);
                        continue;
                    }
                    let stop = worker_stop.clone();
                    let client_sender = sender.clone();
                    #[cfg(test)]
                    let active = worker_active.clone();
                    match thread::Builder::new()
                        .name("mmv-pipe-client".into())
                        .spawn(move || {
                            #[cfg(test)]
                            active.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let _ = serve(pipe, &stop, &client_sender);
                            #[cfg(test)]
                            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        }) {
                        Ok(worker) => connections.push(worker),
                        Err(error) => {
                            let _ = sender.try_send(Err(error));
                            break;
                        }
                    }
                }
                worker_stop.signal();
                for connection in connections {
                    let _ = connection.join();
                }
            })?;
        Ok(Self {
            incoming,
            stop,
            worker: Some(worker),
            #[cfg(test)]
            active,
        })
    }
    /// Nonblocking cancellation; move the server off the UI thread before joining.
    pub fn request_shutdown(&self) {
        self.stop.signal();
    }
    /// Cancels outstanding I/O, drains final completions, and joins every worker.
    pub fn shutdown(&mut self) {
        self.request_shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for PipeServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// False means a local client departed, or an idle accept deadline elapsed.
/// Reset the instance before rearming, including cancellation/connect races.
fn accept_next(pipe: &Handle, stop: &Stop, deadline: Deadline) -> stdio::Result<bool> {
    match io::accept(pipe, stop, deadline) {
        Ok(()) => Ok(true),
        Err(error)
            if error.kind() == stdio::ErrorKind::TimedOut
                || matches!(error.raw_os_error(), Some(code) if code == ERROR_NO_DATA as i32 || code == ERROR_BROKEN_PIPE as i32) =>
        {
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn listener(namespace: &Namespace, first: bool) -> stdio::Result<Handle> {
    let security = UserSecurity::current()?;
    let attributes = security.attributes();
    let pipe = Handle::new(unsafe {
        CreateNamedPipeW(
            namespace.pipe().as_ptr(),
            PIPE_ACCESS_DUPLEX
                | FILE_FLAG_OVERLAPPED
                | if first {
                    FILE_FLAG_FIRST_PIPE_INSTANCE
                } else {
                    0
                },
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            255,
            65536,
            65536,
            0,
            &attributes,
        )
    })?;
    security.verify(pipe.0, SE_FILE_OBJECT)?;
    Ok(pipe)
}
fn serve(
    pipe: Handle,
    stop: &Stop,
    incoming: &mpsc::SyncSender<stdio::Result<Incoming>>,
) -> stdio::Result<()> {
    let request = Request::decode(&io::read_frame(
        &pipe,
        stop,
        Deadline::connection(Instant::now()),
    )?)?;
    io::idle_peer(&pipe)?;
    let id = request.id.clone();
    let deadline = Deadline::response(Instant::now());
    let (sender, response) = mpsc::sync_channel(1);
    struct Alive(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::Release);
        }
    }
    // Covers every return path: disconnect, timeout, shutdown and normal receipt.
    let alive = Alive(Arc::new(std::sync::atomic::AtomicBool::new(true)));
    incoming
        .try_send(Ok(Incoming {
            request,
            reply: Reply {
                id: id.clone(),
                sender,
                alive: alive.0.clone(),
            },
        }))
        .map_err(|_| {
            stdio::Error::new(
                stdio::ErrorKind::WouldBlock,
                "request dispatcher unavailable",
            )
        })?;
    let response = loop {
        io::idle_peer(&pipe)?;
        match response.try_recv() {
            Ok(response) => break response,
            Err(mpsc::TryRecvError::Disconnected) => return Err(invalid("reply abandoned")),
            Err(mpsc::TryRecvError::Empty) => stop.pause(deadline)?,
        }
    };
    if response.id != id {
        return Err(invalid("response ID mismatch"));
    }
    io::write_all(&pipe, &mut response.encode()?, stop, deadline)?;
    // Never FlushFileBuffers: it can block indefinitely on an unreading client.
    // Keep the server handle open only until the bounded client receipt arrives.
    let mut receipt = [0xff];
    io::read_exact(&pipe, &mut receipt, stop, deadline)?;
    if receipt != [0] {
        return Err(invalid("invalid response receipt"));
    }
    Ok(())
}

pub struct PipeClient {
    pipe: Handle,
    stop: Stop,
}
impl PipeClient {
    /// Immediate local connection attempt. Only missing/busy endpoints are retryable;
    /// access/security failures must never be interpreted as absence of a GUI.
    pub(super) fn try_connect(namespace: &Namespace) -> stdio::Result<Self> {
        let pipe = Handle::new(unsafe {
            CreateFileW(
                namespace.pipe().as_ptr(),
                GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                ptr::null_mut(),
            )
        })?;
        UserSecurity::current()?.verify(pipe.0, SE_FILE_OBJECT)?;
        Ok(Self {
            pipe,
            stop: Stop::new()?,
        })
    }
    pub fn connect(namespace: &Namespace, deadline: Deadline) -> stdio::Result<Self> {
        let stop = Stop::new()?;
        loop {
            if deadline.expired(Instant::now()) {
                return Err(io::timeout());
            }
            match Self::try_connect(namespace) {
                Ok(client) => return Ok(client),
                Err(error) if retryable(&error) => stop.pause(deadline)?,
                Err(error) => return Err(error),
            }
        }
    }
    /// Consumes the connection: timeout/disconnect never retries a sent operation.
    /// The response deadline also bounds writes and non-reading operation replies.
    pub fn request(self, request: Request, deadline: Deadline) -> stdio::Result<Response> {
        let mut encoded = request.encode()?;
        io::write_all(&self.pipe, &mut encoded, &self.stop, deadline)?;
        let response = Response::decode(&io::read_frame(&self.pipe, &self.stop, deadline)?)?;
        if response.id != request.id {
            return Err(invalid("response request ID mismatch"));
        }
        io::write_all(&self.pipe, &mut [0], &self.stop, deadline)?;
        Ok(response)
    }
}
pub(super) fn retryable(error: &stdio::Error) -> bool {
    matches!(error.raw_os_error(), Some(code) if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PIPE_BUSY as i32)
}
