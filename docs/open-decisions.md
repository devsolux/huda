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

## 3. Login entry for a moved application

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

## 4. Windows named-pipe squatting

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

## 5. Capture storage growth

- **Evidence:** every `publish_source` stores a new immutable `source_revisions` row with the
  full capture (`src/relation/publish.rs`). Captures are never deduplicated by content, no
  quota applies, and history has no erasure path. A principal with `ingest` on an admitted
  source can therefore grow the database without bound, about 1 MiB per call.
- **Options:**
  - Return the previous result when a capture's SHA-256 and revision equal the current head.
  - Store capture bodies once per SHA-256.
  - Add a per-actor size or rate quota.
- **Constraint:** content-addressed storage is a schema change (see 1). A quota needs a policy
  default.
- **Recommendation:** the head check now, which needs no schema change, and a quota together
  with the migration.

## 6. Relation types in model prompts

- **Evidence:** semantic extraction sends every registered relation type to the local model
  (`src/acquisition/text.rs`). Provider auto-registration adds up to 16 types per Wikidata
  capture. Once the registry passes roughly 800 types, extraction exceeds the 512 KiB model
  input budget and fails with `LIMIT_EXCEEDED`. Question parsing already falls back to the
  deterministic parser in that case.
- **Options:**
  - Send only the source policy's admitted types plus the core types, capped at about 64.
  - Paginate `list_relation_types` and send a ranked subset.
- **Recommendation:** the admitted types. Admission already defines which types a source may
  publish.

## 7. One invalid assertion in a batch

- **Evidence:** in `publish_source`, a normalization or evidence error other than `TYPE_UNKNOWN`
  (cardinality, undeclared role, kind mismatch, an entity not labelled in scope) fails the
  whole batch. Statements of a type the source does not admit are stored as proposals with
  their raw JSON, bounded only by the batch size.
- **Options:**
  - Record each invalid assertion as a bounded `invalid_assertion` proposal (error code and
    hash, not the payload) and publish the rest.
  - Keep the all-or-nothing rule, which is easier to audit.
- **Recommendation:** bounded proposals for model and provider batches, and all-or-nothing for
  explicit JSON imports.

## 8. Entities from sources that publish only proposals

- **Evidence:** `publish_source` inserts a batch's entities and labels before admission is
  considered. Entities from a source whose assertions all stay proposals still become
  searchable and model-visible, and labels are never withdrawn with their source.
- **Options:**
  - Insert entities only when an assertion that references them is active.
  - Mark labels with their source and hide them when no active assertion remains.
- **Constraint:** both change what `explore` and goal-seed resolution see.
- **Recommendation:** source-marked labels. Withdrawal stays non-destructive, because labels
  are hidden rather than erased.
