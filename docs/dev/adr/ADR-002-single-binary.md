# ADR-002: Single binary with feature flags

**Status:** Accepted

## Context

TUI and server could be separate binaries or integrated.

## Decision

Single binary. `joy app` launches TUI, `joy serve` starts server. Both behind Cargo feature flags (`tui`, `server`). Default build includes TUI only, `--features full` includes server.

## Consequences

One install step, shared code, simpler distribution. Binary size ~5-10MB with all features. Feature flags keep the default binary lean for users who only need CLI + TUI.
