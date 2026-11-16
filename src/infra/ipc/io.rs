//! Cancelling a partial frame always discards the TCP connection.
use super::{super::deadline::Deadline, invalid};
use std::io;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};

#[cfg(test)]
pub(super) async fn read_exact(
    stream: &mut (impl AsyncRead + Unpin),
    buffer: &mut [u8],
    deadline: Deadline,
) -> io::Result<()> {
    deadline
        .until(async { stream.read_exact(buffer).await.map(|_| ()) })
        .await
}
pub(super) async fn write_all(
    stream: &mut (impl AsyncWrite + Unpin),
    buffer: &[u8],
    deadline: Deadline,
) -> io::Result<()> {
    deadline.until(stream.write_all(buffer)).await
}
pub(super) async fn read_frame(
    stream: &mut (impl AsyncRead + Unpin),
    deadline: Deadline,
) -> io::Result<Vec<u8>> {
    deadline
        .until(async {
            let size = stream.read_u32_le().await? as usize;
            if size == 0 || size > super::protocol::MAX_FRAME {
                return Err(invalid("invalid frame length"));
            }
            let mut payload = vec![0; size];
            stream.read_exact(&mut payload).await?;
            Ok(payload)
        })
        .await
}
pub(super) fn idle_peer(stream: &TcpStream) -> io::Result<()> {
    match stream.try_read(&mut [0]) {
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(error) => Err(error),
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "TCP peer disconnected",
        )),
        Ok(_) => Err(invalid("unexpected data after request")),
    }
}
pub(super) async fn peer_departed(stream: &TcpStream) -> io::Error {
    loop {
        if let Err(error) = stream.readable().await {
            return error;
        }
        match stream.try_read(&mut [0]) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            Err(error) => return error,
            Ok(0) => return io::Error::new(io::ErrorKind::UnexpectedEof, "TCP peer disconnected"),
            Ok(_) => return invalid("unexpected data after request"),
        }
    }
}
