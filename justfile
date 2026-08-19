# Joy -- Task Runner
# See docs/dev/CONTRIBUTING.md for full documentation

# Load `.env` so release recipes can read credentials like
# CARGO_REGISTRY_TOKEN from the environment. just searches from the
# working directory upward and uses the closest file, so a local
# `joy/.env` wins and otherwise the umbrella's `.env` is picked up.
# This makes `just publish` / `just release` work when run directly
# inside this submodule, not only through the umbrella's release-all.
set dotenv-load

# List recipes
default:
    @just --list

# Run all tests (unit + snapshot + integration)
test: test-unit test-cmd test-int

# Rust tests: lib AND integration tests (crates/*/tests/*.rs) — matches
# what CI runs (cargo test --workspace). Do NOT re-add --lib: it silently
# skips the integration tests (e.g. job_session_mint) and lets a red CI
# slip past a green local check. fast-kdf keeps Argon2id minimal for speed.
test-unit:
    cargo test --workspace --features fast-kdf

# Snapshot tests (trycmd)
test-cmd:
    cargo test -p joy-cli --test cmd --features fast-kdf

# Integration tests (bats)
test-int:
    cargo build -p joy-cli --features fast-kdf
    bats tests/integration/*.bats

# Snapshot tests (insta)
test-snap:
    cargo insta test --workspace

# Update snapshots
test-snap-update:
    cargo insta test --workspace --review

# Coverage report (terminal summary)
test-coverage:
    cargo llvm-cov --workspace

# Coverage report (HTML, opens in browser)
test-coverage-html:
    cargo llvm-cov --workspace --html --open

# Re-run tests on change
test-watch:
    cargo watch -x 'test --workspace'

# Format all code
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Lint all code
lint:
    cargo clippy --workspace -- -D warnings

# Abort if the local Rust stable toolchain is behind the latest release.
# Prevents clippy-version drift between local and CI.
[private]
_toolchain-check:
    #!/usr/bin/env bash
    if rustup check stable 2>/dev/null | grep -q "Update available"; then
        echo "Error: Rust stable toolchain is outdated. Run 'rustup update stable'."
        exit 1
    fi

# Refresh the in-crate copy of the Tutorial used by `joy tutorial`.
# The canonical doc is docs/user/Tutorial.md; cargo package needs a
# copy inside the joy-cli crate. See JOY-017F-FD.
sync-tutorial:
    mkdir -p crates/joy-cli/docs/user crates/joy-cli/docs/ai
    cp docs/user/Tutorial.md crates/joy-cli/docs/user/Tutorial.md
    cp docs/ai/Tutorial.md crates/joy-cli/docs/ai/Tutorial.md

# Run fmt-check, lint, test
check: _toolchain-check sync-tutorial fmt-check lint test

# Lint commit messages for Joy item references (default: main..HEAD)
lint-commits base="main":
    #!/usr/bin/env bash
    set -euo pipefail
    ROOT=$(git rev-parse --show-toplevel)
    ACRONYM=$(grep -E '^acronym:' "$ROOT/.joy/project.yaml" 2>/dev/null | head -1 | sed "s/^acronym:[[:space:]]*//" | tr -d "\"'")
    if [ -z "$ACRONYM" ]; then
        echo "error: no project acronym found in .joy/project.yaml" >&2
        exit 1
    fi
    PATTERN="${ACRONYM}-[0-9A-Fa-f]{4}"
    RANGE="{{base}}..HEAD"
    if ! git rev-parse "{{base}}" >/dev/null 2>&1; then
        echo "error: base ref '{{base}}' not found" >&2
        exit 1
    fi
    COMMITS=$(git log --format="%H %s" "$RANGE" 2>/dev/null)
    if [ -z "$COMMITS" ]; then
        echo "No commits to check in $RANGE"
        exit 0
    fi
    FAILED=0
    while IFS= read -r line; do
        HASH="${line%% *}"
        MSG="${line#* }"
        SHORT="${HASH:0:8}"
        if echo "$MSG" | grep -qE "$PATTERN"; then
            continue
        fi
        if echo "$MSG" | grep -qF '[no-item]'; then
            continue
        fi
        echo "  $SHORT $MSG" >&2
        FAILED=$((FAILED + 1))
    done <<< "$COMMITS"
    if [ "$FAILED" -gt 0 ]; then
        echo "" >&2
        echo "error: $FAILED commit(s) missing $ACRONYM-XXXX item reference" >&2
        echo "  = help: add an item ID or [no-item] tag to commit messages" >&2
        exit 1
    fi
    echo "All commits reference a Joy item."

