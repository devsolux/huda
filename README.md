<p align="center">
  <img src="assets/icons/huda-128.png" alt="Huda application icon" width="96" height="96">
</p>

<h1 align="center">Huda</h1>

<p align="center">
  <strong>A local, relation-first knowledge space.</strong><br>
  Typed n-ary relations with immutable history in one SQLite store.<br>
  Native desktop, CLI, MCP and REST — offline by default.
</p>

<p align="center">
  <a href="https://github.com/devsolux/huda/actions/workflows/desktop.yml"><img src="https://github.com/devsolux/huda/actions/workflows/desktop.yml/badge.svg" alt="Native desktop workflow"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT license"></a>
  <a href="rust-toolchain.toml"><img src="https://img.shields.io/badge/rust-1.99.0-orange.svg" alt="Rust 1.99.0"></a>
  <a href="https://github.com/devsolux/huda/releases"><img src="https://img.shields.io/github/v/release/devsolux/huda" alt="Latest release"></a>
</p>

---

## Overview

Huda stores knowledge as **typed relations**, not documents. Every relation has typed roles,
qualifiers, polarity, modality, conditions and an optional valid time. Every assertion carries an
authenticated technical origin; supporting evidence is optional. History is immutable.

One per-user **Core** owns one SQLite database with a single writer. The native desktop, the
CLI, the MCP server and the opt-in REST server all go through the same application handlers, so
every interface sees the same data and the same rules.

Huda runs entirely on your machine. It does not scan your files, download models or contact
external services unless you explicitly configure and authorize it.

| | |
|---|---|
| Product | Huda |
| Executables | `huda` (CLI, Core, MCP) and `huda-desktop` (native desktop) |
| Application identifier | `com.devsolux.huda` |
| Rust package | `huda` |

## Features

- **Relation-first model** — typed, n-ary relations with qualifiers, polarity, modality,
  conditions and optional valid time. See the [relation model](docs/relation-model.md).
- **Immutable history** — withdrawing a source preserves independent assertions; nothing is
  silently overwritten or deleted.
- **One authority** — one Core, one SQLite store and one writer per profile, shared by every
  interface.
- **Goal-aware queries** — Query Core evaluates typed goal coverage and returns bounded, typed
  paths without persisting inferences.
- **Offline by default** — provider access, automatic admission and local models are separate,
  explicit opt-ins.
- **Optional local LLM** — a single adapter for a model you configure yourself; graph operations
  work without one.
- **Model Context Protocol** — a stdio MCP server targeting MCP `2026-07-28`, and OpenAPI `3.1.2`
  contracts generated from the handler types.
- **Native desktop** — an egui application with a tray menu, opt-in launch at login and
  single-instance activation.
- **Optional code adapter** — Tree-sitter syntax parsing behind the `code-adapter` feature (see
  [NOTICE](NOTICE.md)).

## Project status

Huda is a **source candidate**, not a security-qualified release. Use a disposable profile while
evaluating it.

- The [CI workflow](.github/workflows/desktop.yml) builds, lints, tests and packages all four
  supported targets.
- Native installation, login registration, tray, secret-store and upgrade testing is incomplete.
  So far, only an isolated desktop launch smoke test on Apple Silicon has been run.
- The Windows named-pipe access controls are implemented but not yet natively qualified.
- Packages are not Developer ID–signed, notarized or Authenticode-signed, so Gatekeeper and
  SmartScreen will warn on first launch.
- MCP over HTTP, some API routes and the remote, macOS and Windows Bridge backends are incomplete.

The [gate matrix](docs/verification/gates.md) tracks every release gate and the evidence behind it.

## Supported platforms

| Platform | Target | Package |
|---|---|---|
| macOS, Apple Silicon | `aarch64-apple-darwin` | `.dmg` containing `Huda.app` |
| macOS, Intel | `x86_64-apple-darwin` | `.dmg` containing `Huda.app` |
| Windows, x64 | `x86_64-pc-windows-msvc` | Per-user setup `.exe` and portable `.zip` |
| Linux, x64 | `x86_64-unknown-linux-gnu` | `.deb` built against the Ubuntu 24.04 baseline |

## Installation

