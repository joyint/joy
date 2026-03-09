# Joy -- Backlog

This file is the initial product backlog for Joy. It is structured as Epics, Stories, and Tasks using Joy's own item format. After Phase 0 (`joy init` + `joy add`), the content of this file will be imported into `.joy/` and managed with Joy itself. This file then becomes obsolete.

Items are listed in YAML blocks for direct import. Priorities: critical, high, medium, low. Dependencies are noted where known.

For the overall product vision see [Vision.md](./Vision.md). For architecture see [Architecture.md](./Architecture.md).

---

## EP-0001: Core CLI

The foundation. Minimal commands to manage items from the terminal.

```yaml
id: EP-0001
title: Core CLI
type: epic
status: new
priority: critical
milestone: MS-01
description: |
  Implement the core CLI commands that make Joy usable for daily
  product management. This is the minimum viable product.
```

### Stories and Tasks

```yaml
- id: IT-0001
  title: "joy init: project initialization"
  type: task
  epic: EP-0001
  priority: critical
  description: |
    Create .joy/ directory structure with config.yaml and project.yaml.
    Detect existing Git repo or initialize one.
    Print minimal intro with most important commands.

- id: IT-0002
  title: "joy add: create items interactively"
  type: story
  epic: EP-0001
  priority: critical
  deps: [IT-0001]
  description: |
    Create new items via interactive prompts (title, type, priority, epic, description).
    Support non-interactive mode via flags for scripting.
    Generate unique IDs by scanning existing filenames and incrementing the highest found value.
    Write YAML file to .joy/items/{ID}-{slug}.yaml.

- id: IT-0003
  title: "joy ls: list and filter items"
  type: story
  epic: EP-0001
  priority: critical
  deps: [IT-0001]
  description: |
    List all active items by default (excludes closed and deferred).
    Support filters: --epic, --type, --status, --priority, --mine,
    --blocked, --blocking.
    Support --tree for hierarchical view (epics with children).

- id: IT-0004
  title: "joy status: change item status"
  type: story
  epic: EP-0001
  priority: critical
  deps: [IT-0002]
  description: |
    Change status of an item by ID.
    Warn when closing an epic with open child items.
    Warn when starting an item with open dependencies.
    Auto-close epics when all children are closed (configurable).

- id: IT-0005
  title: "joy: board overview (default command)"
  type: story
  epic: EP-0001
  priority: critical
  deps: [IT-0003]
  description: |
    Running joy without arguments shows a compact project overview.
    Group items by status. Show counts per status.
    Show next milestone with progress.
    Target: render in <100ms for 100 items.

- id: IT-0006
  title: "joy edit: modify existing items"
  type: story
  epic: EP-0001
  priority: high
  deps: [IT-0002]
  description: |
    Edit item fields interactively or via flags.
    Update the updated timestamp automatically.

- id: IT-0007
  title: "joy rm: delete items"
  type: story
  epic: EP-0001
  priority: high
  deps: [IT-0002]
  description: |
    Delete items with confirmation prompt.
    Support --force to skip confirmation.
    Support --cascade on epics to delete all linked items.
    Remove deleted item IDs from other items' deps lists.

- id: IT-0008
  title: "joy show: item detail view"
  type: story
  epic: EP-0001
  priority: high
  deps: [IT-0002]
  description: |
    Display full item details: all fields, dependencies (with status),
    comments, and change history from git log.

- id: IT-0009
  title: "joy deps: dependency management"
  type: story
  epic: EP-0001
  priority: high
  deps: [IT-0002]
  description: |
    Show dependencies for an item (list and --tree view).
    Add and remove dependencies via --add and --rm.
    Detect and reject circular dependencies.

- id: IT-000A
  title: "joy milestone: milestone management"
  type: story
  epic: EP-0001
  priority: medium
  deps: [IT-0002]
  description: |
    Create, list, show, remove milestones.
    Link items to milestones.
    Show progress (items closed / total) and risks (blocked items,
    items with deps outside milestone).

- id: IT-000B
  title: "joy log: change history"
  type: story
  epic: EP-0001
  priority: medium
  deps: [IT-0001]
  description: |
    Chronological log of item changes.
    Support --since for time filtering.
    Support --item for per-item history.
    Derive from git log on .joy/ files.

- id: IT-000C
  title: "joy project: view and edit project metadata"
  type: task
  epic: EP-0001
  priority: medium
  deps: [IT-0001]
  description: |
    Display and edit project.yaml fields (name, acronym, description).
    Interactive mode when called without flags.

- id: IT-000D
  title: "joy completions: shell completion generation"
  type: task
  epic: EP-0001
  priority: medium
  deps: [IT-0001]
  description: |
    Generate completions for bash, zsh, fish, PowerShell, elvish
    via clap_complete.
    Include dynamic completion for item IDs, status values, types.

- id: IT-0029
  title: "joy assign: assign and unassign items"
  type: story
  epic: EP-0001
  priority: medium
  deps: [IT-0001]
  description: |
    Assign an item to a person (email) or agent (agent:role@joy).
    --unassign removes the assignment.
    Validates email format. Updates assignee field in YAML.

- id: IT-002A
  title: "joy comment: add comments to items"
  type: story
  epic: EP-0001
  priority: medium
  deps: [IT-0001]
  description: |
    Add a comment to an item with author (from git config user.email)
    and timestamp. Inline text via argument or interactive via $EDITOR.
    Comments are appended to the comments list in the item YAML.

- id: IT-002B
  title: "Status shortcuts: joy start, joy submit, joy close"
  type: task
  epic: EP-0001
  priority: low
  deps: [IT-0004]
  description: |
    Convenience aliases for common status transitions:
    - joy start [id] -> joy status [id] in-progress
    - joy submit [id] -> joy status [id] review
    - joy close [id] -> joy status [id] closed
    Implemented as clap aliases or thin wrapper commands.
```

