# CLAUDE.md

## Project

Joy is a terminal-native, git-native product management tool. The `joy` binary provides CLI, TUI, and server functionality.

This repository (`joyint/joy`) contains:

| Crate | Purpose | License |
|-------|---------|---------|
| `joy-core` | Shared library: data model, YAML I/O, status logic, deps, git | MIT |
| `joy-cli` | PM CLI binary (clap), includes TUI (ratatui) and server (axum) | MIT |
| `joy-ai` | AI tool dispatch, job tracking | MIT |

Joy shares `joy-core` with [Jot](https://github.com/joyint/jot) (personal todo CLI). Jot depends on joy-core as an external crate.

## Required Reading

Before making any changes, read and follow the rules in these documents:

- `CONTRIBUTING.md` -- coding conventions, testing, CI/CD, commit messages
- `docs/dev/Vision.md` -- product vision, data model, CLI design, AI integration
- `docs/dev/Architecture.md` -- tech stack, repo structure, Cargo workspace, security

For cross-project architecture, ADRs, and business docs see the [umbrella repository](https://github.com/joyint/project).

## Rules

- Do not reference Claude, Anthropic, or AI assistants in code comments, git commits, documentation, or any generated content. No exceptions.
- No emoji in documentation, commit messages, or code comments
- Use Mermaid for all diagrams, never ASCII art
- Fix root causes, not symptoms -- no workarounds or temporary feature flags
- No `unwrap()` or `expect()` in library code (joy-core, joy-ai)
- Run `cargo fmt --all` and `cargo clippy --workspace -- -D warnings` before committing
