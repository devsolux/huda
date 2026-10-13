# Attribution and source provenance

The optional `code-adapter` contains Tree-sitter parser code and embedded parser tests adapted from the supplied code-index input's `src/parser/` into `src/acquisition/code/parser/`, plus syntax constants extracted from its `src/domain.rs` into adapter-local `semantics.rs`. Import paths changed; parser results are not trusted universal relations. Explicit parser size/time limits replace the old code-index environment variables. The adapter conservatively publishes syntactic definition membership only. Retaining these technical parsers is intentional, in accordance with the supplied plan; no resolver, index or application runtime from that input is shipped.

Third-party dependencies are pinned by the committed `Cargo.lock`. This notice is not a completed dependency-license audit or generated SBOM.
