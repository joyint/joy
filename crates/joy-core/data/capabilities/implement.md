# implement

Write code, configuration, and infrastructure.
This capability is exercised during the in-progress phase.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | yes | Core activity: writing code |
| May run tests | yes | Verifying own implementation |
| May create items | no | Implementation follows the plan |
| May change status | yes | Start and submit for review |

## Agent Configuration

```yaml
agent:
  name: implementer
  permissions:
    allowed: [read, search, grep, write, edit, bash]
    denied: [delete]
  applicable_tools: [claude, copilot, vibe, qwen]
```

## Interaction

Default interaction level: confirmed. The implementer works
independently on the code and confirms before irreversible actions.
