# ADR-001: YAML over SQLite for data storage

**Status:** Accepted

## Context

Joy needs to store items, milestones, and config. Options: SQLite (single file), JSON files, YAML files.

## Decision

Individual YAML files per item.

## Consequences

Git-diffable, human-readable, one-file-per-item minimizes merge conflicts, no binary dependencies. Trade-off: slower for large projects (1000+ items) -- acceptable for target audience (small teams). Full-text search requires reading all files (mitigated by caching in later versions).
