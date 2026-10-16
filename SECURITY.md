# Security policy

Huda is a **source candidate**, not a security-qualified release. Read the
[threat model](docs/threat-model.md) and [gate matrix](docs/verification/gates.md) for the
controls that exist in source and the qualification work that remains open.

## Supported versions

| Version | Status |
|---|---|
| 1.2.x | Source candidate; security fixes land on `main` |
| 1.0.x – 1.1.x | Superseded by 1.2.x; upgrade instead of expecting separate fixes |
| < 1.0 | Unsupported |

## Reporting a vulnerability

Do **not** open a public issue, discussion or pull request for a suspected vulnerability.
Report it privately through GitHub's
[private vulnerability reporting](https://github.com/devsolux/huda/security/advisories/new)
for this repository. Include the affected version or commit, target platform, reproduction
steps and impact. Use a disposable profile for any reproduction; never attach real profile
databases, OS secret-store entries, tokens or captured user data.

## In scope

- Local IPC authentication, peer checks, framing limits and the single-writer boundary
- Profile directory/file ownership, permissions, leases and activation markers
- The opt-in loopback REST server (token, Host and Origin checks)
- Provider egress, redirect/proxy denial and public-address checks
- The sandboxed Bridge capture path
- Admission, authorization and immutable-history invariants in the SQLite authority
- Desktop login registration, tray behavior and native packages built by this repository

## Out of scope

- Same-user attackers with full access to the user's account and OS secret store
- Behavior of locally configured models or third-party providers themselves
- Unsigned or ad-hoc-signed test packages being flagged by Gatekeeper/SmartScreen
