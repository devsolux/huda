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
resolved lock (RustSec database of 2026-10-13), and again after the
[2026-10-16 security update](#2026-10-16-security-update). After the
[2026-11-06 security update](#2026-11-06-security-update) it reported no vulnerabilities and six
warnings, listed there. RUSTSEC-2026-0459, published later against the bundled SQLite, is fixed
by the [2026-11-13 security update](#2026-11-13-security-update). After the
[2026-11-13 compatible updates](#2026-11-13-compatible-updates) three warnings remain, listed
there. This is a tool finding, not a security certification.

## Required desktop and shared stack

| Component | Selected version | Primary reference |
|---|---|---|
| Rust | 1.99.0 | https://doc.rust-lang.org/std/ |
| auto-launch | 0.6.0 | https://docs.rs/auto-launch/latest/auto_launch/ |
| tray-icon | 0.26.1 | https://docs.rs/tray-icon/latest/tray_icon/ |
| eframe/egui | 0.36.2, with the `accesskit` feature | https://docs.rs/eframe/0.36.2/eframe/trait.App.html |
| reqwest | 0.13.5 | https://docs.rs/reqwest/latest/reqwest/ |
| keyring | 4.2.0, `v1` facade | https://docs.rs/keyring/latest/keyring/v1/ |
| rusqlite | 0.42.0, bundling SQLite 3.54.1 | https://docs.rs/rusqlite/latest/rusqlite/ |
| serde / serde_json | 1.0.229 / 1.0.154 | https://docs.rs/serde/latest/serde/ · https://docs.rs/serde_json/latest/serde_json/ |
| blake3 / sha2 | 1.8.7 / 0.11.1 | https://docs.rs/blake3/latest/blake3/ · https://docs.rs/sha2/latest/sha2/ |
| getrandom / clap | 0.4.3 / 4.6.7 | https://docs.rs/getrandom/latest/getrandom/ · https://docs.rs/clap/latest/clap/ |
| schemars / httparse | 1.2.3 / 1.10.1 | https://docs.rs/schemars/latest/schemars/ · https://docs.rs/httparse/latest/httparse/ |
| jsonc-parser / toml_edit | 0.34.0 / 0.25.19 | https://docs.rs/jsonc-parser/latest/jsonc_parser/ · https://docs.rs/toml_edit/latest/toml_edit/ |
| Windows bindings / resources | windows-sys 0.61.2 / winresource 0.1.34 | https://docs.rs/windows-sys/latest/windows_sys/ · https://docs.rs/winresource/latest/winresource/ |
| Test fixtures | tempfile 3.27.0 | https://docs.rs/tempfile/latest/tempfile/ |
| Optional `code-adapter` parser | tree-sitter 0.27.2 (grammars below) | https://github.com/tree-sitter/tree-sitter/releases/tag/v0.27.2 |
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
markdown 0.5.4, dart-orchard 0.7.1, typescript 0.23.2, rust 0.24.2, java 0.23.5, cpp 0.23.4,
html 0.23.2, c-sharp 0.23.5, kotlin-ng 1.1.0, ruby 0.23.1 and json 0.24.8. `libc` and optional
`anyhow` retain compatible ranges which the real lock must resolve. The package/MCP/OpenAPI/schema version contracts are
not dependency-update knobs; product version is 1.3.0, MCP 2026-07-28 and OpenAPI 3.1.2.

## GitHub actions: current release commits

| Action | Release | Exact commit |
|---|---|---|
| actions/checkout | v7.0.1 | `3d3c42e5aac5ba805825da76410c181273ba90b1` |
| actions/setup-python | v7.0.0 | `5fda3b95a4ea91299a34e894583c3862153e4b97` |
| actions/upload-artifact | v7.0.2 | `cf430e030ddbb5b0abf93d22962f4752f3646cd9` |
| actions/download-artifact | v8.0.2 | `9000827ccba6bdab643e8b6fd33ac0654aef8333` |
| sigstore/cosign-installer | v4.1.2 | `6f9f17788090df1f26f669e9d70d6ae9567deba6` |

The installer is told to install cosign **v3.1.3** (`cosign-release`), not its v3.0.6 default.

Primary references:
- https://github.com/actions/checkout/releases/tag/v7.0.1
- https://github.com/actions/setup-python/releases/tag/v7.0.0
- https://github.com/actions/upload-artifact/releases/tag/v7.0.2
- https://github.com/actions/download-artifact/releases/tag/v8.0.2
- https://github.com/sigstore/cosign-installer/releases/tag/v4.1.2
- https://github.com/sigstore/cosign/releases/tag/v3.1.3
- https://docs.github.com/en/actions/reference/runners/github-hosted-runners

On 2026-10-16 each pin above was checked against its repository's latest GitHub release and the
tag's commit from `git ls-remote`; every pin is the latest release. `download-artifact` moved
from v8.0.1 to v8.0.2 (HTTP 429 retry handling, `@actions/artifact` 6.3.1); its `action.yml`
still runs on Node 24 and keeps the `digest-mismatch` input. `cosign-installer` v4.1.2 checks
a requested cosign release with its SHA-256-pinned bootstrap binary, against both the
keyless and the KMS Sigstore bundles that cosign v3.1.3 publishes. Python 3.14.8 is still the
newest 3.14 release in the `actions/python-versions` manifest; 3.15.0 is also listed but is
not adopted here. The runner labels `macos-26` (arm64), `macos-26-intel` (x64), `windows-2025`
and `ubuntu-24.04` were checked against `actions/runner-images` on 2026-10-09, and the
`v1.0.0` release run used them on 2026-10-13. This is version-source verification; the
`sign` job passed on a 2026-10-16 `main` dispatch, the `v1.1.0` release run signed and attached
every package, and actionlint was unavailable in the audit environment.
Subsequent updates are manual, reviewed changes; no floating action tags, update bots or
automatic merging are used.

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
second startup service. rusqlite 0.42, like 0.40, gates `u64` conversions behind its
`fallible_uint` feature, keyring 4.2's `v1` feature selects the native Keychain, Windows and Secret Service
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
  `huda contracts` output was byte-identical to the OpenAPI snapshot recorded at the time.
- **winresource 0.1.32–0.1.34** (Windows-only build dependency) speeds up Windows SDK
  discovery, adds per-binary `compile_for` and fixes host-GNU/target-MSVC builds. `compile()`
  still passes the compiled `.res` to every binary through `rustc-link-arg`, under a new file
  name. Not compiled here: there was no Windows host.
- **aws-lc-sys 0.46.0** (via rustls) moves to AWS-LC 5.11.0: non-canonical Ed25519 public keys
  are rejected, ChaCha20-Poly1305 key material is zeroized, and ML-KEM CPU detection no longer
  faults on AVX2-without-BMI2 x86-64 CPUs.
- serde_json, sha2, cc, syn, uuid, smallvec, litrs and unicode-ident: bug fixes and additive
  features only. sha2 raised its MSRV to 1.88, below the pinned 1.99.0.

## 2026-10-16 security update

`cargo audit` against the RustSec database of 2026-10-16 reported four low-severity advisories
in the transitive **rustls 0.23.45** (via reqwest): RUSTSEC-2026-0346 (`Debug` output of client
session values exposes TLS secrets), RUSTSEC-2026-0347 (buffered data sent to the outer server
after an ECH rejection), RUSTSEC-2026-0348 (Brotli certificate decompression can allocate up to
1 GiB before authentication) and RUSTSEC-2026-0349 (an unauthenticated ServerHello can downgrade
later connections to classical key exchange). `cargo update -p rustls --precise 0.23.46` moved
only that package; the committed lock changes no other entry, and `cargo audit` then reported no
advisories. No direct pin changed.

## 2026-11-06 client configuration editors

`huda mcp clients` edits other products' JSON and TOML files without disturbing comments, order
or formatting, which neither `serde_json` nor a hand-written splice does safely. jsonc-parser
0.34.0 is new to the lock, the current release, MIT, with its `cst` feature only and no
dependencies or build script. toml_edit was already locked as a transitive dependency, so the
direct pin stays at 0.25.17 rather than the newer 0.25.18, and the lock gains no second version;
this is the one pin here that was not the current release when set. Its `display` feature adds
only `toml_writer`, already locked. No other package changed.

## 2026-11-06 security update

`cargo audit` against the RustSec database of 2026-11-06 reported three advisories in transitive
packages, all already in the lock at the previous commit. The lock moves only these three; no
direct pin changed.

| Advisory | Package | Reached through | Fix |
|---|---|---|---|
| RUSTSEC-2026-0382, distrusted intermediates may be accepted | rustls-webpki 0.103.15 → 0.103.16 | reqwest, on provider access | Refuses eight named emSign intermediates anywhere in a chain |
| RUSTSEC-2026-0383, trust in distrusted emSign intermediates | webpki-root-certs 1.0.9 → 1.0.11 | rustls-platform-verifier | Removes three emSign roots |
| RUSTSEC-2026-0385, non-constant-time masks and `u128` comparisons | ctutils 0.4.3 → 0.4.8 | digest, under sha2 and hmac | Hardens mask generation; the `u128` part applies to 32-bit targets only, and Huda ships none |

The published archives were read as source diffs before adoption, and their SHA-256 sums match
the lock. None adds a dependency, a build script or `unsafe`, and ctutils still has no file,
network or process access. ctutils 0.4.4 was the first fixed release. 0.4.8 was taken for its
further hardening of 32-bit targets, and also brings a `primitives` module that Huda does not
call; it was four days old, adopted on that reading rather than after a waiting period.

`cargo update` also moved tempfile's `getrandom` range from 0.4.3 to 0.3.4, unrelated to these
fixes. That one line was set back, and `--locked` accepts the result.

Six warnings remain and are not vulnerability findings: zbus_names, zvariant and zvariant_derive
are unmaintained (RUSTSEC-2026-0371, -0370, -0372); tokio 1.53.2 is marked unsound, because
`AsyncFd::new` and related constructors violate I/O safety (RUSTSEC-2026-0386); async-executor
1.14.0 and clipboard-win 5.4.1 are yanked. All were left as they are.

## 2026-11-06 accessibility bridge

eframe's `accesskit` feature gives the desktop a screen-reader bridge. No pin changed: it adds
`accesskit_winit` 0.32.2 under the existing eframe, which is the only existing package whose
dependencies moved, and nothing was removed. The lock gains 27 packages at the versions eframe
0.36.2 requires, and each platform compiles only its own: `accesskit_macos`, `accesskit_windows`
with the `windows` 0.62 family, or `accesskit_unix` with `atspi`, `zbus_xml`, `zbus-lockstep` and
`phf`. `accesskit_consumer` appears at 0.35, 0.36 and 0.38 because those adapters pin different
lines. The bridge talks only to the local accessibility service, never to the network.

## 2026-11-13 security update

`cargo audit` against the RustSec database of 2026-11-13 reported one vulnerability:
RUSTSEC-2026-0459 (CVE-2026-119375) in the SQLite that libsqlite3-sys 0.38.2 bundles (3.53.2).
Before SQLite 3.54.0, a database opened together with a `-wal` file of a different page size can
return uninitialized page-cache memory to ordinary queries. The vector is local: a crafted
database and `-wal` pair has to be opened. libsqlite3-sys 0.39.0 is the first fixed release and
only rusqlite 0.41 and later depend on it, so the direct pin moves rusqlite 0.40.2 → 0.42.0,
which bundles SQLite 3.54.1 through libsqlite3-sys 0.40.0. `cargo update -p rusqlite --precise
0.42.0` moved four lock entries: rusqlite, libsqlite3-sys, and the wasm-only rsqlite-vfs 0.2.0
and sqlite-wasm-rs 0.6.3, which no native target compiles. Nothing was added or removed.

Release-note findings (0.41.0, 0.41.1 and 0.42.0):

- **Breaking API changes** that Huda does not reach: `Statement::column_names`, `columns` and
  `columns_with_metadata` return iterators, zero blobs take a `u64` length, aggregate and window
  functions need `Send`, and `RawValue` is new. Huda's calls (`pragma_update` and
  `pragma_query_value` on the main schema, `set_limit`, `backup::Backup::step`, transactions
  and queries) keep their signatures, and no source line changed.
- 0.41 checks a pragma name against `pragma_list`, but only with its `extra_check` feature,
  which Huda does not enable; otherwise a name is checked to be an identifier, as before
  (confirmed in the 0.42.0 source, `src/pragma.rs`).
- The `bundled`, `backup`, `limits` and `fallible_uint` features are unchanged, and the default
  features are still `cache` and the wasm-only `ffi-sqlite-wasm-rs`. The bundled build no longer
  sets `SQLITE_ENABLE_MEMORY_MANAGEMENT`, which only `sqlite3_release_memory` uses; Huda never
  calls it. The minimum SQLite is 3.45.3, below the bundled 3.54.1.
- `Connection::backup` now takes a progress callback like `restore` (`NO_PROGRESS` for none).
  Huda copies with `backup::Backup::new` and `step`, whose code is unchanged apart from a
  pattern.
- Fixes: use-after-free from safe code in `trace_v2`, `busy_handler` and `Session::apply`,
  `Connection::serialize`, a panic in zero-argument deterministic scalar functions, and leaks on
  error paths. Huda uses `busy_timeout`, not `busy_handler`, and none of the others.

`cargo audit` then reports no vulnerabilities. With the pinned Rust 1.99.0 on native
`aarch64-apple-darwin`, `make check` passed: formatting, Clippy with `-D warnings` for all targets
and features, the SQL and desktop-asset scripts, and the default (150 library tests), headless
(149) and all-feature (502 library and 60 desktop) test runs, each with the 6 interface-contract
and 10 MCP-client integration tests.

## 2026-11-13 compatible updates

The toml_edit pin moves 0.25.17 → 0.25.19, the current release; `cargo update` moves the locked
copy too, so the lock still holds one toml_edit. Its 0.25.18 and 0.25.19 releases only speed up
serialization and report a deserializer `size_hint`; the `mcp_clients` tests, which edit TOML
client files and read them back, pass. `cargo update` then moved 78 packages within their
compatible ranges, 19 of them by a minor version, and nothing else needed setting back (tempfile
keeps `getrandom` 0.4.3). Every other direct pin was already current; eframe 0.37 is a breaking
release and is not part of this step.

It adds three packages, none with a build script or file, network or process access, each by a
known maintainer and MIT or Apache-2.0: **arc-swap** 1.9.5 and **crossbeam-queue** 0.3.14, which
ksni 0.3.7 (the Linux tray) now uses, and **utf_types** 1.0.0, which utf8_iter 1.1.0 (under
`url`/`idna`) now uses. Their own dependencies, rustversion and crossbeam-utils, were already
locked. The `windows-sys` 0.60 family (ten packages) is removed: nothing requires it any more.

Release-note findings:

- **tokio** 1.53.2 → 1.54.1 (under reqwest) includes the 1.53.3 fix for RUSTSEC-2026-0386
  (`AsyncFd::new` I/O safety) and raises its MSRV to 1.85; 1.54.1 reverts a timer change of
  1.54.0. **rustix** 1.1.6 fixes RUSTSEC-2026-0486 (xattr reads of a zero-length buffer).
  async-executor 1.14.1 and clipboard-win 5.4.3 replace the two yanked releases.
- **webpki-root-certs** 1.0.12 removes seven roots that Mozilla distrusted or limited to email:
  Buypass Class 2 and Class 3, DigiCert Trusted Root G4, QuoVadis Root CA 1 G3 and 3 G3, and
  Certigna Root CA. **rustls-webpki** 0.103.17 and **rustls-pki-types** 1.16 add accessors only.
  **aws-lc-sys** 0.47.0 moves to AWS-LC 5.12.0 under **aws-lc-rs** 1.19.0, which adds cipher
  modes, a self-test API and `Clone` for key pairs.
- **cc** 1.8.0 → 1.11.2 builds the bundled SQLite and the tree-sitter grammars. The compile
  commands for tree-sitter's `lib.c` and the Rust grammar's `parser.c` were compared under both
  versions and are identical on macOS; 1.11.0 passes `-march=x86-64` to GCC on x86-64 Linux,
  which only names the baseline instruction set. 1.11.2 was published on the day of this update.
- **tree-sitter** 0.27.2, **tree-sitter-md** 0.5.4 and **tree-sitter-dart-orchard** 0.7.1:
  patch releases of the optional parser.
- **icu** 2.4 (under `idna`, which maps host names in URLs) moves its data to Unicode 18 and
  CLDR 49, so internationalized host names follow the newer mapping tables.
- uuid 1.29 (`NonNilUuid` constructors), toml 1.2 (serializer speed; Windows resource build
  only), arboard 3.7 (clipboard error and `text/uri-list` fixes), async-lock 3.5 and zcheapstr
  1.2 add features and fixes. utf8_iter 1.1 moves its UTF-8 sequence types into utf_types.

`cargo audit` then reports no vulnerabilities and three warnings: zbus_names, zvariant and
zvariant_derive are unmaintained (RUSTSEC-2026-0371, -0370, -0372). They come with the Linux
tray and accessibility bus and were left as they are. `make check` passed with Rust 1.99.0 on
native `aarch64-apple-darwin`, with the same test counts as above. In the first runs, under a
load average near 30 from unrelated processes on a 10-core machine, the all-feature test
`rust_cfg_nested_past_the_cap_is_production_and_does_not_overflow` hit its 5-second parse
deadline. With the same lock it then passed in about one second in seven runs, including a full
`make check`, with tree-sitter 0.27.1 or 0.27.2 and cc 1.8.0 or 1.11.2 alike.
