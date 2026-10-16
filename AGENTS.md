# Repository engineering rules

Read README and the supplied normative plan before editing. Preserve its architecture: one profile/Core/writer, relation-first truth, optional evidence, mandatory technical origin, one local model adapter, Query Core coverage ownership, one bounded acquisition controller, shared handlers and offline defaults. No agents, parallel graph stores or model-owned authority.

Use English for code/docs and the smallest coherent change. Follow the actual typed DTOs and schema. Never resurrect project-index wipes, startup scans, model auto-downloads, npm/plugin desktop substitutes or initialize-protocol fallback. Keep syntax parsing optional and isolated.

Treat inputs, captures and model output as untrusted. Do not relax authorization, evidence optionality, source scope/CAS, immutable history or disabled persisted inference to pass tests. Do not run real provider/model/Bridge/network/migration/destructive tests without explicit consent and disposable fixtures. Never modify a user database to make a test pass.

Qualification commands are in README and `scripts/qualify.sh`. Never invent lockfile, native, LLM or provider success. Review the committed `Cargo.lock` before locked checks and report exact failures/skips. Never call a build a completed G0–G8 release without actual gate evidence.

Git: work only on `main`. Never create, switch to or push another branch, worktree or pull-request branch; commit directly to `main` and push only `origin main`, and only when asked. Never force-push, rebase, amend or reset published commits, or delete branches, tags, releases or release assets, without explicit consent. Keep unrelated uncommitted changes out of commits and never discard them.