# Check tools and deps
doctor:
    #!/usr/bin/env bash
    red=$'\033[31m' orange=$'\033[38;5;208m' reset=$'\033[0m'
    ok()   { local v; v=$("$1" --version 2>/dev/null) && echo "  $2: $v" || echo "  $2: ok"; }
    miss() { printf "  %s%s: MISSING%s\n" "$red" "$1" "$reset"; }
    opt()  { printf "  %s%s: MISSING (optional, %s)%s\n" "$orange" "$1" "$2" "$reset"; }
    command -v cargo         >/dev/null && ok cargo cargo           || miss cargo
    command -v rustfmt       >/dev/null && ok rustfmt rustfmt       || miss rustfmt
    command -v clippy-driver >/dev/null && ok clippy-driver clippy  || miss clippy
    command -v git           >/dev/null && ok git git               || miss git
    cargo --list 2>/dev/null | grep -q insta    && echo "  cargo-insta: ok"    || miss "cargo-insta"
    cargo --list 2>/dev/null | grep -q 'llvm-cov' && echo "  cargo-llvm-cov: ok" || opt "cargo-llvm-cov" "cargo install cargo-llvm-cov"
    cargo --list 2>/dev/null | grep -q watch    && echo "  cargo-watch: ok"    || opt "cargo-watch" "cargo install cargo-watch"
    command -v bats          >/dev/null && ok bats bats             || miss "bats (pacman -S bats)"
    command -v jq            >/dev/null && ok jq jq                 || miss "jq (pacman -S jq)"
    command -v gh            >/dev/null && ok gh "gh (GitHub CLI)" || opt "gh" "https://cli.github.com"

# Install cargo tools for development
setup:
    cargo install cargo-insta

# Install to ~/.local/bin/ (joy plus the plugins: joy-bi, and the forge
# plugins joy-core's identity fallback queries, JOY-0251-AA)
install:
    cargo build --release -p joy-cli -p joy-bi -p joy-github -p joy-gitlab -p joy-gitea && mkdir -p ~/.local/bin && cp target/release/joy target/release/joy-bi target/release/joy-github target/release/joy-gitlab target/release/joy-gitea ~/.local/bin/

# Auto-commit known generated files (.joy/, lockfiles)
[private]
auto-commit:
    #!/usr/bin/env bash
    files=(.joy/ Cargo.lock package-lock.json yarn.lock)
    staged=false
    for f in "${files[@]}"; do
        if git status --porcelain "$f" 2>/dev/null | grep -q .; then
            git add "$f"
            staged=true
        fi
    done
    if [ "$staged" = true ]; then
        git commit --quiet -m "chore: update generated files [no-item]"
        echo "Committed pending changes."
    fi

