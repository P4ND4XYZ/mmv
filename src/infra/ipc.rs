//! Loopback TCP transport (not authenticated). No app/UI state is accessed here.
//! PipeServer/PipeClient names remain internal compatibility names for GUI wiring.
mod io;
mod protocol;
mod service;
#[cfg(test)]
mod tests;

use super::{deadline::Deadline, paths::Namespace};
fn invalid(message: &str) -> stdio::Error {
    stdio::Error::new(stdio::ErrorKind::InvalidData, message)
}
pub use protocol::{Request, Response};
use std::{
    io as stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc,
    },
    thread::{self, JoinHandle},
    time::Instant,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Notify, mpsc, oneshot, watch},
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
    /// Success means the runtime and exact configured TCP listener are ready.
    pub fn bind(namespace: &Namespace) -> stdio::Result<Self> {
        let namespace = namespace.clone();
        let (sender, incoming) = mpsc::channel(CAPACITY);
        let (stop, stopped) = watch::channel(false);
        let disconnected = Arc::new(Notify::new());
        let worker_disconnected = disconnected.clone();
        let (ready, ready_rx) = std_mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("mmv-tcp-runtime".into())
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
                    let first = match TcpListener::bind(namespace.address()).await {
                        Ok(first) => first,
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };
                    if ready.send(Ok(())).is_err() {
                        return;
                    }
                    service::run(first, sender, stopped, worker_disconnected).await;
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

async fn serve(
    mut pipe: TcpStream,
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
            Ok(())
        })
        .await
}

pub struct PipeClient {
    pipe: TcpStream,
}
impl PipeClient {
    pub(super) async fn try_connect(namespace: &Namespace) -> stdio::Result<Self> {
        Ok(Self {
            pipe: TcpStream::connect(namespace.address()).await?,
        })
    }
    pub async fn connect(namespace: &Namespace, deadline: Deadline) -> stdio::Result<Self> {
        loop {
            match deadline.until(Self::try_connect(namespace)).await {
                Ok(client) => return Ok(client),
                Err(error) if retryable(&error) => deadline.pause().await?,
                Err(error) => return Err(error),
            }
        }
    }
    /// Consumes the connection. A sent request is never retried, including when
    /// its outcome is lost. No Pipe-specific receipt or disconnect wait on TCP.
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
                Ok(response)
            })
            .await
    }
}
pub(super) fn retryable(error: &stdio::Error) -> bool {
    matches!(
        error.kind(),
        stdio::ErrorKind::ConnectionRefused
            | stdio::ErrorKind::ConnectionReset
            | stdio::ErrorKind::ConnectionAborted
            | stdio::ErrorKind::NotConnected
    )
}
