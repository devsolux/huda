//! Fixed profile paths and OS-held leases. The lease inode is never unlinked.
use crate::error::{require, Error, Result};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Profile {
    pub(crate) directory: PathBuf,
}
impl Profile {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        require(
            path.is_absolute(),
            "INVALID_INPUT",
            "Profile path must be absolute",
        )?;
        require(
            !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
            "INVALID_INPUT",
            "Parent traversal is not allowed",
        )?;
        #[cfg(windows)]
        if !path.exists() {
            super::windows_security::create_private_directory(path)?;
        }
        #[cfg(unix)]
        if !path.exists() {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path)?;
        }
        let meta = fs::symlink_metadata(path)?;
        require(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "PERMISSION_DENIED",
            "Profile must be a real private directory",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // SAFETY: geteuid has no preconditions and does not mutate process state.
            let uid = unsafe { libc::geteuid() };
            require(
                meta.uid() == uid && meta.mode() & 0o077 == 0,
                "PERMISSION_DENIED",
                "Profile must be owned by the user with mode 0700",
            )?;
        }
        #[cfg(windows)]
        super::windows_security::check_private_path(path)?;
        Ok(Self {
            directory: fs::canonicalize(path)?,
        })
    }
    pub fn default_location() -> Result<Self> {
        Self::new(Self::default_path()?)
    }
    pub fn default_path() -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            Ok(home()?.join("Library/Application Support/Huda"))
        }
        #[cfg(windows)]
        {
            std::env::var_os("LOCALAPPDATA")
                .map(|p| PathBuf::from(p).join("Huda"))
                .ok_or_else(|| Error::invalid("LOCALAPPDATA is unavailable"))
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            Ok(std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(home()?.join(".local/share"))
                .join("huda"))
        }
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn database(&self) -> PathBuf {
        self.directory.join("huda.db")
    }
    pub fn socket(&self) -> PathBuf {
        self.directory.join("core.sock")
    }
    pub fn fingerprint(&self) -> String {
        blake3::hash(self.directory.to_string_lossy().as_bytes())
            .to_hex()
            .to_string()
    }
    pub(crate) fn check_file(&self, name: &str) -> Result<()> {
        let path = self.directory.join(name);
        match fs::symlink_metadata(path) {
            Ok(meta) => {
                require(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "PERMISSION_DENIED",
                    "Managed file is not a regular file",
                )?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    // SAFETY: no arguments or side effects.
                    let uid = unsafe { libc::geteuid() };
                    require(
                        meta.nlink() == 1 && meta.uid() == uid && meta.mode() & 0o077 == 0,
                        "PERMISSION_DENIED",
                        "Managed file has unsafe owner, links or permissions",
                    )?;
                }
                #[cfg(windows)]
                super::windows_security::check_private_path(&self.directory.join(name))?;
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
    pub(crate) fn reserve_database(&self) -> Result<bool> {
        self.check_file("huda.db")?;
        self.check_file("huda.db-wal")?;
        self.check_file("huda.db-shm")?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        match options.open(self.database()) {
            Ok(file) => {
                file.sync_all()?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // An empty file without a WAL is a reservation whose schema transaction never
                // committed (crash, full disk). It holds no user data, so creating the schema
                // in it is not a wipe; any non-empty database is opened and checked as existing.
                let empty = fs::symlink_metadata(self.database())?.len() == 0;
                Ok(empty && !self.directory.join("huda.db-wal").exists())
            }
            Err(e) => Err(e.into()),
        }
    }
    /// The single Core writer lease for this profile.
    pub(crate) fn acquire(&self) -> Result<Lease> {
        self.lease(
            "core.lock",
            "CORE_UNAVAILABLE",
            "Another core owns this profile",
        )
    }
    /// Desktop-instance coordination only: one GUI process per profile, so repeated launches
    /// do not create duplicate trays. It is independent of, and never implies, Core ownership.
    pub fn desktop_lease(&self) -> Result<Lease> {
        self.lease(
            "desktop.lock",
            "DESKTOP_RUNNING",
            "Another Huda desktop already serves this profile",
        )
    }
    /// A second desktop launch leaves this marker; the running desktop shows its window.
    pub fn request_desktop_activation(&self) -> Result<()> {
        self.check_file("desktop.activate")?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        options.open(self.directory.join("desktop.activate"))?;
        Ok(())
    }
    /// A small private value the Core keeps across restarts, such as a provider backoff. A
    /// missing file reads as None; reads are bounded and never follow a symlink.
    pub(crate) fn read_state(&self, name: &str) -> Result<Option<String>> {
        use std::io::Read;
        self.check_file(name)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let file = match options.open(self.directory.join(name)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut value = String::new();
        file.take(256).read_to_string(&mut value)?;
        Ok(Some(value))
    }
    /// Replaces a private state value read by `read_state`.
    pub(crate) fn write_state(&self, name: &str, value: &str) -> Result<()> {
        use std::io::Write;
        self.check_file(name)?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = options.open(self.directory.join(name))?;
        file.write_all(value.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }
    /// Consumes a pending activation marker. Returns whether one was present.
    pub fn take_desktop_activation(&self) -> Result<bool> {
        self.check_file("desktop.activate")?;
        match fs::remove_file(self.directory.join("desktop.activate")) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
    fn lease(&self, name: &str, code: &str, message: &str) -> Result<Lease> {
        self.check_file(name)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let file = options.open(self.directory.join(name))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::MetadataExt;
            let meta = file.metadata()?;
            require(
                meta.is_file() && meta.nlink() == 1,
                "PERMISSION_DENIED",
                "Unsafe lease handle",
            )?;
            // SAFETY: file owns this live descriptor. Closing the file releases flock.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                let held = std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock;
                return Err(lease_error(held, code, message));
            }
        }
        #[cfg(windows)]
        file.try_lock().map_err(|error| {
            lease_error(
                matches!(error, std::fs::TryLockError::WouldBlock),
                code,
                message,
            )
        })?;
        Ok(Lease { _file: file })
    }
}
/// Only a lease held by another process is reported as such; a filesystem that cannot lock
/// (for example some network mounts) must not look like a running Core or desktop.
fn lease_error(held: bool, code: &str, message: &str) -> Error {
    if held {
        Error::new(code, message)
    } else {
        Error::new(
            "IO_ERROR",
            "The profile filesystem cannot provide a safe exclusive lease",
        )
    }
}
#[cfg(not(windows))]
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::invalid("HOME is unavailable"))
}
/// An OS-held exclusive lock on a managed profile file; released when dropped or on exit.
#[derive(Debug)]
pub struct Lease {
    _file: File,
}
