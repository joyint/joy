# Joy Tutorial

A field guide to terminal-native product management.

This tutorial walks you through a complete project setup, told through the lens of everyone's favorite improviser. Because product management, like defusing a bomb with a paperclip, is all about using the right tool at the right moment.

## Contents

- [TL;DR](#tldr)
- [Mission 1: Setting Up Base Camp](#mission-1-setting-up-base-camp-init) -- `init`
- [Mission 2: Building Your Arsenal](#mission-2-building-your-arsenal-add) -- `add`
- [Mission 3: Surveying the Terrain](#mission-3-surveying-the-terrain-ls-show-find) -- `ls`, `show`, `find`
- [Mission 4: Wiring the Circuit](#mission-4-wiring-the-circuit-deps) -- `deps`
- [Mission 5: Into the Field](#mission-5-into-the-field-status-start-submit-close) -- `status`, `start`, `submit`, `close`
- [Mission 6: Setting the Deadline](#mission-6-setting-the-deadline-milestone) -- `milestone`
- [Mission 7: Reading the Black Box](#mission-7-reading-the-black-box-log-release) -- `log`, `release`
- [Mission 8: Calling in Air Support](#mission-8-calling-in-air-support-ai) -- `ai`
- [Mission 9: Adjusting the Rules](#mission-9-adjusting-the-rules-project-config) -- `project`, `config`
- [Bonus: Shell Completions](#bonus-shell-completions)
- [Command Reference](#command-reference)

## TL;DR

```sh
mkdir cookbox && cd cookbox && git init
joy init
joy add epic "Recipe Management"
joy add story "Add a recipe" --parent CB-0001 --priority high
joy add task "Set up database" --parent CB-0001 --priority critical
joy start CB-0003
joy deps CB-0002 --add CB-0003
joy milestone add "MVP" --date 2026-04-01
joy milestone link CB-0002 CB-MS-01
joy submit CB-0003
joy close CB-0003
joy
```

That's the whole loop. Read on for the details.

---

## Mission 1: Setting Up Base Camp (`init`)

Every mission starts with preparation. MacGyver never walks into a building without checking the exits first. You never start coding without setting up your project.

Create a fresh project:

```sh
mkdir cookbox && cd cookbox
git init
joy init
```

Joy creates a `.joy/` directory inside your repo:

```
.joy/
├── project.yaml           Project name, acronym, members, settings
├── config.defaults.yaml   Project defaults (committed)
├── config.yaml            Personal overrides (gitignored)
├── items/                 All your items live here (YAML files)
├── milestones/            Milestone definitions
└── logs/                  Event log (audit trail)
```

Everything is plain text, versioned with git. No database, no cloud dependency. If your hard drive survives, your project plan survives. MacGyver would approve.

You can also name your project explicitly:

```sh
joy init --name "Cookbox" --acronym CB
```

Joy also installs a commit-msg hook that enforces item references in every commit message. This is part of the audit trail -- every code change must link to a Joy item. More on this in Mission 7.

### Joining an Existing Project

If you clone a repo that already uses Joy, run the same command:

```sh
git clone https://github.com/example/cookbox.git
cd cookbox
joy init
```

Joy detects the existing project and switches to onboarding mode: it installs the commit-msg hook and sets up your local environment without touching project data. Think of it as registering for the mission instead of creating a new one.

After onboarding, set up AI tool integration if you use one:

```sh
joy ai setup
```

---

## Mission 2: Building Your Arsenal (`add`)

A Swiss Army knife is only useful if you actually open it. Time to create some work items.

Start with an epic -- the big picture:

```sh
joy add epic "Recipe Management"
```

Joy assigns ID `CB-0001` and creates `.joy/items/CB-0001-recipe-management.yaml`.

Now break it down. MacGyver doesn't try to defuse the whole bomb at once -- he works one wire at a time:

```sh
joy add story "Add a recipe" --parent CB-0001 --priority high
joy add story "Edit a recipe" --parent CB-0001 --priority high
joy add story "List recipes with filters" --parent CB-0001
joy add task "Set up SQLite database" --parent CB-0001 --priority critical --effort 3
```

### Effort

Estimate work with `--effort` on a 1-7 scale: 1=trivial, 2=small, 3=medium, 4=large, 5=major, 6=heavy, 7=massive. It's optional but helps with planning.

### Item Types

| Type | When to use |
|------|-------------|
| `epic` | Large initiative grouping multiple items |
| `story` | User-facing functionality ("As a user, I can...") |
| `task` | Technical work, not directly visible to users |
| `bug` | Something is broken |
| `rework` | Refactoring or improvement of existing code |
| `decision` | Architecture or product decision to document |
| `idea` | Not yet refined -- just capture it before it escapes |

All items start with status `new`. Priorities: `extreme`, `critical`, `high`, `medium` (default), `low`.

---

## Mission 3: Surveying the Terrain (`ls`, `show`, `find`)

Before MacGyver acts, he observes. Get the lay of the land:

```sh
joy ls
```

Filter to find exactly what you need:

```sh
joy ls --type story              # Only stories
joy ls --priority critical       # Only critical items
joy ls --parent CB-0001          # Children of an epic
joy ls --status open             # Only open items
joy ls --members alice@team.com  # Assigned to a specific member
joy ls --members me              # Assigned to you (or --mine)
joy ls --members none            # No assignees
joy ls --members '*'             # Has at least one assignee
joy ls --milestone CB-MS-01      # In a specific milestone
joy ls --blocked                 # Items with unfinished dependencies
joy ls --tag ui                  # Items tagged with "ui"
```

Search by text across all items:

```sh
joy find "database"              # Search titles and descriptions
```

### Tags

Tags are free-text labels for cross-cutting categories -- things like `ui`, `backend`, `security`, or `tech-debt`:

```sh
joy add task "Fix layout" --tags "ui,urgent"
joy edit CB-0004 --tags "ui,search"
```

Tags are comma-separated. Using `--tags` replaces all existing tags. Use `--tags ""` to clear them.

### Views

```sh
joy                              # Board view (items grouped by status)
joy ls --tree                    # Hierarchy view (parent/child tree)
joy show CB-0002                 # Full detail view with comments
```

---

## Mission 4: Wiring the Circuit (`deps`)

MacGyver knows: if you cut the wrong wire, everything blows up. In a project, dependencies are those wires. You need the database before you can add recipes.

```sh
joy deps CB-0002 --add CB-0005
```

This means: `CB-0002` (Add a recipe) depends on `CB-0005` (Set up SQLite database). `CB-0005` must be completed first.

```sh
joy deps CB-0002                 # List dependencies
joy deps CB-0002 --tree          # Show full dependency tree
joy deps CB-0002 --rm CB-0005   # Remove a dependency
```

Joy detects circular dependencies and refuses to create them. No infinite loops on MacGyver's watch.

---

## Mission 5: Into the Field (`status`, `start`, `submit`, `close`)

Time to get your hands dirty. The status workflow:

```
new --> open --> in-progress --> review --> closed
          \                        |
           +-----> deferred <------+
```

Move items through the pipeline:

```sh
joy status CB-0005 open          # Approve for work
joy start CB-0005                # Shortcut: set to in-progress
joy submit CB-0005               # Shortcut: set to review
joy close CB-0005                # Shortcut: set to closed
joy reopen CB-0005               # Reopen a closed/deferred item
```

If an item depends on something unfinished, Joy warns you but does not block. When all children of an epic are closed, the epic auto-closes.

### Assignments and Comments

```sh
joy assign CB-0005               # Assign to yourself (git email)
joy assign CB-0005 pete@phoenix.org  # Assign to someone else
joy comment CB-0005 "Schema looks good, all migrations pass."
```

When starting an item (`joy start`), Joy auto-assigns it to you if no one is assigned yet.

---

## Mission 6: Setting the Deadline (`milestone`)

Every mission has a countdown. Milestones are yours.

```sh
joy milestone add "MVP" --date 2026-04-01
```

Link items to the milestone:

```sh
joy milestone link CB-0002 CB-MS-01
joy milestone link CB-0003 CB-MS-01
joy milestone link CB-0005 CB-MS-01
```

Check progress:

```sh
joy milestone show CB-MS-01      # Progress, risks, blocked items
joy milestone ls                 # All milestones with counts
joy roadmap                      # Full roadmap tree view
```

Children inherit their parent's milestone automatically. If `CB-0001` is linked to `CB-MS-01`, all its children are too -- unless they override it.

---

## Mission 7: Reading the Black Box (`log`, `release`)

MacGyver always reviews the flight recorder after a mission. Joy has one too -- a structured event log that records every action automatically.

```sh
joy log                          # Last 20 events
joy log --since 7d               # Last 7 days
joy log --item CB-0005           # Events for a specific item
joy log --limit 50               # Show more entries
```

Every joy command leaves a trace in `.joy/logs/` -- one file per day, append-only, timestamped to the millisecond:

```
2026-03-11T16:14:32.320Z CB-0005 item.created "Set up SQLite database" [mac@phoenix.org]
2026-03-11T16:15:01.440Z CB-0005 item.status_changed "new -> in-progress" [mac@phoenix.org]
2026-03-11T16:42:18.100Z CB-0005 comment.added "Schema looks good" [pete@phoenix.org]
2026-03-11T17:00:00.000Z CB-0005 comment.added "AI review complete" [ai:claude@joy delegated-by:mac@phoenix.org]
```

These logs are committed to git with your project. Every team member's actions are recorded -- a built-in audit trail. When an AI tool acts on behalf of a human, the log shows both identities via `delegated-by`.

### Commit-Msg Hook

Joy installs a commit-msg hook (via `joy init`) that enforces every commit message references at least one item ID:

```sh
git commit -m "feat(db): add migration CB-0005"     # OK
git commit -m "fix typo"                             # REJECTED
```

The hook reads the project acronym from `.joy/project.yaml` and checks for the pattern `CB-XXXX`. For commits that genuinely have no item (CI config, dependency bumps), use the `[no-item]` tag:

```sh
git commit -m "chore: bump dependencies [no-item]"  # OK
```

In multi-repo setups (umbrella with submodules), each subproject has its own acronym. CI can enforce the same rule with: `just lint-commits`

### Releases

A release in Joy is three explicit steps. Joy never reaches into your build system; it just updates version strings, writes a release record, and talks to your forge. Anything ecosystem-specific (lockfile refresh, uploading to a package registry, running tests) happens between the Joy steps in your project's own release script.

```sh
joy release bump patch               # Step 1: replace "X.Y.Z" in configured files
# ... project-specific steps go here (e.g. refresh a lockfile) ...
joy release record patch             # Step 2: record + commit + tag (local only)
# ... project-specific steps go here (e.g. upload to a registry) ...
joy release publish                  # Step 3: push + forge release
```

`joy release bump` replaces every quoted occurrence of the current version with the next one across the files listed under `release.version-files` in `project.yaml`. It is a text-level operation, not a TOML/JSON/YAML edit, so it catches any workspace dependency pins that happen to reference the same version.

`joy release record` collects all items closed since the last release, groups them by type, lists contributors, and writes a snapshot to `.joy/releases/`. It commits the bumped files and creates the tag locally. At this point nothing has been pushed, so a failed check or typo can be rolled back with `git reset --hard HEAD~1 && git tag -d vX.Y.Z`.

`joy release publish` pushes the commit and tag to the configured remote and creates the forge release (GitHub, GitLab, Gitea, Joyint, ...).

Preview and browse without touching anything:

```sh
joy release show                     # Preview from event log
joy release show v1.0.0              # Show an existing release
joy release ls                       # List all releases
```

Configure which files Joy bumps in `.joy/project.yaml`:

```yaml
release:
  version-files:
  - crates/joy-core/Cargo.toml
  - crates/joy-cli/Cargo.toml
  - crates/joy-ai/Cargo.toml
```

### Editing and Deleting

```sh
joy edit CB-0002 --priority critical
joy edit CB-0002 --title "Add and validate a recipe"
joy rm CB-0006                       # Delete (asks for confirmation)
joy rm CB-0001 -rf                   # Delete epic and all children
```

---

## Mission 8: Calling in Air Support (`ai`)

Even MacGyver accepts help sometimes. Joy integrates with AI coding tools so they can manage your backlog alongside you.

```sh
joy ai setup
```

This does three things:

1. Checks if your project has `Vision.md`, `Architecture.md`, `CONTRIBUTING.md` (offers to create templates if missing)
2. Installs AI instructions and skills into `.joy/ai/`
3. Detects your AI tool (Claude Code, Qwen Code, Mistral Vibe, GitHub Copilot) and configures it with the right permissions and references

After setup, your AI tool knows how to use Joy commands, follows your project conventions, and will offer to help fill in empty documents on first use.

### The Trust Model

Joy's AI Governance is built on five pillars: **Trustship** (who do I trust?), **Guardianship** (what do I protect against?), **Orchestration** (how do I steer work?), **Traceability** (what happened?), and **Settlement** (what did it cost?).

Together they form the Trust Model -- the configuration that governs how humans and AI agents collaborate. It scales naturally: a solo developer has implicit trust (one member, all capabilities, no gates). A team adds explicit trust (members with specific capabilities). An enterprise adds verified trust (gates, cost limits, audit trails). Same workflow, growing accountability.

The rest of this mission covers the parts you can use today: identity (Trustship), the event log (Traceability), and capabilities (Trustship). Gates (Guardianship), cost tracking (Settlement), and AI dispatch (Orchestration) are covered in the [Vision](../dev/Vision.md#ai-governance-the-five-pillars).

### AI Identity

AI tools are registered as project members with an `ai:` prefix:

```sh
joy project member add ai:claude@joy
```

When an AI tool uses Joy commands, it identifies itself with the `--author` flag:

```sh
joy comment CB-0005 "Review complete" --author ai:claude@joy
joy add bug "Crash on empty input" --author ai:claude@joy
```

The event log traces accountability back to the human who started the session:

```
[ai:claude@joy delegated-by:mac@phoenix.org]
```

AI members have the same capabilities as human members, with one exception: **AI members cannot perform manage actions** (adding members, changing capabilities, modifying project settings). Management stays with humans.

If your project has AI members and you run a Joy command without `--author`, Joy shows a warning reminding you to set your identity explicitly.

### Keeping Instructions Current

Run `joy ai setup` again after a Joy update to get the latest instructions. Joy-owned files are updated, your custom rules are preserved. Run `joy ai check` at any time to verify:

```sh
joy ai check                     # Are AI instructions up to date?
```

---

## Mission 9: Adjusting the Rules (`project`, `config`)

Joy starts with zero ceremony. No gates, no approvals, no bureaucracy. Add rules only when you need them.

### Project Metadata

```sh
joy project                      # View project metadata and members
joy project get language          # Get a specific value
joy project set name "Cookbox Pro"   # Set a value (requires manage)
joy project set language de       # Change project language
```

Settable keys: `name`, `description`, `language`. Read-only: `acronym`, `created`.

### Members and Capabilities

Joy tracks project members and their capabilities. Members are added automatically during `joy init` (from `git config user.email`) or manually:

```sh
joy project member add pete@phoenix.org
joy project member add ai:claude@joy --capabilities "implement,review"
joy project member show pete@phoenix.org
joy project member rm pete@phoenix.org
```

Joy defines eleven capabilities across two groups:

**Lifecycle capabilities** (what you can do on items): `conceive`, `plan`, `design`, `implement`, `test`, `review`, `document`

**Management capabilities** (project-level operations): `create`, `assign`, `manage`, `delete`

By default, members have `capabilities: all`. Restrict them when needed -- especially for AI members where you want to control what they can do autonomously.

### Authentication and Onboarding

Joy uses passphrase-derived Ed25519 identity keys. You authenticate once per 24-hour session and every significant action is cryptographically signed.

**First time setup (solo):**

```sh
joy auth init                    # Choose a passphrase; your identity is now registered
```

**Adding a human teammate:**

The admin adds the member and gets a one-time password back. The OTP is shared out-of-band (encrypted chat, in person, etc.).

```sh
joy project member add pete@phoenix.org
# > Added member pete@phoenix.org
# >
# >   One-time password: AB7X-K3M2-PQ9Z
# >
# > Share the OTP with pete@phoenix.org via a trusted channel.
```

Pete redeems the OTP on his own machine, picks his own passphrase, and is ready to go:

```sh
joy auth --otp AB7X-K3M2-PQ9Z    # Prompts for a new passphrase
```

Each member you add this way is cryptographically attested by the admin's key -- Joy rejects any member entry that was manually edited into `project.yaml` without going through `joy project member add`. This runs silently in the background; you only see it when something is wrong.

**Changing your passphrase:**

```sh
joy auth passphrase              # Prompts for current, then new passphrase
```

Your identity key rotates; existing sessions are invalidated; attestations on your entry remain valid.

**Removing a member:**

If the removed member attested others, those attestations transfer automatically to you as the removing admin. No extra step, no ceremony.

```sh
joy project member rm pete@phoenix.org    # Requires your passphrase if there are orphans to re-attest
```

You cannot remove yourself; Joy prints the project's other manage members so you know who to ask.

### AI Delegation Tokens

AI members authenticate via short-lived delegation tokens rather than passphrases. A human with manage capability issues a token; the AI redeems it in its own shell:

```sh
joy auth token add ai:claude@joy           # Prints a token string
joy auth --token <token>                    # AI runs this; gets a 24h session
```

If you suspect a delegation keypair has been compromised, rotate it. All prior tokens for that AI immediately become invalid:

```sh
joy ai rotate ai:claude@joy
```

### Configuration Layering

Joy uses layered configuration where each layer overrides the one below:

```
Layer 4: .joy/config.yaml            Your personal project overrides (gitignored)
Layer 3: ~/.config/joy/config.yaml   Your global settings (all projects)
Layer 2: .joy/config.defaults.yaml   Project defaults (committed, shared)
Layer 1: Code defaults               Built-in fallbacks
```

View the resolved configuration:

```sh
joy config                       # Show all resolved values with sources
joy config get workflow.auto-assign  # Get a specific value
joy config set output.emoji true     # Set a personal override
```

`joy config set` always writes to your personal `.joy/config.yaml` -- your preferences never affect teammates. Project defaults in `config.defaults.yaml` set the shared baseline that the whole team inherits.

Key settings:

| Setting | Default | What it does |
|---------|---------|-------------|
| `workflow.auto-assign` | `true` | Auto-assign items on `joy start` |
| `output.color` | `auto` | Color mode: `auto`, `always`, `never` |
| `output.emoji` | `false` | Show emoji indicators in output |
| `output.short` | `true` | Compact list output (abbreviations) |
| `output.fortune` | `true` | Show occasional quotes in output |

---

## Bonus: Shell Completions

Joy supports tab completion for commands, flags, and item IDs. Add one line to your shell config:

```sh
# Bash (~/.bashrc)
source <(COMPLETE=bash joy)

# Zsh (~/.zshrc)
source <(COMPLETE=zsh joy)

# Fish (config.fish)
source (COMPLETE=fish joy | psub)
```

After reloading your shell:

```sh
joy show CB-<TAB>                # Completes item and milestone IDs
joy sta<TAB>                     # Completes subcommands
joy ls --ty<TAB>                 # Completes flags
```

MacGyver would say: why type when the machine can do it for you?

---

## Command Reference

| Command | What it does |
|---------|-------------|
| `joy init` | Initialize or onboard into a project |
| `joy add <TYPE> <TITLE>` | Create an item |
| `joy ls` | List and filter items |
| `joy` | Board overview |
| `joy show <ID>` | Item detail view |
| `joy edit <ID>` | Modify an item |
| `joy find <TEXT>` | Search items by text |
| `joy status <ID> <STATUS>` | Change item status |
| `joy start/submit/close <ID>` | Status shortcuts |
| `joy reopen <ID>` | Reopen a closed/deferred item |
| `joy rm <ID>` | Delete an item |
| `joy assign <ID> [MEMBER]` | Assign item to member |
| `joy comment <ID> <TEXT>` | Add comment to item |
| `joy deps <ID>` | Manage dependencies |
| `joy milestone` | Manage milestones |
| `joy roadmap` | Milestone roadmap (tree view) |
| `joy log` | Event log (audit trail) |
| `joy release bump <BUMP>` | Step 1: patch version strings in configured files |
| `joy release record <BUMP>` | Step 2: record, commit, tag (local only) |
| `joy release publish` | Step 3: push + create the forge release |
| `joy release show [VERSION]` | Show a release or preview the next |
| `joy release ls` | List all releases |
| `joy project` | View/edit project info and members |
| `joy config` | Show or modify configuration |
| `joy ai setup` | Set up AI tool integration |
| `joy ai check` | Check if AI instructions are current |
| `joy tutorial` | You are here |

Most write commands accept `--author <MEMBER>` to attribute the action to a specific identity.

> "Any problem can be solved with a little ingenuity." -- MacGyver

See also: `joy --help`, `joy <command> --help`, `docs/dev/vision/`
