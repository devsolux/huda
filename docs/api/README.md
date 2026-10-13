# API contracts

Single source: `src/api/contracts.rs` and the actual Rust handler DTOs. `OPERATIONS` maps the same handler to REST and, where allowed, an MCP tool. Privileged registry, consent, erasure/backup and connector configuration are not generic open MCP tools.

After compiling and testing, run:

```sh
huda contracts --output docs/api/openapi.yaml
```

The exporter writes JSON, which is valid YAML 1.2, and declares OpenAPI **3.1.2**. It refuses to overwrite an existing file. `GET /openapi.json` from an explicitly enabled authenticated Core serves the same object. Do not hand-maintain a second schema. A compiler-generated snapshot from the 2026-10-09 audit build is retained as evidence in `docs/verification/openapi-snapshot.json` (28 paths, 95 component schemas); it is dated evidence, not a second maintained contract, so regenerate from `huda contracts` after any DTO change.

The current catalog and routes are the implemented surface. The complete normative REST surface is not yet closed: dedicated registry/identity proposal review, aggregated coverage and some lifecycle/UI routes remain incomplete. Ordinary unsupported outputs are retained as relation/type proposals rather than acquiring ontology authority. Optional MCP-over-HTTP headers/SSE and remote Bridge authentication are not implemented/advertised.

Protocol target: MCP `2026-07-28`, `server/discover`, per-request `_meta`, complete results with `resultType`, `cacheScope` and `ttlMs` where the schema requires them, stdio tools/cancellation. The published `2026-07-28` schema was checked on 2026-10-09; it defines no `initialize` method, so there is no compatibility claim for old initialize-based hosts, and no real-host interoperability report exists. OpenAPI documents REST, not arbitrary MCP JSON-RPC. A loopback REST handler test exercises Host, Bearer, Origin, method and route handling against the shared handlers.
