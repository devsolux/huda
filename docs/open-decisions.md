# Open decisions

Review findings that are confirmed but not changed in source yet. Each one needs a schema
migration, a change of published identity, or native qualification on a target this project
cannot test locally. Each entry gives the evidence, the options and a recommendation, and
waits for a maintainer decision. Nothing here is a qualified result.

## 1. Index for relation-assertion lookups

- **Evidence:** `migrations/0001_authority.sql` indexes `relation_assertions` only by
  `(scope, current_status, relation_id)` and `(source_key, extraction_scope_key,
  current_status)`. `snapshot_relation` and `history_at` (`src/relation/publish.rs`,
  `src/relation/query.rs`) look assertions up by `relation_id`. SQLite therefore scans the
  table once per affected relation. That happens inside the single writer's IMMEDIATE
  transaction, so a 1,000-assertion import against a large store can approach the 120-second
  job deadline.
- **Options:**
  - A `0002` migration adding `CREATE INDEX ... ON relation_assertions(relation_id,
    current_status)`, with `SCHEMA_VERSION` 2 and an explicit, backed-up upgrade path.
  - Leave schema 1 unchanged and accept the cost until a measured large-graph gate demands it.
- **Constraint:** existing profiles must never be modified implicitly, and migration tests
  run only on disposable fixtures.
- **Recommendation:** the migration, together with the planned large-graph performance gate
  (G2), so the gain is measured rather than assumed.

## 2. Code-symbol identity in the optional code adapter

- **Evidence:** `src/acquisition/code/mod.rs` hashes the signature, start line and end line
  into each symbol's canonical key. Adding one line at the top of a file gives every symbol a
  new identity. Code captures are partial by design (`admission.rs`), so the previous
  identities are never withdrawn, and stale "defined in" relations accumulate. Separately,
  `extract_nodes` passes `parent_class` into function bodies, so a function nested in a method
  is published as `Class.nested` with kind `method` (Python, Rust, C#).
- **Options:**
  - Key symbols on (kind, qualified name, signature, ordinal among identical siblings). Let a
    whole-file capture attest completeness for its own extraction scope, so a re-capture
    withdraws symbols that disappeared. Stop class qualification at function bodies.
  - Keep the current keys and document code captures as append-only snapshots.
- **Constraint:** either change alters the canonical keys of code entities that are already
  published. Existing assertions stay valid but are not linked to the new identities.
- **Recommendation:** the first option as one versioned extractor change
  (`tree-sitter-syntax-v2`), with a recorded differential run over the parser corpus.

## 3. Provider backoff across Core restarts

- **Evidence:** the Wikimedia `Retry-After`, 503 and `maxlag` backoff lives in memory
  (`src/acquisition/providers/wikimedia.rs`). The Core retires after 300 idle seconds, and
  the next on-demand start may ask the provider again before the backoff it requested has
  ended. Expansion now stops at once with `rate_limited` while a backoff is active, but only
  within one Core lifetime.
- **Options:**
  - Persist the next allowed request time in the database. This needs a migration (see 1).
  - Persist it in a private managed file in the profile (0600, `O_NOFOLLOW`, like the lease
    files).
  - Keep the Core alive while a backoff is pending. A seven-day `Retry-After` would keep it
    running for seven days.
- **Recommendation:** the managed profile file. It needs no schema change, and it is read
  once at Core start.

## 4. Login entry for a moved application

- **Evidence:** auto-launch 0.6.0's `is_enabled` only checks that the LaunchAgent plist, the
  XDG autostart file or the Run value exists (`desktop/startup.rs`). After `Huda.app` is moved
  or reinstalled elsewhere, the desktop still reports "Login launch is registered", although
  the entry points at a missing executable. Turning the option off and on again repairs it.
- **Options:**
  - Read each backend's entry and compare its program path with the current executable:
    - macOS: `ProgramArguments` in the plist.
    - Linux: the `Exec=` line.
    - Windows: the Run value, which needs the windows-sys registry feature.
    The desktop would then report "registered for another location; turn it on again to
    repair".
  - Keep the existence check and document the repair step.
- **Constraint:** the comparison must be qualified natively on all three systems.
- **Recommendation:** the comparison, qualified with the remaining native login gates (G8).

## 5. Windows named-pipe squatting

- **Evidence:** the pipe name `\\.\pipe\com.devsolux.huda-<profile fingerprint>`
  (`src/runtime/transport/windows.rs`) is derived from the profile path. Any local user can
  compute it and create it first. The Core then fails to bind (`FILE_FLAG_FIRST_PIPE_INSTANCE`),
  so this is a cross-user denial of service. It fails closed: the client's SID check prevents
  credential disclosure.
- **Options:**
  - Add an unguessable nonce to the pipe name, kept in a DACL-private file in the profile and
    read by clients before they connect.
  - Accept the fail-closed denial of service and document it in the threat model.
- **Constraint:** this changes the Windows transport and needs native Windows race and DACL
  testing.
- **Recommendation:** the nonce file, implemented together with the outstanding native Windows
  pipe qualification (G1).
