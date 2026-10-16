# Native desktop build and packaging — Huda 1.0.0

## Prerequisites and reproducibility

Rust 1.99.0, a native C/C++ compiler, Python >=3.11, and one reviewed real Cargo.lock are required. SQLite is bundled. The desktop is egui/eframe 0.36.2 with glow, tray-icon 0.26.1 and auto-launch 0.6.0. The core/headless feature path does not require either desktop integration. Builds never download a model or register login startup.

The repository carries the `Cargo.lock` resolved by Cargo 1.99.0 on 2026-10-09 and re-resolved for the 2026-10-12 dependency upgrade; CI uses it with `--locked` and only resolves a candidate when no lock is committed. Review `cargo tree --locked`, license obligations and advisories before release (`cargo audit` found none on 2026-10-13). Native CI carries the same lock artifact to all four targets, compares its SHA-256 and refuses to replace a conflicting committed lock. No CI job independently updates dependencies.

The tree is rustfmt-formatted with the pinned toolchain and `cargo fmt --all -- --check` passes; keep it that way before committing. Clippy is strict (`-D warnings`) and passes for all targets and features on macOS; no tests are weakened to claim success. Workflow formatting is independent of matrix compilation solely to obtain more diagnostics; a red format job remains a failed workflow run and blocks release qualification. Release runs also fail unless the tag equals `v` plus the `Cargo.toml` version, and uploaded artifact names use that version.

## Apple Silicon / macOS M4

Use a native arm64 terminal, not Rosetta. `./dev.sh --help` lists doctor, lock, fmt, check, test, build, run, bundle, dmg and all. This script targets `aarch64-apple-darwin` explicitly, sets minimum macOS 13.0 and never sets `target-cpu=native`, keeping the app usable across supported Apple Silicon rather than tying output to one M4 CPU. It runs the pinned rustup toolchain's own binaries (resolved through `rustup which`) and verifies the rustfmt/clippy components, so a Homebrew `cargo` earlier on `PATH` cannot silently substitute another compiler. On 2026-10-09 `doctor`, `check`, `test`, `build` and packaging ran successfully on a macOS 26 Apple Silicon host.

`./dev.sh dmg` builds both binaries, puts them in Huda.app/Contents/MacOS, includes the approved ICNS and Cargo identity/version, ad-hoc signs the binaries and bundle, verifies that signature, creates a DMG, and verifies its image. These local test signatures **do not** bypass Gatekeeper or constitute Developer ID/notarization. No signing certificate/account is used by the default scripts. Do not distribute a candidate as signed/notarized; separately authorize/configure Developer ID, hardened runtime, notarization and native smoke qualification for production.

## Intel macOS

On a real Intel Mac with the same toolchain:

```sh
cargo generate-lockfile  # only when no reviewed lock exists
cargo fmt --all -- --check
cargo clippy --locked --target x86_64-apple-darwin --all-targets --all-features -- -D warnings
cargo test --locked --target x86_64-apple-darwin --all-features
MACOSX_DEPLOYMENT_TARGET=13.0 cargo build --locked --release --target x86_64-apple-darwin --features desktop,rest --bins
python3 scripts/package_desktop.py --target x86_64-apple-darwin
```

The CI runner is explicitly `macos-26-intel`; Rosetta is rejected as a substitute for native Intel qualification.

## Windows AMD64

Install the MSVC desktop build tools/Windows SDK, native Rust, Python and **Inno Setup 7.1.0** explicitly. Its commercial licensing conditions are the publisher's; verify your applicable license before commercial use. The workflow checks the immutable installer asset's SHA-256 plus its Authenticode publisher before installing that build tool into the disposable runner. Huda itself never performs this installation.

```powershell
cargo generate-lockfile # only when no reviewed lock exists
cargo fmt --all -- --check
cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings
cargo test --locked --target x86_64-pc-windows-msvc --all-features
cargo build --locked --release --target x86_64-pc-windows-msvc --features desktop,rest --bins
$env:HUDA_ISCC = 'C:\Program Files\Inno Setup 7\ISCC.exe'
python scripts/package_desktop.py --target x86_64-pc-windows-msvc
```

Output includes a portable ZIP and per-user installer `.exe` (not MSI). Both executables carry icon/version/manifest resources. The installer uses lowest privileges and `%LOCALAPPDATA%\Programs\Huda`; it creates a Huda shortcut with AppUserModelID `com.devsolux.huda`. It does not register startup, start the application, install a service, stop a shared Core forcibly or erase the Huda profile on removal. Its uninstaller removes only a matching login entry pointing to that installation; the desktop writes that entry as `"<path>\huda-desktop.exe" --background`, which the uninstaller's quoted-path match recognizes. Quit the app/Core intentionally before replacing in-use executables; no force-close path is added.

