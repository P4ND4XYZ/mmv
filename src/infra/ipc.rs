//! Current-user local Named Pipes. No app/UI state is accessed here.
mod io;
mod protocol;
mod service;
#[cfg(test)]
mod tests;

use super::{
    deadline::Deadline,
    security::{Handle, Namespace, UserSecurity, invalid},
};
pub use protocol::{Request, Response};
use std::{
    io as stdio, ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc,
    },
    thread::{self, JoinHandle},
    time::Instant,
};
use tokio::{
    net::windows::named_pipe::{NamedPipeClient, NamedPipeServer},
    sync::{Notify, mpsc, oneshot, watch},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE},
    Security::Authorization::SE_FILE_OBJECT,
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, READ_CONTROL, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    },
    System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    },
};

const CAPACITY: usize = 128;
/// Transport identity, never the externally supplied request ID/document generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectionId(u64);
struct Peer {
    id: ConnectionId,
    alive: AtomicBool,
    disconnected: Arc<Notify>,
}
impl Drop for PeerLifetime {
    fn drop(&mut self) {
        // Publish state BEFORE waking the UI. Notify coalesces all departures into
        // one permit, independently of the bounded request queue's capacity.
        self.0.alive.store(false, Ordering::Release);
        self.0.disconnected.notify_one();
    }
}
struct PeerLifetime(Arc<Peer>);

/// Consumed on send; retains neither a kernel handle nor the connection task.
pub struct Reply {
    id: crate::app::ExternalRequestId,
    sender: oneshot::Sender<Response>,
    peer: Arc<Peer>,
}
impl Reply {
    #[cfg(test)]
    pub fn connection_id(&self) -> ConnectionId {
        self.peer.id
    }
    pub fn is_alive(&self) -> bool {
        self.peer.alive.load(Ordering::Acquire)
    }
    pub fn send(self, response: Response) -> stdio::Result<()> {
        if response.id != self.id {
            return Err(invalid("reply request ID mismatch"));
        }
        if !self.is_alive() {
            return Err(stdio::Error::new(
                stdio::ErrorKind::BrokenPipe,
                format!("reply receiver gone (connection {})", self.peer.id.0),
            ));
        }
        self.sender
            .send(response)
            .map_err(|_| stdio::Error::new(stdio::ErrorKind::BrokenPipe, "reply receiver gone"))
    }
}
pub struct Incoming {
    pub request: Request,
    pub reply: Reply,
}
type Inbox = mpsc::Receiver<stdio::Result<Incoming>>;

pub struct PipeServer {
    incoming: Option<Inbox>,
    disconnected: Arc<Notify>,
    stop: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}
impl PipeServer {
    /// Ready means the dedicated current_thread runtime AND the first secure
    /// listener exist. Runtime, thread, ACL and Pipe initialization failures return
    /// here rather than appearing as a successfully bound but unreachable server.
    pub fn bind(namespace: &Namespace) -> stdio::Result<Self> {
        let namespace = namespace.clone();
        let (sender, incoming) = mpsc::channel(CAPACITY);
        let (stop, stopped) = watch::channel(false);
        let disconnected = Arc::new(Notify::new());
        let worker_disconnected = disconnected.clone();
        let (ready, ready_rx) = std_mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("mmv-pipe-runtime".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let first = match listener(&namespace, true) {
                        Ok(first) => first,
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };
                    if ready.send(Ok(())).is_err() {
                        return;
                    }
                    service::run(namespace, first, sender, stopped, worker_disconnected).await;
                });
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                incoming: Some(incoming),
                disconnected,
                stop,
                worker: Some(worker),
            }),
            result => {
                let _ = worker.join();
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => stdio::Error::other("IPC runtime exited before readiness"),
                })
            }
        }
    }
    pub fn take_incoming(&mut self) -> Option<Inbox> {
        self.incoming.take()
    }
    pub fn disconnect_notifications(&self) -> Arc<Notify> {
        self.disconnected.clone()
    }
    /// Nonblocking. The owner must move joining off the UI executor.
    pub fn request_shutdown(&self) {
        self.stop.send_replace(true);
    }
    /// Stops acceptance, cancels/reaps all connection tasks, ends runtime, joins.
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

