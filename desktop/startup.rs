//! Desktop-only, explicitly requested OS login registration. No Core/provider authority.
use auto_launch::{
    AutoLaunch, AutoLaunchBuilder, LinuxLaunchMode, MacOSLaunchMode, WindowsEnableMode,
};
use huda::error::{require, Error, Result};
use std::path::Path;

fn executable_argument(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| Error::invalid("Autostart requires a Unicode executable path"))?;
    require(
        path.is_absolute() && !text.chars().any(char::is_control),
        "INVALID_INPUT",
        "Autostart executable must be an absolute path without control characters",
    )?;
    // Do not create a permanent login entry pointing into a mounted DMG / App Translocation.
    #[cfg(target_os = "macos")]
    require(
        !text.starts_with("/Volumes/") && !text.contains("/AppTranslocation/"),
        "INVALID_INPUT",
        "Move Huda.app to Applications before enabling launch at login",
    )?;
    #[cfg(target_os = "macos")]
    require(
        !text.contains(['&', '<', '>', '"', '\'']),
        "INVALID_INPUT",
        "This installation path cannot be represented safely by auto-launch's LaunchAgent backend",
    )?;
    #[cfg(target_os = "linux")]
    {
        // auto-launch 0.6 writes Exec verbatim. Quote the executable as one XDG argument.
        // Reject escape/field-code characters until their double-unescaping is qualified.
        require(
            !text.contains(['"', '\\', '$', '`', '%']),
            "INVALID_INPUT",
            "Autostart path contains unsupported desktop-entry escape characters",
        )?;
        Ok(format!("\"{text}\""))
    }
    #[cfg(windows)]
    {
        // canonicalize produces a verbatim prefix on Windows. The Run key and installer
        // use a normal absolute Win32 path; do not strand the login entry on uninstall.
        let path = text.strip_prefix(r"\\?\").unwrap_or(text);
        require(
            !path.starts_with("UNC\\") && !path.starts_with(r"\\"),
            "INVALID_INPUT",
            "Login launch must point to a local Windows installation",
        )?;
        // auto-launch 0.6 writes `<app_path> --background` verbatim into the Run value.
        // Quote the executable so a space in the install path stays one program token.
        require(
            !path.contains('"'),
            "INVALID_INPUT",
            "Login launch path cannot contain quotation marks",
        )?;
        Ok(format!("\"{path}\""))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Ok(text.to_owned())
    }
}

fn launcher(enabling: bool) -> Result<AutoLaunch> {
    #[cfg(target_os = "linux")]
    {
        // Upstream 0.6 hardcodes ~/.config. Never silently register to the wrong XDG directory.
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            let home =
                std::env::var_os("HOME").ok_or_else(|| Error::invalid("HOME is unavailable"))?;
            require(Path::new(&config) == Path::new(&home).join(".config"),
                "UNSUPPORTED_PLATFORM", "auto-launch 0.6 does not support a custom XDG_CONFIG_HOME; no login entry was changed")?;
        }
    }
    let path = std::env::current_exe()?.canonicalize()?;
    #[cfg(windows)]
    let path = {
        let value = path.to_string_lossy();
        std::path::PathBuf::from(value.strip_prefix(r"\\?\").unwrap_or(value.as_ref()))
    };
    // Only a new/updated entry needs a safe launch path. Status/removal must still work
    // after relocating an app into a path which cannot safely be registered.
    let path = if enabling {
        executable_argument(&path)?
    } else {
        path.to_string_lossy().into_owned()
    };
    AutoLaunchBuilder::new()
        .set_app_name(huda::APP_ID)
        .set_app_path(&path)
        .set_args(&["--background"])
        .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
        .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
        .set_linux_launch_mode(LinuxLaunchMode::XdgAutostart)
        .build()
        .map_err(|_| {
            Error::new(
                "AUTOSTART_ERROR",
                "Cannot configure this user's login registration",
            )
        })
}

/// None is a read-only status refresh. Some(...) is only supplied after explicit UI input.
pub fn apply(requested: Option<bool>) -> Result<bool> {
    let launcher = launcher(requested == Some(true))?;
    if let Some(enabled) = requested {
        if enabled {
            // Enabling again is also an explicit repair after relocating the installed app.
            launcher
                .enable()
                .map_err(|_| Error::new("AUTOSTART_ERROR", "OS rejected login registration"))?;
        } else {
            launcher.disable().map_err(|_| {
                Error::new(
                    "AUTOSTART_ERROR",
                    "OS rejected removal of login registration",
                )
            })?;
        }
    }
    let actual = launcher.is_enabled().map_err(|_| {
        Error::new(
            "AUTOSTART_ERROR",
            "OS login-registration status is unavailable",
        )
    })?;
    if let Some(expected) = requested {
        require(
            actual == expected,
            "AUTOSTART_ERROR",
            "OS registration did not reach the requested state",
        )?;
    }
    // Registration existence is not a promise that OS/session policy will permit login launch.
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_relative_and_control_paths_without_os_registration() {
        assert!(executable_argument(Path::new("relative/huda")).is_err());
        let bad = if cfg!(windows) {
            "C:\\Huda\nInjected.exe"
        } else {
            "/tmp/Huda\nInjected"
        };
        assert!(executable_argument(Path::new(bad)).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn windows_verbatim_prefix_is_not_written_to_login_registration() {
        assert_eq!(
            executable_argument(Path::new(r"\\?\C:\Users\Test\Huda\huda-desktop.exe")).unwrap(),
            r#""C:\Users\Test\Huda\huda-desktop.exe""#
        );
        assert_eq!(
            executable_argument(Path::new(r"C:\Users\Test Name\Huda\huda-desktop.exe")).unwrap(),
            r#""C:\Users\Test Name\Huda\huda-desktop.exe""#
        );
        assert!(executable_argument(Path::new(r#"C:\Huda\"quoted".exe"#)).is_err());
        assert!(executable_argument(Path::new(r"\\server\share\huda-desktop.exe")).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_spaces_are_one_exec_argument_and_field_codes_are_rejected() {
        assert_eq!(
            executable_argument(Path::new("/opt/Huda App/huda-desktop")).unwrap(),
            "\"/opt/Huda App/huda-desktop\""
        );
        assert!(executable_argument(Path::new("/opt/Huda%F/huda-desktop")).is_err());
    }
}
