# Contributing

This document covers the coding conventions, testing strategy, CI/CD pipeline, and commit message format for the Joy repository.

For product vision and data model see [docs/dev/Vision.md](docs/dev/Vision.md). For technology choices and architecture see [docs/dev/Architecture.md](docs/dev/Architecture.md). For cross-project architecture and ADRs see the [umbrella repository](https://github.com/joyint/project).

---

## Documentation Rules

**No emoji in technical documentation.** Emoji are a runtime feature of the CLI (configurable, deactivatable). They do not belong in technical docs (vision, architecture, ADRs, code comments) or commit messages. README.md and user-facing materials may use emoji sparingly for warmth.

**No ASCII diagrams.** Always use Mermaid for diagrams. This applies to architecture diagrams, flowcharts, state machines, sequence diagrams, and any other visual representation. Mermaid renders natively on GitHub, in most editors, and in documentation tools.

**No ASCII box-drawing** for architecture or flow visualizations. File tree listings (using standard `tree` output characters) are acceptable because they represent actual file system structure, not abstract concepts.

---

## Coding Conventions

**Fix root causes, not symptoms.** Do not add workarounds, feature flags, or conditional logic for temporary problems. If something is missing, create it. If something is broken, fix it. The codebase should always reflect the intended state, not the current gaps.

### Rust

**Edition:** 2021 (or latest stable)

**Formatting:** `rustfmt` with default settings. No custom overrides -- consistency over preference. Always run `cargo fmt --all` before committing.

**Linting:** `clippy` at `warn` level in CI, with `#[deny(clippy::all)]` in library crates. Run `cargo clippy --workspace -- -D warnings` before pushing. Pedantic lints enabled selectively.

**Naming:**

- Types: `PascalCase`
- Functions/methods: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Crate names: `joy-core`, `joy-cli` (kebab-case)
- Module names: `snake_case`

**Error handling:**

- Core libraries (`joy-core`) use `thiserror` enums -- every error type is explicit and matchable
- CLI crates (`joy-cli`) use `anyhow` for convenient error propagation to the user
- No `unwrap()` or `expect()` in library code. Allowed in tests and in CLI `main()` only.

**Dependencies:** Minimize. Every new dependency must justify its inclusion. Prefer stdlib and well-maintained crates with few transitive dependencies.

---

## License Headers

Every source file must start with a license header using [SPDX](https://spdx.dev/learn/handling-license-info/) format.

**MIT files** (`joy-core`, `joy-cli`, `joy-ai`):

```rust
// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT
```

The header goes on the first line of the file, before any `#![...]` attributes, imports, or code. One blank line separates the header from the rest of the file.

---

## Testing Strategy

### Philosophy

**Test-Driven Development (TDD)** is the default workflow. Write the test first, watch it fail, implement the minimum to pass, refactor. This applies especially to core libraries where correctness of the data model and status logic is critical.

### Test Levels

**Unit tests** (Rust `#[cfg(test)]` modules):

- Every public function in core libraries has unit tests
- Data model serialization/deserialization roundtrips
- Status transition validation
- Dependency cycle detection
- ID generation and collision prevention

**Integration tests** (`tests/` directory):

- CLI command execution against real `.joy/` directories
- Full workflows: init, add, status, deps, ls

**Snapshot tests** (for CLI output):

- CLI output is snapshot-tested with `insta`
- Ensures formatting changes are intentional
- Both color and no-color variants

### Test Commands

```sh
just test              # Run all tests
just test-unit         # Rust unit tests only
just test-int          # Integration tests only
just test-snap         # Snapshot tests (update with just test-snap-update)
just test-coverage     # With coverage report
just test-watch        # Re-run on file change
```

### Coverage Target

Aim for >80% line coverage on core libraries. No hard enforcement -- coverage is a signal, not a goal.

---

## CI/CD and Release Pipeline

### Continuous Integration

Every push and pull request triggers:

1. **Format check** -- `cargo fmt --check`
2. **Lint** -- `cargo clippy -- -D warnings`
3. **Test** -- Full test suite (unit + integration + snapshots)
4. **Build** -- Debug build for all targets

### Release Pipeline

Releases are triggered by Git tags (`v0.1.0`, `v1.0.0`, etc.).

**Build matrix:**

| Target | OS | Arch |
|--------|----|------|
| CLI binary | Linux, macOS, Windows | x86_64, aarch64 |

**Artifacts:**

- Standalone binaries (tar.gz, zip)
- Homebrew formula: `brew install joydev/tap/joyint`
- Cargo install: `cargo install joyint`

---

## Task Runner

Use `just` (justfile) as the project task runner. Preferred over Makefiles for clarity and cross-platform support.

---

## Commit Messages

Use conventional commits. Format: `type(scope): description`

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`

Scopes: `core`, `cli`, `tui`, `ai`, `docs`

Examples:

```
feat(core): add dependency cycle detection
fix(cli): handle missing .joy/ directory gracefully
test(core): add roundtrip tests for item serialization
```

No emoji in commit messages.
