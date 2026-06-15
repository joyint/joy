# Contributing

How we develop and deliver Joy: workflow, decisions, documentation rules, coding conventions, testing, CI/CD, and commit messages. This document is self-contained.

For the product vision and data model see [VISION.md](./VISION.md); for the technical overview see [ARCHITECTURE.md](./ARCHITECTURE.md).

## How We Work

Every change is tied to a Joy item; this is what makes the audit trail usable. The loop:

1. **Pick or create an item.** Found a bug or a needed change? `joy add bug "..."` before touching code.
2. **Plan it.** `joy comment <ID> "Plan: ..."`.
3. **Start it.** `joy start <ID>` before writing code.
4. **Implement and commit.** Each commit references the item ID (see [Commit Messages](#commit-messages)).
5. **Record the result.** `joy comment <ID> "[x] done this, [x] done that"`.
6. **Close it.** `joy close <ID>` after the work is committed.

Never change code without an item; untracked changes are invisible to governance.

At the start of a task, read the relevant decisions: `joy ls -D` in this repository, and also the ecosystem-wide decisions in the Joyint umbrella project (Joydev-internal). They are binding policy.

## Architecture Decisions

Architecture decisions are recorded as Joy **decision items**, not as markdown files. Each is titled `ADR: <topic>` and referenced by its item id and title (for example `JOY-01CC-94 - ADR: Git as sync backend`), resolvable with `joy show <ID>`.

A decision is an **ADR** when it records an architecturally significant choice: structure, dependencies, interfaces and contracts, data formats, or a cross-cutting property (security, licensing, naming). Process, triage, or naming-confirmation decisions are plain decisions without the `ADR:` prefix.

An ADR lives in the project whose code or contract it governs; genuinely cross-subproject ADRs (naming, open-core licensing, terminology, the AI governance taxonomy, the documentation and source-of-truth conventions) live in the Joyint umbrella project and apply here too.

## Documentation Rules

The repository documents are README.md, VISION.md, ARCHITECTURE.md, and this file. They are a **first, comprehensive orientation, not the specification**; the specification lives in the Joy items, and the code is the ground truth.

- **README is user-facing only.** No technical detail; link to ARCHITECTURE.md where needed.
- **No code duplication in docs.** Do not copy code, signatures, or config schemas; reference the concrete files instead.
- **Describe what exists.** Document the actual code and the real relationships (for example, that AI agents invoke the `joy` CLI); do not document absent or unimplemented features. Ground every concrete claim (crates, commands, models) in the code and the concept docs, not in older docs or crate names.
- **Cite Joy items** in text by id and title (for example `JOY-01CC-94 - ADR: Git as sync backend`).
- **External documents** are not scattered inline; collect them in a closing `## References` section.
- **No emoji** in technical docs or commit messages (README and user-facing materials may use them sparingly). The CLI's emoji are a runtime feature, configurable and deactivatable.
- **No ASCII diagrams or box-drawing.** Use Mermaid. File-tree listings with standard tree characters are fine, since they show real filesystem structure.

## Coding Conventions

**Fix root causes, not symptoms.** Do not add workarounds, feature flags, or conditional logic for temporary problems. If something is missing, create it; if something is broken, fix it. The codebase should reflect the intended state, not the current gaps.

### Rust

- **Edition:** 2021 (or latest stable).
- **Formatting:** `rustfmt` with default settings; run `cargo fmt --all` before committing.
- **Linting:** `clippy` at `warn` in CI, with `#[deny(clippy::all)]` in library crates. Run `cargo clippy --workspace -- -D warnings` before pushing.
- **Naming:** types `PascalCase`, functions and modules `snake_case`, constants `SCREAMING_SNAKE_CASE`, crates kebab-case (`joy-core`).
- **Error handling:** library crates (`joy-core`) use `thiserror` enums; the CLI (`joy-cli`) uses `anyhow`. No `unwrap()` or `expect()` in library code (allowed in tests and CLI `main()` only).
- **Dependencies:** minimize; every new dependency must justify its inclusion.

### CLI Output Style

All command output goes through the shared helpers in `crates/joy-cli/src/color.rs`; never write raw ANSI codes or hardcode widths. Commands that print structured data open with a full-width header and close with a footer; sub-sections use an underlined section heading; key-value pairs are aligned. Status is shown with check/warn/cross marks. All colour respects the `output.color` setting and `NO_COLOR`, and the width comes from `terminal_width()`.

## License Headers

Every source file starts with an [SPDX](https://spdx.dev/learn/handling-license-info/) header on the first line, before any attributes or imports, separated by one blank line:

```rust
// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT
```

## Testing Strategy

**Test-driven development** is the default: write the test first, watch it fail, implement the minimum to pass, refactor. This matters most in `joy-core`, where the data model and status logic must be correct.

- **Unit tests** (Rust `#[cfg(test)]`): every public function in core libraries, serialization roundtrips, status-transition validation, dependency-cycle detection, ID generation.
- **Snapshot tests** (`crates/joy-cli/tests/cmd/`, `trycmd`): CLI output as `.toml` cases (command, args, expected stdout, exit code); use `...` for varying output. For formatting changes and exit-code contracts.
- **Integration tests** (`tests/integration/`, `bats`): the real `joy` binary against real `.joy/` directories, each test setting up a temp project. For new commands/flags and multi-step workflows.

Every new CLI command or flag gets at least one integration test; output-critical commands also get a snapshot test. Tests are part of "done", written with the implementation, not deferred. When closing a bug, reference the covering test in a comment.

Run `just test` (all layers) before closing any implementation item; all must pass. Aim for over 80% line coverage in core libraries (a signal, not a gate).

```sh
just test        # all (unit + snapshot + integration)
just test-unit   # Rust unit tests
just test-cmd    # snapshot tests (trycmd)
just test-int    # integration tests (bats)
```

## CI/CD and Release

**Before every push, run `just check`** and make sure it is green. It runs the toolchain check, tutorial sync, format check, lint, and the full test suite - the same gate CI enforces. Do not push a red tree.

Every push and pull request runs the same checks in CI (format check, lint, full test suite) plus a debug build.

Releases are cut locally with the `justfile`, not from CI. `just release [patch|minor|major]` bumps the version, refreshes `Cargo.lock`, runs `just check` (and rolls the bump back if it fails), records the release, and tags the commit locally; `just publish` then publishes the crates to crates.io (idempotent, safe to re-run) and creates the forge release. Pushing the tag triggers the CI release build for the target binaries. Secrets such as `CARGO_REGISTRY_TOKEN` come from the environment, never from vendor-locked CI logic.

## Commit Messages

Conventional commits: `type(scope): description`. Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`. Scopes: `core`, `cli`, `tui`, `ai`, `docs`.

Every commit subject references at least one Joy item id (for example `JOY-0042-AB`), or `[no-item]` for infrastructure commits with no logical backlog item. A `commit-msg` hook enforces this. No emoji.

AI members end every commit with two trailers:

```
Co-Authored-By: <Tool> <tool-email>
Delegated-By: <operator email from the token redemption>
```

Examples:

```
feat(core): add dependency cycle detection [JOY-0042-AB]
fix(cli): handle missing .joy/ directory gracefully [JOY-0015-9C]
chore: bump dependencies [no-item]
```

## References

- [VISION.md](./VISION.md), [ARCHITECTURE.md](./ARCHITECTURE.md), [README.md](./README.md)
- [Joyint ecosystem](https://github.com/joyint)
