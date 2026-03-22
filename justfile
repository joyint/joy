# Joy -- Task Runner
# See docs/dev/CONTRIBUTING.md for full documentation

# List recipes
default:
    @just --list

# Run all tests
test:
    cargo test --workspace

# Rust unit tests only
test-unit:
    cargo test --workspace --lib

# Integration tests only
test-int:
    cargo test --workspace --test '*'

# Snapshot tests (insta)
test-snap:
    cargo insta test --workspace

# Update snapshots
test-snap-update:
    cargo insta test --workspace --review

# Coverage report (HTML)
test-coverage:
    cargo llvm-cov --workspace --html

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

# Install cargo tools for development
setup:
    cargo install cargo-insta

# Install to ~/.local/bin/
install:
    cargo build --release -p joyint && mkdir -p ~/.local/bin && cp target/release/joy ~/.local/bin/joy

# Release (bump: patch, minor, or major)
release bump="patch": check
    #!/usr/bin/env bash
    set -euo pipefail
    if git describe --tags --exact-match HEAD >/dev/null 2>&1; then
        echo "No changes since last tag, skipping."
        exit 0
    fi
    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: working tree is not clean."
        exit 1
    fi
    # Joy release (if this is a Joy project)
    if [ -f ".joy/project.yaml" ] && command -v joy >/dev/null 2>&1; then
        joy release create "{{bump}}" || exit 1
        # Read version from the latest release YAML
        tag=$(ls -1 .joy/releases/*.yaml 2>/dev/null | sort | tail -1 | sed 's/.*-\(v[0-9].*\)\.yaml/\1/')
    else
        current=$(git describe --tags --abbrev=0 2>/dev/null || echo "v0.0.0")
        current="${current#v}"
        major=$(echo "$current" | cut -d. -f1)
        minor=$(echo "$current" | cut -d. -f2)
        patch=$(echo "$current" | cut -d. -f3)
        case "{{bump}}" in
            major) tag="v$((major + 1)).0.0" ;;
            minor) tag="v${major}.$((minor + 1)).0" ;;
            patch) tag="v${major}.${minor}.$((patch + 1))" ;;
            *) echo "Error: bump must be patch, minor, or major"; exit 1 ;;
        esac
    fi
    semver="${tag#v}"
    # Cargo version bump (if crates exist)
    if [ -d "crates" ]; then
        for f in $(find crates -name Cargo.toml); do
            if grep -q '^version = ' "$f"; then
                sed -i "s/^version = \".*\"/version = \"${semver}\"/" "$f"
                echo "  ${f} -> ${semver}"
            fi
        done
        cargo generate-lockfile 2>/dev/null || cargo check 2>/dev/null
    fi
    git add -A
    git commit --quiet -m "bump to ${tag} [no-item]"
    # Annotated tag with release notes (shown as GitHub Release body)
    if [ -f ".joy/project.yaml" ] && command -v joy >/dev/null 2>&1; then
        joy release show "${tag}" | git tag -a "${tag}" -F -
    else
        git tag "${tag}"
    fi
    git push --quiet && git push --quiet origin "${tag}"
    echo "Released ${tag}"
