//! Local overlapped Pipe I/O. Each operation retains its OVERLAPPED, event and
//! borrowed buffer until final completion, including on cancellation.
use super::super::{
    deadline::Deadline,
    security::{Handle, invalid},
};
use std::{io, ptr, time::Instant};
use windows_sys::Win32::{
    Foundation::{ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Storage::FileSystem::{ReadFile, WriteFile},
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{ConnectNamedPipe, PeekNamedPipe},
        Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject},
    },
};

pub(super) struct Stop(Handle);
impl Stop {
    pub fn new() -> io::Result<Self> {
        Ok(Self(Handle::new(unsafe {
            CreateEventW(ptr::null(), 1, 0, ptr::null())
        })?))
    }
    pub fn signal(&self) {
        unsafe {
            SetEvent(self.0.0);
        }
    }
    pub fn signalled(&self) -> bool {
        unsafe { WaitForSingleObject(self.0.0, 0) == WAIT_OBJECT_0 }
    }
    pub fn pause(&self, deadline: Deadline) -> io::Result<()> {
        if deadline.expired(Instant::now()) {
            return Err(timeout());
        }
        let millis = deadline.remaining(Instant::now()).as_millis().clamp(1, 10) as u32;
        if unsafe { WaitForSingleObject(self.0.0, millis) } == WAIT_OBJECT_0 {
            Err(stopped())
        } else {
            Ok(())
        }
    }
}
pub(super) fn timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "IPC deadline exceeded")
}
pub(super) fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "IPC server stopped")
}

/// The guard is deliberately stack-local and cannot outlive the pipe or buffer.
struct Pending<'a> {
    pipe: &'a Handle,
    overlapped: OVERLAPPED,
    _event: Handle,
    active: bool,
}
impl<'a> Pending<'a> {
    fn new(pipe: &'a Handle) -> io::Result<Self> {
        let event = Handle::new(unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) })?;
        let overlapped = OVERLAPPED {
            hEvent: event.0,
            ..unsafe { std::mem::zeroed() }
        };
        Ok(Self {
            pipe,
            overlapped,
            _event: event,
            active: false,
        })
    }
    fn finish(&mut self, stop: &Stop, deadline: Deadline) -> io::Result<u32> {
        let handles = [self.overlapped.hEvent, stop.0.0];
        if stop.signalled() {
            return Err(stopped());
        }
        if deadline.expired(Instant::now()) {
            return Err(timeout());
        }
        let millis = deadline
            .remaining(Instant::now())
            .as_millis()
            .clamp(1, u32::MAX as u128 - 1) as u32;
        match unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, millis) } {
            WAIT_OBJECT_0 => {
                let mut bytes = 0;
                let ok =
                    unsafe { GetOverlappedResult(self.pipe.0, &self.overlapped, &mut bytes, 0) };
                // The event is manual-reset and dedicated to this single operation.
                self.active = false;
                if ok == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(bytes)
                }
            }
            value if value == WAIT_OBJECT_0 + 1 => Err(stopped()),
            WAIT_TIMEOUT => Err(timeout()),
            _ => Err(io::Error::last_os_error()),
        }
    }
}
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                // CancelIoEx is only a cancellation request. ERROR_NOT_FOUND can
                // mean completion won the race; drain in either case, before any
                // stack OVERLAPPED, event or caller buffer can be destroyed.
                CancelIoEx(self.pipe.0, &self.overlapped);
                let mut bytes = 0;
                GetOverlappedResult(self.pipe.0, &self.overlapped, &mut bytes, 1);
            }
        }
    }
}

pub(super) fn accept(pipe: &Handle, stop: &Stop, deadline: Deadline) -> io::Result<()> {
    let mut pending = Pending::new(pipe)?;
    let ok = unsafe { ConnectNamedPipe(pipe.0, &mut pending.overlapped) };
    if ok != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) {
        return Ok(());
    }
    if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
        return Err(error);
    }
    pending.active = true;
    pending.finish(stop, deadline).map(|_| ())
}
fn transfer(
    pipe: &Handle,
    buffer: &mut [u8],
    writing: bool,
    stop: &Stop,
    deadline: Deadline,
) -> io::Result<usize> {
    if stop.signalled() {
        return Err(stopped());
    }
    if deadline.expired(Instant::now()) {
        return Err(timeout());
    }
    let mut pending = Pending::new(pipe)?;
    let ok = unsafe {
        if writing {
            WriteFile(
                pipe.0,
                buffer.as_ptr(),
                buffer.len() as u32,
                ptr::null_mut(),
                &mut pending.overlapped,
            )
        } else {
            ReadFile(
                pipe.0,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                ptr::null_mut(),
                &mut pending.overlapped,
            )
        }
    };
    if ok == 0 && io::Error::last_os_error().raw_os_error() != Some(ERROR_IO_PENDING as i32) {
        return Err(io::Error::last_os_error());
    }
    pending.active = true;
    let count = pending.finish(stop, deadline)? as usize;
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "pipe disconnected",
        ));
    }
    Ok(count)
}
pub(super) fn read_exact(
    pipe: &Handle,
    mut buffer: &mut [u8],
    stop: &Stop,
    deadline: Deadline,
) -> io::Result<()> {
    while !buffer.is_empty() {
        let count = transfer(pipe, buffer, false, stop, deadline)?;
        buffer = &mut buffer[count..];
    }
    Ok(())
}
pub(super) fn write_all(
    pipe: &Handle,
    buffer: &mut [u8],
    stop: &Stop,
    deadline: Deadline,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < buffer.len() {
        offset += transfer(pipe, &mut buffer[offset..], true, stop, deadline)?;
    }
    Ok(())
}
pub(super) fn read_frame(pipe: &Handle, stop: &Stop, deadline: Deadline) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    read_exact(pipe, &mut header, stop, deadline)?;
    let size = u32::from_le_bytes(header) as usize;
    if size == 0 || size > super::protocol::MAX_FRAME {
        return Err(invalid("invalid frame length"));
    }
    let mut payload = vec![0; size];
    read_exact(pipe, &mut payload, stop, deadline)?;
    Ok(payload)
}
/// While waiting for app completion, detect disconnect or extra unsolicited data
/// without leaving a blocking read/task alive after the client has gone away.
pub(super) fn idle_peer(pipe: &Handle) -> io::Result<()> {
    let mut available = 0;
    if unsafe {
        PeekNamedPipe(
            pipe.0,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut available,
            ptr::null_mut(),
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
