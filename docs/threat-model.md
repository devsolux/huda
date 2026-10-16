# Threat model — source candidate, not an audit attestation

## Protected authority

Relations, independent assertions, history, source generations and policy epochs are durable user data. Their single write path enforces typed admission and authenticated scopes. Model/source/provider/MCP text is untrusted. No prompt can create Caller, consent, registered types, arbitrary URL/command access or a writer. The DB engine forbids immutable-row rewrites, inconsistent heads and inferred persistence.

## Primary controls in source

Private profile and lease; Unix peer checks plus hashed capabilities; bounded message framing; fixed loopback REST with token/Host/Origin checks; no anonymous listener; independent provider flags; fixed read endpoints with redirect/proxy denial and public-address checks; namespace/source/type boundaries; policy/generation/job CAS; safe partial scope reconciliation; immutable capture/history; read-only readers; non-overwriting backup; no automatic repair/index wipe/model download; optional code parser without filesystem access; outbound process capture through a fail-closed OS sandbox.

## Remaining security obligations

This source compiles, lints and passes its tests on native macOS arm64 (2026-10-09) and `cargo audit` reports no advisories, but it has not passed fuzzing, malicious-host/native ACL tests, four-platform crash/upgrade tests or an independent security audit. Windows user-only DACL, SID-verified local named-pipe and overlapped cancellation source is now present and compiles and passes its tests in CI, but native security/race testing is not complete. Unix parent-path/TOCTOU, filesystem eligibility (NFS/sync mounts), free-space behavior and same-user threat boundaries require hardening/qualification. OS secret-store backend and per-client isolation need real tests; OS same-user credential access is not independent sandboxing.

Provider DNS deadlines, qualification of the persisted backoff, HTTP-date Retry-After, redirect/SSRF fixtures, cancellation and revocation races require completion. The sandboxed Bridge needs executable-descriptor pinning against same-user replacement, tighter aggregate child/piped-I/O behavior, real bubblewrap qualification, outbound HTTP/auth support, and target-specific Mac/Windows isolation. No missing backend has an insecure fallback.

The answer validator validates IDs and required structure, not natural-language entailment or hallucination absence. Counterexample and held-out multilingual evaluations are mandatory. Source input may poison an authorized graph if its independent admission policy is overly broad; `active` does not certify truth. No model has been run in qualification.

Coarse transaction timestamps/counters remain visible; strict timing/side-channel noninterference has not been proven. Scope metadata/types and exports/backups require privacy review. SQLite/backups are not automatically encrypted. Automatic erasure/restore is intentionally unavailable until coherent retention and recovery are qualified.

See the gate matrix. Do not infer security completion from the SQL tests.

## Desktop and packaging update controls

Autostart is off unless the user registers it. Windows uses CurrentUser, not Dynamic/AllUsers;
Linux uses XDG session startup, not a system service. Unsupported registration paths/configs
fail with a visible error. Linux keeps a minimized, recoverable window when a tray host may
be absent. UI callbacks wake the eframe event loop, and hidden stale input is not used to
re-handle a close event.

The Windows pipe rejects remote clients, checks kernel-provided peer PID/SID before sending
a credential, disables client impersonation, and uses the first-instance flag to reject name
squatting. Pending overlapped I/O is cancelled and drained before borrowed buffers are freed.
These are source controls, not a tested security guarantee. No unsafe Send implementation is
added to make pending native operations transferable.

The desktop lease file and activation marker live in the private profile directory under
the same ownership, mode, symlink and hard-link checks as every managed file; the marker is
empty and carries no instruction, so a planted marker can at most raise the window. A
malformed IPC envelope is answered with a redacted error and never dispatched.

Workflow permissions are read-only and actions use immutable commit pins. The workflow does
not run on pull requests, and no signing credentials exist. Only the keyless signing job
receives `id-token: write`; it runs after every gate, checks out no code, signs only files
listed in verified checksums and verifies each Sigstore bundle against this workflow's
identity. A Sigstore bundle proves workflow/ref/commit provenance, not package safety. Only
the final release-asset job receives `contents: write`; it runs after signing on a release a
maintainer published, checks out no code, and attaches already-built packages and bundles
without replacing assets. The
read-only job token authenticates only the Inno Setup release-metadata lookup and is removed
from the environment before that installer runs. Inno Setup is downloaded only
inside a disposable Windows build runner and checked against its release asset digest plus
Authenticode publisher. Native test packages are unsigned/ad-hoc, not notarized or release
certified. Dependency/native-license and binary dependency review remain mandatory.