# Local-only release: bump version files, refresh Cargo.lock, record,
# commit, tag. No push, no crates.io publish, no forge release.
# Follow with `just publish` once this succeeds.
# Release (bump: patch, minor, or major)
release bump="patch":
    #!/usr/bin/env bash
    set -euo pipefail
    if git describe --tags --exact-match HEAD >/dev/null 2>&1; then
        echo "No changes since last tag, skipping."
        exit 0
    fi
    just auto-commit
    if ! command -v joy >/dev/null 2>&1 || ! [ -f ".joy/project.yaml" ]; then
        echo "No Joy project found. Use joy init to set up."
        exit 1
    fi
    if ! joy release show >/dev/null 2>&1; then
        echo "No items closed since last release."
        exit 0
    fi
    # Joy logs every invocation to .joy/logs/YYYY-MM-DD.log, so the
    # `joy release show` above may have dirtied the tree. Absorb those
    # writes before the clean check, otherwise the release aborts on
    # its own audit trail.
    just auto-commit
    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: working tree is not clean."
        exit 1
    fi
    echo "Updating external dependencies..."
    cargo update
    just auto-commit
    echo "Bumping version files..."
    joy release bump "{{bump}}"
    echo "Refreshing Cargo.lock..."
    cargo update --workspace
    echo "Checking (format, lint, test)..."
    if ! just check > /dev/null 2>&1; then
        echo "Checks failed. Run 'just check' for details. Rolling bump back."
        git restore crates/ Cargo.lock
        exit 1
    fi
    joy release record "{{bump}}"
    tag=$(git describe --tags --exact-match HEAD 2>/dev/null || echo "unknown")
    echo "Tagged ${tag} locally. Run 'just publish' to ship."

# Publish workspace crates to crates.io, then push and create the
# forge release. Reads CARGO_REGISTRY_TOKEN from the environment
# (umbrella's `.env` is loaded automatically; CI sets it from its
# secret store). Skips crates whose current version is already
# published, so re-running after a partial failure is safe. See
# ADR-032 for the local-first release paradigm.
# Upload crates to crates.io only. Idempotent: already-uploaded
# versions are skipped. CI's publish.yml calls this directly; the
# forge release is handled separately by `joy release publish`.
publish-crates: sync-tutorial
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
        echo "Error: CARGO_REGISTRY_TOKEN is not set."
        echo "  - Local: add it to the umbrella's .env (see .env.example)."
        echo "  - CI: export it from the runner's secret store."
        exit 1
    fi
    # Order matters: dependents after dependencies.
    # joy-bi rides after joy-core (its only internal dependency); the two
    # forge plugins have none. Every workspace member is either in this
    # list or carries publish = false -- the gap that made JOY-0247-E1.
    crates=(joy-model joy-chat joy-core joy-bi joy-github joy-gitlab joy-gitea joy-chat-store joy-ai joy-cli)
    for crate in "${crates[@]}"; do
        version=$(cargo pkgid --quiet -p "$crate" 2>/dev/null | sed 's/.*[#@]\(.*\)/\1/')
        if [ -z "$version" ]; then
            echo "Warning: could not resolve version for $crate, skipping."
            continue
        fi
        # Idempotency hint via cargo search (lags behind the registry by a
        # minute or two, so we still need the post-publish guard below).
        if cargo search "$crate" --limit 1 2>/dev/null | grep -qE "^$crate = \"$version\""; then
            echo "$crate $version already on crates.io, skipping."
            continue
        fi
        echo "Publishing $crate $version..."
        # Capture output; treat "already uploaded" as success so a duplicate
        # run (e.g. local + CI on the same tag) is harmless instead of
        # erroring out the whole publish step.
        if ! out=$(cargo publish -p "$crate" 2>&1); then
            # Two cargo error variants both mean "version already published":
            # - "is already uploaded": registry rejected the upload
            # - "already exists on crates.io index": cargo's pre-check
            if echo "$out" | grep -qE "is already uploaded|already exists on crates.io index"; then
                echo "$crate $version already on crates.io (registry confirmed), skipping."
            else
                echo "$out" >&2
                exit 1
            fi
        else
            echo "$out"
        fi
        # Brief wait so the next crate (which may depend on this one) sees
        # the new version in the registry index.
        sleep 5
    done
    echo "crates.io uploads complete."

# Full publish: upload crates + push + create forge release. This is
# what `just release-all -P` in the umbrella calls per sub. Running
# crates.io upload first means a failed upload leaves only a local
# tag to drop.
# Publish workspace crates, then push + forge release.
publish: publish-crates
    joy release publish
