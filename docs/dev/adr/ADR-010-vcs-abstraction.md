# ADR-010: VCS abstraction layer

**Status:** Accepted

## Context

Joy uses Git for repository detection (init), user identity (user.email), and version tags. These calls are scattered across multiple modules with duplicated code. Future support for other VCS (e.g. Jujutsu, Pijul) requires a clean abstraction.

## Decision

All VCS operations are encapsulated behind a `Vcs` trait in `joy-core`. A `GitVcs` implementation handles the Git-specific logic. Modules call trait methods instead of spawning `git` processes directly.

The trait covers: repository detection, user email, version tags (list, latest, describe). It does not cover committing or branching -- Joy does not manage VCS state beyond reading metadata.

## Consequences

Single point of change for VCS operations. No duplicated `get_git_email()` functions. Adding a new VCS backend requires implementing the trait without touching CLI code. Trade-off: slight indirection, but the trait surface is small (4-5 methods).
