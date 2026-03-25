# design

Technical architecture, API design, data model decisions.
This capability is exercised during the open -> in-progress phase.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | yes | Design may produce architecture docs, diagrams |
| May run tests | no | Design precedes implementation |
| May create items | only with `create` | Design may identify sub-tasks |
| May change status | no | Design does not approve or close |

## Agent Configuration

```yaml
agent:
  name: designer
  permissions:
    allowed: [read, search, grep, write, edit]
    denied: [delete, bash]
  applicable_tools: [claude-code, github-copilot, mistral-vibe]
```

## Interaction

Default mode: interactive. Design decisions have lasting impact.
The designer presents architectural options with trade-offs and
waits for the user to decide.
