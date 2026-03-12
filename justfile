# Joy -- Task Runner
# See docs/dev/CONTRIBUTING.md for full documentation

mod app
mod cli 'crates/joy-cli/justfile'

# List available recipes
default:
    @just --list

# Run all tests (Rust + App)
test:
    cargo test --workspace && just app test

# Rust unit tests only
test-unit:
    cargo test --workspace --lib

# Integration tests only
test-int:
    cargo test --workspace --test '*'

# Snapshot tests (insta)
test-snap:
    cargo insta test --workspace

# Update snapshot tests
test-snap-update:
    cargo insta test --workspace --review

# Test coverage report (HTML)
test-coverage:
    cargo llvm-cov --workspace --html

# Re-run tests on file change
test-watch:
    cargo watch -x 'test --workspace'

# Format all code (Rust + App)
fmt:
    cargo fmt --all && just app fmt

# Check formatting without changes
fmt-check:
    cargo fmt --all -- --check && just app fmt-check

# Lint all code (Rust + App)
lint:
    cargo clippy --workspace -- -D warnings && just app lint

# Run fmt-check, lint, and test
check:
    just fmt-check && just lint && just test

# Check installed tools and dependencies
doctor:
    @echo "=== Root ==="
    @command -v cargo >/dev/null && echo "  cargo: $(cargo --version)" || echo "  cargo: MISSING"
    @command -v rustfmt >/dev/null && echo "  rustfmt: $(rustfmt --version)" || echo "  rustfmt: MISSING"
    @command -v clippy-driver >/dev/null && echo "  clippy: $(clippy-driver --version)" || echo "  clippy: MISSING"
    @command -v git >/dev/null && echo "  git: $(git --version)" || echo "  git: MISSING"
    @just cli doctor
    @just app doctor

# Install all components
install:
    just cli install && just app install

# Bump version, commit, tag, and push a release (auto-bumps minor if no version given)
release version="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "{{version}}" ]; then
        v="{{version}}"
        semver="${v#v}"
    else
        # Auto-detect: read current version, bump minor
        current=$(grep '^version = ' crates/joy-cli/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
        major=$(echo "$current" | cut -d. -f1)
        minor=$(echo "$current" | cut -d. -f2)
        patch=$(echo "$current" | cut -d. -f3)
        semver="${major}.$((minor + 1)).0"
        read -rp "Release v${semver}? [y/N] " confirm
        if [[ "$confirm" != [yY] ]]; then
            echo "Aborted."
            exit 0
        fi
    fi
    tag="v${semver}"

    # Ensure clean working tree
    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: working tree is not clean. Commit or stash changes first."
        exit 1
    fi

    # Detect current version from root Cargo.toml workspace members
    for f in crates/joy-cli/Cargo.toml crates/joy-core/Cargo.toml crates/joy-ai/Cargo.toml app/src-tauri/Cargo.toml; do
        if [ -f "$f" ]; then
            sed -i "s/^version = \".*\"/version = \"${semver}\"/" "$f"
            echo "  bumped $f -> ${semver}"
        fi
    done

    # Update lock file
    cargo generate-lockfile 2>/dev/null || cargo check 2>/dev/null

    # Commit, tag, push
    git add -A
    git commit -m "bump to ${tag}"
    git tag "${tag}"
    git push && git push origin "${tag}"
    echo "Released ${tag}"
