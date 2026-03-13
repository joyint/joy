# ADR-009: E-mail as user identity with OAuth authentication

**Status:** Accepted

## Context

Joy needs a user identity model for role-based access (status rules), item assignment, change attribution, and sync authentication. Options: platform-specific handles (e.g. GitHub username), custom Joy accounts, or e-mail addresses.

## Decision

E-mail address as the universal user identifier. Authentication via OAuth 2.0 with GitHub, GitLab, and Gitea as initial providers.

Locally, the e-mail is read from `git config user.email` -- zero configuration for CLI users. On the server, users authenticate via OAuth. The server matches the OAuth-provided e-mail against project role definitions. JWTs are issued after login for subsequent API calls.

AI agents use a synthetic `agent:role@joy` identity (e.g. `agent:implementer@joy`) to distinguish agent actions from human actions.

## Consequences

E-mail is provider-independent: switching from GitHub to GitLab or Gitea (or adding Google, Microsoft later) requires no migration of roles, assignments, or history. Every developer already has an e-mail in their Git config, so the CLI works without additional setup. The trade-off is that e-mail addresses can change -- but so can usernames on any platform, and e-mail changes are less frequent. The `agent:` prefix convention cleanly separates human and AI identities without a separate account system.
