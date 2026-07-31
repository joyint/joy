# test

Verify correctness, write tests, validate behavior.
This capability is exercised during the in-progress phase.

## Constraints

| Rule | Value | Reason |
|------|-------|--------|
| May modify files | yes | Writing test code |
| May run tests | yes | Core activity: running tests |
| May create items | no | Test findings are reported as comments |
| May change status | no | Testing does not approve or close |

## Agent Configuration

```yaml
agent:
  name: tester
  permissions:
    allowed: [read, search, grep, write, edit, bash]
    denied: [delete]
  applicable_tools: [claude, copilot, vibe, qwen]
```

## Interaction

Default interaction level: autonomous. The tester works independently;
governance gates remain the only checkpoints.
