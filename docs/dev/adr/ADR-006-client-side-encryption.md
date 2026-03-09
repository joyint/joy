# ADR-006: Client-side encryption with cleartext metadata

**Status:** Accepted

## Context

E2E encryption is desired for synced data, but the Web UI and server-side features need some visibility.

## Decision

Encrypt item content (title, description, comments) client-side. Keep metadata (IDs, status, priority, timestamps, project name) in cleartext.

## Consequences

Server can provide overview dashboards and status aggregation without accessing sensitive content. Full search requires client-side decryption. Key management adds UX complexity. Web UI must handle crypto in-browser.
