# Joy — Product Management CLI

> For shared concepts (data model, identity, dispatch, principles) see [README.md](./README.md).
> For the platform and sync layer see [joyint-com.md](./joyint-com.md).

## Target Audience

- Solo developers managing side projects with AI agents
- Small development teams (2-10) replacing Jira or Linear
- Enterprises in regulated industries needing AI governance, audit trails, and compliance

## Item Types

| Type | Purpose |
|------|---------|
| `epic` | Large feature or initiative, groups other items |
| `story` | User-facing functionality |
| `task` | Technical work, not directly user-facing |
| `bug` | Defect to fix |
| `rework` | Refactoring or improvement of existing code |
| `decision` | Architectural or product decision to document |
| `idea` | Spontaneous idea, not yet refined into a concrete item |

## Status Workflow

The default status flow:

```mermaid
stateDiagram-v2
    [*] --> new
    new --> open : approve
    new --> deferred : defer
    open --> in_progress : start
    state in_progress : in-progress
    open --> deferred : defer
    in_progress --> review : submit
    review --> closed : accept
    review --> in_progress : rework
    deferred --> new : reopen
    closed --> [*]
```

`blocked` is not a manual state — it is computed automatically from dependencies.

The status model is intentionally minimal — most teams need 5-6 states, not 15.

### Status Rules

