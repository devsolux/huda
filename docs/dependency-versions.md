# Dependency and CI version record

First verified against upstream release/documentation sources on 2026-10-09. Upgraded on
**2026-10-12**: every direct dependency was re-checked against crates.io, exact pins moved to
the current stable release, compatible ranges re-resolved with `cargo update`, and the release
notes of every changed crate reviewed (see [2026-10-12 upgrade](#2026-10-12-upgrade)). Cargo
1.99.0 re-resolved the committed `Cargo.lock` (541 packages with crates.io checksums, Cargo lock
format 4); on 2026-10-13 it was formatted, linted with `clippy --all-targets --all-features -D
warnings`, tested (42 default-feature, 42 headless and 392 all-feature library tests plus the
desktop and integration tests) and release-built on native `aarch64-apple-darwin` only. Every
exact pin below was the current stable release on crates.io at resolution time. This is not a
license audit or a four-platform compile; `cargo audit` reported no advisories against the
resolved lock (RustSec database of 2026-10-13), which is a tool finding, not a security
certification.

## Required desktop and shared stack

| Component | Selected version | Primary reference |
|---|---|---|
| Rust | 1.99.0 | https://doc.rust-lang.org/std/ |
| auto-launch | 0.6.0 | https://docs.rs/auto-launch/latest/auto_launch/ |
| tray-icon | 0.26.1 | https://docs.rs/tray-icon/latest/tray_icon/ |
| eframe/egui | 0.36.2 | https://docs.rs/eframe/0.36.2/eframe/trait.App.html |
| reqwest | 0.13.5 | https://docs.rs/reqwest/latest/reqwest/ |
| keyring | 4.2.0, `v1` facade | https://docs.rs/keyring/latest/keyring/v1/ |
| rusqlite | 0.40.2 | https://docs.rs/rusqlite/latest/rusqlite/ |
| serde / serde_json | 1.0.229 / 1.0.154 | https://docs.rs/serde/latest/serde/ · https://docs.rs/serde_json/latest/serde_json/ |
| blake3 / sha2 | 1.8.7 / 0.11.1 | https://docs.rs/blake3/latest/blake3/ · https://docs.rs/sha2/latest/sha2/ |
| getrandom / clap | 0.4.3 / 4.6.7 | https://docs.rs/getrandom/latest/getrandom/ · https://docs.rs/clap/latest/clap/ |
| schemars / httparse | 1.2.3 / 1.10.1 | https://docs.rs/schemars/latest/schemars/ · https://docs.rs/httparse/latest/httparse/ |
| Windows bindings / resources | windows-sys 0.61.2 / winresource 0.1.34 | https://docs.rs/windows-sys/latest/windows_sys/ · https://docs.rs/winresource/latest/winresource/ |
| Test fixtures | tempfile 3.27.0 | https://docs.rs/tempfile/latest/tempfile/ |
| Optional `code-adapter` parser | tree-sitter 0.27.1 (grammars below) | https://github.com/tree-sitter/tree-sitter/releases/tag/v0.27.1 |
| CI Python | 3.14.8 | https://www.python.org/ |
| Windows installer compiler | Inno Setup 7.1.0 | https://github.com/jrsoftware/issrc/releases/tag/is-7_1_0 |

The source accepts Python >=3.11 for developer scripts; the CI runtime selects the verified
stable 3.14.8 rather than a 3.15 release candidate. Linux retains Ubuntu 24.04 as an explicit
ABI baseline, not a claim it is the latest Ubuntu distribution. Runner labels are selected
for native architecture, not merely an unqualified `-latest` string.

Exact dependency pins satisfy the requested minimum auto-launch/tray versions while avoiding
unreviewed future breaking changes. The optional `code-adapter` grammars are **not required by
the shipped desktop feature set**; they use compatible ranges at their current release lines:
javascript, go, python and css 0.25.0, c 0.24.2, php 0.25.1, bash 0.25.1, swift 0.7.4,
markdown 0.5.3, dart-orchard 0.7.0, typescript 0.23.2, rust 0.24.2, java 0.23.5, cpp 0.23.4,
html 0.23.2, c-sharp 0.23.5, kotlin-ng 1.1.0, ruby 0.23.1 and json 0.24.8. `libc` and optional
`anyhow` retain compatible ranges which the real lock must resolve. The package/MCP/OpenAPI/schema version contracts are
not dependency-update knobs; product version remains 1.0.0, MCP 2026-07-28 and OpenAPI 3.1.2.

## GitHub actions: current release commits

| Action | Release | Exact commit |
|---|---|---|
| actions/checkout | v7.0.1 | `3d3c42e5aac5ba805825da76410c181273ba90b1` |
| actions/setup-python | v7.0.0 | `5fda3b95a4ea91299a34e894583c3862153e4b97` |
| actions/upload-artifact | v7.0.2 | `cf430e030ddbb5b0abf93d22962f4752f3646cd9` |
| actions/download-artifact | v8.0.1 | `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` |

Primary references:
- https://github.com/actions/checkout/releases/tag/v7.0.1
- https://github.com/actions/setup-python/releases/tag/v7.0.0
- https://github.com/actions/upload-artifact/releases/tag/v7.0.2
- https://raw.githubusercontent.com/actions/download-artifact/3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c/package.json
- https://raw.githubusercontent.com/actions/download-artifact/main/package.json
- https://docs.github.com/en/actions/reference/runners/github-hosted-runners

All four pinned commits were re-verified on 2026-10-09 against the GitHub tag objects
(`actions/checkout` v7.0.1, `actions/setup-python` v7.0.0, `actions/upload-artifact` v7.0.2,
`actions/download-artifact` v8.0.1, all Node 24 actions). `download-artifact` v8.0.2 was
published on 2026-10-07; the reviewed v8.0.1 pin is retained; adopting it is a manual,
reviewed change. The `digest-mismatch` input exists at the pinned download commit, Python 3.14.8 is
present in the `actions/python-versions` manifest for all four runner platforms, and the
runner labels `macos-26` (arm64), `macos-26-intel` (x64), `windows-2025` and `ubuntu-24.04`
are listed by `actions/runner-images`. This is version-source verification, not an executed
workflow; actionlint was unavailable in the audit environment. Subsequent updates are manual,
reviewed changes; no floating action tags, update bots or automatic merging are used.

## Protocol pins

The MCP `2026-07-28` schema is published upstream (`schema/2026-07-28/schema.json`). It defines
`server/discover`, removes `initialize`, requires `io.modelcontextprotocol/protocolVersion` and
`io.modelcontextprotocol/clientCapabilities` in every request `_meta`, requires `resultType` on
results and `cacheScope`/`ttlMs` on `DiscoverResult` and `ListToolsResult`, and reserves
`-32022` for unsupported protocol versions. Huda's stdio server and Bridge client follow these
definitions; the `cacheScope`/`ttlMs` fields were added during the audit. OpenAPI 3.1.2 is the
published specification at `spec.openapis.org/oas/v3.1.2.html`.

## Compatibility changes applied

The desktop implements current eframe `App::logic` and `App::ui`; hidden tray callbacks request
repaint and close handling only uses fresh UI input. The Linux tray selects KSNI to avoid
introducing an independent GTK event loop. auto-launch explicitly selects CurrentUser,
LaunchAgent and XDG backends rather than its administrative/dynamic defaults.

The one blocking reqwest stack enables the current `query`/`rustls` feature names and disables
implicit retries on every existing client so provider/Bridge accounting remains explicit.
The keyring facade changes call paths, not Huda's persisted credential service or actor names.

Upstream auto-launch 0.6.0 Linux source hardcodes `~/.config` and writes its Exec path directly;
its macOS backend writes plist `<string>` values without XML escaping and its Windows backend
writes `<app_path> <args>` verbatim into the Run value (confirmed by reading the 0.6.0 crate
source). Huda quotes ordinary Linux and Windows paths, rejects unsafe unqualified escaping, and
reports custom XDG config unsupported rather than registering in the wrong location. macOS
XML/path and Windows verbatim-prefix safeguards are in the same small desktop adapter, not a
second startup service. rusqlite 0.40 gates `u64` conversions behind its `fallible_uint`
feature, keyring 4.2's `v1` feature selects the native Keychain, Windows and Secret Service
stores, tray-icon 0.26 requires exactly the `ksni` or `libappindicator` feature on Linux, and
eframe 0.36 drives the desktop through `App::logic`/`App::ui`; all were confirmed from the
resolved crate sources, and the desktop compiles against them on macOS.

No new backend silently enables network access, model downloads, system startup or publishing.

## 2026-10-12 upgrade

Direct pins moved: serde_json 1.0.151 → 1.0.154, sha2 0.11.0 → 0.11.1, schemars 1.2.2 → 1.2.3
and winresource 0.1.31 → 0.1.34. Optional parser: tree-sitter 0.25.10 → 0.27.1, javascript, go
and python 0.23 → 0.25.0, css 0.23.2 → 0.25.0, c 0.23.4 → 0.24.2, php 0.23.11 → 0.25.1, bash
0.23.3 → 0.25.1, swift 0.6.0 → 0.7.4, markdown 0.3.2 → 0.5.3 and dart-orchard 0.3.2 → 0.7.0.
`cargo update` also moved transitive aws-lc-rs 1.18.2 / aws-lc-sys 0.46.0, cc 1.8.0, syn 3.0.9,
uuid 1.28.0, smallvec 1.16.3, litrs 1.0.1 and unicode-ident 1.0.27. No package was added or
removed. Every other direct dependency was already current, and Rust 1.99.0 is the latest
stable release.

sha2 0.11.1, schemars 1.2.3 and winresource 0.1.34 were published on the day of the upgrade.
They were adopted after reading their release notes or source diffs rather than after a
waiting period.

Release-note findings and the changes they required:

- **tree-sitter 0.26/0.27** (breaking Rust API): `Node::{child, named_child}` take `u32` and
  `child_count` returns `u32`, while `named_child_count` still returns `usize`; progress
  callbacks return `ControlFlow<()>`; `set_timeout_micros` and other deprecated functions are
  removed; 0.27 also fixes several memory-safety issues. A crate-local `NamedChildAt` keeps the
  `0..named_child_count()` walks in one index type with a checked conversion, and the parse
  deadline callback returns `ControlFlow::Break` once the deadline passes.
- **tree-sitter-dart-orchard 0.4–0.7** restructured the AST. Calls are `method_invocation`
  nodes instead of a flat `identifier` + `selector(argument_part)` pair, and signatures are
  paired with their bodies in `function_declaration` / `method_declaration`. Call extraction
  handles `method_invocation` and the residual flat form (`getFn()()`), and the doc-comment
  climb includes the new wrappers. Dart symbol names, spans and signatures, which feed
  `code:DEFINED_IN` canonical keys, are unchanged. New are cascade and `super.` call candidates,
  and Dart 3.10 dot-shorthand files that now parse without errors, so the malformed-input
  gate no longer refuses them.
- **Other grammars**: a differential run of the old and new locks over an 88-file,
  19-language corpus produced byte-identical symbols, relations and parse-error status for
  every language except Dart, whose only differences are the improvements above.
- **schemars 1.2.3** puts an internally tagged enum's tag first in its schema. The regenerated
  `huda contracts` output is byte-identical to `docs/verification/openapi-snapshot.json`.
- **winresource 0.1.32–0.1.34** (Windows-only build dependency) speeds up Windows SDK
  discovery, adds per-binary `compile_for` and fixes host-GNU/target-MSVC builds. `compile()`
  still passes the compiled `.res` to every binary through `rustc-link-arg`, under a new file
  name. Not compiled here: there was no Windows host.
- **aws-lc-sys 0.46.0** (via rustls) moves to AWS-LC 5.11.0: non-canonical Ed25519 public keys
  are rejected, ChaCha20-Poly1305 key material is zeroized, and ML-KEM CPU detection no longer
  faults on AVX2-without-BMI2 x86-64 CPUs.
- serde_json, sha2, cc, syn, uuid, smallvec, litrs and unicode-ident: bug fixes and additive
  features only. sha2 raised its MSRV to 1.88, below the pinned 1.99.0.
