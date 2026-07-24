# AI jobs and AI members (git-native)

AI orchestration data is git-native, like everything in Joy. joy-core owns
the models, so the CLI, the desktop app, and the platform all read and
write the same records. The forge is the source of truth; the audit trail
is `.joy/logs`; the history is git; the work product of a repo job is a
branch on the forge.

## Jobs are items — `.joy/jobs/<id>.yaml`

A delegated unit of AI work is a first-class **job item** (`ItemType::Job`)
stored under `.joy/jobs/`, kept apart from the product backlog: deletable
without touching the backlog, invisible to default views, lower merge
contention. The `-JOB-` segment in the id routes between the two
directories. List them with `joy ls -J`; create one with `joy add job`.
The job payload (scope, budget, window) rides on the item.

The legacy parallel record store at `.joy/ai/jobs/<id>.yaml` is retired;
the `m_2026_07_remove_ai_jobs` repo migration removes any leftover.

## AI members carry their own execution config

An AI member is a project member whose id is `ai:<tool>@joy` (see the
canonical naming rules in `joy_ai::naming`). How the member runs — which
ACP **adapter** drives it (`claude-code` | `mistral-vibe` | `qwen-code` |
`copilot` | `mock`) — is recorded on the `project.yaml` member itself, so
the platform can route turns without a per-member file. Provider API keys
are **not** in the repo: platform keys live in the platform database
(account-scoped), and locally run tools use their own subscription or
environment.

The legacy execution-config store at `.joy/ai/agents/<member>.yaml` is
retired; the `m_2026_07_remove_ai_agents` repo migration removes any
leftover.

## Interaction level vs. agent mode

Two distinct oversight concepts, easy to confuse:

- **Interaction level** (`InteractionLevel`): the project's three-step
  oversight ladder (JI-0166-D8): `autonomous`, `confirmed`, `proposing`.
  The per-project default is `interaction-level.default` in config;
  per-capability defaults live in `project.defaults.yaml`, member defaults
  next to the capabilities in `project.yaml`.
- **Agent mode** (`AgentMode`): the ACP permission mode a chat turn runs
  under (`plan` / `accept-edits` / `autonomous`). It is per-(member,
  delegator) on a chat, never a project-wide interaction level.

## CLI

```
joy ai init         # set up AI tool integration and register AI members
joy ai reset        # remove AI tool config / delegations
joy ai tutorial     # the operational guide for AI assistants
joy ls -J           # list job items
```

The desktop app and the platform delegate, review, and approve through the
same joy-core models; the platform additionally runs the member in a
sandbox container and pushes the branch. Audit for every job action lands
in `.joy/logs`.
