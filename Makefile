# Huda · the checks a commit needs, and the git and GitHub helpers (AGENTS.md, Git workflow).
#
# `main` is the only branch, here and on GitHub. These targets never force push, never rewrite
# history and never skip a hook. Works with the GNU Make 3.81 that macOS ships.
#
#   make help
#   make check
#   make github-push GITHUB_COMMIT_MSG='<type>: <what changed>'

SHELL := /bin/bash
.DEFAULT_GOAL := help
# Make 3.81 has no .SHELLFLAGS: recipes of more than one command start with this instead.
STRICT := set -euo pipefail

GITHUB_ORG  ?= devsolux
GITHUB_REPO ?= huda
GITHUB_FULL ?= $(GITHUB_ORG)/$(GITHUB_REPO)
# Used by github-remote-ssh, when origin is added by hand instead of by `gh repo create`.
GIT_REMOTE_SSH ?= git@github.com:$(GITHUB_FULL).git

# Commit subject for github-push: `<type>: <what changed>` or `<type>(<area>): <what changed>`,
# as in the history (fix, feat, test, docs, build, ci, release; `fix(ipc): ...`).
GITHUB_COMMIT_MSG ?=
export GITHUB_COMMIT_MSG

CARGO  ?= cargo
PYTHON ?= python3
# Empty builds for the host. ./dev.sh passes aarch64-apple-darwin, so its output and the
# checks share target/<triple>/.
CARGO_TARGET ?=
TARGET_FLAG := $(if $(CARGO_TARGET),--target $(CARGO_TARGET))

# The workflow github-workflow starts: ci.yml, or release.yml for a release dry run.
WORKFLOW ?= ci.yml

# Actions outside GitHub's own that the workflows use. github-allow-action allows these and
# nothing else; scripts/test_desktop_assets.py holds the same list for the workflow files.
ALLOWED_ACTIONS := sigstore/cosign-installer@*

# Written into the body of every commit github-push makes. Keep in step with lint and test.
CHECKS_PASSED := workflows, fmt, clippy, check_sqlite, test_desktop_assets, test (default, headless, all features)
RUN_CHECKS = if ! $(MAKE) --no-print-directory check; then echo "error: the checks failed; nothing was committed or pushed"; exit 1; fi

# Files that hold secrets by their name. Never staged by github-push (.gitignore lists them too).
SECRET_FILES := '.env' '.env.*' '*.pem' '*.key' '*.p8' '*.p12' '*.pfx' '*.keystore' '*.jks' ':!.env.example'

.PHONY: help check lint test check-workflows git-check github-check github-auth git-init git-main \
	require-repo require-main require-origin github-initial-commit github-repo-create \
	github-remote-ssh github-verify github-pull github-push github-bootstrap \
	github-allow-action github-workflow

help: ## Show the targets
	@awk 'BEGIN { FS = ":.*## " } /^[a-zA-Z0-9_-]+:.*## / { printf "  %-24s %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

check: lint test ## Run every check a commit needs (the list CI runs on each target)

# `--locked`: a check never rewrites Cargo.lock.
lint: check-workflows ## Workflows, formatting, strict Clippy, source and SQLite checks
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --locked $(TARGET_FLAG) --all-targets --all-features -- -D warnings
	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) scripts/check_sqlite.py
	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) scripts/test_desktop_assets.py

test: ## Default, headless and all-feature Rust tests
	$(CARGO) test --locked $(TARGET_FLAG)
	$(CARGO) test --locked $(TARGET_FLAG) --no-default-features
	$(CARGO) test --locked $(TARGET_FLAG) --all-features

