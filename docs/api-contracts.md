# API contracts

Single source: `src/api/contracts.rs` and the actual Rust handler DTOs. `OPERATIONS` maps the same handler to REST and, where allowed, an MCP tool. Privileged registry, consent, erasure/backup and connector configuration are not generic open MCP tools. `huda mcp clients` is a local CLI and desktop feature over other products' files, and no operation here reaches it.

After compiling and testing, run:

```sh
huda contracts --output openapi.json
```

The exporter writes JSON, which is valid YAML 1.2, and declares OpenAPI **3.1.2**. It refuses to overwrite an existing file. `GET /openapi.json` from an explicitly enabled authenticated Core serves the same object. Do not hand-maintain a second schema. No copy is kept in the repository: generate it with `huda contracts` whenever it is needed, so it always matches the handler types.

The current catalog and routes are the implemented surface. The complete normative REST surface is not yet closed: dedicated registry/identity proposal review, aggregated coverage and some lifecycle/UI routes remain incomplete. Ordinary unsupported outputs are retained as relation/type proposals rather than acquiring ontology authority. Optional MCP-over-HTTP headers/SSE and remote Bridge authentication are not implemented/advertised.

Protocol target: MCP `2026-07-28`, `server/discover`, per-request `_meta`, complete results with `resultType`, `cacheScope` and `ttlMs` where the schema requires them, stdio tools/cancellation. The published `2026-07-28` schema was checked on 2026-10-09; it defines no `initialize` method, so there is no compatibility claim for old initialize-based hosts. The one real-host report is negative: Claude Code 2.1.282 opens with `initialize` and cannot connect (open decision 9). OpenAPI documents REST, not arbitrary MCP JSON-RPC. A loopback REST handler test exercises Host, Bearer, Origin, method and route handling against the shared handlers.
