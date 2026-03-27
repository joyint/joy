# Joy Tutorial

A field guide to terminal-native product management.

This tutorial walks you through a complete project setup, told through the lens of everyone's favorite improviser. Because product management, like defusing a bomb with a paperclip, is all about using the right tool at the right moment.

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
├── project.yaml           Project name, acronym, settings
├── config.defaults.yaml   Project defaults (committed)
├── config.yaml            Personal overrides (gitignored)
├── items/                 All your items live here (YAML files)
└── milestones/            Milestone definitions
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
joy add task "Set up SQLite database" --parent CB-0001 --priority critical
```

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

## Mission 3: Surveying the Terrain (`ls`, `show`)

Before MacGyver acts, he observes. Get the lay of the land:

```sh
joy ls
```

Output:

```
ID       Type   Priority  Status  Title
CB-0002  story  high      new     Add a recipe
CB-0003  story  high      new     Edit a recipe
CB-0004  story  medium    new     List recipes with filters
CB-0005  task   critical  new     Set up SQLite database
```

Filter to find exactly what you need:

```sh
joy ls --type story              # Only stories
joy ls --priority critical       # Only critical items
joy ls --parent CB-0001          # Children of an epic
joy ls --status open             # Only open items
joy ls --mine                    # Assigned to you
joy ls --blocked                 # Items with unfinished dependencies
joy ls --tag ui                  # Items tagged with "ui"
```

### Tags

Tags are free-text labels for cross-cutting categories that don't fit into type, status, or priority -- things like `ui`, `backend`, `security`, or `tech-debt`. Set them when creating or editing items:

```sh
joy add task "Fix layout" --tags "ui,urgent"
joy edit CB-0004 --tags "ui,search"
```

Tags are comma-separated. Using `--tags` replaces all existing tags. Use `--tags ""` to clear them.

### Extra Columns and Detail View

```sh
joy ls -s milestone,assignee     # Add milestone and assignee columns
joy                              # Board view (items grouped by status)
joy show CB-0002                 # Full detail view
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
```

If an item depends on something unfinished, Joy warns you but does not block. MacGyver doesn't always follow the manual either -- but he knows the risks.

### Assignments and Comments

```sh
joy assign CB-0005               # Assign to yourself (git email)
joy assign CB-0005 pete@phoenix.org  # Assign to someone else
joy comment CB-0005 "Schema looks good, all migrations pass."
```

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
joy milestone show CB-MS-01
joy milestone ls
```

Children inherit their parent's milestone automatically. If `CB-0001` is linked to `CB-MS-01`, all its children are too -- unless they override it.

---

## Mission 7: Reading the Black Box (`log`, `roadmap`, `release`)

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
2026-03-11T16:45:30.200Z CB-0002 dep.added "CB-0005" [mac@phoenix.org]
```

These logs are committed to git with your project. Every team member's actions are recorded -- a built-in audit trail.

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

In multi-repo setups (umbrella with submodules), each subproject has its own acronym. A commit in the Joy repo needs a `JOY-XXXX` reference, a commit in the umbrella needs a `JI-XXXX` reference.

CI can enforce the same rule with: `just lint-commits`

### Roadmap and Releases

For the big picture, use the roadmap -- a tree view grouped by milestone:

```sh
joy roadmap
```

When you ship a version, create a release:

```sh
joy release create patch             # Next patch version (default)
joy release create minor             # Next minor version
joy release create major             # Next major version
joy release create patch --title "Bug fixes"
```

Joy collects all items closed since the last release, groups them by type, lists contributors, and writes a release snapshot to `.joy/releases/`. Preview without creating:

```sh
joy release show                     # Preview from event log
joy release show v1.0.0              # Show an existing release
joy release ls                       # List all releases
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

Run it again after a Joy update to get the latest instructions:

```sh
joy ai setup
```

Joy-owned files are updated, your custom rules are preserved. If your project contains nested Joy projects (submodules, monorepo), run `joy ai setup` in each one separately.

---

## Mission 9: Adjusting the Rules (`project`, `config`)

Joy starts with zero ceremony. No gates, no approvals, no bureaucracy. Add rules only when you need them.

```sh
joy project                      # View project metadata
joy project set name "Cookbox Pro"
joy project set description "Recipe management for pros"
```

Joy uses layered configuration:

```
.joy/config.defaults.yaml   Project defaults (committed, shared)
~/.config/joy/config.yaml   Personal global settings (all projects)
.joy/config.yaml            Personal project overrides (gitignored)
```

Each layer overrides the one below. `joy config set` writes to your personal `.joy/config.yaml` -- your preferences never affect teammates. Project defaults in `config.defaults.yaml` set the shared baseline.

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

After reloading your shell, try:

```sh
joy show JOY-<TAB>               # Completes item and milestone IDs
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
| `joy status <ID> <STATUS>` | Change item status |
| `joy start/submit/close <ID>` | Status shortcuts |
| `joy reopen <ID>` | Reopen a closed/deferred item |
| `joy rm <ID>` | Delete an item |
| `joy assign <ID> [MEMBER]` | Assign item to member |
| `joy comment <ID> <TEXT>` | Add comment to item |
| `joy deps <ID>` | Manage dependencies |
| `joy milestone` | Manage milestones |
| `joy log` | Event log (audit trail) |
| `joy release create <BUMP>` | Create a release (patch/minor/major) |
| `joy release show [VERSION]` | Show a release or preview the next |
| `joy release ls` | List all releases |
| `joy roadmap` | Milestone roadmap (tree view) |
| `joy project` | View/edit project info |
| `joy config` | Show current configuration |
| `joy ai setup` | Set up AI tool integration |
| `joy ai check` | Check if AI instructions are current |
| `joy tutorial` | You are here |

> "Any problem can be solved with a little ingenuity." -- MacGyver

See also: `joy --help`, `joy <command> --help`, `docs/dev/Vision.md`
