# manage

Manage project configuration and member administration.
This is a management capability that controls access to project-level
operations.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | no | Managing is a Joy operation, not a file operation |
| May run tests | no | Not relevant |
| Joy commands allowed | `joy project member add/rm`, `joy config set` | Core permissions this capability grants |
| Typically held by | Project leads, administrators | High-trust capability |

## Governance

Without this capability, a member cannot add or remove project members,
or change project configuration. This prevents AI tools from modifying
the project's governance structure. Only explicitly trusted members
should hold `manage`.

## Agent Configuration

```yaml
agent:
  name: manager
  permissions:
    allowed: [read, search, grep]
    denied: [write, edit, delete, bash]
  applicable_tools: [claude-code, github-copilot, mistral-vibe, qwen-code]
```