**One process with dimmers, not multiple processes with switches.** There is exactly one workflow per project. It is not templated, not selectable, not importable. Instead, individual transitions can be tightened or loosened via rules in `.joy/project.yaml` (see [Architecture.md](../Architecture.md#project-roles-and-status-rules-joyprojectyaml)).

A solo founder uses Joy with zero rules — every transition is open. A team adds a gate on `review -> closed` so only leads can accept work. A regulated project adds a second gate on `new -> open` for triage. Same workflow, different strictness. This scales from "no process" to "controlled process" without switching modes, importing templates, or learning a new concept.

Available gates:

- **new -> open** (triage gate): only approvers can move items into the backlog
- **review -> closed** (acceptance gate): only approvers can close items, optionally requiring green CI

Each gate supports `requires_role`, `requires_ci`, and `allow_ai`. By default all transitions are unrestricted. Gates are opt-in. AI agents can be excluded from gated transitions via `allow_ai: false`.

### Dependencies

Dependencies are modeled as a simple list of item IDs in the `deps` field. One direction only: "I need X before I can start Y."

**Automatic behaviors:**

- Starting an item with open deps triggers a warning (not a block)
- Closing an item notifies dependents that they are unblocked
- `joy ls --blocked` shows all items waiting on dependencies
- Cycle detection prevents circular dependencies

---

## CLI Commands

### Project

```sh
joy                                     # Board/overview — the most used command
joy init                                # Initialize new project
  joy init --name "Joyint" --acronym JI

joy project                             # View/edit project info (interactive)
  joy project --name "Joyint Platform"

joy log                                 # Event log (audit trail)
  joy log --since 7d
  joy log --item JOY-002A
  joy log --limit 50                    # max entries (default: 20)

joy roadmap                             # Milestone roadmap (tree view)
  joy roadmap --all                    # include closed and deferred items

joy find [query]                        # Search items by text
  joy find "payment"                   # case-insensitive, matches title and description

joy board                               # Board overview (same as bare joy)
  joy board --all                      # show all items (no limit per group)
```

### Items

```sh
joy add <TYPE> <TITLE> [OPTIONS]         # Create new item
  joy add story "Login Page" --parent JOY-0001 --priority high
  joy add bug "Crash bei Umlauten" --version v0.5.0
  joy add epic "Auth System"

joy edit [id]                           # Edit item
  joy edit JOY-002A --title "Payment v2" --priority critical
  joy edit JOY-002A --version v0.5.0    # set version tag
  joy edit JOY-002A --version none      # remove version tag

joy rm [id]                             # Delete item (with confirmation)
  joy rm JOY-002A
  joy rm JOY-002A --force                # skip confirmation
  joy rm JOY-0001 --recursive             # item + all descendants
  joy rm -rf JOY-0001                     # same as --recursive --force

joy ls                                  # List and filter items
  joy ls                                # active items (excludes closed and deferred)
  joy ls --all                          # all items including closed and deferred
  joy ls --parent JOY-0001                # items of a parent (and descendants)
  joy ls --type bug                     # only bugs
  joy ls --status in-progress           # by status
  joy ls --blocked                      # items with open deps
  joy ls --priority critical            # by priority
  joy ls --mine                         # assigned to me
  joy ls --milestone JOY-MS-01              # by milestone (includes inherited)
  joy ls --tree                         # hierarchical tree view
  joy ls --tree --group milestone       # tree grouped by milestone
  joy ls --tag backend                  # by tag
  joy ls --version v0.5.0               # by version tag
  joy ls --show milestone,assignee      # extra columns (milestone, assignee, parent)

joy show [id]                           # Detail view
  joy show JOY-002A                      # all info, deps, history, comments
```

### Status

```sh
joy status [id] [state]                 # Change status
  joy status JOY-002A in-progress
  joy status JOY-002A closed             # warns if dependents still open
  joy status JOY-0001 closed              # warns if child items still open
  # Adding children to a closed parent triggers a warning


# Shortcuts for common transitions
joy start [id]                          # alias for: joy status [id] in-progress
joy submit [id]                         # alias for: joy status [id] review
joy close [id]                          # alias for: joy status [id] closed
joy reopen [id]                         # alias for: joy status [id] open
```

### Release

```sh
joy release                             # Show items for latest git tag
  joy release v0.5.0                    # Show items tagged with v0.5.0
```

Version tags link items to git releases. Use `--version` on `joy add` or `joy edit` to tag items.
If no git tags exist, `joy release` without arguments shows a usage hint (no automatism).

### Assignment

```sh
joy assign [id] [email]                 # Assign item to a person or agent
  joy assign JOY-002A orchidee@joyint.com
  joy assign JOY-002A --unassign         # remove assignment
```

### Comments

```sh
joy comment [id] [text]                 # Add a comment to an item
  joy comment JOY-002A "Looks good, ready to merge."
```

### Dependencies

```sh
joy deps [id]                           # Show dependencies
  joy deps JOY-002A                      # list
  joy deps JOY-002A --tree               # tree view
  joy deps JOY-002A --add JOY-0017        # add dependency
  joy deps JOY-002A --rm JOY-0017         # remove dependency
```

### Milestones

```sh
joy milestone add [name]                # Create milestone
  joy milestone add "Beta Release" --date 2026-06-01

joy milestone ls                        # List milestones

joy milestone rm [id]                   # Delete milestone
joy milestone show [id]                 # Detail: items, progress, risks

joy milestone link [item-id] [ms-id]    # Assign item to milestone
  joy milestone link JOY-002A JOY-MS-01

joy milestone unlink [item-id]          # Remove item from milestone

joy milestone edit [id]                 # Modify milestone
  joy milestone edit JOY-MS-01 --title "Beta" --date 2026-07-01
```

### Sync & Collaboration

```sh
joy sync                                # Bidirectional sync (default)
  joy sync --push                       # upload only
  joy sync --pull                       # download only
  joy sync --auto                       # background sync

joy clone [url]                         # Clone project from remote
  joy clone joyint.com/joydev/platform
```

### App & Server

```sh
joy app                                 # TUI (default)

joy serve                               # Start server (for remote sync + web UI)
  joy serve --config server.yaml
  joy serve --daemon                    # Run as background process
```

### Shell Completions & Help

```sh
joy completions [shell]                 # Generate shell completions
  joy completions bash
  joy completions zsh
  joy completions fish

joy tutorial                            # Read the tutorial in a pager
```

---

## AI Integration

Joy supports two modes of AI integration:

### Tool mode (MS-02): Joy as a tool for AI agents

External AI agents use Joy's CLI as a tool. The agent calls `joy` commands
to read, create, and manage items. This already works today via skill
definitions (e.g. the `/joy` Claude Code skill). `joy ai setup tool`
formalizes this by generating the appropriate config files for the detected
AI tool.

Supported tools for tool mode:

| Tool | Config ID | Integration |
|------|-----------|-------------|
| Claude Code (Anthropic) | `claude-code` | Skill file + CLAUDE.md |
| GitHub Copilot (GitHub) | `github-copilot` | `.github/copilot-instructions.md` |
| Mistral Vibe (Mistral) | `mistral-vibe` | Project instructions |
| Qwen Code (Alibaba) | `qwen-code` | Project instructions |

No own agent runtime, no API calls, no cost tracking needed. The AI tool
handles everything -- Joy just provides the product management interface.

### Agent mode (MS-05): Joy dispatches to AI

Joy actively dispatches work to external AI tools and tracks results:

| Tool | Config ID | Command |
|------|-----------|---------|
| Claude Code (Anthropic) | `claude-code` | `claude` |
| Mistral Vibe (Mistral) | `mistral-vibe` | `vibe` |
| GitHub Copilot (GitHub) | `github-copilot` | `copilot` |
| Qwen Code (Alibaba) | `qwen-code` | `qwen` |

Each project configures one tool via `joy ai setup agent`. Joy is the
**dispatcher**, not the **runtime** -- it prepares context, invokes the
tool, and tracks the outcome.

**Workflows:**
- **Estimation:** AI estimates effort and cost from item context
- **Planning:** AI proposes epic breakdown into stories and tasks
- **Implementation:** AI generates code, Joy tracks the job
- **Review:** AI reviews implementation against acceptance criteria
- **Status Intelligence:** Joy suggests status updates from git activity

### AI Commands

```sh
joy ai setup tool                       # Generate config for external AI agent (MS-02)
joy ai setup agent [tool]               # Configure Joy as AI dispatcher (MS-05)
  joy ai setup agent claude-code
  joy ai setup agent mistral-vibe --model devstral-small

joy ai estimate [id]                    # Estimate effort and cost
  joy ai estimate JOY-002A
  joy ai estimate JOY-0001                # estimate all items in epic

joy ai plan [id]                        # Break epic into items
  joy ai plan JOY-0001

joy ai implement [id]                   # AI agent implements item
  joy ai implement JOY-002A
  joy ai implement JOY-002A --budget 5.00

joy ai review [id]                      # AI reviews implementation
  joy ai review JOY-002A

joy ai status                           # Show running AI jobs
  joy ai status --history               # include completed jobs
```

### Agent Configuration

```yaml
# .joy/config.yaml (ai section)
ai:
  mode: agent                  # tool | agent
  tool: claude-code            # claude-code | mistral-vibe | github-copilot | qwen-code
  command: claude              # CLI command to invoke
  model: auto                  # model name or "auto" (tool default)
  max_cost_per_job: 10.00
  currency: EUR
```

### Cost Tracking (agent mode)

Every AI job logs its cost:

```yaml
# .joy/ai/jobs/JOY-JOB-000F.yaml
id: JOY-JOB-000F
item: JOY-002A
type: implement
tool: claude-code
status: completed
started: 2026-03-09T14:00:00Z
completed: 2026-03-09T14:12:00Z
tokens_in: 45200
tokens_out: 12800
cost: 0.42
currency: EUR
result:
  branch: feat/JOY-002A-payment-integration
  commits: 3
  files_changed: 7
```

Aggregated cost views available via `joy ai status --costs` per item, epic, milestone, or time range.

---

## Bootstrapping: Building Joy with Joy

Joy is built using itself from the earliest possible moment.

### MS-01 — Core CLI (complete since v0.5.0)

The CLI is the foundation. All core commands are implemented and Joy manages
its own backlog since v0.5.0:

- `joy init`, `joy add`, `joy ls`, `joy show`, `joy edit`, `joy rm`
- `joy status`, `joy start`, `joy submit`, `joy close`, `joy reopen`
- `joy assign`, `joy comment`, `joy deps`, `joy milestone`, `joy log`
- `joy roadmap`, `joy find`, `joy board`, `joy project`, `joy tutorial`
- Semantic ANSI colors, emoji indicators, compact table formatting
- Structured event log, shell completions

### MS-02 — AI tool mode

Joy as a tool/skill for external AI agents. The CLI is already usable by
AI agents (e.g. Claude Code via `/joy` skill). This phase formalizes it:

- `joy ai setup tool` — generate config files for external AI agents
  (skill definitions, MCP tool specs, or equivalent for each tool)
- Standardized instructions that work across AI tools
- No own agent runtime — the external tool calls `joy` commands

### MS-03 — Sync and Server

- `joy serve` -- HTTP server (REST API, Git gateway, CalDAV)
- `joy sync` -- push/pull via Git remote
- `joy clone` -- clone remote project
- OAuth authentication (GitHub, Gitea)
- E2E encryption (AES-256-GCM, always active on joyint.com)
- Local key management (`joy key init`)

### MS-04 — Web UI and Portal

- SolidJS web frontend (board, item management, roadmap) for Joy and Jot
- Embedded in `joy serve`
- CalDAV server for Jot (VTODO bridge to Apple Reminders, Google Calendar)
- Notification service (due dates, status changes, mentions)
- joyint.com deployment (managed hosting)
- Tauri native app (desktop and mobile)

### MS-05 — AI agent mode

Joy dispatches work to AI APIs and tracks results:

- `joy ai setup agent` — configure AI tool and model
- `joy ai estimate` — effort and cost estimation
- `joy ai plan` — break epic into items
- `joy ai implement` — dispatch to configured AI tool
- `joy ai review` — automated code review
- `joy ai status` — monitor jobs, track costs
- Job logging and cost tracking

### MS-06 — TUI

- `joy app` — ratatui-based terminal UI
- Board view, item detail panel, dependency graph
