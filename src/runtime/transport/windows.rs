//! Private local Windows byte pipes. Every pending OVERLAPPED operation is drained
//! before its buffer, event or OVERLAPPED allocation can be dropped.
use crate::error::{require, Error, Result};
use crate::runtime::{windows_security as security, Profile};
use std::cell::Cell;
use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
    PeekNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

fn name(profile: &Profile) -> Result<Vec<u16>> {
    security::wide(OsStr::new(&format!(
        r"\\.\pipe\{}-{}",
        crate::APP_ID,
        profile.fingerprint()
    )))
}
fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: only successful handle-creating API return values are passed here, exactly once.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

struct Operation {
    overlapped: Box<OVERLAPPED>,
    _event: OwnedHandle,
}
impl Operation {
    fn new() -> io::Result<Self> {
        // SAFETY: non-inheritable unnamed manual-reset event; no borrowed input pointers.
        let event = owned(unsafe { CreateEventW(null(), 1, 0, null()) })?;
        // SAFETY: an OVERLAPPED is initialized with zeros as required by the Windows API.
        let mut overlapped: Box<OVERLAPPED> = Box::new(unsafe { std::mem::zeroed() });
        overlapped.hEvent = event.as_raw_handle();
        Ok(Self {
            overlapped,
            _event: event,
        })
    }
    fn cancel_and_drain(&mut self, pipe: HANDLE) {
        let mut transferred = 0;
        // SAFETY: pipe and OVERLAPPED remain live. CancelIoEx alone is not sufficient:
        // GetOverlappedResult(wait=true) waits until the kernel releases the buffer.
        unsafe {
            CancelIoEx(pipe, &*self.overlapped);
            GetOverlappedResult(pipe, &*self.overlapped, &mut transferred, 1);
        }
    }
    fn finish(
        &mut self,
        pipe: HANDLE,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> io::Result<usize> {
        let deadline = Instant::now() + timeout;
        loop {
            if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
                self.cancel_and_drain(pipe);
                return Err(io::Error::new(
                    if cancelled.load(Ordering::Acquire) {
                        io::ErrorKind::Interrupted
                    } else {
                        io::ErrorKind::TimedOut
                    },
                    "Private IPC operation ended",
                ));
            }
            // SAFETY: the event is owned for the full wait and operation duration.
            let wait = unsafe { WaitForSingleObject(self.overlapped.hEvent, 10) };
            if wait == WAIT_TIMEOUT {
                continue;
            }
            if wait != WAIT_OBJECT_0 {
                let error = io::Error::last_os_error();
                self.cancel_and_drain(pipe);
                return Err(error);
            }
            let mut bytes = 0;
            // SAFETY: event was signalled and the OVERLAPPED storage is still pinned in its Box.
            if unsafe { GetOverlappedResult(pipe, &*self.overlapped, &mut bytes, 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
            return Ok(bytes as usize);
        }
    }
}

pub(in crate::runtime) struct Stream {
    pipe: Arc<OwnedHandle>,
    cancelled: Arc<AtomicBool>,
    read_timeout: Cell<Duration>,
    write_timeout: Cell<Duration>,
    server: bool,
}
impl Stream {
    fn new(pipe: OwnedHandle, server: bool) -> Self {
        Self {
            pipe: Arc::new(pipe),
            cancelled: Arc::new(AtomicBool::new(false)),
            read_timeout: Cell::new(Duration::from_secs(3)),
            write_timeout: Cell::new(Duration::from_secs(3)),
            server,
        }
    }
    pub(in crate::runtime) fn connect(profile: &Profile) -> Result<Self> {
        let name = name(profile)?;
        // SECURITY_IDENTIFICATION prevents a server from impersonating this client.
        // The client still authenticates the server's OS SID before sending any credential.
        // SAFETY: terminated name, no borrowed security descriptor, result owned below.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                null_mut(),
            )
        };
        match owned(handle) {
            Ok(pipe) => Ok(Self::new(pipe, false)),
            Err(error)
                if matches!(
                    error.raw_os_error().map(|n| n as u32),
                    Some(ERROR_FILE_NOT_FOUND) | Some(ERROR_PIPE_BUSY)
                ) =>
            {
                Err(Error::new(
                    "CORE_UNAVAILABLE",
                    "Private Core pipe is not ready",
                ))
            }
            Err(error) => Err(error.into()),
        }
    }
    pub(in crate::runtime) fn verify_peer(&self) -> Result<()> {
        let mut pid = 0;
        // SAFETY: live connected kernel pipe and valid PID output, not caller-controlled IDs.
        let ok = unsafe {
            if self.server {
                GetNamedPipeClientProcessId(self.pipe.as_raw_handle(), &mut pid)
            } else {
                GetNamedPipeServerProcessId(self.pipe.as_raw_handle(), &mut pid)
            }
        };
        require(
            ok != 0,
            "PERMISSION_DENIED",
            "Windows IPC peer identity is unavailable",
        )?;
        security::verify_peer(pid)
    }
    pub(in crate::runtime) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            pipe: self.pipe.clone(),
            cancelled: self.cancelled.clone(),
            read_timeout: Cell::new(self.read_timeout.get()),
            write_timeout: Cell::new(self.write_timeout.get()),
            server: self.server,
        })
    }
    pub(in crate::runtime) fn set_read_timeout(&self, value: Option<Duration>) -> io::Result<()> {
        self.read_timeout
            .set(value.unwrap_or(Duration::from_secs(120)));
        Ok(())
    }
    pub(in crate::runtime) fn set_write_timeout(&self, value: Option<Duration>) -> io::Result<()> {
        self.write_timeout
            .set(value.unwrap_or(Duration::from_secs(3)));
        Ok(())
    }
    pub(in crate::runtime) fn shutdown(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub(in crate::runtime) fn disconnected_or_extra_data(&self) -> bool {
        let mut available = 0;
        // SAFETY: PeekNamedPipe with null buffer only checks availability; consumes no bytes.
        let ok = unsafe {
            PeekNamedPipe(
                self.pipe.as_raw_handle(),
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        };
        ok == 0 || available > 0
    }
}
impl Read for Stream {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = data.len().min(u32::MAX as usize) as u32;
        let mut operation = Operation::new()?;
        let mut bytes = 0;
        // SAFETY: buffer and boxed OVERLAPPED stay live until completion or cancellation/drain.
        let ok = unsafe {
            ReadFile(
                self.pipe.as_raw_handle(),
                data.as_mut_ptr(),
                count,
                &mut bytes,
                &mut *operation.overlapped,
            )
        };
        if ok != 0 {
            return Ok(bytes as usize);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
            return Ok(0);
        }
        if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(error);
        }
        match operation.finish(
            self.pipe.as_raw_handle(),
            self.read_timeout.get(),
            &self.cancelled,
        ) {
            Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
            result => result,
        }
    }
}
impl Write for Stream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = data.len().min(u32::MAX as usize) as u32;
        let mut operation = Operation::new()?;
        let mut bytes = 0;
        // SAFETY: borrowed bytes and pinned OVERLAPPED outlive all native use, including drain.
        let ok = unsafe {
            WriteFile(
                self.pipe.as_raw_handle(),
                data.as_ptr(),
                count,
                &mut bytes,
                &mut *operation.overlapped,
            )
        };
        if ok != 0 {
            return Ok(bytes as usize);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(error);
        }
        operation.finish(
            self.pipe.as_raw_handle(),
            self.write_timeout.get(),
            &self.cancelled,
        )
    }
    // FlushFileBuffers on a pipe can wait indefinitely for a peer to consume bytes.
    // Write completion suffices for this framed protocol; there is no userspace buffer here.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Pending {
    stream: Option<Stream>,
    operation: Operation,
    waiting: bool,
}
impl Pending {
    fn new(profile: &Profile, first: bool) -> Result<Self> {
        let descriptor = security::Descriptor::private(false)?;
        let attributes = descriptor.attributes();
        let name = name(profile)?;
        let flags = PIPE_ACCESS_DUPLEX
            | FILE_FLAG_OVERLAPPED
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        // SAFETY: inputs live through creation; private DACL is attached atomically.
        let pipe = owned(unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                flags,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                32,
                16384,
                16384,
                0,
                &attributes,
            )
        })?;
        let stream = Stream::new(pipe, true);
        let mut operation = Operation::new()?;
        // SAFETY: stream and boxed operation are retained together by Pending until completion.
        let ok =
            unsafe { ConnectNamedPipe(stream.pipe.as_raw_handle(), &mut *operation.overlapped) };
        let waiting = if ok != 0 {
            false
        } else {
            match io::Error::last_os_error().raw_os_error().map(|n| n as u32) {
                Some(ERROR_PIPE_CONNECTED) => false,
                Some(ERROR_IO_PENDING) => true,
                _ => return Err(io::Error::last_os_error().into()),
            }
        };
        Ok(Self {
            stream: Some(stream),
            operation,
            waiting,
        })
    }
    fn ready(&mut self) -> io::Result<bool> {
        if !self.waiting {
            return Ok(true);
        }
        // SAFETY: retained event belongs to the pending connect operation.
        let wait = unsafe { WaitForSingleObject(self.operation.overlapped.hEvent, 0) };
        if wait == WAIT_TIMEOUT {
            return Ok(false);
        }
        if wait != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut bytes = 0;
        let stream = self.stream.as_ref().ok_or(io::ErrorKind::NotConnected)?;
        // SAFETY: event completed, retained operation and pipe are live.
        let ok = unsafe {
            GetOverlappedResult(
                stream.pipe.as_raw_handle(),
                &*self.operation.overlapped,
                &mut bytes,
                0,
            )
        };
        self.waiting = false;
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(true)
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if self.waiting {
            if let Some(stream) = &self.stream {
                self.operation.cancel_and_drain(stream.pipe.as_raw_handle());
            }
        }
    }
}
pub(in crate::runtime) struct Listener {
    profile: Profile,
    pending: Pending,
}
impl Listener {
    pub(in crate::runtime) fn bind(profile: &Profile) -> Result<Self> {
        // FILE_FLAG_FIRST_PIPE_INSTANCE makes name squatting a hard error, not a fallback.
        Ok(Self {
            profile: profile.clone(),
            pending: Pending::new(profile, true)?,
        })
    }
    pub(in crate::runtime) fn accept(&mut self) -> io::Result<Stream> {
        if !self.pending.ready()? {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        // Keep an instance alive continuously; do not release/reclaim the pipe name per request.
        let next = Pending::new(&self.profile, false).map_err(io::Error::other)?;
        let mut completed = std::mem::replace(&mut self.pending, next);
        completed
            .stream
            .take()
            .ok_or_else(|| io::ErrorKind::NotConnected.into())
    }
}
