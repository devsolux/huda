# Architecture — 1.1.0

## Authority and responsibility

`runtime/profile.rs` owns the per-profile lease, private files and version-fenced open path. `storage/Store` retains one connection behind `api::App`'s mutex. IPC clients do not open SQLite. Native GUI and MCP helpers are proxies; REST runs only inside the explicitly started Core. Background acquisition is bounded in-process work, not a second service or graph.

`relation/model.rs`, `registry.rs`, and `condition.rs` define typed domain values, immutable type versions, canonicalization and three-valued conditions. `relation/publish.rs` is the semantic publication boundary for direct writes and source batches. Schema, scope, generation, policy epoch, idempotency and current job state are checked in the write transaction. The SQL engine additionally enforces immutable history/ownership, current-head foreign keys, active historical-set membership, mandatory technical-origin fields and the disabled inferred epistemic class.

`relation/query.rs` filters assertions before relation assembly, derives caller-visible version fingerprints, evaluates GoalSpec slots/needs, and returns bounded typed paths. Paths do not become direct relations or persisted inference. Negative, conditional, planned, hypothetical, stale or undetermined-time relations remain disclosed rather than silently satisfying a positive factual slot. The current implementation is bounded and conservative, not a general semantic reasoner.

`answer/` interprets once, compiles one authorized context, calls the single configured local model and validates markers/status outside it. The deterministic no-model fallback recognizes a limited set of English question patterns and exact visible labels; other languages require model/gold qualification. A failed local model is not replaced by a remote service or a fake answer.

`acquisition/expand.rs` consumes the existing typed goal and Query Core needs. `providers/wikimedia.rs` owns only fixed read/discover/capture operations. `structured.rs` and `text.rs` produce the same typed proposals. `publish.rs` remains the only semantic writer. Provider property registration is restricted to separately authorized `wikidata:P…` versions and never creates core mappings, identity merges or inference rules.

`bridge/` captures only explicitly configured MCP reads, through the same admission/source batch. `mcp/`, `api/rest.rs` and the native desktop contain protocol/UI adaptation, not a second query, consent, answer or publication implementation.

## Physical schema choices

The plan's logical responsibilities do not require one physical table each. Typed role bindings, relation identities, assertion versions and relation historical sets are explicit tables. Entity labels/aliases are scoped in `entity_labels`. Type versions are immutable composite keys in `relation_types`. Evidence is optional, assertion-version-scoped JSON (`evidence_json`) with capture revision/hash validation; source captures live in `source_revisions`. No evidence foreign key can prevent a legitimate direct assertion. Raw unresolved relation/type outputs use `relation_proposals`, not a parallel truth store.

No vector/FTS/code index is installed. Index failures cannot erase authority because no wipe/rebuild authority route exists. Existing code-index databases are never opened or reclassified as Huda relations. `migrations/0001_authority.sql` initializes a **new** Huda authority; it is not an authenticated, lossless migration of unavailable historical Huda data.

## Source lifecycle

Identity is source + stable assertion anchor + canonical relation + pinned extractor semantics, not job or goal ID. A partial extraction adds or updates eligible assertions. Replacement is permitted only for that exact stable scope, with complete enumeration, active publication permission and no unsupported outputs. Turning off auto-admission cannot downgrade a previously active head through an unreviewed refresh. Scope refreshes and source withdrawal preserve other sources and direct assertions. New source revisions make unrefreshed findings stale; they do not silently delete them.

Jobs record commits in the same transaction as source publication. Cancellation/failure/restart cannot imply that an earlier atomic commit was undone. Restart recovery preserves committed-source reports. There are no automatic retry loops, recursive crawlers, model downloads or destructive recovery.

## Deliberate limits

Implementation/qualification gaps are in `verification/gates.md`. Especially: the Windows private-pipe/DACL source is compiled and tested in CI but not natively qualified; actual filesystem/native sandbox behavior is not qualified; no remote Bridge or MCP HTTP/SSE; limited provider datatype/rank mappings and query-language fallback; no persisted inference; no real model/provider/native gold results. These are open obligations, not replacements of the normative architecture.

## Desktop integration boundary

`desktop/shell.rs` alone owns tray lifetime and native window commands. `desktop/startup.rs`
changes this user's login registration only following explicit UI input. Neither module has
an authority connection or provider permissions. `desktop/main.rs` holds the per-profile
desktop lease from `runtime/profile.rs` for its lifetime; that lease coordinates desktop
instances only and is distinct from the Core writer lease, so it never starts, stops or
implies Core or acquisition activity. On login, `--background` starts the same
native desktop; its requests still reach the same on-demand Core. Closing/quit never kills a
Core serving another client. Native transport IO is under `runtime/transport/`; framing,
scoped authentication, deadlines and dispatch remain shared in `runtime/ipc.rs`.

Product identity is Huda / `com.devsolux.huda`. Existing profile directories and the
`huda.<fingerprint>` credential service remain intentionally stable. Package staging reads
Cargo metadata; no source database or second settings/semantic authority is introduced.
