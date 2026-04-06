# plan

Break down work, scope items, prioritize, and estimate effort.
This capability is exercised during the new -> open phase.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | no | Planning scopes work, does not produce code |
| May run tests | no | No code to test at this stage |
| May create items | only with `create` | Breaking down epics requires item creation |
| May change status | yes | Move items through triage |

## Agent Configuration

```yaml
agent:
  name: planner
  permissions:
    allowed: [read, search, grep]
    denied: [write, edit, delete, bash]
  applicable_tools: [claude-code, github-copilot, mistral-vibe]
```

## Interaction

Default mode: interactive. The planner presents options with rationale
and waits for the user to decide on scope, priority, and breakdown.
