//! The sole IPC runtime thread. Tasks/channels are bounded independently.
use super::*;
use tokio::{net::windows::named_pipe::NamedPipeServer, task::JoinSet};
use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_NO_DATA};

pub(super) async fn run(
    namespace: Namespace,
    mut next: io::Pipe<NamedPipeServer>,
    sender: mpsc::Sender<stdio::Result<Incoming>>,
    mut stopped: watch::Receiver<bool>,
    disconnected: Arc<Notify>,
    #[cfg(test)] active: Arc<std::sync::atomic::AtomicUsize>,
) {
    let mut tasks = JoinSet::new();
    let mut next_id = 0u64;
    loop {
        tokio::select! {
            biased;
            _ = stopped.changed() => break,
            _ = tasks.join_next(), if !tasks.is_empty() => {},
            connected = next.connect() => {
                match connected {
                    Ok(()) => {},
                    Err(error) if matches!(error.raw_os_error(), Some(code) if code == ERROR_NO_DATA as i32 || code == ERROR_BROKEN_PIPE as i32) => {
                        // A client may close before ConnectNamedPipe. Replace the
                        // instance while the old handle still protects the name.
                        match listener(&namespace, false) {
                            Ok(replacement) => { next = replacement; continue; },
                            Err(error) => { let _ = sender.try_send(Err(error)); break; }
                        }
                    },
                    Err(error) => { let _ = sender.try_send(Err(error)); break; },
                }
                let replacement = match listener(&namespace, false) {
                    Ok(pipe) => pipe,
                    Err(error) => { let _ = sender.try_send(Err(error)); break; }
                };
                let pipe = std::mem::replace(&mut next, replacement);
                if tasks.len() >= CAPACITY {
                    drop(pipe); // Rejection, not an await-space task or an extra queue.
                    continue;
                }
                let Some(id) = next_id.checked_add(1) else { break; };
                next_id = id;
                let peer = Arc::new(Peer { id: ConnectionId(id), alive: AtomicBool::new(true), disconnected: disconnected.clone() });
                let incoming = sender.clone();
                #[cfg(test)]
                let count = Active::new(active.clone());
                tasks.spawn(async move {
                    #[cfg(test)]
                    let _count = count;
                    let _ = serve(pipe, incoming, peer).await;
                });
            }
        }
    }
    // No listener remains when cancellation starts. Drop all futures (including
    // partial I/O), then reap their completion before letting the runtime end.
    drop(next);
    tasks.shutdown().await;
}

#[cfg(test)]
struct Active(Arc<std::sync::atomic::AtomicUsize>);
#[cfg(test)]
impl Active {
    fn new(count: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}
#[cfg(test)]
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
