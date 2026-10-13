# Huda native packaging

The implementation is `scripts/package_desktop.py`; it consumes already-built native binaries,
Cargo metadata and the approved icon. It refuses wrong architecture, missing helper, missing
lock or existing output. See `docs/desktop-build.md` (from the repository root).

macOS produces an ad-hoc local-test app/DMG, Windows an unsigned per-user Setup EXE and portable
ZIP, Linux a candidate Ubuntu 24.04 DEB. These are build recipes; **no binaries are included in
the source tree**.

Neither signing/notarization nor G0–G8 completion is inferred from packaging. Native tests and
real install/upgrade/uninstall qualification remain mandatory. No release is created or
published automatically; CI only attaches these test packages to a release a maintainer published.
