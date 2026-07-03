# AI Jobs and Agents (git-native)

AI orchestration data is git-native, like everything in Joy: an AI **job**
is `.joy/ai/jobs/<id>.yaml` and an AI **agent** config is
`.joy/ai/agents/<member>.yaml`. joy-core owns the model, so the CLI, the
desktop app, and the platform all read and write the same records. The forge
is the source of truth; the audit trail is `.joy/logs`; the history is git;
the work product of a repo job is a branch on the forge. No database is
involved (the platform's Postgres is only chat pub/sub and account info).

## Job record — `.joy/ai/jobs/<id>.yaml`

The current state of one unit of AI work derived from an item:

```yaml
id: 0f2c1e9a4b7d
item: LP-0002-FE
type: implement            # implement | review | estimate | analyze | plan
actor: ai:claude@joy       # the AI member doing the work
delegated_by: horst@example.com
status: awaiting-approval  # see lifecycle below
branch: joy/claude/LP-0002-FE-0f2c1e   # repo work only
budget: { max_cents: 500, currency: EUR }
cost:   { spent_cents: 1, tokens: 240 }
created: 2026-07-04T00:08:00Z
updated: 2026-07-04T00:08:07Z
result: "created hello.txt -- proposal on branch … awaiting approval"
reviews:                   # human review rounds, oldest first
  - at: 2026-07-04T00:10:00Z
    by: horst@example.com
    decision: request-changes   # request-changes | approve
    feedback: make the greeting warmer
```

Money is held in whole cents to avoid floating-point drift. The record is
committed and pushed on every meaningful status change, so anyone viewing
the item sees the current status.

### Lifecycle

```
queued -> running -> awaiting-approval
                       |          ^
        (human)        |          |
   request-changes ->  changes-requested -> running --+
   approve         ->  done
   (failure)       ->  failed        (cancel) -> cancelled
```

The gate holds at `awaiting-approval`; the human either requests changes
(another round runs on the same branch, feedback carried in) or approves
(the branch merges, the job is `done`).

## Agent config — `.joy/ai/agents/<member>.yaml`

How an AI member runs. The API key is a secret referenced out of band
(platform secret store / OS keychain) and is **never** stored here.

```yaml
member: ai:claude@joy
adapter: mock              # mock | claude-code | mistral-vibe | qwen-code
model: claude-sonnet-4
provider: anthropic
default_mode: collaborative
budget_default: { max_cents: 500, currency: EUR }
```

## CLI

```
joy ai jobs                # list jobs, newest first
joy ai jobs --item LP-0002 # only that item's jobs
joy ai agents              # list agent configs (never shows a key)
```

The desktop app and the platform delegate, review, and approve through the
same joy-core model; the platform additionally runs the agent in a sandbox
container and pushes the branch. Audit for every job action lands in
`.joy/logs`.
