//! Tokio owns all OVERLAPPED state and buffers. Cancelling a frame always drops
//! its connection: partial reads/writes are never resumed as a new operation.
use super::super::{deadline::Deadline, security::invalid};
use std::{
    io,
    ops::{Deref, DerefMut},
    os::windows::io::{AsRawHandle, RawHandle},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    net::windows::named_pipe::NamedPipeServer,
};

/// Mio deliberately lets buffered writes survive NamedPipe's Drop. Explicitly
/// cancel before handing teardown back to Tokio; Mio retains its own buffers and
/// OVERLAPPED until IOCP completion. No manual completion wait or borrowed buffer.
/// See the Windows proof: dropping alone can retain a nonreading peer forever.
pub(super) struct Pipe<T: AsRawHandle>(pub T);
impl<T: AsRawHandle> Drop for Pipe<T> {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::IO::CancelIoEx(self.0.as_raw_handle(), std::ptr::null());
        }
    }
}
impl<T: AsRawHandle> Deref for Pipe<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T: AsRawHandle> DerefMut for Pipe<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
impl<T: AsRawHandle> AsRawHandle for Pipe<T> {
    fn as_raw_handle(&self) -> RawHandle {
        self.0.as_raw_handle()
    }
}
impl<T: AsRawHandle + AsyncRead + Unpin> AsyncRead for Pipe<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}
impl<T: AsRawHandle + AsyncWrite + Unpin> AsyncWrite for Pipe<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

pub(super) async fn read_exact(
    pipe: &mut (impl AsyncRead + Unpin),
    buffer: &mut [u8],
    deadline: Deadline,
) -> io::Result<()> {
    deadline
        .until(async { pipe.read_exact(buffer).await.map(|_| ()) })
        .await
}
pub(super) async fn write_all(
    pipe: &mut (impl AsyncWrite + Unpin),
    buffer: &[u8],
    deadline: Deadline,
) -> io::Result<()> {
    deadline.until(pipe.write_all(buffer)).await
}
pub(super) async fn read_frame(
    pipe: &mut (impl AsyncRead + Unpin),
    deadline: Deadline,
) -> io::Result<Vec<u8>> {
    deadline
        .until(async {
            let size = pipe.read_u32_le().await? as usize;
            if size == 0 || size > super::protocol::MAX_FRAME {
                return Err(invalid("invalid frame length"));
            }
            let mut payload = vec![0; size];
            pipe.read_exact(&mut payload).await?;
            Ok(payload)
        })
        .await
}
/// A single nonwaiting admission check, before the operation enters the queue.
/// Tokio/Mio may already have prefetched bytes, so kernel PeekNamedPipe alone
/// cannot see every invalid tail. Conversely, the kernel check catches a peer
/// change before Tokio has published its next readiness/completion notification.
/// This is not a wait/poll loop: a later peer change can still race admission,
/// just as it could race the baseline's check; accepted operations continue.
pub(super) fn idle_peer(pipe: &NamedPipeServer) -> io::Result<()> {
    match pipe.try_read(&mut [0]) {
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
        Err(error) => return Err(error),
        Ok(0) => {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "pipe disconnected",
            ));
        }
        Ok(_) => return Err(invalid("unexpected data after request")),
    }
    let mut available = 0;
    if unsafe {
        windows_sys::Win32::System::Pipes::PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if available != 0 {
        return Err(invalid("unexpected data after request"));
    }
    Ok(())
}

/// A oneshot alone cannot notice a departed client. Exactly one readiness/read
/// future monitors disconnect and unsolicited bytes while the app is working.
/// Cancelling this future consumes no data (try_read happens only when ready).
pub(super) async fn peer_departed(pipe: &NamedPipeServer) -> io::Error {
    loop {
        if let Err(error) = pipe.readable().await {
            return error;
        }
        match pipe.try_read(&mut [0]) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            Err(error) => return error,
            Ok(0) => return io::Error::new(io::ErrorKind::UnexpectedEof, "pipe disconnected"),
            Ok(_) => return invalid("unexpected data after request"),
        }
    }
}
