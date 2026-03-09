# ADR-004: Portal as source of truth for sync

**Status:** Accepted

## Context

Sync could be distributed (like Git) or centralized.

## Decision

Central portal is the canonical state. CLI pushes/pulls.

## Consequences

PM data (status, priorities, assignments) doesn't merge well. Last-write-wins with conflict detection is simpler. Fits "one person or small team" use case. Distributed use without a server is still possible via Git alone (manual merge).
