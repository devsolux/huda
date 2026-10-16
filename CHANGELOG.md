# Changelog

## Unreleased

### Fixes

- Query Core: a typed connection between seeds that resolve to one entity is no longer covered
  by an empty path; the negated twin of a mid-path step is checked for conflicts; and a hub
  seed whose context exceeds 512 KiB is trimmed and marked `truncated` instead of failing with
  `LIMIT_EXCEEDED`. Path steps and conflict sides always stay, and each slot keeps a basis.
- A committed `assert_relation` request is replayed from its record even after the policy epoch
  moved on. A retried source import is identified by its request inputs, not by data a model
  may extract differently. Records written by earlier builds still replay.
- `add_principal` validates its input, epoch and actor under one store lock before it writes
  the OS secret store. `huda init` and `add_principal` issue fresh credentials instead of
  adopting an entry an earlier profile at the same path left behind.
- Expansion checks its item and article caps before searching, starts a term only when the
  term's worst case fits the request budget, and ends with `exhausted` or `rate_limited`
  (a `partial` job that keeps its report) instead of failing.
- Wikimedia reads stay serial but cancellable while they wait for the limiter. They stop at once
  when a provider backoff outlasts the budget, and they bound each response body by the
  request deadline rather than per read. The local model client bounds its response the same
  way.
- A model input over its byte budget falls back to the deterministic question parser and
  reports `model_input_exceeds_budget` instead of failing the request.
- The code adapter shortens an over-long symbol or file name and digests an oversized key,
  instead of refusing the whole file. Keys that fit are unchanged.
- `huda core` reports readiness only after its IPC listener is bound.
- The desktop shows a startup failure that happens before its window opens in a small window
  of its own, and it reports once when a relaunch marker cannot be read.

### Documentation

- `docs/open-decisions.md` records five confirmed findings that need a maintainer decision:
  an assertion index migration, code-symbol identity, persistent provider backoff, login entries
  for a moved app, and Windows pipe-name squatting.

## 1.1.0 — 2026-10-16

### Fixes

- Provider consent stored by an earlier version stays valid after an upgrade: the stored
  User-Agent keeps the operator contact, and its `Huda/<version>` token names the running
  build. Previously every 1.0.0 policy read back as "not configured" under a new version.
- Relation types are stored under their registered key, so `@01` and `@1` no longer split one
  relation into two identities that hide conflicts from goal coverage.
- `explore`/`search_knowledge` with a query keep matching entities that have no returned
  relation. An `In` condition with no value of the fact's literal type is UNKNOWN, like `Eq`.
- A database error while rechecking a caller is reported as a storage error, not as a revoked
  caller.
- The answer validator checks every marker inside a compound citation such as `[R1, R99]`.
  Model extraction may not attach evidence; technical origin comes from the capture.
- The optional code adapter bounds destructuring-export extraction to 64 nesting levels and 64
  names per declarator, so untrusted input can no longer overflow the Core's stack.
- Provider egress also rejects 192.0.0.0/24, 192.88.99.0/24, 2001::/23 and 6to4 2002::/16.
- The opt-in REST listener survives a client that resets before `accept`, and a Core whose REST
  listener stops shuts down instead of serving IPC alone.
- A filesystem that cannot provide a lock reports `IO_ERROR` instead of claiming that another
  Core or desktop is running. A Core that exits during an on-demand start is reported after one
  second instead of a five-second generic timeout.
- A panicking MCP tool call answers with a JSON-RPC internal error and frees its request slot.
- The desktop offers **Initialize profile** again when a request reports `AUTH_REQUIRED`, for
  example after an interrupted first initialization.

### Dependencies

- rustls 0.23.45 → 0.23.46 (RUSTSEC-2026-0346, -0347, -0348 and -0349, all low severity).
  No other lock entry changed; `cargo audit` is clean.

### Documentation

- Version 1.1.0 across the manifest, Windows application manifest, docs and templates; the
  security policy now supports 1.1.x.
- README notes where `huda` is installed on each OS and builds the "other targets" example for
  the packaged target. Docs record that CI compiles and tests the Windows source (it is still
  not natively qualified), that signing has run on `main`, and that workflow dispatches happen
  on `main`.

### Build, packaging and CI

- Keyless Sigstore signing: once every gate passes, a new `sign` job (the only one with
  `id-token: write`, no code checkout) signs each package and the combined `SHA256SUMS` with
  cosign v3.1.3 and verifies every bundle against the workflow identity. Releases also get one
  `<file>.sigstore.json` per asset. These bundles are not OS code signatures; packages stay
  unsigned/ad-hoc for Gatekeeper/SmartScreen.
- The signing job signs only the packages that the per-target `SHA256SUMS` list, each exactly
  once.
- Release runs are no longer cancelled by a newer run for the same ref; only superseded
  dispatch runs are.
- `actions/download-artifact` v8.0.2 and `sigstore/cosign-installer` v4.1.2, SHA-pinned.

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
