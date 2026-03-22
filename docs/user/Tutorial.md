JOY(tutorial)                    Joy User Manual                    JOY(tutorial)

NAME
    joy tutorial -- a field guide to terminal-native product management

SYNOPSIS
    joy init, joy add, joy ls, joy status, joy deps, joy milestone, joy log,
    joy roadmap, joy ai

DESCRIPTION
    Joy is a terminal-native, git-native product management tool. Everything
    lives in plain YAML inside your repo. No server, no browser, no context
    switch. You plan, track, and ship from the same terminal where you code.

    This tutorial walks you through a complete project setup, told through the
    lens of everyone's favorite improviser. Because product management, like
    defusing a bomb with a paperclip, is all about using the right tool at
    the right moment.

TL;DR
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

    That's the whole loop. Read on for the details.

MISSION 1: SETTING UP BASE CAMP (init)

    Every mission starts with preparation. MacGyver never walks into a
    building without checking the exits first. You never start coding
    without setting up your project.

    Create a fresh project:

        mkdir cookbox && cd cookbox
        git init
        joy init

    Joy creates a .joy/ directory inside your repo:

        .joy/
        +-- project.yaml           Project name, acronym, settings
        +-- config.defaults.yaml   Project defaults (committed)
        +-- config.yaml            Personal overrides (gitignored)
        +-- items/                 All your items live here (YAML files)
        +-- milestones/            Milestone definitions

    Everything is plain text, versioned with git. No database, no cloud
    dependency. If your hard drive survives, your project plan survives.
    MacGyver would approve.

    You can also name your project explicitly:

        joy init --name "Cookbox" --acronym CB

    Joy also installs a commit-msg hook that enforces item references in
    every commit message. This is part of the audit trail -- every code
    change must link to a Joy item. More on this in Mission 7.

    JOINING AN EXISTING PROJECT

    If you clone a repo that already uses Joy, run the same command:

        git clone https://github.com/example/cookbox.git
        cd cookbox
        joy init

    Joy detects the existing project and switches to onboarding mode:
    it installs the commit-msg hook and sets up your local environment
    without touching project data. Think of it as registering for the
    mission instead of creating a new one.

    After onboarding, set up AI tool integration if you use one:

        joy ai setup

MISSION 2: BUILDING YOUR ARSENAL (add)

    A Swiss Army knife is only useful if you actually open it. Time to
    create some work items.

    Start with an epic -- the big picture:

        joy add epic "Recipe Management"

    Joy assigns ID CB-0001 and creates .joy/items/CB-0001-recipe-management.yaml.

    Now break it down. MacGyver doesn't try to defuse the whole bomb at
    once -- he works one wire at a time:

        joy add story "Add a recipe" --parent CB-0001 --priority high
        joy add story "Edit a recipe" --parent CB-0001 --priority high
        joy add story "List recipes with filters" --parent CB-0001
        joy add task "Set up SQLite database" --parent CB-0001 --priority critical

    Item types and when to use them:

        epic       Large initiative grouping multiple items
        story      User-facing functionality ("As a user, I can...")
        task       Technical work, not directly visible to users
        bug        Something is broken
        rework     Refactoring or improvement of existing code
        decision   Architecture or product decision to document
        idea       Not yet refined -- just capture it before it escapes

    All items start with status "new". Priorities: extreme, critical, high,
    medium (default), low.

