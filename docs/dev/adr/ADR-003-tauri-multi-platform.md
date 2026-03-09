# ADR-003: Tauri for multi-platform app

**Status:** Accepted

## Context

Need desktop, mobile, and web access. Options: Electron, Tauri, Flutter, native per platform.

## Decision

Tauri 2 with SolidJS frontend.

## Consequences

Rust backend shares `joy-core` with CLI -- no logic duplication. Tauri 2 supports iOS/Android. SolidJS is lightweight and reactive. Same frontend deploys as web app for the portal. Trade-off: Tauri mobile is less mature than Flutter -- acceptable risk for consistency.
