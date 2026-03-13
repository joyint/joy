# ADR-004: Git as sync backend

**Status:** Accepted (revised 2026-03)

Supersedes the original "Portal as source of truth" decision.

## Context

The original decision used a centralized portal with a custom sync protocol as the canonical state. This required a full application server with a database and a custom conflict resolution layer.

The revised strategy uses Git itself as the sync backend. YAML files in `.joy/` are already versioned with Git. Using a Git remote as the sync target eliminates the need for a custom database and sync protocol.

## Decision

Git is the sync backend. There is no custom sync protocol and no application database.

- **CLI users** sync via `git push` / `git pull` -- no server needed
- **WebUI and CalDAV users** go through a thin REST API on joyint.com that executes Git operations server-side
- **Notification service** watches the Git repo for due dates and status changes

The server is a gateway to Git plus a cron for notifications. Not an application server with its own state.

**Bring Your Own Git (BYOG):** Users choose where their data lives -- on joyint.com (Git hosting included) or on their own GitHub/Gitea. Both options get the same services (WebUI, CalDAV, Notifications). This lowers the trust barrier and storage costs.

Last-write-wins with conflict detection remains the merge strategy. PM data (status, priorities, assignments) does not merge well with three-way merge. Git's file-level conflict markers are sufficient for the rare case of concurrent edits to the same item.

## Consequences

Massively reduced server complexity and operational cost. No database ops, no custom sync code, no migration tooling. The server is stateless (reads/writes Git). Self-hosting is trivial -- any Git server works for basic sync. BYOG means users can keep their data on infrastructure they already trust.

Trade-off: Git is not optimized for frequent small writes from many concurrent users. For the target scale (teams of 1-50), this is not a problem. If it becomes one, a write-ahead buffer in front of Git can absorb bursts without changing the architecture.
