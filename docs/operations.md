# Operations — 1.2.0 source candidate

## Qualification before use

Do not run this candidate against important or irreplaceable data. Start with an empty private test profile. The original archive had no authentic lockfile or baseline tests; the committed `Cargo.lock` is a 2026-10-09 resolution, re-resolved for the 2026-10-12 dependency upgrade, and the Rust tests are new source tests, not restored original tests. They have executed on native macOS arm64 only. Review dependency resolution, licenses, native C ABI and exact target results before treating any build as qualified.

The manifest keeps one blocking reqwest stack, serde/serde_json and rusqlite. The required desktop stack also includes optional auto-launch and tray-icon dependencies. These do not belong to the headless feature set or own domain logic. Bundled SQLite and optional Tree-sitter grammars require a C toolchain. Linux desktop/secret-store builds may require distribution development packages for D-Bus, OpenGL, X11/Wayland, xkbcommon and pkg-config. The exact package/ABI set must be established on the chosen release baseline, not inferred from this document.

Generate a fresh `Cargo.lock` explicitly with `cargo generate-lockfile`, inspect `cargo tree --locked` and `cargo metadata --locked`, then run the exact checks in README. Do not represent it as the original lock. The `scripts/qualify.sh` script intentionally refuses to proceed without a lockfile. It does not publish, sign, deploy or create a release. The old workflows are removed. The new `.github/workflows/desktop.yml` is a read-only native build/qualification workflow; see `desktop-build.md`.

## Profiles, credentials, Core lifetime

Use the OS user-data path or an explicit absolute private directory. Unix requires the profile to be user-owned, mode 0700, with regular singly linked 0600 managed files. The lease inode is never deleted. The Unix socket is private and the process checks peer credentials plus a scoped token. Windows now has user-private DACL and SID-verified local named-pipe source, with an exclusive file lease. CI compiles, lints and tests it on `windows-2025`, but it has not been natively qualified on Windows; it no longer contains the old blanket unsupported-platform stub.

`huda init` explicitly creates an empty authority and owner/MCP credentials in an available unlocked OS secret store. Tokens are stored under service `huda.<profile fingerprint>`, account `owner`, `mcp`, or an explicitly created principal. The database stores hashes, not plaintext tokens. `init` and `add_principal` replace any secret-store entry already present for that service and account (for example from a deleted profile at the same path) instead of adopting it; a refused request writes nothing. Applications integrating with REST obtain their configured scoped credential from that OS store; do not place tokens in command lines, URLs, logs or source files. There is no anonymous REST mode or plaintext credential fallback. Same-OS-user malware/compromise is not an isolation guarantee.

`huda core --idle-seconds 0` is an explicit foreground Core. GUI/CLI/MCP can start the helper on demand and share it. Merely listing MCP tools/discovering the server needs no database or model. GUI closing does not request Core shutdown. OS process termination may cancel work; committed SQLite transactions and the WAL remain authoritative. Native shutdown/crash drills are still required.

Model configuration is an explicit per-Core JSON input. A Core started on demand without that file is graph-only. The adapter accepts only `127.0.0.1:<port>/v1/chat/completions`, disables proxies and redirects, requests bounded strict JSON, and rejects model tool calls. Verify model weights/tokenizer/license yourself; the running process cannot be attested by a supplied digest string. No implicit remote fallback or download exists.

## Database recovery and backup

A new profile receives a Huda application ID and schema version atomically. An unknown/version-mismatched/corrupt existing database fails closed. No `DROP`, index-version wipe or automatic restore is used against authority. Readers are opened read-only/query-only; the application mutex is the sole writer dispatch boundary, with SQLite immediate transactions as the next consistency layer.

The privileged `backup` handler takes a simple basename, creates a new private file, runs SQLite Backup, checks integrity and fsyncs. It never overwrites an existing backup. Backups are not inherently encrypted; they contain all profile data, including private assertions/captures. Protect them with OS disk encryption and private storage. Automatic restore, erasure and uninstall cleanup are deliberately absent. Restore/migration procedures need native tests and explicit retention-aware authorization; no old code edge is silently imported as truth.

## Provider and Bridge limits

Provider reads remain off until profile online policy and provider `may_fetch` are explicitly set. Going online requires `admin`, and provider consent can be replaced only by a caller that can write its scope. A provider-created source accepts only that provider's captures. Namespace/type/source publication permissions remain separate. Only configured public phrases leave the machine; the application does not classify arbitrary private context as safe public text. New topics outside the approved phrase set remain blocked. Consent is per provider, not per selected page/item, but this conservative phrase policy limits free-form discovery.

Expansion limits: two rounds, 12 HTTP reads, five Wikidata items, three Wikipedia pages, one shared serial provider client, bounded response/capture/model sizes and 120-second job deadlines. A term starts only when its worst case (three reads for Wikidata, two for Wikipedia) fits the remaining budget, and the item/page caps are checked before searching. Budget exhaustion, a provider backoff or a provider timeout ends the job as `partial` with stop reason `exhausted`, `rate_limited` or `provider_timeout`, keeping its report. Captures preserve provider revision, hash, attribution and license. Structured mapping currently accepts only qualified item/string normal-rank statements without qualifiers. Other ranks, qualifiers, novalue/somevalue, units/times or datatypes stay proposals; they are never silently converted to positive facts. Wikipedia HTML is inert untrusted model input, not a browser or qualified rich-document parser.

