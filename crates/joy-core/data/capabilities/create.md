# create

Create new items in the backlog.
This is a management capability that controls access to `joy add`.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | no | Creating items is a Joy operation, not a file operation |
| May run tests | no | Not relevant |
| Joy commands allowed | `joy add` | Core permission this capability grants |
| Requires combination | Often combined with `plan` or `conceive` | Creating items without planning context is rarely useful |

## Governance

Without this capability, a member cannot create items. This prevents
AI tools from generating items autonomously. A planner needs
`conceive, plan, create` to define and create items. An implementer
with only `implement` cannot add items -- discovered bugs must be
reported to someone with `create`.

## Agent Configuration

```yaml
agent:
  name: creator
  permissions:
    allowed: [read, search, grep]
    denied: [write, edit, delete, bash]
  applicable_tools: [claude-code, github-copilot, mistral-vibe, qwen-code]
```
