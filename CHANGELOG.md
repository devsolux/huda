# Changelog

## 1.0.0 — 2026-10-14

First Huda release: product **Huda**, Rust package `huda`, executables `huda` and
`huda-desktop`, application identifier `com.devsolux.huda`.

### Core and relation authority

- One per-user Core, one SQLite authority and one writer behind shared application handlers for
  GUI, CLI, MCP and opt-in REST.
- Typed/n-ary relations with qualifiers, polarity, modality, conditions and optional valid time.
  Evidence is optional; authenticated technical origin is mandatory. History is immutable,
  source-owned withdrawal preserves independent assertions, persistent inference stays disabled.
- Query Core owns GoalSpec coverage/needs; one bounded acquisition controller and one local LLM
  adapter. No model is bundled or downloaded; providers are off until explicitly authorized.
- A failed or interrupted first initialization leaves an empty `huda.db` that the next start
  treats as an unused reservation rather than `DB_INCOMPATIBLE`.
- The deterministic question parser rejects malformed questions as `INVALID_INPUT`; "What depends
  on X?" asks for X's dependents.
- Staleness checks read a capture's revision and expiry once per source per query. `explore` and
  snapshot label caps disclose truncation. `CANCELLED`, `GOAL_ALREADY_MET` and
  `TYPE_REGISTRY_CONFLICT` map to HTTP 409. Cancelling a job keeps `committed_sources`.
- Expansion spends no egress on Wikipedia pages it would discard without a local model.

### Runtime and security

- Unix lock/private socket and Windows DACL/SID-verified named pipe with an exclusive lease;
  common bounded authentication/dispatch; malformed or forged IPC envelopes get a redacted error
  frame. Accepted IPC/REST streams are blocking so socket deadlines hold on macOS/BSD. IPC client
  slots are released by a guard; the client tolerates one second of clock skew.
- Owner/MCP credentials live only in the OS secret store; existing profiles and
  `huda.<profile-fingerprint>` secret entries keep their names.
- Implicit HTTP retries are disabled so provider/Bridge budgets cannot be evaded. Provider
  User-Agent validation derives the package version. The Linux Bridge executable digest uses
  `sha256_hex`.

### Interfaces

- MCP `2026-07-28` stdio with `server/discover`, `resultType`, `cacheScope` and `ttlMs`; no
  `initialize` fallback. OpenAPI `3.1.2` contract export. Null/empty optional model tool-call
  fields are accepted; real tool requests stay forbidden.

### Desktop

- eframe 0.36.2 desktop with tray (tray-icon 0.26.1, KSNI on Linux) and opt-in per-user login
  launch (auto-launch 0.6.0), off by default. One desktop per profile through a desktop lease
  independent of the Core writer lease; a second launch hands off to the running window, and a
  stale activation marker never shows the window on a `--background` start.
- Approved icon as PNG 16–1024, ICO and ICNS. Profile initialization is delegated to the
  packaged `huda` helper; the desktop never opens SQLite.

### Optional code adapter

- Tree-sitter syntax parsing behind the `code-adapter` feature (tree-sitter 0.27.1, 19 grammars).
  It emits normally admitted relations and owns no database, scanner or startup work. Dart
  extraction follows dart-orchard 0.7 (`method_invocation`, cascade and `super.` calls, Dart 3.10
  dot shorthand).

### Build, packaging and CI

- Rust 1.99.0, retained `Cargo.lock` (re-resolved 2026-10-12, 541 packages); exact pins listed in
  `docs/dependency-versions.md`.
- `dev.sh` for native Apple Silicon (pinned rustup toolchain, no global installs). Packaging
  produces macOS `.app`/`.dmg`, Windows per-user Setup `.exe` and portable ZIP, and a Linux
  `.deb`; Windows Run-key paths are quoted.
- Four-target SHA-pinned GitHub workflow on manual dispatch and published `v*` releases. A
  release run fails unless the tag equals `v` plus the Cargo version. The final
  `release-assets` job (the only one with `contents: write`) attaches the test packages and one
  combined `SHA256SUMS` to a maintainer-published release and never replaces an existing asset.
- `SECURITY.md`, issue/PR templates and CODEOWNERS.

### Qualification

Executed on native macOS / Apple Silicon only: formatting, strict Clippy, default/headless/
all-feature tests, SQLite tests, locked release build, helper smoke, ad-hoc `.app`/`.dmg`
packaging, an isolated-HOME desktop launch smoke and `cargo audit`. Intel macOS, Windows and Linux
builds, real login/tray/secret-store sessions, signing/notarization and the G0–G8 gates remain
open; packages are unsigned or ad-hoc test candidates. See `docs/verification/gates.md`.