---

## EP-0002: Terminal Output and UX

How Joy communicates with the user in the terminal.

```yaml
id: EP-0002
title: Terminal Output and UX
type: epic
status: new
priority: high
milestone: MS-01
description: |
  Define and implement the visual language of Joy's terminal output.
  Colors, indicators, formatting, and user preferences.
```

### Stories and Tasks

```yaml
- id: IT-000E
  title: Semantic color scheme for terminal output
  type: story
  epic: EP-0002
  priority: high
  description: |
    Implement color scheme using console/owo-colors crate.
    Semantic mapping:
      - Status new: white/default
      - Status open: blue
      - Status in-progress: yellow
      - Status review: cyan
      - Status closed: green
      - Status deferred: dim/gray
      - Priority critical: red bold
      - IDs: bold
      - Warnings: yellow
      - Errors: red
    Deactivation via --no-color flag and NO_COLOR env var
    (no-color.org standard).
    Auto-detect non-TTY (pipes, redirects) and disable colors.

- id: IT-000F
  title: Emoji indicators for item types and status
  type: story
  epic: EP-0002
  priority: medium
  description: |
    Use emoji as visual indicators for type and status at runtime.
    Type mapping (emoji -> text fallback):
      - epic: clipboard icon -> [epic]
      - story: book icon -> [story]
      - task: wrench icon -> [task]
      - bug: bug icon -> [bug]
      - rework: recycle icon -> [rework]
      - decision: lightbulb icon -> [decision]
    Status mapping:
      - open: circle -> *
      - in-progress: circle -> *
      - review: eyes icon -> ?
      - closed: checkmark -> +
      - deferred: pause icon -> ~
    Deactivation via --no-emoji flag and JOY_NO_EMOJI=1 env var.

- id: IT-0010
  title: Output format configuration in config.yaml
  type: task
  epic: EP-0002
  priority: medium
  deps: [IT-000E, IT-000F]
  description: |
    Persist output preferences in .joy/config.yaml:
      output:
        color: auto | always | never
        emoji: true | false
    CLI flags override config. Env vars override flags.

- id: IT-0011
  title: Compact table formatting for joy ls
  type: task
  epic: EP-0002
  priority: medium
  deps: [IT-0003, IT-000E]
  description: |
    Aligned columns for ID, title, status, priority.
    Respect terminal width, truncate long titles.
    Consistent spacing in tree view.
```

---

## EP-0003: AI Integration

AI-powered estimation, planning, implementation, and review.

```yaml
id: EP-0003
title: AI Integration
type: epic
status: new
priority: high
milestone: MS-02
description: |
  Integrate external AI tools (Claude Code, Mistral Vibe) as first-class collaborators.
  Joy dispatches work to the configured tool and tracks results, costs, and
  status. No own agent runtime -- one tool per project, with model or auto.
```

### Stories and Tasks

