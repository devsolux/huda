use crate::error::{require, Error, Result};
use crate::runtime::Profile;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

pub(in crate::runtime) struct Stream(UnixStream);
impl Stream {
    pub(in crate::runtime) fn connect(profile: &Profile) -> Result<Self> {
        if let Ok(meta) = std::fs::symlink_metadata(profile.socket()) {
            // SAFETY: geteuid has no preconditions.
            require(
                meta.file_type().is_socket()
                    && meta.uid() == unsafe { libc::geteuid() }
                    && meta.mode() & 0o077 == 0,
                "PERMISSION_DENIED",
                "Unsafe IPC socket",
            )?;
        }
        match UnixStream::connect(profile.socket()) {
            Ok(stream) => Ok(Self(stream)),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                Err(Error::new(
                    "CORE_UNAVAILABLE",
                    "Private Core socket is not ready",
                ))
            }
            Err(error) => Err(error.into()),
        }
    }
    pub(in crate::runtime) fn verify_peer(&self) -> Result<()> {
        peer_is_current_user(&self.0)
    }
    pub(in crate::runtime) fn try_clone(&self) -> std::io::Result<Self> {
        self.0.try_clone().map(Self)
    }
    pub(in crate::runtime) fn set_read_timeout(
        &self,
        value: Option<Duration>,
    ) -> std::io::Result<()> {
        self.0.set_read_timeout(value)
    }
    pub(in crate::runtime) fn set_write_timeout(
        &self,
        value: Option<Duration>,
    ) -> std::io::Result<()> {
        self.0.set_write_timeout(value)
    }
    pub(in crate::runtime) fn shutdown(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
    pub(in crate::runtime) fn disconnected_or_extra_data(&self) -> bool {
        use std::os::fd::AsRawFd;
        let mut byte = 0u8;
        // SAFETY: live socket and one-byte valid output buffer. MSG_PEEK consumes nothing.
        let result = unsafe {
            libc::recv(
                self.0.as_raw_fd(),
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        if result >= 0 {
            return true;
        }
        !matches!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        )
    }
}
impl Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(out)
    }
}
impl Write for Stream {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.write(data)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
pub(in crate::runtime) struct Listener {
    listener: UnixListener,
    path: PathBuf,
}
impl Listener {
    pub(in crate::runtime) fn bind(profile: &Profile) -> Result<Self> {
        let path = profile.socket();
        require(
            path.as_os_str().as_encoded_bytes().len() < 100,
            "INVALID_INPUT",
            "Profile path is too long for private Unix IPC",
        )?;
        // The calling App already holds the profile's writer lease.
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            // SAFETY: geteuid has no preconditions.
            require(
                meta.file_type().is_socket() && meta.uid() == unsafe { libc::geteuid() },
                "PERMISSION_DENIED",
                "Refusing to replace an unsafe socket",
            )?;
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, path })
    }
    pub(in crate::runtime) fn accept(&mut self) -> std::io::Result<Stream> {
        let (stream, _) = self.listener.accept()?;
        // BSD/macOS accepted sockets inherit O_NONBLOCK from the polling listener. Frame
        // deadlines rely on blocking reads/writes bounded by socket timeouts.
        stream.set_nonblocking(false)?;
        Ok(Stream(stream))
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn peer_is_current_user(stream: &UnixStream) -> Result<()> {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "linux")]
    {
        let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: buffer/length are valid and the stream owns a live Unix socket.
        let status = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                credentials.as_mut_ptr().cast(),
                &mut size,
            )
        };
        require(
            status == 0 && size as usize == std::mem::size_of::<libc::ucred>(),
            "PERMISSION_DENIED",
            "IPC peer credentials are unavailable",
        )?;
        // SAFETY: getsockopt succeeded and wrote exactly a ucred value.
        let credentials = unsafe { credentials.assume_init() };
        // SAFETY: geteuid has no preconditions.
        require(
            credentials.uid == unsafe { libc::geteuid() },
            "PERMISSION_DENIED",
            "IPC peer belongs to a different user",
        )
    }
    #[cfg(target_os = "macos")]
    {
        let mut uid = 0;
        let mut gid = 0;
        // SAFETY: both output pointers are valid and the socket is live.
        require(
            unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0,
            "PERMISSION_DENIED",
            "IPC peer credentials are unavailable",
        )?;
        // SAFETY: geteuid has no preconditions.
        require(
            uid == unsafe { libc::geteuid() },
            "PERMISSION_DENIED",
            "IPC peer belongs to a different user",
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = stream;
        Err(Error::new(
            "UNSUPPORTED_PLATFORM",
            "Peer authorization backend is not qualified",
        ))
    }
}
