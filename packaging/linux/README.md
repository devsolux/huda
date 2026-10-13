# Linux AMD64 native candidate

The build baseline is Ubuntu 24.04 / x86_64-unknown-linux-gnu. The new Python
packager stages real desktop/helper siblings in /usr/lib/huda, creates /usr/bin symlinks,
installs com.devsolux.huda.desktop and hicolor icons, and invokes dpkg-deb. It does not run
post-install hooks, start a Core, register autostart or remove profiles on uninstall.

Tray integration uses KSNI without a GTK application loop. A shell tray host and Secret
Service must be qualified; Linux background mode remains a recoverable minimized window.
Actual ELF dependencies, clean DEB install, X11/Wayland, lock/IPC and desktop login tests are
still required. Custom XDG_CONFIG_HOME autostart is explicitly unsupported by the pinned
upstream auto-launch 0.6.0 implementation, not silently redirected.