MISSION 3: SURVEYING THE TERRAIN (ls, show)

    Before MacGyver acts, he observes. Get the lay of the land:

        joy ls

    Output:

        ID       Type   Priority  Status  Title
        CB-0002  story  high      new     Add a recipe
        CB-0003  story  high      new     Edit a recipe
        CB-0004  story  medium    new     List recipes with filters
        CB-0005  task   critical  new     Set up SQLite database

    Filter to find exactly what you need:

        joy ls --type story              Only stories
        joy ls --priority critical       Only critical items
        joy ls --parent CB-0001          Children of an epic
        joy ls --status open             Only open items
        joy ls --mine                    Assigned to you
        joy ls --blocked                 Items with unfinished dependencies
        joy ls --tag ui                  Items tagged with "ui"

    Tags are free-text labels for cross-cutting categories that don't fit
    into type, status, or priority -- things like "ui", "backend", "security",
    or "tech-debt". Set them when creating or editing items:

        joy add task "Fix layout" --tags "ui,urgent"
        joy edit CB-0004 --tags "ui,search"

    Tags are comma-separated. Using --tags replaces all existing tags.
    Use --tags "" to clear them.

    Show extra columns:

        joy ls -s milestone,assignee     Add milestone and assignee columns

    See the full board (items grouped by status):

        joy

    Inspect a single item in detail:

        joy show CB-0002

    This displays all fields, dependencies, comments, and history.

MISSION 4: WIRING THE CIRCUIT (deps)

    MacGyver knows: if you cut the wrong wire, everything blows up. In a
    project, dependencies are those wires. You need the database before you
    can add recipes.

        joy deps CB-0002 --add CB-0005

    This means: CB-0002 (Add a recipe) depends on CB-0005 (Set up SQLite
    database). CB-0005 must be completed first.

    View the dependency chain:

        joy deps CB-0002
        joy deps CB-0002 --tree

    Remove a dependency:

        joy deps CB-0002 --rm CB-0005

    Joy detects circular dependencies and refuses to create them. No
    infinite loops on MacGyver's watch.

MISSION 5: INTO THE FIELD (status, start, submit, close)

    Time to get your hands dirty. The status workflow:

        new --> open --> in-progress --> review --> closed
                  \                        |
                   +-----> deferred <------+

    Move items through the pipeline:

        joy status CB-0005 open          Approve for work
        joy start CB-0005                Shortcut: set to in-progress
        joy submit CB-0005               Shortcut: set to review
        joy close CB-0005                Shortcut: set to closed

    If an item depends on something unfinished, Joy warns you but does not
    block. MacGyver doesn't always follow the manual either -- but he knows
    the risks.

    Assign work to yourself (uses your git email):

        joy assign CB-0005

    Or to someone else:

        joy assign CB-0005 pete@phoenix.org

    Add a comment before closing, like a field report:

        joy comment CB-0005 "Schema looks good, all migrations pass."

MISSION 6: SETTING THE DEADLINE (milestone)

    Every mission has a countdown. Milestones are yours.

        joy milestone add "MVP" --date 2026-04-01

    Link items to the milestone:

        joy milestone link CB-0002 CB-MS-01
        joy milestone link CB-0003 CB-MS-01
        joy milestone link CB-0005 CB-MS-01

    Check progress:

        joy milestone show CB-MS-01
        joy milestone ls

    Children inherit their parent's milestone automatically. If CB-0001 is
    linked to CB-MS-01, all its children are too -- unless they override it.

    Remove a milestone:

        joy milestone rm CB-MS-01

