# Relation model — 1.0.0

Entities use namespace + kind + canonical key. Names are scoped labels, never global identity. Relation identity uses the registered type version, named role bindings, identity-significant registered qualifiers, polarity, modality, normalized conditions and valid time. Arrays of unordered role values are canonicalized; ordered roles preserve order. Decimal literals use a constrained canonical string spelling, not platform-dependent floats.

A direct assertion has real authenticated origin, actor scope, status, epistemic kind and immutable versions, with evidence defaulting to `[]`. Its relation can be active and used in a model answer without any document. Imported assertions additionally have immutable capture/source revisions, stable anchor/scope and pinned extractor metadata. Publication means availability under policy, not truth or evidential proof.

Multiple assertions can share one canonical relation. Withdrawal changes only owned assertions and appends history. Caller-visible relation versions are fingerprints of only the caller-visible assertion versions; global hidden assertion-set IDs are not returned. Relation/history queries filter assertions first. Public transaction ticks may expose coarse profile activity; strict timing/side-channel noninterference is not a qualified claim.

Conditions support bounded `true`, `false`, `all`, `any`, `not`, `eq`, `neq`, `in`, `gt`, `gte`, `lt`, `lte`, `exists` with TRUE/FALSE/UNKNOWN. Unknown facts are distinct from authoritative absence. Unsupported ordering (for example numeric decimal strings) remains UNKNOWN. Domain conditions never create authenticated scopes or provider permissions.

Valid time uses half-open millisecond intervals. Unknown bounds are not replaced with the current date. Polarity/modality and conflict disclosures affect coverage. Persistent `inferred` is rejected in Rust and SQL until dependency/invalidation lifecycle is qualified. No path is silently published as a direct relation.

Read the actual serialized contract using `huda contracts`; the illustrative input shapes in the supplied plan are design examples, not a competing hand-written schema.
