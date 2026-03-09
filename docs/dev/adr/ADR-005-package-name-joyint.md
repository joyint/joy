# ADR-005: Package name `joyint`, binary name `joy`

**Status:** Accepted

## Context

The `joy` name collides with existing packages on crates.io, npm, and a C++ build tool (harnesslabs/joy).

## Decision

Publish as `joyint` on registries, install as `joy` binary.

## Consequences

Short binary name for daily use, unique package name for distribution. Common pattern (ripgrep -> rg, fd-find -> fd).