# No workflow reads a stored secret (the release signs with the run's own identity), and every
# action is pinned to a full commit.
check-workflows: ## Workflows use no stored secret and pin every action to a commit
	@$(STRICT); \
	FILES=$$(find .github/workflows .github/actions -type f \( -name '*.yml' -o -name '*.yaml' \) 2>/dev/null || true); \
	if [ -z "$$FILES" ]; then echo "error: no workflow found in .github/workflows"; exit 1; fi; \
	if grep -nE '(secrets\.|secrets\[|secrets: *inherit)' $$FILES; then \
		echo "error: a workflow reads a stored secret; releases sign with the run's own identity"; exit 1; \
	fi; \
	if grep -nE '^[[:space:]]*(- )?uses:' $$FILES | grep -vE 'uses: *(\./|[^@[:space:]]+@[0-9a-f]{40}([[:space:]]|$$))'; then \
		echo "error: pin every action to a full commit (uses: owner/name@<40 hex> # vX.Y.Z)"; exit 1; \
	fi; \
	echo "workflows: no stored secret, every action pinned"

git-check:
	@command -v git >/dev/null 2>&1 || { echo "error: git is required"; exit 1; }

github-check: git-check
	@command -v gh >/dev/null 2>&1 || { echo "error: GitHub CLI (gh) is required: https://cli.github.com"; exit 1; }

require-repo: git-check
	@git rev-parse --git-dir >/dev/null 2>&1 || { echo "error: not a git repository (run: make git-init)"; exit 1; }
	@# A lock is either a running git command or what an interrupted one left behind. Only a
	@# person can tell which, so nothing is removed here.
	@if test -f "$$(git rev-parse --git-dir)/index.lock"; then \
		echo "error: $$(git rev-parse --git-dir)/index.lock exists. Wait for the other git command, or remove the file if none is running."; \
		exit 1; \
	fi

require-main: require-repo
	@BRANCH=$$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true); \
	if [ "$$BRANCH" != "main" ]; then \
		echo "error: main is the only branch; refusing on '$${BRANCH:-detached HEAD}'"; \
		exit 1; \
	fi

require-origin: require-repo
	@git remote get-url origin >/dev/null 2>&1 || { echo "error: no origin remote (run: make github-repo-create or make github-remote-ssh)"; exit 1; }

github-auth: github-check ## Sign the GitHub CLI in (interactive, once per machine)
	gh auth login

git-init: git-check ## Create the repository with main as its branch, if there is none
	@if git rev-parse --git-dir >/dev/null 2>&1; then \
		echo "Git repository already initialized."; \
	else \
		git init -b main; \
	fi

git-main: require-repo ## Make sure the current branch is main (renames only a repository without commits)
	@BRANCH=$$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true); \
	if [ "$$BRANCH" = "main" ]; then \
		echo "Already on main."; \
	elif ! git rev-parse --verify --quiet HEAD >/dev/null; then \
		git symbolic-ref HEAD refs/heads/main; \
		echo "Branch set to main (no commits yet)."; \
	else \
		echo "error: on '$${BRANCH:-detached HEAD}', and the repository has commits. Bring the work to main with a fast-forward or a merge; a branch is not renamed over main."; \
		exit 1; \
	fi

github-initial-commit: require-main ## First commit, only when there is no history yet
	@if git rev-parse --verify --quiet HEAD >/dev/null; then \
		echo "Repository already has commits; skipping the initial commit."; \
	else \
		$(STRICT); \
		git add -A; \
		if git diff --cached --quiet; then \
			echo "error: nothing to commit"; exit 1; \
		fi; \
		git commit -m "build: initial commit"; \
	fi

github-repo-create: github-check require-main ## Create the private repository on GitHub and push main
	@if git remote get-url origin >/dev/null 2>&1; then \
		echo "Remote origin already exists:"; \
		git remote -v; \
		echo "Skipping gh repo create. Run: make github-push"; \
	else \
		gh repo create "$(GITHUB_FULL)" --private --source=. --remote=origin --push; \
	fi

github-remote-ssh: require-repo ## Add origin over SSH by hand (only if origin is missing)
	@if git remote get-url origin >/dev/null 2>&1; then \
		echo "error: origin is already set:"; git remote -v; exit 1; \
	fi
	git remote add origin "$(GIT_REMOTE_SSH)"
	@echo "Added origin $(GIT_REMOTE_SSH). Next: make github-push"

