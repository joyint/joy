# review

Verify correctness, quality, and adherence to project standards.
This capability is exercised during the review -> closed transition.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | no | Review observes, it does not change |
| May run tests | yes | Verifying existing tests is part of review |
| May create items | no | Findings are reported as comments |
| May change status | yes | Accept or rework |

## Agent Configuration

```yaml
agent:
  name: reviewer
  permissions:
    allowed: [read, search, grep, test]
    denied: [write, edit, delete]
  applicable_tools: [claude-code, github-copilot, mistral-vibe]
```

## Interaction

Default mode: interactive. The reviewer presents findings with rationale
and waits for the assignee or author to decide on each point.
