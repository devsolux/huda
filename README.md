# Huda

<img src="assets/icons/huda-128.png" alt="Huda application icon" width="96" height="96">

**Local, Tree-sitter-powered, and relation-first knowledge space.** Product **Huda** · application identifier **`com.devsolux.huda`** · Rust package `huda` · native executables `huda` and `huda-desktop`.

> **Version 1.0.0 — unsigned, with native build evidence on Apple Silicon only.** The 2026-10-09 audit resolved a real `Cargo.lock`, made the tree compile on Rust 1.99.0, formatted it, and executed strict Clippy, all three test configurations, a locked release build of both executables, helper smoke, ad-hoc app/DMG packaging and an isolated-HOME desktop launch on native `aarch64-apple-darwin`. Intel macOS, Windows and Linux builds, real login registration, tray interaction, secret-store initialization, signing and the plan's native/semantic/provider gates remain unexecuted; see [verification](docs/verification/commands.json) and the [audit register](docs/verification/audit-2026-10-09.md).

## Native macOS M4 / Apple Silicon

Install Apple Command Line Tools, native Rust/rustup and Python 3.11+ explicitly. `dev.sh` does not install global tools, invoke sudo, register startup, delete profiles or download models. It rejects Rosetta and non-arm64 hosts, and it runs the pinned rustup toolchain's own `cargo`/`rustc` even when a Homebrew or distribution Rust comes first on `PATH`.

```sh
chmod +x dev.sh
rustup toolchain install 1.99.0 --profile minimal --component rustfmt --component clippy
./dev.sh doctor
./dev.sh lock       # explicit network dependency resolution; inspect/retain Cargo.lock
./dev.sh fmt        # rustfmt with the pinned toolchain; the tree is formatted and checked in CI
./dev.sh check      # formatting, strict Clippy and source/SQLite checks
./dev.sh test       # default, headless and all-feature Rust tests
./dev.sh build      # native aarch64-apple-darwin, desktop + helper
./dev.sh run        # debug desktop
./dev.sh dmg        # release .app + .dmg, ad-hoc signed for local testing only
```

`./dev.sh all` performs check → test → build → packaging. Artifacts go under `dist/aarch64-apple-darwin/`. Existing packages are never silently replaced: move previous output before packaging again. The app contains both sibling executables in `Contents/MacOS/`. DMG/ad-hoc signing does **not** constitute Developer ID signing/notarization. Move Huda.app to Applications before enabling login launch.

## All native targets and GitHub builds

| Target | Runner | Native package output configured in CI |
|---|---|---|
| Apple Silicon | `macos-26` / `aarch64-apple-darwin` | `.app` staging and `.dmg` containing it |
| Intel macOS | `macos-26-intel` / `x86_64-apple-darwin` | Separate `.app` staging and `.dmg` |
| Windows AMD64 | `windows-2025` / `x86_64-pc-windows-msvc` | Per-user Setup `.exe` and portable ZIP |
| Linux AMD64 | `ubuntu-24.04` / `x86_64-unknown-linux-gnu` | `.deb`, fixed Ubuntu 24.04/glibc baseline |

[`.github/workflows/desktop.yml`](.github/workflows/desktop.yml) runs only on manual dispatch and on published GitHub releases whose tag starts with `v` (for example `v1.0.0`). Pull requests and branch pushes do not trigger it, so run `./dev.sh check` and `./dev.sh test` locally, or dispatch the workflow on the branch, before merging. It resolves **one** dependency lock when absent, shares and verifies that exact lock across native jobs, checks all feature sets with `--locked`, builds both binaries, runs displayless helper smoke checks and packages only after those native job checks pass. Formatting is a separate mandatory gate so build diagnostics can still be collected when format debt exists. A release run fails unless its tag equals `v` plus the `Cargo.toml` version. All jobs must pass on the dispatched branch or release tag before release qualification.

