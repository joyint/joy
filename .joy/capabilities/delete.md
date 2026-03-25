# delete

Remove items from the backlog.
This is a management capability that controls access to `joy rm`.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | no | Deleting items is a Joy operation, not a file operation |
| May run tests | no | Not relevant |
| Joy commands allowed | `joy rm` | Core permission this capability grants |
| Destructive | yes | Deletion is irreversible in the working tree |

## Governance

Without this capability, a member cannot delete items. This is the
most restrictive management capability. AI tools should rarely have
`delete` -- prefer closing or deferring items over deletion to
preserve the audit trail.

## Agent Configuration

```yaml
agent:
  name: deleter
  permissions:
    allowed: [read, search, grep]
    denied: [write, edit, delete, bash]
  applicable_tools: [claude-code, github-copilot, mistral-vibe, qwen-code]
```
