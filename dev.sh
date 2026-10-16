#!/usr/bin/env bash
# Native Apple Silicon development. Compatible with macOS's bundled Bash 3.2.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
cd "$ROOT"
TARGET=aarch64-apple-darwin
FEATURES=desktop,rest
COMMAND="${1:-help}"
[ "$#" -eq 0 ] || shift
usage() {
  cat <<'EOF'
Huda 1.2.0 — native macOS Apple Silicon (M1/M2/M3/M4 and later)
Usage: ./dev.sh {doctor|lock|fmt|check|test|build|run|bundle|dmg|all|help}
  doctor   Check native arm64 host, Xcode CLI tools, Python and pinned Rust.
  lock     Explicitly resolve and validate a real Cargo.lock (network required).
  fmt      Format Rust sources using the pinned toolchain (edits source).
  check    Check formatting and compile/lint all features, with --locked.
  test     Run default, headless and desktop/REST/optional-adapter Rust tests.
  build    Build release huda + huda-desktop for aarch64-apple-darwin.
  run      Run the desktop in debug mode; no provider or login opt-in implied.
  bundle   Build and package an ad-hoc-signed Huda.app for local testing.
  dmg      Build Huda.app + DMG for local testing, not notarized distribution.
  all      Check + test + build + app/DMG packaging.
Existing package output is not overwritten; move prior dist artifacts explicitly.
No sudo, global installs, profile reset, autostart registration or model download.
EOF
}
fail() { printf 'ERROR: %s\n' "$*" >&2; exit 2; }
case "$COMMAND" in help|-h|--help) usage; exit 0;;
  doctor|lock|fmt|check|test|build|run|bundle|dmg|all) ;;
  *) usage >&2; fail "Unknown command: $COMMAND";; esac
[ "$#" -eq 0 ] || fail "Unexpected arguments; no shell-forwarded build or application options are accepted"
[ "$(uname -s)" = Darwin ] || fail 'This script targets native macOS arm64, not Linux/Windows.'
[ "$(uname -m)" = arm64 ] || fail 'Use an arm64 terminal and native Rust; x86_64/Rosetta is not supported here.'
[ "$(sysctl -in sysctl.proc_translated 2>/dev/null || true)" != 1 ] || fail 'Rosetta is not a native arm64 build.'
command -v xcrun >/dev/null 2>&1 || fail 'Install Apple Command Line Tools explicitly before continuing.'
xcrun --find clang >/dev/null
xcrun --sdk macosx --show-sdk-path >/dev/null
command -v python3 >/dev/null 2>&1 || fail 'Python 3.11 or newer is required.'
python3 -c 'import sys; assert sys.version_info >= (3,11), "Python >=3.11 required"'
command -v rustup >/dev/null 2>&1 || fail 'Install native rustup explicitly; this script never downloads a toolchain implicitly.'
TOOLCHAIN="$(python3 -c 'import tomllib; print(tomllib.load(open("rust-toolchain.toml","rb"))["toolchain"]["channel"])')"
# Use an explicit installed toolchain to avoid rustup shims silently installing one.
# grep reads to EOF here: `grep -q` after a pipe can SIGPIPE the producer under pipefail.
rustup toolchain list | grep -E "^${TOOLCHAIN}-aarch64-apple-darwin([[:space:]]|$)" >/dev/null ||
  fail "Install explicitly: rustup toolchain install $TOOLCHAIN --profile minimal --component rustfmt --component clippy"
export RUSTUP_TOOLCHAIN="$TOOLCHAIN-aarch64-apple-darwin"
# A Homebrew or distribution cargo earlier on PATH ignores RUSTUP_TOOLCHAIN; run the pinned
# toolchain's own binaries explicitly instead of whichever cargo/rustc happens to come first.
# Resolve cargo separately: a failing inner substitution would be masked by dirname.
TOOLCHAIN_CARGO="$(rustup which --toolchain "$RUSTUP_TOOLCHAIN" cargo)" ||
  fail "The pinned toolchain $RUSTUP_TOOLCHAIN has no cargo; reinstall it explicitly."
TOOLCHAIN_BIN="$(dirname "$TOOLCHAIN_CARGO")"
export PATH="$TOOLCHAIN_BIN:$PATH"
for component in rustfmt clippy; do
  rustup component list --toolchain "$RUSTUP_TOOLCHAIN" --installed | grep "^$component" >/dev/null ||
    fail "Install explicitly: rustup component add $component --toolchain $RUSTUP_TOOLCHAIN"
done
[ "$(rustc -vV | sed -n 's/^host: //p')" = aarch64-apple-darwin ] || fail 'The selected Rust toolchain is not native arm64.'
export MACOSX_DEPLOYMENT_TARGET=13.0
# Preserve portable M-series output; no target-cpu=native or global CFLAGS changes.
if [ "$COMMAND" = doctor ]; then
  rustc --version; cargo --version; python3 --version
  printf 'Native arm64 prerequisites found. Lockfile: '
  if [ -f Cargo.lock ]; then echo present; else echo 'missing (run ./dev.sh lock explicitly)'; fi
  exit 0
fi
if [ "$COMMAND" = lock ]; then
  if [ -f Cargo.lock ]; then
    echo 'Existing lock preserved; validating it without updating dependencies.'
    cargo metadata --locked --format-version 1 >/dev/null
  else
    cargo generate-lockfile
    cargo metadata --locked --format-version 1 >/dev/null
    echo 'Resolved Cargo.lock. Review and retain it before claiming a reproducible release.'
  fi
  exit 0
fi
if [ "$COMMAND" = fmt ]; then cargo fmt --all; exit 0; fi
[ -f Cargo.lock ] || fail 'Cargo.lock is absent. Run ./dev.sh lock, review it, then retry.'
check() {
  cargo fmt --all -- --check
  cargo clippy --locked --target "$TARGET" --all-targets --all-features -- -D warnings
  PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_sqlite.py
  PYTHONDONTWRITEBYTECODE=1 python3 scripts/test_desktop_assets.py
}
test_project() {
  cargo test --locked --target "$TARGET"
  cargo test --locked --target "$TARGET" --no-default-features
  cargo test --locked --target "$TARGET" --all-features
}
build() { cargo build --locked --release --target "$TARGET" --features "$FEATURES" --bins; }
case "$COMMAND" in
  check) check;;
  test) test_project;;
  build) build;;
  # The desktop starts its sibling huda helper; build both so the helper exists and is current.
  run) cargo build --locked --target "$TARGET" --features "$FEATURES" --bins
    cargo run --locked --target "$TARGET" --features "$FEATURES" --bin huda-desktop;;
  bundle) build; python3 scripts/package_desktop.py --target "$TARGET" --format app;;
  dmg) build; python3 scripts/package_desktop.py --target "$TARGET" --format all;;
  all) check; test_project; build; python3 scripts/package_desktop.py --target "$TARGET" --format all;;
esac
