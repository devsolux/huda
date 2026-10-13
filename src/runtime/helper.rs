//! The helper is a fixed sibling, never an executable supplied by a prompt or PATH.
use crate::error::{require, Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn executable() -> Result<PathBuf> {
    sibling(&std::env::current_exe()?)
}

fn sibling(current: &Path) -> Result<PathBuf> {
    let name = if cfg!(windows) { "huda.exe" } else { "huda" };
    let path = current.with_file_name(name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| {
        Error::new(
            "CORE_UNAVAILABLE",
            "The packaged huda helper is missing beside the desktop executable",
        )
    })?;
    require(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "PERMISSION_DENIED",
        "The packaged helper must be a regular executable",
    )?;
    Ok(path)
}

pub fn command() -> Result<Command> {
    let mut command = Command::new(executable()?);
    // The console helper retains normal CLI behavior. Only GUI/on-demand children are hidden.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    // Keep a mutable binding on all targets because Windows sets creation flags.
    #[cfg(not(windows))]
    let _ = &mut command;
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renamed_desktop_still_resolves_fixed_sibling() {
        let directory = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "huda.exe" } else { "huda" };
        let helper = directory.path().join(name);
        std::fs::write(&helper, b"test fixture, not executed").unwrap();
        assert_eq!(
            sibling(&directory.path().join("Renamed GUI.exe")).unwrap(),
            helper
        );
    }
    #[test]
    fn missing_helper_does_not_respawn_the_gui() {
        let directory = tempfile::tempdir().unwrap();
        assert!(sibling(&directory.path().join("Huda")).is_err());
    }
}
