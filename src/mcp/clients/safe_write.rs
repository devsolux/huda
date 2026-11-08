//! Changing a file that belongs to another product. It may hold other servers' keys, so nothing
//! here widens who can read it, a file keeps the mode its owner gave it, and no write leaves it
//! half done. Errors name no path and no content.
use super::invalid_config;
use crate::error::{require, Error, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

const BACKUP_SUFFIX: &str = ".huda-backup";
/// Far above any client configuration, and small enough to read in one step.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

fn unsafe_file() -> Error {
    Error::new(
        "CLIENT_CONFIG_UNSAFE",
        "This client's file cannot be changed safely, so it was left as it was",
    )
}

/// The text of a file, or nothing when it does not exist yet. Only a bounded regular file is
/// read: a pipe at this path would otherwise hang the caller.
pub fn read_or_empty(path: &Path) -> Result<String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error.into()),
    };
    require(
        metadata.is_file() && metadata.len() <= MAX_FILE_BYTES,
        "CLIENT_CONFIG_INVALID",
        "This client's file is not a regular file of a sensible size",
    )?;
    fs::read_to_string(path).map_err(|error| match error.kind() {
        ErrorKind::InvalidData => invalid_config(),
        _ => error.into(),
    })
}

/// The file behind `path`. Dotfile managers link these files, and renaming over a link would
/// replace it. A link to nothing is refused rather than turned into a new file.
fn real_target(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(real) => Ok(real),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                Err(unsafe_file())
            } else {
                Ok(path.to_path_buf())
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn private_new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    Ok(options.open(path)?)
}

/// Keeps the file as it was before Huda first changed it, readable by whoever could read that
/// file. An existing backup stays, or a later write would lose the original.
fn back_up(path: &Path, target: &Path, current: &str) -> Result<()> {
    if current.is_empty() {
        return Ok(());
    }
    let mut backup = path.as_os_str().to_os_string();
    backup.push(BACKUP_SUFFIX);
    let backup = PathBuf::from(backup);
    let mut file = match private_new_file(&backup) {
        Ok(file) => file,
        Err(_) if fs::symlink_metadata(&backup).is_ok() => return Ok(()),
        Err(error) => return Err(error),
    };
    file.write_all(current.as_bytes())?;
    file.sync_all()?;
    fs::set_permissions(&backup, fs::metadata(target)?.permissions())?;
    Ok(())
}