```yaml
- id: IT-0012
  title: "joy ai setup: tool and model configuration"
  type: story
  epic: EP-0003
  priority: high
  description: |
    Configure one AI tool per project (claude-code, mistral-vibe, github-copilot, qwen-code).
    Detect installed CLI tools. Set model name or "auto".
    Write tool config to .joy/config.yaml (ai section).
    Store API key in credentials.yaml (project-local or global).

- id: IT-0013
  title: "joy ai estimate: effort and cost estimation"
  type: story
  epic: EP-0003
  priority: high
  deps: [IT-0012]
  description: |
    AI analyzes item description, codebase context, and dependencies.
    Estimate effort in hours and cost based on provider rates.
    Support estimating all items in an epic.

- id: IT-0014
  title: "joy ai plan: break epic into items"
  type: story
  epic: EP-0003
  priority: high
  deps: [IT-0012]
  description: |
    Given an epic, AI proposes breakdown into stories, tasks, dependencies.
    Present proposal for human review.
    Approved items are created via joy add.

- id: IT-0015
  title: "joy ai implement: dispatch to external agent CLI"
  type: story
  epic: EP-0003
  priority: high
  deps: [IT-0012]
  description: |
    Prepare context (item YAML, relevant code paths, branch name).
    Invoke configured AI tool (e.g. claude, vibe).
    Track job in .joy/ai/jobs/.
    Support --budget flag.
    Joy is the dispatcher, the external tool handles code generation.

- id: IT-0016
  title: "joy ai review: automated code review"
  type: story
  epic: EP-0003
  priority: medium
  deps: [IT-0012]
  description: |
    AI reviews implementation against acceptance criteria and conventions.
    Structured output: pass/fail with comments.

- id: IT-0017
  title: "joy ai status: monitor AI jobs"
  type: story
  epic: EP-0003
  priority: medium
  deps: [IT-0015]
  description: |
    Show running AI jobs with progress.
    Support --history for completed jobs.
    Support --costs for cost aggregation per item, epic, milestone, or time range.

- id: IT-0018
  title: AI cost tracking and job logging
  type: task
  epic: EP-0003
  priority: high
  deps: [IT-0015]
  description: |
    Every AI job writes a JOB-{ID}.yaml with:
    item, type, provider, model, status, timestamps,
    tokens_in, tokens_out, cost, currency, result.
    Next JOB ID derived from scanning existing filenames.

- id: IT-0019
  title: Status intelligence from git activity
  type: story
  epic: EP-0003
  priority: low
  deps: [IT-0012, IT-0004]
  description: |
    AI analyzes git log, branch names, and commit messages.
    Suggest status updates for items based on development activity.
    Example: "IT-002A has 15 commits on feat/payment -- suggest review?"
```

---

## EP-0004: TUI

Terminal user interface for visual project overview.

```yaml
id: EP-0004
title: TUI
type: epic
status: new
priority: medium
milestone: MS-02
description: |
  Ratatui-based terminal UI launched via joy app.
  Board view, item detail, dependency graph.
```

### Stories and Tasks

```yaml
- id: IT-001A
  title: "joy app: basic board view"
  type: story
  epic: EP-0004
  priority: high
  description: |
    Kanban-style board with columns per status.
    Navigate items with keyboard.
    Open item detail with Enter.

- id: IT-001B
  title: "TUI: item detail panel"
  type: story
  epic: EP-0004
  priority: medium
  deps: [IT-001A]
  description: |
    Show full item details in a side panel or overlay.
    Edit status and priority inline.

- id: IT-001C
  title: "TUI: dependency tree view"
  type: story
  epic: EP-0004
  priority: low
  deps: [IT-001A]
  description: |
    Visual dependency graph in the TUI.
    Highlight blocked items and critical paths.
```

---

## EP-0005: Sync and Server

Push/pull synchronization with a central portal.

```yaml
id: EP-0005
title: Sync and Server
type: epic
status: new
priority: medium
milestone: MS-02
description: |
  Synchronize .joy/ data with a central server (joyint.com or self-hosted).
  End-to-end encryption for item content. Last-write-wins conflict resolution.
```

### Stories and Tasks

```yaml
- id: IT-001D
  title: "joy serve: HTTP server with REST API"
  type: story
  epic: EP-0005
  priority: high
  description: |
    Axum-based server started via joy serve.
    Support --daemon for background mode.
    Support --config for custom config file.
    REST API for all CRUD operations on items, milestones.
    Behind server feature flag in Cargo.

- id: IT-001E
  title: "joy sync: push and pull"
  type: story
  epic: EP-0005
  priority: high
  deps: [IT-001D]
  description: |
    Bidirectional sync (pull then push) by default.
    Support --push and --pull for one-directional sync.
    Support --auto for background file-watch sync.
    Last-write-wins with conflict detection and warning.

- id: IT-001F
  title: "joy clone: clone project from remote"
  type: story
  epic: EP-0005
  priority: medium
  deps: [IT-001D]
  description: |
    Clone a project from a remote server URL.
    Download all items, milestones, config.
    Set up sync remote in local config.

- id: IT-0020
  title: "Client-side encryption for synced data (v2)"
  type: story
  epic: EP-0005
  priority: low
  deps: [IT-001E]
  description: |
    Deferred to v2. In v1, sync uses HTTPS without content encryption.
    v2 design: AES-256-GCM key per project, stored in
    ~/.config/joy/keys/{project-id}.key. Encrypt item content (title,
    description, comments) before push. Keep metadata in cleartext.
    See ADR-006 for the full design.

- id: IT-0021
  title: OAuth authentication with GitHub and Gitea
  type: task
  epic: EP-0005
  priority: high
  deps: [IT-001D]
  description: |
    OAuth 2.0 authentication for sync with GitHub and Gitea as initial providers.
    E-mail address as user identity (matched against git config user.email locally).
    Server issues JWTs after OAuth login.
    Store tokens in OS keychain (keyring crate) with fallback
    to credentials.yaml (project-local or global).
    See ADR-009 for the identity model.
```

