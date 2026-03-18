# Joy -- Task Runner
# See docs/dev/CONTRIBUTING.md for full documentation

mod app
mod crates

# List recipes
default:
    @just --list

# Run all tests
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
    cargo fmt --all && just app fmt

# Check formatting
fmt-check:
    cargo fmt --all -- --check && just app fmt-check

# Lint all code
lint:
    cargo clippy --workspace -- -D warnings && just app lint

# Run fmt-check, lint, test
check:
    just fmt-check && just lint && just test

# Check tools and deps
doctor:
    @echo "=== Root ==="
    @command -v cargo >/dev/null && echo "  cargo: $(cargo --version)" || echo "  cargo: MISSING"
    @command -v rustfmt >/dev/null && echo "  rustfmt: $(rustfmt --version)" || echo "  rustfmt: MISSING"
    @command -v clippy-driver >/dev/null && echo "  clippy: $(clippy-driver --version)" || echo "  clippy: MISSING"
    @command -v git >/dev/null && echo "  git: $(git --version)" || echo "  git: MISSING"
    @just crates cli doctor
    @just app doctor

# Install all components
install:
    just crates cli install && just app install

# Release (auto patch bump from latest git tag)
release version="":
    #!/usr/bin/env bash
    set -euo pipefail
    v="{{version}}"
    if [ -n "$v" ]; then
        semver="${v#v}"
    else
        current=$(git describe --tags --abbrev=0 2>/dev/null || echo "v0.0.0")
        current="${current#v}"
        major=$(echo "$current" | cut -d. -f1)
        minor=$(echo "$current" | cut -d. -f2)
        patch=$(echo "$current" | cut -d. -f3)
        semver="${major}.${minor}.$((patch + 1))"
    fi
    tag="v${semver}"

    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: working tree is not clean."
        exit 1
    fi

    just crates version "${semver}"
    just app version "${semver}"
    cargo generate-lockfile 2>/dev/null || cargo check 2>/dev/null

    git add -A
    git commit -m "bump to ${tag}"
    git tag "${tag}"
    git push && git push origin "${tag}"
    echo "Released ${tag}"
