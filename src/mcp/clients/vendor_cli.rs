//! A client's own command, and whether a client runs. An app started from Finder has launchd's
//! short `PATH`, so a command the user's terminal finds is also looked for in the usual folders.
//! The program name comes from the catalog and no shell is involved.
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const HOME_FOLDERS: [&str; 4] = [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin"];
const SYSTEM_FOLDERS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];
const COMMAND_LIMIT: Duration = Duration::from_secs(20);
const CHECK_LIMIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);

pub fn unavailable() -> Error {
    Error::new(
        "CLIENT_CLI_UNAVAILABLE",
        "The client's own command is missing or failed; nothing was changed by Huda",
    )
}

/// Waits at most `limit`, and kills what overruns it.
fn status_within(mut command: Command, limit: Duration) -> Option<ExitStatus> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().ok()? {
            return Some(status);
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(POLL);
    }
}

/// Where `program` is, when the user has it.
pub fn find(program: &str, home: &Path) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    HOME_FOLDERS
        .iter()
        .map(|folder| home.join(folder))
        .chain(SYSTEM_FOLDERS.iter().map(PathBuf::from))
        .chain(on_path.into_iter().filter(|folder| folder.is_absolute()))
        .map(|folder| folder.join(program))
        .find(|path| path.is_file())
}

/// Runs a command to its end, and refuses anything but success.
pub fn run(program: &Path, arguments: &[&str]) -> Result<()> {
    let mut command = Command::new(program);
    command.args(arguments);
    match status_within(command, COMMAND_LIMIT) {
        Some(status) if status.success() => Ok(()),
        _ => Err(unavailable()),
    }
}

/// Whether a process of this exact name runs. `-a` counts this process's own ancestors, because
/// Huda started from a client's terminal descends from that client.
pub fn is_running(process: &str) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let mut command = Command::new("/usr/bin/pgrep");
    command.args(["-a", "-x", process]);
    status_within(command, CHECK_LIMIT).is_some_and(|status| status.success())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn program(path: &Path, script: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn the_usual_folders_are_found_without_a_path() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(find("huda-test-tool", home.path()), None);
        let tool = home.path().join(".local/bin/huda-test-tool");
        program(&tool, "exit 0");
        assert_eq!(find("huda-test-tool", home.path()), Some(tool));
    }

    #[test]
    fn success_and_failure_come_from_the_exit_code() {
        let folder = tempfile::tempdir().unwrap();
        let good = folder.path().join("good");
        let bad = folder.path().join("bad");
        program(&good, "exit 0");
        program(&bad, "exit 3");
        assert!(run(&good, &[]).is_ok());
        assert_eq!(run(&bad, &[]).unwrap_err().code, "CLIENT_CLI_UNAVAILABLE");
    }

    #[test]
    fn a_command_that_overruns_its_limit_is_stopped() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        assert!(status_within(command, Duration::from_millis(50)).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_running_process_is_found_by_its_exact_name() {
        let mut sleeper = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        assert!(is_running("sleep"));
        assert!(!is_running("slee"));
        assert!(!is_running("huda-no-such-process"));
        let _ = sleeper.kill();
        let _ = sleeper.wait();
    }
}