github-verify: require-repo ## Show remotes and branches; fail if any branch but main exists
	@echo "=== remotes ==="; git remote -v
	@if git remote get-url origin >/dev/null 2>&1; then git fetch --prune origin; fi
	@echo "=== branches ==="; git branch -a
	@EXTRA=$$(git for-each-ref --format='%(refname:short)' refs/heads refs/remotes \
		| grep -v -e '^main$$' -e '^origin/main$$' -e '^origin/HEAD$$' -e '^origin$$' || true); \
	WORKTREES=$$(git worktree list --porcelain | grep -c '^worktree ' || true); \
	if [ -n "$$EXTRA" ]; then \
		echo "error: main is the only branch, but these exist:"; echo "$$EXTRA" | sed 's/^/  /'; \
		exit 1; \
	fi; \
	if [ "$$WORKTREES" -gt 1 ]; then \
		echo "error: extra worktrees exist (git worktree list); main is worked on in one checkout"; \
		exit 1; \
	fi
	@echo "main is the only branch."

# Brings origin/main into the local main.
# - Same commit, or local commits ahead: nothing to do.
# - GitHub is ahead: fast-forward.
# - Both have commits the other lacks: a normal merge. Never a rebase, never a reset.
# - The worktree must be clean before anything is changed.
github-pull: require-main require-origin ## Bring origin/main into main (fast-forward, else a normal merge)
	@$(STRICT); \
	git fetch origin main 2>/dev/null || { echo "origin has no main yet; nothing to pull."; exit 0; }; \
	LOCAL=$$(git rev-parse --verify --quiet HEAD || true); \
	REMOTE=$$(git rev-parse --verify --quiet refs/remotes/origin/main || true); \
	if [ -z "$$REMOTE" ]; then \
		echo "origin has no main yet; nothing to pull."; \
	elif [ "$$LOCAL" = "$$REMOTE" ]; then \
		echo "Already up to date ($$LOCAL)."; \
	elif [ -n "$$LOCAL" ] && git merge-base --is-ancestor "$$REMOTE" "$$LOCAL"; then \
		echo "main already contains origin/main; nothing to pull."; \
	elif [ -n "$$(git status --porcelain)" ]; then \
		echo "error: origin/main has new commits, but the worktree is not clean. Commit first (make github-push does both)."; \
		exit 1; \
	elif [ -z "$$LOCAL" ] || git merge-base --is-ancestor "$$LOCAL" "$$REMOTE"; then \
		git merge --ff-only refs/remotes/origin/main; \
		echo "Fast-forwarded main to $$REMOTE."; \
	else \
		git merge --no-edit refs/remotes/origin/main; \
		echo "Merged origin/main into main. Run the checks before pushing."; \
	fi

# Runs every check, commits everything that changed as ONE commit, and pushes main. For work
# that is more than one logical change, commit each part by hand first; this then only pushes.
# - Refuses on any branch but main, and without a `<type>: <what changed>` message when there
#   is something to commit.
# - Refuses files that hold secrets by their name, and scans what is staged with gitleaks.
# - Hooks run. Nothing is skipped.
# - A rejected push is integrated with a fast-forward or a normal merge, checked again and
#   pushed again. It is never forced.
github-push: github-check require-main require-origin ## Check, commit and push main: GITHUB_COMMIT_MSG='<type>: <what changed>'
	@if [ -n "$$(git status --porcelain)" ]; then \
		PATTERN='^[a-z][a-z0-9]*(\([a-z0-9/_-]+\))?: [^[:space:]].*$$'; \
		if ! [[ "$$GITHUB_COMMIT_MSG" =~ $$PATTERN ]]; then \
			echo "error: there are changes to commit. Usage: make github-push GITHUB_COMMIT_MSG='<type>: <what changed>'"; \
			exit 1; \
		fi; \
		command -v gitleaks >/dev/null 2>&1 || { echo "error: gitleaks is required to commit (https://github.com/gitleaks/gitleaks)"; exit 1; }; \
	fi
	@$(STRICT); $(RUN_CHECKS)
	@$(STRICT); \
	if [ -n "$$(git status --porcelain)" ]; then \
		git add -A; \
		SECRETS=$$(git diff --cached --name-only -- $(SECRET_FILES)); \
		if [ -n "$$SECRETS" ]; then \
			git reset --quiet; \
			echo "error: refusing to commit files that hold secrets:"; echo "$$SECRETS" | sed 's/^/  /'; \
			exit 1; \
		fi; \
		if ! gitleaks git --pre-commit --staged --redact --no-banner; then \
			git reset --quiet; \
			echo "error: gitleaks found a secret in the staged changes; nothing was committed"; \
			exit 1; \
		fi; \
		git commit -m "$$GITHUB_COMMIT_MSG" -m "Checks passed: $(CHECKS_PASSED)"; \
	else \
		echo "Nothing to commit."; \
	fi
	@$(STRICT); \
	if git push -u origin main; then \
		exit 0; \
	fi; \
	echo "Push rejected. Integrating origin/main (fast-forward or a normal merge)."; \
	git fetch origin main; \
	if git merge-base --is-ancestor HEAD refs/remotes/origin/main; then \
		git merge --ff-only refs/remotes/origin/main; \
	else \
		git merge --no-edit refs/remotes/origin/main; \
	fi; \
	$(RUN_CHECKS); \
	git push -u origin main

