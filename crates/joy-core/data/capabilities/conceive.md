# conceive

Define product vision, requirements, and process design.
This capability is exercised during the new -> open phase.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | no | Conceiving shapes ideas, not code |
| May run tests | no | No code to test at this stage |
| May create items | only with `create` | Conceiving alone does not grant item creation |
| May change status | yes | Approve items from new to open |

## Agent Configuration

```yaml
agent:
  name: conceiver
  permissions:
    allowed: [read, search, grep]
    denied: [write, edit, delete, bash]
  applicable_tools: [claude-code, github-copilot, mistral-vibe]
```

## Interaction

Default mode: pairing. Conceiving is a co-creation activity where
human and AI explore ideas together step by step.
