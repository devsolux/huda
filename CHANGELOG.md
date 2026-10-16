# Changelog

## 1.2.0 — 2026-10-16

### Changed

- A goal context that, together with its relation markers, exceeds 512 KiB is trimmed instead of
  refused with `LIMIT_EXCEEDED`. Relations outside every slot basis go first; path steps,
  conflict sides and one basis per slot stay. The context is marked `truncated`, and
  `goal_met` is false.
- Expansion ends as a `partial` job with stop reason `exhausted`, `rate_limited` or
  `provider_timeout`, keeping its report, when its budget runs out, a provider asks for
  backoff or a provider response times out. Previously the job failed.
- `huda init` and `add_principal` replace a secret-store entry left at the same profile path
  instead of adopting it. A refused request still writes nothing.
- A provider backoff (`Retry-After`, 503, `maxlag`) is kept in the private profile file
  `provider-backoff`, so it outlasts an idle-retired Core.
- Lifting the profile-wide offline switch requires `admin`. A policy holder can no longer
  replace provider consent that belongs to a scope it cannot write.
- Typed output of a Bridge process may not attach evidence, and what it asserts is recorded as
  `extracted`.

### Security

- A source keeps the capture origin of its history. A provider-created source accepts only that
  provider's captures, so an `ingest` principal can no longer publish forged provider facts,
  or withdraw real ones, under the provider-created policy.
- A reviewer's rejection is durable. A refresh of the same statement goes back to review
  instead of reopening it, even under auto-admission.
- Semantic extraction (ingest, Bridge text and Wikipedia expansion) shows the model only the
  entities and labels of the scope its output is published to, not the caller's other scopes.
- `configure_bridge` checks the scope before it examines any path, so its errors are no
  file-existence oracle. Paths under `/proc`, `/sys` and `/dev` are refused.

### Fixes

- Query Core: seeds that resolve to one entity no longer cover a typed connection through an
  empty path, and the negated twin of a mid-path step is checked for conflicts.
- A committed `assert_relation` request is replayed from its record after the policy epoch
  moved on; a revoked caller still gets no replay. A retried source import is identified by
  its request inputs, not by data a model may extract differently. Records written by earlier
  builds still replay.
- `add_principal` validates its input, epoch and actor before the OS secret store is written,
  and issuance is serialized without holding the store lock across a secret-store prompt.
- Expansion checks its item and page caps before searching and starts a term only when its
  worst case fits the request budget. A repeated expansion of an unchanged cached capture
  replays the earlier publication.
- Source admission compares the registered type key, as review does, so `@01` and `@1` are
  admitted alike. A Wikidata string longer than the 4096-byte literal bound is skipped,
  instead of failing the whole item.
- Wikimedia reads stay serial but cancellable while they wait for the limiter, stop at once
  when a backoff outlasts the budget, and bound each response body by the request deadline.
  Policy rechecks during a body read run at most four times a second. The local model client
  bounds its response the same way.
- When the question parser's model input exceeds its byte budget, the deterministic parser is
  used. When an answer's input does, the response reports `model_input_exceeds_budget` instead
  of failing.
- The code adapter shortens an over-long symbol or file name and digests an oversized key,
  instead of refusing the whole file. Keys that fit are unchanged.
- `huda core` reports readiness only after its IPC listener is bound.
- The desktop shows a startup failure that happens before its window opens in a window of its
  own, and reports once when a relaunch marker cannot be read.

### Build and tests

- `scripts/test_desktop_assets.py` checks that the Windows application manifest and the model
  manifest carry the Cargo version.
- Regression tests cover each fix above.

### Documentation

- Version 1.2.0 across the manifest, Windows application manifest, docs and templates; the
  security policy now supports 1.2.x.
- `docs/open-decisions.md` records eight confirmed findings that need a maintainer decision:
  - an assertion index migration;
  - code-symbol identity;
  - login entries for a moved app;
  - Windows pipe-name squatting;
  - capture storage growth;
  - relation types in model prompts;
  - invalid assertions within a batch;
  - entities from proposal-only sources.
- The docs describe trimming, replay, credential replacement, expansion stop reasons and the
  persisted backoff. The cosign example takes the release tag as a variable, and the gate
  matrix and Windows notes record that CI builds, tests and signs all four targets.

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
