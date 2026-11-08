# macOS bundles

Use `./dev.sh dmg` for native Apple Silicon, or the explicit Intel commands in
`docs/desktop-build.md` from the repository root. `package_desktop.py` generates Info.plist
from Cargo identity/version, installs both siblings in Contents/MacOS, embeds the approved
ICNS, ad-hoc signs and verifies local-test packages, and creates/verifies a DMG.

Default packages are **not Developer ID signed or notarized**. Move the bundle outside the
DMG/App Translocation before enabling login launch. Deleting Huda.app never authorizes deleting
the profile in `~/.devsolux/huda`. `./dev.sh run` starts the debug build from a development
`target/aarch64-apple-darwin/debug/Huda.app`, so it carries the product's name and identifier as
the packaged app does. Native Keychain/tray/LaunchAgent/install tests remain open.
