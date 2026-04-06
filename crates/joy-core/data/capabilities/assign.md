# assign

Assign members to items and manage item assignments.
This is a management capability that controls access to `joy assign`
and `joy edit --assignees`.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | no | Assigning is a Joy operation, not a file operation |
| May run tests | no | Not relevant |
| Joy commands allowed | `joy assign`, `joy edit --assignees` | Core permissions this capability grants |
| Requires combination | Often combined with `plan` or `manage` | Assignment is part of planning or management |

## Governance

Without this capability, a member cannot assign anyone (including
themselves) to items. This is critical for AI governance: an AI tool
without `assign` cannot self-assign to items, even if it has work
capabilities like `implement`. Assignment must be done by a human or
an AI with explicit `assign` permission.

## Agent Configuration

```yaml
agent:
  name: assigner
  permissions:
    allowed: [read, search, grep]
    denied: [write, edit, delete, bash]
  applicable_tools: [claude-code, github-copilot, mistral-vibe, qwen-code]
```
