# ADR-008: Open Core Licensing Model

**Status:** Accepted (revised)

## Context

Joy needs a licensing model that balances two goals: maximizing adoption of the core tool and enabling commercial monetization. Fully open source maximizes adoption but leaves no revenue path. Fully proprietary limits community contributions and trust.

## Decision

Almost everything is MIT. Only the native app shell is commercial:

- **MIT** for all Rust crates (`joy-core`, `joy-cli`, `joy-ai`), the SolidJS web frontend (`web/`), and all documentation. The entire CLI, server, web UI, and AI dispatch are open source. Self-hosting gives you the full experience.
- **Commercial license (Joydev GmbH)** for `app/` (Tauri native shell for desktop and mobile). The native app adds offline support, OS integration, and push notifications on top of the shared web frontend.

Revenue comes from:

- **joyint.com** -- managed hosting (uptime, backups, scaling, support SLAs)
- **Managed AI quota** -- proxy for AI calls with billing and quota management
- **Native app** -- desktop and mobile via app stores
- **Support and SLAs** -- guaranteed response times, priority fixes

Joy is a complete, honestly open product. joyint.com sells convenience and operational value, not artificially locked features. This follows the GitLab/Gitea model.

## Consequences

`cargo install joyint --features full` produces a fully MIT binary including server and embedded web UI. Self-hosters get the same features as joyint.com users. The native app is the only commercial component. Contributors to any Rust crate or the web frontend contribute under MIT.