MISSION 7: READING THE BLACK BOX (log, roadmap, edit, rm)

    MacGyver always reviews the flight recorder after a mission. Joy has
    one too -- a structured event log that records every action automatically.

        joy log                          Last 20 events
        joy log --since 7d               Last 7 days
        joy log --item CB-0005           Events for a specific item
        joy log --limit 50               Show more entries

    Every joy command leaves a trace in .joy/logs/ -- one file per day,
    append-only, timestamped to the millisecond:

        2026-03-11T16:14:32.320Z CB-0005 item.created "Set up SQLite database" [mac@phoenix.org]
        2026-03-11T16:15:01.440Z CB-0005 item.status_changed "new -> in-progress" [mac@phoenix.org]
        2026-03-11T16:42:18.100Z CB-0005 comment.added "Schema looks good" [pete@phoenix.org]
        2026-03-11T16:45:30.200Z CB-0002 dep.added "CB-0005" [mac@phoenix.org]

    These logs are committed to git with your project. Every team member's
    actions are recorded. Think of it as a built-in audit trail -- who did
    what, when, and to which item. No separate tracking tool needed.

    COMMIT-MSG HOOK: CLOSING THE LOOP

    Joy installs a commit-msg hook (via joy init) that enforces every
    commit message references at least one item ID:

        git commit -m "feat(db): add migration CB-0005"     # OK
        git commit -m "fix typo"                            # REJECTED

    The hook reads the project acronym from .joy/project.yaml and checks
    for the pattern CB-XXXX. If the commit has no item reference, you get:

        error: commit message must reference a Joy item
          |
          | fix typo
          | ^ no CB-XXXX item ID found
          |
          = help: add an item ID to your commit message (e.g. CB-0001)
          = note: use [no-item] tag for infrastructure commits without a Joy item

    For commits that genuinely have no item (CI config, dependency bumps),
    use the [no-item] tag. It is visible in git history and auditable:

        git commit -m "chore: bump dependencies [no-item]"  # OK

    In multi-repo setups (umbrella with submodules), each subproject has
    its own acronym. A commit in the Joy repo needs a JOY-XXXX reference,
    a commit in the umbrella needs a JI-XXXX reference. Cross-project work
    requires items in each affected project.

    CI can enforce the same rule with: just lint-commits

    The display converts UTC timestamps to your local timezone:

        2026-03-11 17:14:32.320 (+01:00) - CB-0005 - item.created - "Set up SQLite database" [mac@phoenix.org]

    For the big picture, use the roadmap -- a tree view grouped by milestone:

        joy roadmap

    It shows your milestones with their items nested underneath, progress
    counts, and the full hierarchy. One glance, and you know where the
    mission stands. MacGyver calls it situational awareness.

    When you ship a version, create a release:

        joy release create patch             Next patch version (default)
        joy release create minor             Next minor version
        joy release create major             Next major version
        joy release create patch --title "Bug fixes"

    Joy collects all items closed since the last release from the event
    log, groups them by type, lists contributors, and writes a release
    snapshot to .joy/releases/CB-v1.0.1.yaml. It shows a preview and
    asks for confirmation before writing.

    Preview what the next release would contain without creating it:

        joy release show                     Preview from event log
        joy release show v1.0.0              Show an existing release
        joy release ls                       List all releases

    If you reopen a released item, Joy warns you:

        joy reopen CB-0005
        warning: CB-0005 is included in release v1.0.0
          = note: reopening a released item means the fix was incomplete
          = help: consider creating a new bug item instead
          Reopen anyway? [y/N]

    Version tags on items are still useful for tracking bugs. Tag a bug
    with the version where it was found:

        joy add bug "Crash on empty input" --version v0.9.0
        joy ls --version v0.9.0              All items for a version
        joy ls --type bug --version v0.9.0   Bugs found in v0.9.0

    Need to adjust something? Edit on the fly:

        joy edit CB-0002 --priority critical
        joy edit CB-0002 --title "Add and validate a recipe"
        joy edit CB-0002 --milestone CB-MS-01

    Made something by mistake? Remove it:

        joy rm CB-0006                   Delete (asks for confirmation)
        joy rm CB-0001 -rf               Delete epic and all children

MISSION 8: CALLING IN AIR SUPPORT (ai)

    Even MacGyver accepts help sometimes. Joy integrates with AI coding
    tools so they can manage your backlog alongside you.

    Set up AI integration for your project:

        joy ai setup

    This does three things:
    1. Checks if your project has Vision.md, Architecture.md, CONTRIBUTING.md
       (offers to create templates if missing)
    2. Installs AI instructions and skills into .joy/ai/
    3. Detects your AI tool (Claude Code, Qwen Code, Mistral Vibe) and
       configures it with the right permissions and references

    After setup, your AI tool knows how to use Joy commands, follows your
    project conventions, and will offer to help fill in empty documents
    on first use.

    Run it again after a Joy update to get the latest instructions:

        joy ai setup

    Joy-owned files are updated, your custom rules are preserved.

    If your project contains nested Joy projects (submodules, monorepo),
    run joy ai setup in each one separately. AI tool permissions are
    per-project and not inherited from parent directories. Joy will warn
    you about unconfigured nested projects during setup.

    Future missions: AI agents that estimate, plan, implement, and review.
    Stay tuned for joy ai estimate, joy ai plan, joy ai implement.