The private IPC/DACL implementation is new source and must pass native review and tests before use with real data. Inno packaging does not establish that it is correct. Authenticode signing of **Huda** is not configured; SmartScreen/reputation and real Windows login/Defender/upgrade tests remain release gates.

## Linux AMD64

Build on the fixed Ubuntu 24.04 baseline. The workflow installs C/pkg-config/X11/Wayland/xkbcommon/OpenGL/SSL build dependencies. There is no GTK application loop: tray-icon uses KSNI. Desktop sessions need a StatusNotifier-compatible shell to display a tray and an available unlocked Secret Service for credentials. GNOME without a tray host remains usable through the minimized window; no shell extension is silently installed.

```sh
cargo generate-lockfile # only when no reviewed lock exists
cargo fmt --all -- --check
cargo clippy --locked --target x86_64-unknown-linux-gnu --all-targets --all-features -- -D warnings
cargo test --locked --target x86_64-unknown-linux-gnu --all-features
cargo build --locked --release --target x86_64-unknown-linux-gnu --features desktop,rest --bins
python3 scripts/package_desktop.py --target x86_64-unknown-linux-gnu
```

The `.deb` places both real binaries in `/usr/lib/huda`, with `/usr/bin` symlinks, a correctly named desktop entry, and hicolor PNGs. No maintainer hook starts Huda, registers login startup or deletes user data. Runtime package dependencies are an explicit Ubuntu 24.04 candidate baseline; verify them against the actual ELF `DT_NEEDED`/loader dependencies and a clean install before release. Other distributions/glibc versions are not claimed supported merely because a `.deb` exists. AppImage is not implemented.

## Signing and acceptance boundaries

Build/packaging output is labelled `unqualified` and contains lock/provenance metadata. File hashes detect corruption, not safety, licensing or reproducibility on their own. Artifacts are uploaded only after their native build/test job passes; no GitHub Release is created or published automatically. Once every gate and native job passes, the `sign` job (the only job with `id-token: write`, and without a code checkout) verifies each target's `SHA256SUMS`, requires it to list exactly the downloaded packages, and signs every package plus one combined `SHA256SUMS` keylessly with Sigstore cosign: the job's GitHub OIDC token is exchanged for a short-lived Fulcio certificate naming this workflow, ref and commit, and each signature is recorded in the public Rekor transparency log. It then verifies every bundle against that identity. Dispatch runs sign too, so a branch dispatch exercises the release path; their certificates name the branch, not a release tag. For a release a maintainer publishes, the final `release-assets` job (the only job with `contents: write`, and without a code checkout) re-checks the checksums and attaches the packages, the combined `SHA256SUMS` and one `<file>.sigstore.json` bundle per file. It fails rather than replace an existing asset, so delete stale assets explicitly before re-running a release build. Require **every** workflow job and the plan's native/semantic/security gates before release.

Verify a downloaded release asset with cosign 3.x, pinning the identity to the release tag:

```bash
cosign verify-blob \
  --bundle Huda-1.0.1-x86_64-unknown-linux-gnu.deb.sigstore.json \
  --certificate-identity https://github.com/devsolux/huda/.github/workflows/desktop.yml@refs/tags/v1.0.1 \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  Huda-1.0.1-x86_64-unknown-linux-gnu.deb
```

A valid bundle shows which workflow run produced the file. It is not Developer ID signing, notarization or Authenticode, does not change Gatekeeper/SmartScreen behavior and does not qualify the package. Releases published before signing was added (including `v1.0.0`) carry only `SHA256SUMS`.

On every OS test actual launch, first initialization/secret-store prompt, two simultaneous MCP clients and GUI sharing one writer, read-only queries, login enable/disable/relocation, tray show/close/quit, missing tray host, sleep/resume, cancellation/EOF, backup/WAL crash recovery, upgrade/uninstall data retention and authorized offline/online behavior. Test Windows pipe squatting/different SID/remote rejection, DACL inheritance, long/Unicode paths and cancellation lifetime; test macOS Keychain/LaunchAgent/app translocation and Linux X11/Wayland/Secret Service. Record actual results. On 2026-10-09 only the macOS arm64 launch/relaunch/background-relaunch smoke with an isolated HOME, the helper CLI smoke and ad-hoc packaging were executed; everything else in this list remains open.
