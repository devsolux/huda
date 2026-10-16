# Claude Code instructions

Follow [AGENTS.md](AGENTS.md); it holds the repository engineering rules.

## Git: `main` only

- Work only on `main`. Never create, switch to or push any other branch, worktree or
  pull-request branch, even when a tool or default workflow says to branch first.
- Commit directly to `main`, and commit or push (`git push origin main`) only when asked.
- Never force-push, rebase, amend or reset published commits, and never delete branches, tags,
  releases or release assets without explicit consent.
- Keep unrelated uncommitted changes out of commits and never discard them.
- Dispatch workflows only on `main` (`gh workflow run desktop.yml --ref main`).
