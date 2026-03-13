# ADR-006: Client-side encryption with cleartext metadata

**Status:** Accepted (revised 2026-03)

## Context

Data on joyint.com must be E2E-encrypted. The server should never have access to item content. At the same time, server-side features (notifications, CalDAV scheduling, dashboards) need some visibility into item metadata.

## Decision

All data on joyint.com is always E2E-encrypted (AES-256-GCM). The encryption key stays on the client device.

**Encrypted:** title, description, comments, custom fields -- everything that constitutes item content.

**Cleartext metadata:** id, status, priority, due_date, timestamps, project name. Required for server-side notifications, CalDAV VTODO scheduling, and overview dashboards.

**Decryption by interface:**

- CLI decrypts locally
- WebUI decrypts client-side in the browser (Web Crypto API)
- CalDAV is a conscious opt-in: the user authorizes joyint.com to decrypt for CalDAV delivery (e.g. via `jot key share`). Without opt-in, no CalDAV -- only CLI and WebUI

**Key management:** Per-project key stored in OS keychain (via `keyring` crate) with fallback to `~/.config/joy/keys/{project-id}.key`. Key generation via `joy key init` or `jot key init`.

## Consequences

The server stores only encrypted blobs plus cleartext metadata. No access to sensitive content. CalDAV integration requires explicit trust delegation from the user -- privacy is the default, convenience is opt-in.

Trade-off: full-text search requires client-side decryption (slower for large projects). Cleartext metadata exposes structure (how many items, their status distribution) but not content.
