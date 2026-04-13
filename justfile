# Joy -- Task Runner
# See docs/dev/CONTRIBUTING.md for full documentation

# List recipes
default:
    @just --list

# Run all tests (unit + snapshot + integration)
test: test-unit test-cmd test-int

# Rust unit tests only
test-unit:
    cargo test --workspace --lib

# Snapshot tests (trycmd)
test-cmd:
    cargo test -p joy-cli --test cmd

# Integration tests (bats)
test-int:
    cargo build -p joy-cli
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

# Run fmt-check, lint, test
check: fmt-check lint test

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
    command -v gh            >/dev/null && ok gh "gh (GitHub CLI)" || opt "gh" "https://cli.github.com"

# Install cargo tools for development
setup:
    cargo install cargo-insta

# Install to ~/.local/bin/
install:
    cargo build --release -p joy-cli && mkdir -p ~/.local/bin && cp target/release/joy ~/.local/bin/joy

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

# Release (bump: patch, minor, or major)
release bump="patch":
    #!/usr/bin/env bash
    set -euo pipefail
    if git describe --tags --exact-match HEAD >/dev/null 2>&1; then
        echo "No changes since last tag, skipping."
        exit 0
    fi
    just auto-commit
    # Check if there is something to release before running checks
    if ! command -v joy >/dev/null 2>&1 || ! [ -f ".joy/project.yaml" ]; then
        echo "No Joy project found. Use joy init to set up."
        exit 1
    fi
    if ! joy release show >/dev/null 2>&1; then
        echo "No items closed since last release."
        exit 0
    fi
    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: working tree is not clean."
        exit 1
    fi
    echo "Updating dependencies..."
    cargo update
    just auto-commit
    echo "Checking (format, lint, test)..."
    if ! just check > /dev/null 2>&1; then
        echo "Checks failed. Run 'just check' for details."
        exit 1
    fi
    just auto-commit
    joy release create "{{bump}}" --full

# Reads CARGO_REGISTRY_TOKEN from the environment (umbrella's `.env` is loaded
# automatically by the umbrella justfile; CI sets it from its secret store).
# Skips a crate when the current version is already published.
# See ADR-032 for the local-first release paradigm.
# Publish workspace crates (joy-core, joy-cli, joy-ai) to crates.io
publish:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
        echo "Error: CARGO_REGISTRY_TOKEN is not set."
        echo "  - Local: add it to the umbrella's .env (see .env.example)."
        echo "  - CI: export it from the runner's secret store."
        exit 1
    fi
    # Order matters: dependents after dependencies.
    crates=(joy-core joy-cli joy-ai)
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
            if echo "$out" | grep -q "is already uploaded"; then
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
    echo "Publish complete."

# Reads GH_TOKEN from the environment. Builds for the current platform's
# native target by default (cross-compiling all targets locally needs
# extra tooling); CI's release.yml builds the full matrix. Uploads the
# built artifacts to the GitHub release for HEAD's tag, creating the
# release if necessary.
# See ADR-032 for the local-first release paradigm.
# Build platform binaries locally and upload to the GitHub release
release-binaries:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${GH_TOKEN:-}" ]; then
        echo "Error: GH_TOKEN is not set."
        echo "  - Local: add it to the umbrella's .env (see .env.example)."
        echo "  - CI: provided automatically as github.token."
        exit 1
    fi
    tag=$(git describe --tags --exact-match HEAD 2>/dev/null || true)
    if [ -z "$tag" ]; then
        echo "Error: HEAD is not on a tag. Run 'just release' first."
        exit 1
    fi
    echo "Building binaries for $tag..."
    rm -rf target/distrib
    if ! command -v dist >/dev/null 2>&1; then
        echo "Installing cargo-dist..."
        cargo install cargo-dist --locked
    fi
    dist build
    mkdir -p target/distrib/upload
    find target/distrib -maxdepth 1 -type f \
        \( -name "*.tar.xz" -o -name "*.zip" \
           -o -name "*-installer.sh" -o -name "*-installer.ps1" \
           -o -name "*.sha256" -o -name "sha256.sum" \) \
        -exec mv {} target/distrib/upload/ \;
    if gh release view "$tag" >/dev/null 2>&1; then
        echo "Uploading to existing release $tag..."
        gh release upload "$tag" target/distrib/upload/* --clobber
    else
        echo "Creating release $tag with artifacts..."
        gh release create "$tag" target/distrib/upload/* --title "$tag" --generate-notes
    fi
    echo "Binary release complete."
