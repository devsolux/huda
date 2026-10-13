#!/bin/sh
# Run only in a disposable qualification environment with a reviewed real Cargo.lock.
set -eu
cd "$(dirname "$0")/.."
if [ ! -f Cargo.lock ]; then
  printf '%s\n' 'BLOCKED: Cargo.lock is absent. Explicitly resolve, inspect and retain a real lock first.' >&2
  exit 2
fi
command -v cargo >/dev/null 2>&1 || { echo 'BLOCKED: Cargo is unavailable.' >&2; exit 2; }
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --no-default-features
cargo test --locked --all-features
cargo build --locked --release --features desktop,rest,code-adapter
python3 scripts/check_sqlite.py
python3 scripts/test_desktop_assets.py
# Real native/security/model/provider/interop/installer gates remain separate.
