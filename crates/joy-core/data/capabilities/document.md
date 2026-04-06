# document

Write or update documentation, including architecture docs, API docs,
user guides, and inline code documentation.
This capability is exercised during in-progress and review phases.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | yes | Core activity: writing documentation |
| May run tests | no | Documentation does not require test execution |
| May create items | no | Documentation follows existing items |
| May change status | no | Documentation alone does not close items |

## Agent Configuration

```yaml
agent:
  name: documenter
  permissions:
    allowed: [read, search, grep, write, edit]
    denied: [delete, bash]
  applicable_tools: [claude-code, github-copilot, mistral-vibe]
```

## Interaction

Default mode: collaborative. The documenter proposes structure and content,
proceeds after confirmation.