# Sequential on purpose (safe with make -j): init, main, first commit, create on GitHub, verify.
github-bootstrap: ## New machine or new repository: init, first commit, create on GitHub, verify
	@$(MAKE) github-check
	@$(MAKE) git-init
	@$(MAKE) git-main
	@$(MAKE) github-initial-commit
	@$(MAKE) github-repo-create
	@$(MAKE) github-verify

# Repository settings the workflows rely on, set once per repository (needs admin rights):
# - Actions may run, but only GitHub's own actions and the ones in ALLOWED_ACTIONS.
# - The job token is read-only unless a job asks for more, and Actions cannot approve pull
#   requests. Only the publish job of release.yml asks for `contents: write`.
github-allow-action: github-check ## Repo settings: allow only the actions the workflows use, read-only job token
	@$(STRICT); \
	set -f; PATTERNS=(); \
	for ACTION in $(ALLOWED_ACTIONS); do PATTERNS+=(-f "patterns_allowed[]=$$ACTION"); done; \
	gh api -X PUT "repos/$(GITHUB_FULL)/actions/permissions" -F enabled=true -f allowed_actions=selected; \
	gh api -X PUT "repos/$(GITHUB_FULL)/actions/permissions/selected-actions" \
		-F github_owned_allowed=true -F verified_allowed=false "$${PATTERNS[@]}"; \
	gh api -X PUT "repos/$(GITHUB_FULL)/actions/permissions/workflow" \
		-f default_workflow_permissions=read -F can_approve_pull_request_reviews=false; \
	echo "Actions on $(GITHUB_FULL): GitHub's own and $(ALLOWED_ACTIONS); read-only job token."

# The workflows start by hand only. They check the commit that is on GitHub's main, so this
# refuses until main is pushed. `WORKFLOW=release.yml` is a release dry run: it builds, packages
# and signs, and publishes nothing (only a pushed tag v<version> publishes).
github-workflow: github-check require-main require-origin ## Start CI on GitHub for main (WORKFLOW=release.yml: release dry run)
	@$(STRICT); \
	case "$(WORKFLOW)" in ci.yml|release.yml) ;; *) echo "error: WORKFLOW is ci.yml or release.yml"; exit 1;; esac; \
	git fetch --quiet origin main; \
	if [ "$$(git rev-parse HEAD)" != "$$(git rev-parse refs/remotes/origin/main)" ]; then \
		echo "error: main and origin/main differ; the workflow would not check this commit. Run: make github-push"; \
		exit 1; \
	fi; \
	gh workflow run "$(WORKFLOW)" --repo "$(GITHUB_FULL)" --ref main; \
	echo "Started $(WORKFLOW). Watch it with: gh run watch --repo $(GITHUB_FULL)"
