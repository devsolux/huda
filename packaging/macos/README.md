# macOS bundles

Use `./dev.sh dmg` for native Apple Silicon, or the explicit Intel commands in
`docs/desktop-build.md` from the repository root. `package_desktop.py` generates Info.plist
from Cargo identity/version, installs both siblings in Contents/MacOS, embeds the approved
ICNS, ad-hoc signs and verifies local-test packages, and creates/verifies a DMG.

Default packages are **not Developer ID signed or notarized**. Move the bundle outside the
DMG/App Translocation before enabling login launch. Deleting Huda.app never authorizes deleting
`~/Library/Application Support/Huda`. Native Keychain/tray/LaunchAgent/install tests remain open.