MISSION 9: ADJUSTING THE RULES (project, config)

    Joy starts with zero ceremony. No gates, no approvals, no bureaucracy.
    Add rules only when you need them.

    View project metadata:

        joy project

    Edit it:

        joy project --name "Cookbox Pro" --description "Recipe management for pros"

    View current configuration:

        joy config

    Joy uses layered configuration:

        .joy/config.defaults.yaml   Project defaults (committed, shared)
        ~/.config/joy/config.yaml   Personal global settings (all projects)
        .joy/config.yaml            Personal project overrides (gitignored)

    Each layer overrides the one below. joy config set writes to your
    personal .joy/config.yaml -- your preferences never affect teammates.
    Project defaults in config.defaults.yaml set the shared baseline.

    To add workflow rules, edit .joy/project.yaml:

        roles:
          approver: [orchidee@joyint.com]

        status_rules:
          new -> open:
            requires_role: approver
          review -> closed:
            requires_role: approver
            requires_ci: true
            allow_ai: false

    Remove rules to go back to zero ceremony. There are no templates, no
    modes, no workflow engine. Just rules you add or remove.

BONUS MISSION: SHELL COMPLETIONS

    Joy supports tab completion for commands, flags, and item IDs. Add one
    line to your shell config:

        Bash:  source <(COMPLETE=bash joy)        # ~/.bashrc
        Zsh:   source <(COMPLETE=zsh joy)         # ~/.zshrc
        Fish:  source (COMPLETE=fish joy | psub)  # config.fish

    After reloading your shell, try:

        joy show JOY-<TAB>               Completes item and milestone IDs
        joy sta<TAB>                     Completes subcommands
        joy ls --ty<TAB>                 Completes flags

    MacGyver would say: why type when the machine can do it for you?

MISSION 10: SYNCING WITH HQ (sync)

    For collaboration, sync your project with a remote:

        joy sync --push                  Push to joyint.com or self-hosted
        joy sync --pull                  Pull changes from others
        joy clone joyint.com/orchidee/cookbox

    Sync uses Git as the backend. Data on joyint.com is E2E-encrypted.

REFERENCE

    Command                         What it does
    -------                         ------------
    joy init                        Initialize or onboard into a project
    joy add <TYPE> <TITLE>          Create an item
    joy ls                          List and filter items
    joy                             Board overview
    joy show <ID>                   Item detail view
    joy edit <ID>                   Modify an item
    joy status <ID> <STATUS>        Change item status
    joy start/submit/close <ID>     Status shortcuts
    joy reopen <ID>                 Reopen a closed/deferred item
    joy rm <ID>                     Delete an item
    joy assign <ID> [EMAIL]         Assign item to person
    joy comment <ID> <TEXT>         Add comment to item
    joy deps <ID>                   Manage dependencies
    joy milestone                   Manage milestones
    joy log                         Event log (audit trail)
    joy release create <BUMP>       Create a release (patch/minor/major)
    joy release show [VERSION]      Show a release or preview the next
    joy release ls                  List all releases
    joy roadmap                     Milestone roadmap (tree view)
    joy project                     View/edit project info
    joy config                      Show current configuration
    joy ai setup                    Set up AI tool integration
    joy ai check                    Check if AI instructions are current
    joy completions <SHELL>         Generate shell completions
    joy tutorial                    You are here

    "Any problem can be solved with a little ingenuity." -- MacGyver

SEE ALSO
    joy --help, joy <command> --help, docs/dev/Vision.md
