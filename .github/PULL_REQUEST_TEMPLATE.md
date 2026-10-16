## Summary

<!-- What changes and why. Link the issue if there is one. -->

## Verification

The native workflow runs only on manual dispatch and published `v*` releases, so run the
checks locally (maintainers can dispatch the workflow on `main`) and paste the real results.

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --locked --all-targets --all-features -- -D warnings`
- [ ] `cargo test --locked`, `--no-default-features` and `--all-features`
- [ ] `python3 scripts/check_sqlite.py` and `python3 scripts/test_desktop_assets.py`
- [ ] Platforms actually exercised: <!-- e.g. aarch64-apple-darwin -->

## Invariants (see AGENTS.md)

- [ ] No relaxed authorization, evidence optionality, source scope/CAS, immutable history or
      disabled persisted inference
- [ ] No startup scans, model auto-downloads, index wipes or new network/provider calls by default
- [ ] No real provider/model/Bridge/network/migration tests run without consent and
      disposable fixtures; no user database modified
- [ ] Failures and skipped checks are reported as such, not claimed as passing
