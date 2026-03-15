# Joy -- Contributing

This document covers the essentials for contributing to Joy. For the full contributing guide (shared across all Joyint repositories), see the [umbrella CONTRIBUTING.md](https://github.com/joyint/project/blob/main/CONTRIBUTING.md).

For product vision and data model see the [vision documents](./vision/README.md). For technology choices and architecture see [Architecture.md](./Architecture.md).

The product backlog lives in `.joy/items/` and is managed with the `joy` CLI. Run `joy` for a board overview, `joy ls` to list items, `joy show <ID>` for details.

---

## Quick Reference

**Formatting and linting:**

```sh
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

**Testing:**

```sh
just test              # All tests
just test-unit         # Unit tests only
just test-snap         # Snapshot tests (update with just test-snap-update)
```

**Commit messages:** Conventional commits -- `type(scope): description`

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`

Scopes: `core`, `cli`, `tui`, `ai`, `docs`

No emoji in commit messages.

---

## Joy-Specific Conventions

### Rust

- **Core library** (`joy-core`): `thiserror` for errors, no `unwrap()`/`expect()`
- **CLI** (`joy-cli`): `anyhow` for error propagation, `clap` derive API
- **Snapshot tests** with `insta` for CLI output formatting

### License Headers

All source files start with an SPDX license header:

```rust
// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT
```

See [ADR-008](./adr/ADR-008-open-core-licensing.md) for licensing details.

### Task Runner

```sh
just test              # Run all tests
just fmt               # Format all code
just lint              # Lint all code
just check             # fmt-check + lint + test
just release v0.1.0    # Tag and push release
just cli dev           # Run CLI in dev mode
just cli build         # Release build
just cli install       # Install binary locally
```

### Documentation

- No emoji in technical docs
- Mermaid for all diagrams, no ASCII art
- File tree listings are acceptable (actual filesystem structure)
