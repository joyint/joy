# ADR-007: Yarn over npm

**Status:** Accepted

## Context

Need a Node.js package manager for the Tauri frontend. Options: npm, Yarn, pnpm.

## Decision

Yarn 4 (Berry) with PnP or node-modules linker.

## Consequences

Faster installs, deterministic builds, zero-install possible. Tauri officially supports Yarn. Slightly more setup than npm but better performance and reproducibility.
