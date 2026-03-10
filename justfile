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

# Tag and push a release
release tag:
    git tag {{tag}} && git push origin {{tag}}
