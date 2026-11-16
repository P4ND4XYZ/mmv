//! The sole IPC runtime thread. Tasks/channels are bounded independently.
use super::*;
use tokio::task::JoinSet;

pub(super) async fn run(
    listener: TcpListener,
    sender: mpsc::Sender<stdio::Result<Incoming>>,
    mut stopped: watch::Receiver<bool>,
    disconnected: Arc<Notify>,
) {
    let mut tasks = JoinSet::new();
    let mut next_id = 0u64;
    loop {
        tokio::select! {
            biased;
            _ = stopped.changed() => break,
            _ = tasks.join_next(), if !tasks.is_empty() => {},
            connected = listener.accept() => {
                let (stream, _) = match connected {
                    Ok(connection) => connection,
                    Err(error) => { let _ = sender.try_send(Err(error)); break; }
                };
                if tasks.len() >= CAPACITY { drop(stream); continue; }
                let Some(id) = next_id.checked_add(1) else { break; };
                next_id = id;
                let peer = Arc::new(Peer { id: ConnectionId(id), alive: AtomicBool::new(true), disconnected: disconnected.clone() });
                let incoming = sender.clone();
                tasks.spawn(async move { let _ = serve(stream, incoming, peer).await; });
            }
        }
    }
    drop(listener);
    tasks.shutdown().await;
}