---

## EP-0006: Web UI and Portal

SolidJS web frontend served by joy serve, shared with native app.

```yaml
id: EP-0006
title: Web UI and Portal
type: epic
status: new
priority: medium
milestone: MS-03
description: |
  SolidJS web frontend in web/ (MIT). Embedded in joy serve.
  Board view, item management, roadmap, AI job dispatch.
  Shared with the Tauri native app.
```

### Stories and Tasks

```yaml
- id: IT-0022
  title: Web frontend scaffolding with SolidJS
  type: task
  epic: EP-0006
  priority: high
  description: |
    Set up web/ project with SolidJS + TypeScript + Vite + Tailwind.
    Yarn as package manager. API client for joy serve REST API.

- id: IT-0023
  title: "Web: board view (Kanban)"
  type: story
  epic: EP-0006
  priority: high
  deps: [IT-0022]
  description: |
    Kanban board with drag-and-drop status changes.
    Filter by epic, type, priority, assignee.
    Responsive layout for desktop and mobile browsers.

- id: IT-0024
  title: "Web: item detail and editing"
  type: story
  epic: EP-0006
  priority: high
  deps: [IT-0022]
  description: |
    View and edit all item fields.
    Inline editing for status and priority.
    Comment thread with add/edit/delete.

- id: IT-0025
  title: "Web: visual roadmap and dependency graph"
  type: story
  epic: EP-0006
  priority: medium
  deps: [IT-0023]
  description: |
    Timeline view with milestones and epic progress.
    Interactive dependency graph (highlight blocked, critical path).

- id: IT-0026
  title: "Web: AI job dispatch and monitoring"
  type: story
  epic: EP-0006
  priority: medium
  deps: [IT-0022, IT-0017]
  description: |
    Dispatch AI jobs (estimate, plan, implement, review) from the web UI.
    Monitor job progress in real time.
    Review and approve AI work.

- id: IT-0027
  title: "Web: encryption key handling in browser (v2)"
  type: task
  epic: EP-0006
  priority: low
  deps: [IT-0020, IT-0022]
  description: |
    Deferred to v2 (depends on IT-0020).
    Web Crypto API for client-side encryption/decryption in browser.
    User provides key via paste or session storage.
    Same encryption as CLI (AES-256-GCM).

- id: IT-0028
  title: Embed web frontend in joy serve
  type: task
  epic: EP-0006
  priority: high
  deps: [IT-001D, IT-0022]
  description: |
    Embed built web assets into joy-cli binary (rust-embed or include_dir).
    joy serve serves API and web UI on the same port.
    No separate frontend deployment needed for self-hosting.

- id: IT-002C
  title: Tauri native shell wrapping web frontend
  type: task
  epic: EP-0006
  priority: medium
  deps: [IT-0022]
  description: |
    Tauri 2 project in app/ wrapping web/ frontend.
    Desktop (macOS, Linux, Windows) and mobile (iOS, Android).
    Offline support via local .joy/ access through joy-core.
    OS integration: push notifications, file associations.
    Commercially licensed.

- id: IT-002D
  title: joyint.com deployment
  type: task
  epic: EP-0006
  priority: medium
  deps: [IT-001D, IT-0028]
  description: |
    Deploy joy serve to joyint.com as managed service.
    CI/CD pipeline for automatic deployment on release.
    SSL, domain config, monitoring, backups.
```

---

## Milestones

```yaml
- id: MS-01
  title: "Phase 0+1: Dogfood-ready CLI"
  date: 2026-05-01
  description: |
    Joy can manage its own development.
    Core CLI commands, terminal UX, shell completions.
    Epics: EP-0001, EP-0002.

- id: MS-02
  title: "Phase 2+3: AI and Sync"
  date: 2026-09-01
  description: |
    AI dispatch to configured tool (Claude Code, Mistral Vibe),
    estimation and review, server sync (HTTPS, no encryption in v1).
    Epics: EP-0003, EP-0004, EP-0005.

- id: MS-03
  title: "Phase 3+4: Web UI, Native App, Portal"
  date: 2026-11-01
  description: |
    Web UI embedded in joy serve, Tauri native app,
    joyint.com managed service launch.
    Epic: EP-0006.
```
