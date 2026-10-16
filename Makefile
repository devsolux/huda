VERSION ?= 1.1.0

# -----------------------------------------------------------------------------
# GitHub — CLI auth, local repo init, gh repo create, push (override as needed)
# -----------------------------------------------------------------------------

GITHUB_ORG  ?= devsolux
GITHUB_REPO ?= huda
GITHUB_FULL ?= $(GITHUB_ORG)/$(GITHUB_REPO)
# Fallback if you add origin by hand instead of gh repo create:
GIT_REMOTE_SSH ?= git@github.com:$(GITHUB_FULL).git

.PHONY: git-check github-check github-auth git-init git-main github-initial-commit \
	github-repo-create github-remote-ssh github-verify github-pull github-push github-bootstrap

git-check:
	@command -v git >/dev/null 2>&1 || { echo "error: git is required"; exit 1; }

github-check: git-check
	@command -v gh >/dev/null 2>&1 || { echo "error: GitHub CLI (gh) is required — https://cli.github.com"; exit 1; }

# Interactive: authenticate gh with GitHub (run once per machine)
github-auth: github-check
	gh auth login

git-init:
	@if test -d .git; then \
		echo "Git repository already initialized (.git exists)."; \
	else \
		git init -b main; \
		echo "Initialized empty Git repository (branch: main)."; \
	fi

# Ensure current branch is named main (no-op if already main)
git-main:
	@git rev-parse --git-dir >/dev/null 2>&1 || { echo "error: not a git repository (run: make git-init)"; exit 1; }
	@BRANCH=$$(git symbolic-ref --short HEAD 2>/dev/null || true); \
	if [ -z "$$BRANCH" ]; then \
		git symbolic-ref HEAD refs/heads/main && echo "Default branch set to main (no commits yet)."; \
	elif [ "$$BRANCH" != "main" ]; then \
		git branch -M main && echo "Renamed branch to main."; \
	else \
		echo "Already on branch main."; \
	fi

# First commit only when there is no history yet
github-initial-commit:
	@git rev-parse --git-dir >/dev/null 2>&1 || { echo "error: not a git repository (run: make git-init)"; exit 1; }
	@if git rev-parse HEAD >/dev/null 2>&1; then \
		echo "Repository already has commits; skipping initial commit."; \
	else \
		git add -A && \
		if git diff --cached --quiet; then \
			echo "Nothing staged to commit; add files or adjust .gitignore, then commit manually."; \
			exit 1; \
		fi && \
		git commit -m "Initial commit" && \
		echo "Created initial commit."; \
	fi

# Create private repo on GitHub and set origin (skips if origin already exists)
github-repo-create: github-check
	@if git remote get-url origin >/dev/null 2>&1; then \
		echo "Remote origin already exists:"; \
		git remote -v; \
		echo "Skipping gh repo create. Run: make github-push"; \
	else \
		gh repo create $(GITHUB_FULL) --private --source=. --remote=origin --push; \
	fi

# Optional: add SSH remote by hand (only if origin is missing)
github-remote-ssh: github-check
	@if git remote get-url origin >/dev/null 2>&1; then \
		echo "Remote origin already set:"; git remote -v; exit 1; \
	fi
	git remote add origin "$(GIT_REMOTE_SSH)"
	@echo "Added origin $(GIT_REMOTE_SSH) — run: git push -u origin main"

github-verify:
	@echo "=== git remote -v ==="
	@git remote -v
	@echo ""
	@echo "=== git branch ==="
	@git branch -a

# Fetch origin/main and update the local main branch only when GitHub has a newer commit.
# - Same commit: no-op.
# - Local commits ahead of origin/main: no-op; use github-push when ready.
# - Remote commits ahead of local main: fast-forward only.
# - Diverged history or a dirty worktree during an update: stop without changing files.
github-pull: git-check
	@git rev-parse --git-dir >/dev/null 2>&1 || { echo "error: not a git repository"; exit 1; }
	@git remote get-url origin >/dev/null 2>&1 || { echo "error: no origin remote (run: make github-repo-create or github-remote-ssh)"; exit 1; }
	@BRANCH=$$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true); \
	if [ "$$BRANCH" != "main" ]; then \
		echo "error: github-pull expects branch main (current: $${BRANCH:-detached})"; \
		exit 1; \
	fi
	@# Guard against a stale lock left by an interrupted git operation.
	@if test -f .git/index.lock; then \
		echo "warning: found .git/index.lock; removing stale lock"; \
		rm -f .git/index.lock; \
	fi
	@git fetch origin main
	@REMOTE=$$(git rev-parse --verify refs/remotes/origin/main 2>/dev/null || true); \
	LOCAL=$$(git rev-parse --verify HEAD 2>/dev/null || true); \
	if [ -z "$$REMOTE" ]; then \
		echo "No commit found on origin/main; nothing to pull."; \
	elif [ -z "$$LOCAL" ]; then \
		if [ -n "$$(git status --porcelain)" ]; then \
			echo "error: local repository has no commit and the worktree is not clean; refusing to overwrite files"; \
			exit 1; \
		fi; \
		git checkout -B main origin/main && \
		echo "Updated main to remote commit $$REMOTE."; \
	elif [ "$$LOCAL" = "$$REMOTE" ]; then \
		echo "Already up to date ($$LOCAL); no committed change on GitHub."; \
	elif git merge-base --is-ancestor "$$REMOTE" "$$LOCAL"; then \
		echo "Local main contains origin/main and has committed changes; nothing to pull."; \
	elif git merge-base --is-ancestor "$$LOCAL" "$$REMOTE"; then \
		if [ -n "$$(git status --porcelain)" ]; then \
			echo "error: origin/main has a newer commit, but the worktree is not clean; commit or stash local changes first"; \
			exit 1; \
		fi; \
		git merge --ff-only origin/main && \
		echo "Updated main: $$LOCAL -> $$REMOTE"; \
	else \
		echo "error: local main and origin/main have diverged; resolve with an explicit merge or rebase"; \
		exit 1; \
	fi

# Stages all changes, commits with message = short local datetime (YYYYMMDD-HHMMSS) if needed, then pushes.
# Custom message: make github-push GITHUB_COMMIT_MSG='wip'
GITHUB_COMMIT_MSG ?=

github-push: github-check
	@git rev-parse --git-dir >/dev/null 2>&1 || { echo "error: not a git repository"; exit 1; }
	@git remote get-url origin >/dev/null 2>&1 || { echo "error: no origin remote (run: make github-repo-create or github-remote-ssh)"; exit 1; }
	@# Guard against a stale lock left by an interrupted git operation.
	@if test -f .git/index.lock; then \
		echo "warning: found .git/index.lock; removing stale lock"; \
		rm -f .git/index.lock; \
	fi
	@STAMP=$$(date +%Y%m%d-%H%M%S); \
	git add -A; \
	if git diff --cached --quiet; then \
		echo "No changes to commit; pushing."; \
	else \
		if [ -n "$(GITHUB_COMMIT_MSG)" ]; then \
			git commit -m "$(GITHUB_COMMIT_MSG)" && echo "Committed (custom message)."; \
		else \
			git commit -m "$$STAMP" && echo "Committed: $$STAMP"; \
		fi; \
	fi
	git push -u origin main

# Full flow (sequential — safe with make -j): init → main → commit → gh create → verify
github-bootstrap:
	@$(MAKE) github-check
	@$(MAKE) git-init
	@$(MAKE) git-main
	@$(MAKE) github-initial-commit
	@$(MAKE) github-repo-create
	@$(MAKE) github-verify