/// Writes beside `target`, so the rename stays on one file system and the file is whole at every
/// moment. A new file is owner-only.
fn replace(target: &Path, text: &str) -> Result<()> {
    let name = target
        .file_name()
        .ok_or_else(unsafe_file)?
        .to_string_lossy();
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|_| unsafe_file())?;
    let temporary = target.with_file_name(format!(".{name}.huda-{}", u64::from_le_bytes(random)));
    let written = (|| -> Result<()> {
        let mut file = private_new_file(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(target) {
            fs::set_permissions(&temporary, metadata.permissions())?;
        }
        fs::rename(&temporary, target)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written?;
    #[cfg(unix)]
    if let Some(parent) = target.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Applies `edit` to the file: it gets the current text and answers the new one, or nothing to
/// leave the file alone. A refused edit writes nothing. The result is checked by reading it back.
pub fn update(path: &Path, edit: impl FnOnce(&str) -> Result<Option<String>>) -> Result<()> {
    let target = real_target(path)?;
    let current = read_or_empty(&target)?;
    let Some(updated) = edit(&current)? else {
        return Ok(());
    };
    // A read-only file is the user's choice, and a rename would go around it.
    if fs::metadata(&target).is_ok_and(|metadata| metadata.permissions().readonly()) {
        return Err(unsafe_file());
    }
    if let Some(parent) = target.parent() {
        if !parent.exists() {
            create_private_folder(parent)?;
        }
    }
    back_up(path, &target, &current)?;
    replace(&target, &updated)?;
    require(
        read_or_empty(&target)? == updated,
        "CLIENT_CONFIG_UNSAFE",
        "The change could not be read back from this client's file",
    )
}

fn create_private_folder(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    Ok(builder.create(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added(current: &str) -> Result<Option<String>> {
        Ok(Some(format!("{current}+")))
    }

    fn backup_of(path: &Path) -> PathBuf {
        let mut backup = path.as_os_str().to_os_string();
        backup.push(BACKUP_SUFFIX);
        PathBuf::from(backup)
    }

    #[test]
    fn a_new_file_gets_its_folder_and_no_backup() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("new/deep/config.json");
        update(&path, added).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "+");
        assert!(!backup_of(&path).exists());
    }

    #[test]
    fn only_the_first_change_makes_a_backup() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("config.json");
        fs::write(&path, "original").unwrap();
        update(&path, added).unwrap();
        update(&path, added).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "original++");
        assert_eq!(fs::read_to_string(backup_of(&path)).unwrap(), "original");
    }

    #[test]
    fn a_refused_or_empty_edit_writes_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("config.json");
        fs::write(&path, "original").unwrap();
        assert!(update(&path, |_| Err(invalid_config())).is_err());
        update(&path, |_| Ok(None)).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        assert!(!backup_of(&path).exists());
    }

    #[test]
    fn no_temporary_file_stays_behind() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("config.json");
        fs::write(&path, "original").unwrap();
        update(&path, added).unwrap();
        let names: Vec<_> = fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn a_read_only_file_is_left_alone() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("config.json");
        fs::write(&path, "original").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(
            update(&path, added).unwrap_err().code,
            "CLIENT_CONFIG_UNSAFE"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    }

    #[test]
    fn a_folder_at_the_path_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(
            read_or_empty(folder.path()).unwrap_err().code,
            "CLIENT_CONFIG_INVALID"
        );
    }

    #[test]
    fn text_that_is_not_unicode_is_a_broken_file_and_stays() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("config.json");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(
            update(&path, added).unwrap_err().code,
            "CLIENT_CONFIG_INVALID"
        );
        assert_eq!(fs::read(&path).unwrap(), [0xff, 0xfe, 0x00]);
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::{symlink, FileTypeExt, PermissionsExt};

        fn mode(path: &Path) -> u32 {
            fs::metadata(path).unwrap().permissions().mode() & 0o777
        }

        #[test]
        fn a_linked_file_stays_linked_and_its_target_changes() {
            let folder = tempfile::tempdir().unwrap();
            let real = folder.path().join("dotfiles-config.json");
            let link = folder.path().join("config.json");
            fs::write(&real, "original").unwrap();
            symlink(&real, &link).unwrap();
            update(&link, added).unwrap();
            assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
            assert_eq!(fs::read_to_string(&real).unwrap(), "original+");
        }

        #[test]
        fn a_link_to_nothing_is_refused_and_creates_nothing() {
            let folder = tempfile::tempdir().unwrap();
            let missing = folder.path().join("missing.json");
            let link = folder.path().join("config.json");
            symlink(&missing, &link).unwrap();
            assert_eq!(
                update(&link, added).unwrap_err().code,
                "CLIENT_CONFIG_UNSAFE"
            );
            assert!(!missing.exists());
        }

        #[test]
        fn a_new_file_is_owner_only_and_a_file_keeps_its_mode() {
            let folder = tempfile::tempdir().unwrap();
            let new = folder.path().join("new.json");
            update(&new, added).unwrap();
            assert_eq!(mode(&new), 0o600);
            let shared = folder.path().join("shared.json");
            fs::write(&shared, "original").unwrap();
            fs::set_permissions(&shared, fs::Permissions::from_mode(0o640)).unwrap();
            update(&shared, added).unwrap();
            assert_eq!(mode(&shared), 0o640);
        }

        #[test]
        fn a_backup_is_never_more_readable_than_its_file() {
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("private.json");
            fs::write(&path, "{\"TOKEN\":\"secret\"}").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            update(&path, added).unwrap();
            assert_eq!(mode(&backup_of(&path)), 0o600);
        }

        #[test]
        fn a_planted_backup_link_is_not_followed() {
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("config.json");
            let elsewhere = folder.path().join("elsewhere");
            fs::write(&path, "original").unwrap();
            fs::write(&elsewhere, "other").unwrap();
            symlink(&elsewhere, backup_of(&path)).unwrap();
            update(&path, added).unwrap();
            assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "other");
        }

        #[test]
        fn a_pipe_at_the_path_is_refused_instead_of_hanging() {
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("pipe");
            let name = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
            // SAFETY: name is a valid terminated path that lives through the call.
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            assert!(fs::metadata(&path).unwrap().file_type().is_fifo());
            assert_eq!(
                read_or_empty(&path).unwrap_err().code,
                "CLIENT_CONFIG_INVALID"
            );
        }
    }
}
