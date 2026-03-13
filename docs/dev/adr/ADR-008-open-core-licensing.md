# ADR-008: Open Core Licensing Model

**Status:** Accepted (revised 2026-03)

## Context

Joy (and its companion Jot) need a licensing model that balances adoption of the core tools with sustainable commercial revenue. The model must support a path from free CLI usage to paid enterprise deployment.

## Decision

The CLI tools and shared core are MIT. Server-side components and native apps are commercially licensed.

**MIT (free, open source):**

- `joy-core` -- shared data model, YAML I/O, status logic, dependency management
- `jot-core` -- todo extension (recurrence, RRULE) on top of joy-core
- `joy-cli` -- product management CLI
- `jot-cli` -- personal todo CLI
- Data format (`.joy/` YAML files) -- no lock-in, always readable
- Documentation

**Commercial (Joydev GmbH):**

- API server (REST API, Git gateway)
- CalDAV server (VTODO bridge to Apple Reminders, Google Calendar)
- Notification service (due dates, status changes, mentions)
- Web UI (SolidJS frontend served by the API server)
- Native apps (Tauri desktop and mobile)
- joyint.com managed hosting

Revenue comes from:

- **joyint.com** -- managed hosting with Git storage, CalDAV, WebUI, notifications
- **Self-hosted commercial license** -- for organizations running their own server
- **Native apps** -- desktop and mobile via app stores
- **Enterprise tier** -- SSO, audit dashboards, compliance reports, SLA

## Consequences

The free CLI tools are the acquisition channel. Developers and AI agents use them at zero cost. When a team needs collaboration (sync, WebUI, notifications) or mobile access (CalDAV), they upgrade to the commercial platform. This creates a natural funnel without artificial feature restrictions on the CLI.

Self-hosting requires a commercial license for server components. This protects against hosting parasites (cloud providers offering a competing service from open-source code) -- a lesson learned from Elasticsearch, MongoDB, and Redis.

Contributors to joy-core, jot-core, joy-cli, and jot-cli contribute under MIT. Server and app contributions require a CLA or are done by Joydev GmbH.
