# Windows AMD64 native candidate

The runtime now contains private DACL/local named-pipe/SID-verification source; the earlier
blanket unsupported-platform stub is removed. This does not establish a successful Windows
build or native security qualification. Native MSVC tests are a required workflow gate.

`Huda.iss` builds a per-user, non-administrative installer using Inno Setup 7.1.0. Product
version/identifier/staging paths are supplied by `scripts/package_desktop.py` from Cargo
metadata. The approved icon and asInvoker/DPI/long-path manifest are embedded by build.rs.
An additional portable ZIP keeps both executables together.

No startup registration or launch happens during install. The desktop registers login launch
as `"<install dir>\huda-desktop.exe" --background` (quoted, because auto-launch 0.6.0 writes
the Run value verbatim). Uninstall preserves all relation profile data and removes only this
install's own matching HKCU login entry. Signing Huda,
SmartScreen/Defender, Unicode/long paths and real install/upgrade tests remain unqualified.