Actions are pinned to verified current release commits; Rust is pinned to 1.99.0. No dependency-update bot is configured; Cargo and action updates are manual, reviewed changes and are never merged automatically. The workflow never creates or publishes a release. After every job passes, a `sign` job without a code checkout signs each package and one combined `SHA256SUMS` keylessly with [Sigstore cosign](https://docs.sigstore.dev/cosign/) and verifies every signature; for a release a maintainer publishes, a final job attaches the packages, `SHA256SUMS` and their `.sigstore.json` bundles, and it never replaces existing assets. The bundles prove which workflow, ref and commit produced a file; they are not Developer ID, notarization or Authenticode signatures, so the packages remain unsigned/ad-hoc test candidates for the OS. No signing secrets or provider/model calls are configured. The repository contains the real `Cargo.lock` that Cargo 1.99.0 resolved on 2026-10-09 and re-resolved for the 2026-10-12 dependency upgrade (541 packages with crates.io checksums); CI consumes it with `--locked` and only resolves a candidate lock when none is committed. Compiled packages are not included. See [build operations](docs/desktop-build.md) and [dependency verification](docs/dependency-versions.md).

## Icon, tray and login launch

The earlier approved blue H/network artwork is preserved as `assets/icons/source.png`. PNG sizes 16–1024, multi-size ICO and ICNS are included; normal builds do not need Pillow or regenerate the icon. `scripts/generate_icons.py` reproduces the conversions when Pillow is explicitly available.

The desktop uses **auto-launch 0.6.0** and **tray-icon 0.26.1**. Launch at login starts **off**. Enable/disable it explicitly from the desktop checkbox or tray menu. Windows registration is per-user, macOS uses a LaunchAgent, and Linux uses XDG session startup. No installer silently registers startup or starts a Core.

The tray offers Show Huda, Ask Huda, Launch at login and Quit Huda. Keep running after close is session-only and off on normal launch. Login starts use `--background`; Linux deliberately remains recoverable as a minimized window when its shell may not display a tray icon. Tray and worker events wake the hidden UI. Quitting the desktop never terminates a shared Core serving another client.

**Known platform limits:** auto-launch 0.6.0 does not honor a custom Linux `XDG_CONFIG_HOME`; Huda reports that unsupported configuration instead of creating an ineffective entry. Unrepresentable launch paths are rejected; Windows Run-key paths are quoted. A registered OS login entry is not proof the operating system permits it to run. One desktop runs per profile: a second manual launch exits and asks the running desktop to show its window through a per-profile desktop lease that is independent of the Core writer lease; a second `--background` launch exits silently. An isolated-HOME launch/relaunch smoke ran on Apple Silicon; real login registration, tray interaction and display tests on the other targets remain unexecuted.

## Architecture and data continuity

GUI / CLI / MCP / opt-in REST → shared application handlers → **one per-user Core, one SQLite authority and one writer**.

Manual assertions, JSON, approved extraction, provider captures and Bridge captures use the same publication boundary. Query Core alone evaluates typed GoalSpec coverage/needs. One bounded acquisition controller consumes those results; one local LLM adapter handles distinct interpretation/extraction/answer tasks. No second graph, model judge, cloud service, agent or provider-proxy MCP is added.

Relations have typed/n-ary roles, qualifiers, polarity, modality, conditions and optional valid time. **Evidence may be empty; authenticated technical origin is required.** Published `active` status is not a proof of truth. Source-owned withdrawal preserves independent assertions and immutable history. Persistent inferred assertions remain disabled.

Default profile locations (credentials are stored as `huda.<profile-fingerprint>` OS-secret entries):

| OS | Default profile |
|---|---|
| macOS | `~/Library/Application Support/Huda` |
| Windows | `%LOCALAPPDATA%\Huda` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/huda` |

Windows now has restrictive DACL/local named-pipe/peer-SID source at the existing runtime boundary; it is **not yet native-security-qualified**. Use disposable profiles while qualifying.

## Explicit offline startup after a successful build

Install both executables beside one another (the packaging scripts already do this), then:

```sh
huda init
huda status
huda-desktop
huda mcp stdio
```

The desktop also offers **Initialize profile**, which explicitly delegates to its packaged helper. It never opens SQLite. Initialization creates owner/MCP credentials in an available unlocked OS secret store. No plaintext fallback exists. Ordinary requests start one Core on demand; no request means no hidden scan, model download or provider fetch. The Core retires after 300 idle seconds. `huda core --idle-seconds 0` explicitly holds it in the foreground.

```sh
huda call list_relation_types --input examples/empty.json
huda call explore_relations --input examples/empty.json
huda ask "What does Project A depend on in production?"
huda contracts --output openapi.json
```

The authored [manual gold runner](examples/manual_gold.py) creates an evidence-free direct relation plus an independent JSON assertion, withdraws the JSON source and checks the surviving user relation. Run it only against a fresh test profile initialized with the same `--profile` path. It has not been executed against Rust here.

No model is bundled. Independently configure a local model using [the example](examples/local-model.example.json), then explicitly run `huda core --idle-seconds 0 --model-config /absolute/path/to/local-model.json`. Without it, graph operations remain available; the app does not invent an AI answer. Null/empty optional tool-call fields are accepted, but actual model tool requests remain forbidden.

Provider egress and registration/structured/semantic admission are independently authorized, off by default. Context is not consent. Protocol targets remain MCP `2026-07-28` and OpenAPI `3.1.2`; both were checked against the published MCP schema and OpenAPI specification on 2026-10-09, and the stdio server now returns the `cacheScope`/`ttlMs` fields that revision requires. MCP HTTP, some API routes and remote/macOS/Windows Bridge backends remain incomplete. See [operations](docs/operations.md).

## Scope and evidence

Huda ships no code-index runtime, file watcher, model download, npm/Claude-plugin distribution or destructive recovery. Syntax parsing is **adapted, attributed and optional** behind `code-adapter` (see [NOTICE](NOTICE.md)); it has no independent database, filesystem scanner or code-first startup.

Actual executed checks on native macOS 26 / Apple Silicon with Rust 1.99.0: `cargo fmt --check`, `cargo clippy --all-targets --all-features -D warnings`, **36 default-feature, 36 headless and 384 all-feature Rust tests** plus integration tests, a locked release build of `huda` and `huda-desktop`, helper `--help/--version` smoke, ad-hoc-signed `.app`/`.dmg` packaging, an isolated-HOME desktop launch/relaunch smoke, `cargo audit` (no advisories), **19 SQLite tests**, **15 offline desktop/packaging checks** and shell/YAML validation. Exact commands, environment, failures and blocked checks are in [verification](docs/verification/commands.json).

[Gate matrix](docs/verification/gates.md) · [Architecture](docs/architecture.md) · [Threat model](docs/threat-model.md)