Download the package for your platform from
[GitHub Releases](https://github.com/devsolux/huda/releases), then verify it against the
release's `SHA256SUMS`. On Linux:

```bash
sha256sum --check --ignore-missing SHA256SUMS
```

On macOS:

```bash
shasum -a 256 --check --ignore-missing SHA256SUMS
```

On Windows, compare the output of `Get-FileHash -Algorithm SHA256 <file>` in PowerShell with the
matching line in `SHA256SUMS`.

Newer releases also include a [Sigstore](https://docs.sigstore.dev/cosign/) bundle
(`<file>.sigstore.json`) for each asset. Set `TAG` to the release tag and `FILE` to the
downloaded package, then verify it with cosign 3.x:

```bash
cosign verify-blob --bundle "$FILE.sigstore.json" --certificate-identity "https://github.com/devsolux/huda/.github/workflows/desktop.yml@refs/tags/$TAG" --certificate-oidc-issuer https://token.actions.githubusercontent.com "$FILE"
```

A valid bundle proves which workflow run built the file. It is not an operating-system code
signature. Releases published before signing was introduced include only `SHA256SUMS`.

On macOS, move `Huda.app` to Applications before enabling launch at login.

## Quick start

The packages install `huda` and `huda-desktop` side by side, which both executables require.

```bash
huda init         # create an empty offline profile and its credentials
huda status       # start the Core on demand and report its state
huda-desktop      # open the native desktop
huda mcp stdio    # serve MCP over stdio
```

The desktop can also initialize a profile with **Initialize profile**. Credentials live only in
an unlocked OS secret store; there is no plaintext fallback.

Requests start one Core on demand. The Core never scans, downloads or fetches anything on its
own, and it exits after 300 idle seconds. `huda core --idle-seconds 0` keeps it in the
foreground.

```bash
huda call list_relation_types --input examples/empty.json
huda call explore_relations --input examples/empty.json
huda ask "What does Project A depend on in production?"
huda contracts --output openapi.json
```

`huda contracts` never overwrites an existing file. Use `--profile <dir>` with any command to
work with a profile other than the default. See [examples](examples/README.md) for the
sample inputs.

## Configuration

### Profiles

| OS | Default profile directory |
|---|---|
| macOS | `~/Library/Application Support/Huda` |
| Windows | `%LOCALAPPDATA%\Huda` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/huda` |

Each profile's credentials are stored as `huda.<profile-fingerprint>` entries in the OS secret
store.

### Local model (optional)

No model is bundled or downloaded. Create a configuration from
[the example](examples/local-model.example.json), which stays invalid until you supply the
model's identity, digest and license, then start the Core with it:

```bash
huda core --idle-seconds 0 --model-config /absolute/path/to/local-model.json
```

The adapter only connects to a loopback endpoint (`127.0.0.1:<port>/v1/chat/completions`) and
rejects model tool calls. Without a model, graph operations stay available, and Huda never
fabricates an answer.

### Providers and REST

Provider access and automatic admission are authorized independently and are off by default.
The REST server is opt-in, listens only on `127.0.0.1`, and requires a build with the `rest`
feature (included in release packages):

```bash
huda core --rest-port 8787
```

See [operations](docs/operations.md) for credentials, backup, recovery and provider consent.

## Desktop integration

- **Tray menu:** Show Huda, Ask Huda, Launch at login and Quit Huda.
- **Launch at login** is off by default and enabled only from the desktop or the tray. It uses a
  per-user Run key on Windows, a LaunchAgent on macOS and XDG autostart on Linux. Installers never
  register startup.
- **Single instance:** one desktop runs per profile. A second launch brings the running window to
  the front, and a second `--background` launch exits silently.
- **Shared Core:** quitting the desktop never stops a Core that is serving another client.

Known limitations:

- Launch at login does not support a custom Linux `XDG_CONFIG_HOME`. Huda reports this instead of
  writing an entry that would not work.
- On Linux shells without a tray host, the desktop stays available as a minimized window.
- A registered login entry does not guarantee that the operating system will run it.

## Architecture

```text
Desktop · CLI · MCP (stdio) · REST (opt-in, loopback)
                       │
          shared application handlers
                       │
           one per-user Core, one writer
                       │
                SQLite authority
```

Manual assertions, JSON imports, approved extraction, provider captures and Bridge captures all
pass through the same publication boundary. Query Core alone evaluates goal coverage. One bounded
acquisition controller acts on its results, and one local LLM adapter handles interpretation,
extraction and answers.

Huda deliberately has no second graph, vector or code index, file watcher, cloud service, agent
layer or destructive recovery path. See [architecture](docs/architecture.md) for the details.

## Building from source

**Requirements:**

- Rust 1.99.0 with `rustfmt` and `clippy`; the version is pinned in
  [`rust-toolchain.toml`](rust-toolchain.toml)
- Python 3.11 or later
- A native C/C++ toolchain for your platform

The committed `Cargo.lock` is used with `--locked` everywhere. SQLite is bundled.

### macOS (Apple Silicon)

[`dev.sh`](dev.sh) wraps the full workflow for native `aarch64-apple-darwin`. It never installs
global tools, runs `sudo`, registers startup, deletes profiles or downloads models. It also
always runs the pinned toolchain, even when another Rust comes first on `PATH`.

```bash
rustup toolchain install 1.99.0 --profile minimal --component rustfmt --component clippy
./dev.sh doctor   # check the host and toolchain
./dev.sh check    # formatting, strict Clippy, source and SQLite checks
./dev.sh test     # default, headless and all-feature tests
./dev.sh build    # desktop and helper binaries
./dev.sh run      # debug desktop
./dev.sh dmg      # release .app and .dmg, ad-hoc signed for local testing
./dev.sh all      # check, test, build and package
```

Packages are written to `dist/aarch64-apple-darwin/`, and existing packages are never
overwritten.

### Other targets

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --release --features desktop,rest --bins
python3 scripts/package_desktop.py --target x86_64-unknown-linux-gnu
```

The platform prerequisites are covered in [build and packaging](docs/desktop-build.md):

- Apple Command Line Tools on macOS
- MSVC build tools and Inno Setup 7.1.0 on Windows
- the system libraries on Ubuntu 24.04

### Continuous integration

[`desktop.yml`](.github/workflows/desktop.yml) runs on manual dispatch and on published releases
whose tag is `v` followed by the `Cargo.toml` version. It does not run on pushes or pull
requests, so run the checks above locally first. Each run:

1. Verifies the committed dependency lock and shares it with every job.
2. Lints, tests, builds and packages all four targets, with formatting as a separate gate.
3. Signs every package and the combined `SHA256SUMS` keylessly with Sigstore cosign, then
   verifies each signature.
4. For a published release, attaches the packages, checksums and signature bundles. It never
   replaces existing assets.

Actions are pinned to release commits and updated manually. See
[dependency versions](docs/dependency-versions.md) for the pinned versions.

## Documentation

| Document | Contents |
|---|---|
| [Architecture](docs/architecture.md) | Authority, schema choices, source lifecycle and deliberate limits |
| [Relation model](docs/relation-model.md) | Entity and relation identity, roles, qualifiers and canonicalization |
| [Operations](docs/operations.md) | Profiles, credentials, Core lifetime, backup and recovery |
| [API contracts](docs/api/README.md) | Handler, REST and MCP operation contracts |
| [Build and packaging](docs/desktop-build.md) | Per-platform build, packaging and signing boundaries |
| [Threat model](docs/threat-model.md) | Protected authority, controls and open security obligations |
| [Gate matrix](docs/verification/gates.md) | Release gates and their recorded evidence |
| [Evaluations](docs/evaluations.md) | Evaluation contract for native and semantic gates |
| [Dependency versions](docs/dependency-versions.md) | Pinned crates, actions and protocol versions |

## Security

Please report vulnerabilities privately through
[GitHub private vulnerability reporting](https://github.com/devsolux/huda/security/advisories/new),
not in a public issue. See the [security policy](SECURITY.md) and the
[threat model](docs/threat-model.md).

## Contributing

Read [AGENTS.md](AGENTS.md) for the engineering rules and architecture invariants. Run the checks
listed in the [pull request template](.github/PULL_REQUEST_TEMPLATE.md), and report failed or
skipped checks as such.

## License

Huda is released under the [MIT License](LICENSE). Third-party attributions, including the adapted
Tree-sitter code, are listed in [NOTICE](NOTICE.md).