**Unqualified resource/race details:** OS DNS resolution is blocking and cannot yet be preempted by the job deadline; a provider backoff is kept in the private profile file `provider-backoff`, so it outlasts an idle-retired Core, but per-second spacing is in-memory; HTTP-date Retry-After handling and race tests need completion; candidate/negative caching is limited; total model-token accounting, real API/rate/SSRF/topic-drift tests and semantic gold scores are missing. Policies are rechecked at checkpoints and inside publication, but stress-tested revocation linearization is not claimed. These prevent G5 acceptance.

MCP Bridge config is a privileged user operation selecting an exact native executable SHA-256, fixed arguments, one fixed read, optional exact input file, source/scope and retention. Only Linux with explicitly installed bubblewrap is implemented. It clears the environment, isolates network/PID namespaces, exposes fixed read-only system/executable/input mounts and applies child resource limits. No raw shell, arbitrary model-generated commands, remote HTTP or unsandboxed fallback. Credentials and Huda profile files are not mounted. Sandbox integrity, same-user executable replacement, subprocess I/O bounds and real MCP capture interoperability need qualification; absence is reported, not bypassed.

## Interface limits

Native IPC and REST use bounded JSON messages (1 MiB); internal capture batches have a separate 4 MiB ceiling. An IPC envelope that cannot be parsed, including one carrying unknown or forged authority fields, receives one redacted `INVALID_INPUT` frame with an empty correlation ID; oversized frames are cut at the limit and the connection is closed without dispatch. Query caps are 4,000 visible assertions, 2,000 visible entities, 512 visited entities, fanout 64 and path depth 3. Truncation is disclosed and prevents complete-goal claims. A goal context that, with its relation markers, exceeds 512 KiB is trimmed instead of refused: relations outside every slot basis go first, path steps and conflict sides stay, `truncated` is set and `goal_met` is false. History is bounded to 100 visible revisions; use as-of reads when larger. These are not representative 10k/100k/1M performance qualifications.

MCP stdio answers `server/discover` and `tools/list` with the `resultType`, `cacheScope` (`private`) and `ttlMs` (`0`) fields the 2026-07-28 schema requires; these are protocol metadata, not caching or consent decisions. REST: enable only `huda core --rest-port PORT` with the `rest` feature. Exact `Host: 127.0.0.1:PORT`, Bearer auth, no browser Origin/Cookie, no chunked uploads/pipelining, 8 KiB/32-header limit, 1 MiB JSON, three-second aggregate request-read bounds and bounded concurrent clients. POST requires JSON; GET/DELETE use documented query parameters. MCP HTTP/SSE is not advertised. The runtime OpenAPI source is `api/contracts.rs` deriving schemas from handler DTOs; `huda contracts` exports the same document without starting Core.

The desktop is an advanced native JSON-form client, not a finished consumer UX. Display, keyboard/accessibility, root consent forms, model configuration UX, automatic post-job focus-safe re-ask and real OS install/update behavior remain open.

## Login launch and tray

See `desktop-build.md` for exact native build commands. Launch at login is a checkbox/menu
operation, not an install-time side effect. Status comes from the OS registration, not an
unrelated cached boolean. OS session policy can disable a registered entry independently.
Enable again after relocating an app to repair its registered executable path.

Closing a regular desktop launch exits by default. The session-only Keep running after close
setting may hide to tray on macOS/Windows. Login launches request this behavior; on Linux the
window is minimized rather than hidden because tray availability cannot reliably prove that
a shell displays the icon. Quit Huda always exits the GUI and cancels its current IPC request,
not the shared Core or unrelated jobs. One desktop process runs per profile: it holds
`desktop.lock` (an OS lease that is independent of the Core's `core.lock`), a second manual
launch writes the empty `desktop.activate` marker and exits, and the running desktop consumes
the marker within about a second and shows its window. A second `--background` launch exits
without activating anything. The lease is released by the OS when the process ends.

macOS: install Huda.app outside a DMG/App Translocation before enabling login launch. auto-launch
0.6.0 needs paths without unsupported XML characters. Linux: that version hardcodes
`~/.config/autostart`; a custom `XDG_CONFIG_HOME` is explicitly reported unsupported rather than
silently writing to the wrong directory. Spaces are safely quoted as one Exec argument;
unsupported escape/field-code characters are rejected. Windows: registration is HKCU only.
Disabling/removing an entry does not delete relation data. Windows uninstallation removes only
this installation's own matching login entry and installed files, not `%LOCALAPPDATA%\Huda`.

No real autostart entry, keychain credential or installer has been exercised in qualification
yet. Test these user-visible operations on each native target before distribution.