fn listener(namespace: &Namespace, first: bool) -> stdio::Result<io::Pipe<NamedPipeServer>> {
    let security = UserSecurity::current()?;
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
            &security.attributes(),
        )
    })?;
    security.verify(pipe.0, SE_FILE_OBJECT)?;
    // Sole ownership transfers even on registration error. The runtime is entered.
    unsafe { NamedPipeServer::from_raw_handle(pipe.into_raw()).map(io::Pipe) }
}

async fn serve(
    mut pipe: io::Pipe<NamedPipeServer>,
    incoming: mpsc::Sender<stdio::Result<Incoming>>,
    peer: Arc<Peer>,
) -> stdio::Result<()> {
    let _lifetime = PeerLifetime(peer.clone());
    let request =
        Request::decode(&io::read_frame(&mut pipe, Deadline::connection(Instant::now())).await?)?;
    io::idle_peer(&pipe)?;
    let id = request.id.clone();
    let deadline = Deadline::response(Instant::now());
    let (sender, response) = oneshot::channel();
    incoming
        .try_send(Ok(Incoming {
            request,
            reply: Reply {
                id: id.clone(),
                sender,
                peer,
            },
        }))
        .map_err(|_| {
            stdio::Error::new(
                stdio::ErrorKind::WouldBlock,
                "request dispatcher unavailable",
            )
        })?;
    deadline
        .until(async {
            let response = tokio::select! {
                // A ready disconnect/extra byte takes precedence over app completion.
                biased;
                error = io::peer_departed(&pipe) => return Err(error),
                response = response => response.map_err(|_| invalid("reply abandoned"))?,
            };
            if response.id != id {
                return Err(invalid("response ID mismatch"));
            }
            io::write_all(&mut pipe, &response.encode()?, deadline).await?;
            // Tokio/Mio buffers writes; never drop the handle before this receipt.
            // No FlushFileBuffers, and no wait beyond the same response deadline.
            let mut receipt = [0xff];
            io::read_exact(&mut pipe, &mut receipt, deadline).await?;
            if receipt != [0] {
                return Err(invalid("invalid response receipt"));
            }
            Ok(())
        })
        .await
}

pub struct PipeClient {
    pipe: io::Pipe<NamedPipeClient>,
}
impl PipeClient {
    /// Immediate attempt; only missing/busy endpoints are retryable. Creating and
    /// verifying with Win32 preserves READ_CONTROL and identification-only SQOS.
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
        unsafe {
            NamedPipeClient::from_raw_handle(pipe.into_raw()).map(|pipe| Self {
                pipe: io::Pipe(pipe),
            })
        }
    }
    pub async fn connect(namespace: &Namespace, deadline: Deadline) -> stdio::Result<Self> {
        loop {
            if deadline.expired(Instant::now()) {
                return Err(Deadline::timeout());
            }
            match Self::try_connect(namespace) {
                Ok(client) => return Ok(client),
                Err(error) if retryable(&error) => deadline.pause().await?,
                Err(error) => return Err(error),
            }
        }
    }
    /// Consumes connection; partial frame timeout closes it, never retries it.
    pub async fn request(
        mut self,
        request: Request,
        deadline: Deadline,
    ) -> stdio::Result<Response> {
        deadline
            .until(async {
                io::write_all(&mut self.pipe, &request.encode()?, deadline).await?;
                let response = Response::decode(&io::read_frame(&mut self.pipe, deadline).await?)?;
                if response.id != request.id {
                    return Err(invalid("response request ID mismatch"));
                }
                io::write_all(&mut self.pipe, &[0], deadline).await?;
                // Mio may still own the receipt write. Wait for the server to read it
                // and close, so teardown cancellation cannot discard the receipt.
                match io::read_exact(&mut self.pipe, &mut [0], deadline).await {
                    Err(error)
                        if matches!(
                            error.kind(),
                            stdio::ErrorKind::UnexpectedEof | stdio::ErrorKind::BrokenPipe
                        ) =>
                    {
                        Ok(response)
                    }
                    Err(error) => Err(error),
                    Ok(()) => Err(invalid("unexpected data after response receipt")),
                }
            })
            .await
    }
}
pub(super) fn retryable(error: &stdio::Error) -> bool {
    matches!(error.raw_os_error(), Some(code) if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PIPE_BUSY as i32)
}
