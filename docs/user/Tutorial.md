# Joy Tutorial

A reference walk-through for terminal-native product management with Joy.

This tutorial covers a complete project setup, from `joy init` through encryption, AI delegation, and updates. Each chapter focuses on one verb (or small group of verbs) and shows the commands, expected output, and the decisions you typically make at that stage.

## Contents

- [TL;DR](#tldr)
- [1. Initializing a Project](#1-initializing-a-project) - `init`
- [2. Creating Items](#2-creating-items) - `add`
- [3. Listing, Searching, and Showing Items](#3-listing-searching-and-showing-items) - `ls`, `show`, `find`
- [4. Dependencies](#4-dependencies) - `deps`
- [5. Status Transitions](#5-status-transitions) - `status`, `start`, `submit`, `close`
- [6. Milestones](#6-milestones) - `milestone`
- [7. Audit Log and Releases](#7-audit-log-and-releases) - `log`, `release`
- [8. AI Tool Integration](#8-ai-tool-integration) - `ai`
- [9. Project Configuration](#9-project-configuration) - `project`, `config`
- [10. Updating joy](#10-updating-joy) - `update`
- [11. Encryption with Crypt](#11-encryption-with-crypt) - `crypt`
- [12. Chats](#12-chats) - `chat`
- [Bonus: Cross-Directory Queries](#bonus-cross-directory-queries-w)
- [Bonus: Shell Completions](#bonus-shell-completions)
- [Bonus: Machine-Readable Output](#bonus-machine-readable-output)
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

## 1. Initializing a Project

`joy init` creates the `.joy/` directory that holds every Joy artefact for the project: items, milestones, configuration, and the event log.

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

Everything is plain text, versioned with git. No database, no cloud dependency: if the working tree survives, the project plan survives.

You can also name your project explicitly:

```sh
joy init --name "Cookbox" --acronym CB
```

Joy also installs a commit-msg hook that enforces item references in every commit message. This is part of the audit trail - every code change must link to a Joy item. More on this in chapter 7.

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
joy ai init
```

---

## 2. Creating Items

`joy add` creates items. The first positional argument is the type, the second is the title.

Start with an epic - the big picture:

```sh
joy add epic "Recipe Management"
```

Joy assigns ID `CB-0001` and creates `.joy/items/CB-0001-recipe-management.yaml`.

Now break it down into smaller items:

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
| `idea` | Not yet refined - just capture it before it escapes |
| `job` | An assignment of work over a scope of items, executed by an AI or human assignee |

All items start with status `new`. Priorities: `extreme`, `critical`, `high`, `medium` (default), `low`.

Jobs take the scope as a third positional argument (required, comma-separated item IDs; an epic means its subtree):

```sh
joy add job "Implement recipe basics" CB-0002,CB-0003
```

Jobs live in `.joy/jobs/` with IDs like `CB-JOB-0001-A3` and stay out of the normal views; `joy ls -J` and `joy board -J` show them (`-Ja` includes closed ones). Approving a job (`joy approve`) is the release that authorizes its execution - on the Joyint platform, `joy start` then hands it to the assignee. `joy show` on a job lists its scope, budget, window, and recorded execution attempts; `joy show` on an item lists the jobs it belongs to. Closing a job asks per scope item whether to close that item too. Moving jobs through their gates is governed by the `jobs` capability, separate from `review`, so who accepts AI work and who accepts the product items can differ.

---

## 3. Listing, Searching, and Showing Items

Three commands cover most read paths: list, show one item, search by text.

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

Tags are free-text labels for cross-cutting categories - things like `ui`, `backend`, `security`, or `tech-debt`:

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

## 4. Dependencies

`joy deps` declares ordering between items. Recording "X depends on Y" is the project-management equivalent of "Y must finish before X can".

```sh
joy deps CB-0002 --add CB-0005
```

This means: `CB-0002` (Add a recipe) depends on `CB-0005` (Set up SQLite database). `CB-0005` must be completed first.

```sh
joy deps CB-0002                 # List dependencies
joy deps CB-0002 --tree          # Show full dependency tree
joy deps CB-0002 --rm CB-0005   # Remove a dependency
```

Joy detects circular dependencies and refuses to create them.

---

## 5. Status Transitions

Items move through a fixed state machine:

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
joy stop CB-0005                 # Shortcut: back to open (aborts a running job)
joy close CB-0005                # Shortcut: set to closed
joy reopen CB-0005               # Reopen a closed/deferred item
```

If an item depends on something unfinished, Joy warns you but does not block. When all children of an epic are closed, the epic auto-closes.

### Assignments and Comments

```sh
joy assign CB-0005               # Assign to yourself (git email)
joy assign CB-0005 pete@example.com  # Assign to someone else
joy comment CB-0005 "Schema looks good, all migrations pass."
joy comment CB-0005              # Opens $EDITOR for a longer note
joy comment edit CB-0005 1 "Schema looks good (verified all migrations)."
joy comment rm CB-0005 2 --force # Delete comment #2
```

`joy comment <ID>` without TEXT opens your editor on an empty tempfile; saving an empty buffer aborts. Editor resolution: `--editor <cmd>`, then `joy config set editor`, then `$VISUAL`, then `$EDITOR`. Comment indices for `edit` and `rm` are 1-based and match what `joy show <ID>` prints.

When starting an item (`joy start`), Joy auto-assigns it to you if no one is assigned yet.

---

## 6. Milestones

`joy milestone` groups items under a date target.

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

Children inherit their parent's milestone automatically. If `CB-0001` is linked to `CB-MS-01`, all its children are too - unless they override it.

---

## 7. Audit Log and Releases

Joy maintains a structured event log that records every state-changing action automatically.

```sh
joy log                          # Last 20 events
joy log --since 7d               # Last 7 days
joy log CB-0005                  # Events for a specific item
joy log --limit 50               # Show more entries
```

Every joy command leaves a trace in `.joy/logs/` - one file per day, append-only, timestamped to the millisecond:

```
2026-03-11T16:14:32.320Z CB-0005 item.created [mac@example.com]
2026-03-11T16:15:01.440Z CB-0005 item.status_changed "new -> in-progress" [mac@example.com]
2026-03-11T16:42:18.100Z CB-0005 comment.added [pete@example.com]
2026-03-11T17:00:00.000Z CB-0005 comment.added [ai:claude@joy delegated-by:mac@example.com]
```

The log records only structural facts: who did what, when, on which item. Titles, descriptions, and comment text are not written to the log - they live in the item file itself, behind whatever Crypt zone protects it. The log stays as a faithful audit trail even when item content is later encrypted. State transitions (`new -> in-progress`), member IDs, and item / milestone IDs do appear, because they are needed to interpret the event.

These logs are committed to git with your project. Every team member's actions are recorded - a built-in audit trail. When an AI tool acts on behalf of a human, the log shows both identities via `delegated-by`.

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

`joy release publish` pushes the commit and tag to the configured remote and creates the forge release. The forge is auto-detected from your git remotes - a single supported remote is used silently, multiple supported remotes prompt on a TTY (or require `--forge` in CI). Today only GitHub (via the `gh` CLI) has a publish backend; GitLab and Gitea are detection-aware but route to no-op until their backends land.

Override the auto-detection when you need to. The override lives in `project.yaml` for repeat runs, or on the command line for a one-shot:

```sh
joy project set forge github         # lock in a specific forge
joy project set forge none           # explicit opt-out: push the tag only
joy project set forge ""             # clear the override, return to auto-detect
joy project get forge                # read the current value (exit 1 when unset)
joy release publish --forge none     # one-shot opt-out for this run
```

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
joy edit CB-0002 --type bug          # Change item type
joy rm CB-0006                       # Delete (asks for confirmation)
joy rm CB-0001 -rf                   # Delete epic and all children
```

---

## 8. AI Tool Integration

Joy integrates with AI coding tools so they can manage the backlog alongside you, under explicit delegation tokens and capability gates.

```sh
joy ai init
```

This does four things:

1. Checks if your project has the Vision, Architecture, and Contributing docs (offers to create templates if missing).
2. Bootstraps your authentication inline if `joy auth init` has not run yet, so the whole setup is one passphrase.
3. Detects your installed AI tools (Claude Code, Qwen Code, Mistral Vibe, GitHub Copilot CLI) and writes their tool-specific instruction files (`.claude/CLAUDE.md`, `.qwen/QWEN.md`, `AGENTS.md`, `.github/copilot-instructions.md`) plus the `/joy` skill where the tool supports skills.
4. Registers each detected tool as an `ai:<name>@joy` member with attested capabilities.

The tool-specific instruction files are intentionally short: they tell the AI its member ID, the correct `Co-Authored-By:` trailer for commits (with the canonical brand and email per tool), and point it at `joy ai tutorial` as the operational guide. `joy ai tutorial` covers the CLI surface, the authentication flow, the item lifecycle, commit conventions, and minimum hygiene rules; together with the project's authoritative docs, that is everything the AI needs.

For AI tools that `joy ai init` cannot auto-detect (e.g. GitHub Copilot Chat in VS Code, Cursor's built-in chat, any other chat-only AI), register a member by hand:

```sh
joy project member add ai:copilot-chat@joy
```

`joy project member add` for an `ai:` ID skips the OTP machinery and prints the next steps for issuing a delegation token.

### The Trust Model

Joy's AI Governance is built on five pillars: **Trustship** (who do I trust?), **Guardianship** (what do I protect against?), **Orchestration** (how do I steer work?), **Traceability** (what happened?), and **Settlement** (what did it cost?).

Together they form the Trust Model - the configuration that governs how humans and AI agents collaborate. It scales naturally: a solo developer has implicit trust (one member, all capabilities, no gates). A team adds explicit trust (members with specific capabilities). An enterprise adds verified trust (gates, cost limits, audit trails). Same workflow, growing accountability.

The rest of this mission covers the parts you can use today: identity (Trustship), the event log (Traceability), and capabilities (Trustship). Gates (Guardianship), cost tracking (Settlement), and AI dispatch (Orchestration) are covered in the [Vision](../dev/Vision.md#ai-governance-the-five-pillars).

### AI Identity

AI tools are registered as project members with an `ai:` prefix:

```sh
joy project member add ai:claude@joy          # detected automatically by `joy ai init`
joy project member add ai:copilot-chat@joy    # manual entry for chat-only tools
```

When an AI runs a Joy command, it authenticates with the delegation token you handed it; the token tells the CLI which AI member is acting and which human delegated. There is no `--author` flag, and the AI does not need to repeat its identity per call. The event log traces accountability back to that human:

```
[ai:claude@joy delegated-by:horst@joydev.com]
```

AI members have the same capabilities as human members, with one exception: **AI members cannot perform manage actions** (adding members, changing capabilities, modifying project settings). Management stays with humans.

### Chats, Sealing, and the Delegate Button

Team chats live in the repository (`refs/joy/chats`) and are **always sealed**: every message is encrypted for the chat's participants with their member keys, on the writing device. A clone without an identity cannot persist a chat at all (`joy auth init` first). The Joyint platform never holds a chat key - it stores and transports sealed bytes; reading and writing happen in your app, with your unlocked seed. The same chat opens identically in the web app and the desktop app.

An AI answers in a chat **only under an explicit delegation from the person addressing it**. In the apps, delegation is a button: open the AI member's card in Settings, press **Delegate**, and confirm with your own passphrase - that mints the delegation token behind the scenes. **Undelegate** revokes it immediately, running sessions included. Without your delegation, the AI does not appear in your @mention suggestions, and a mention typed anyway gets a refusal notice instead of a turn. Whatever a delegated AI does - chat replies, items, commits - is recorded as the AI member acting `delegated-by` you, never under your identity and never under an invented one.

### Keeping Instructions Current

You usually do not have to run anything explicitly. Every joy invocation
checks whether this clone is in sync with the running binary and
quietly refreshes the AI instruction files (and the rest of the
joy-managed state) when it sees a version mismatch. When that happens
joy prints a one-line `joy X.Y.Z: synced this repo (...)` notice on
stderr; if your AI tool's instruction file is mentioned in that
output, re-read it before continuing the session.

If you want an explicit audit, run:

```sh
joy update --check               # Read-only: every joy-managed artefact
joy update                       # Refresh anything that is stale
```

`joy update` also handles the binary self-update when joy was
installed via the cargo-dist installer. See "Updating joy" below.

---

## 9. Project Configuration

Joy starts with zero ceremony: no gates, no approvals. Add rules only when the project actually needs them.

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
joy project member add pete@example.com
joy project member add ai:claude@joy --capabilities "implement,review"
joy project member show pete@example.com
joy project member rm pete@example.com
```

Joy defines eleven capabilities across two groups.

**Lifecycle capabilities** govern what a member can do on items:

| Capability | What it grants |
|---|---|
| `conceive` | Frame a problem and propose direction (typically on `idea`/`epic`). |
| `plan` | Break work down: scope, effort, milestones. |
| `design` | Settle the technical approach for an item. |
| `implement` | Write the code or content. |
| `test` | Verify behaviour and add tests. |
| `review` | Approve work from someone else and gate `submit -> closed`. |
| `document` | Update user- or developer-facing docs. |

**Management capabilities** govern project-level operations:

| Capability | What it grants |
|---|---|
| `create` | Create new items (`joy add`). |
| `assign` | Assign items to members (`joy assign`). |
| `manage` | Add/edit members, change project settings. |
| `delete` | Remove items (`joy rm`). |

`joy project member add` defaults to the lifecycle set plus `create` and `assign`. `manage` and `delete` must be granted explicitly. AI members never get `manage` even when their entry says so - that is enforced at runtime.

### Interaction Levels

Each capability also carries an interaction level that tells AI tools how much autonomy they have. Levels apply to every member; for humans they are a team agreement, for AI members they are enforced through the tools. Joy defines three levels, from least to most oversight:

- `autonomous` - work independently; only stop at governance gates
- `confirmed` - work independently; confirm before irreversible actions
- `proposing` - propose; the human decides every step

The effective level for a `(member, capability)` pair is resolved across these layers, each overriding the previous:

1. **Project defaults** (`.joy/project.defaults.yaml`) - ship with sensible defaults per capability (e.g. `proposing` for `conceive`, `confirmed` for `implement`, `autonomous` for `test`).
2. **Project overrides** (`.joy/project.yaml`) - per-capability settings the team agrees on for this project, under `interaction-level`.
3. **Member defaults** (`.joy/project.yaml`) - the member's own default next to its capabilities: a global `interaction-level` on the member entry, plus per-capability values in the expert view.
4. **Personal preference** (`.joy/config.yaml`) - per-user override under `interaction-level.default`, applied to capabilities the project hasn't pinned.
5. **Item override** - a single item can request a different level via its `interaction-level` field, taking effect only for that item.

A per-capability `max-interaction-level` on the member entry acts as a floor on oversight: the resolved level is clamped toward `proposing`, never relaxed.

Inspect what is in force with:

```sh
joy project member show ai:claude@joy   # All capabilities, current level + source
joy project member show pete@example.com
```

The output's third column shows the level and (in brackets) where it was set. Tools and AI agents read this command and follow the level shown - they do not re-derive it.

### Gates (Status Rules)

By default every status transition is allowed. Add gates only when the project needs them. Gates live in `.joy/project.yaml` under `status_rules`:

```yaml
status_rules:
  review_to_closed:
    allow_ai: false        # AI members may not close items
  in_progress_to_review:
    allow_ai: true
```

Today only `allow_ai` is honored at runtime; more rule kinds (e.g. `requires_role`, `requires_ci`) are part of the vision and not yet enforced. The key follows the pattern `<from>_to_<to>` using the lower-case status names.

### Solo to Enterprise

Joy scales to the level of ceremony you actually need:

- **Solo:** one member, `capabilities: all`, no `status_rules`. Run `joy init` and start working.
- **Small team:** add members with explicit capability sets (e.g. AI tools restricted to `implement,review,document`). Interaction levels stay at project defaults.
- **Enterprise:** turn on gates (`status_rules`), tighten interaction levels per capability, set `allow_ai: false` on transitions where humans must sign off, and rely on the event log for audit.

The same workflow works at every scale - you only opt into more controls.

### Authentication and Onboarding

Joy uses passphrase-derived Ed25519 identity keys. You authenticate once per 24-hour session and every significant action is cryptographically signed.

**First time setup (solo):**

```sh
joy auth init                    # Choose a passphrase; your identity is now registered
# > Authentication initialized for you@example.com.
# > Public key registered. Session active (24h).
# >
# > RECOVERY KEY (write this down now, it is shown only once):
# >
# >     joy_r_<64-hex-characters>
# >
# > Use it with `joy auth recover --recovery-key` if you ever forget
# > your passphrase. Joy never stores the plaintext recovery key.
```

The recovery key is a one-shot escape hatch. It unlocks the
same identity keypair that your passphrase does, so you can reset the
passphrase from a new machine without losing access to anything you
have signed or encrypted under that identity.

**Adding a human teammate:**

The admin adds the member and gets a one-time password back. The OTP is shared out-of-band (encrypted chat, in person, etc.).

```sh
joy project member add pete@example.com
# > Added member pete@example.com
# >
# >   One-time password: AB7X-K3M2-PQ9Z
# >
# > Share the OTP with pete@example.com via a trusted channel.
```

Pete redeems the OTP on his own machine, picks his own passphrase, and is ready to go:

```sh
joy auth --otp AB7X-K3M2-PQ9Z    # Prompts for a new passphrase
```

Each member you add this way is cryptographically attested by the admin's key - Joy rejects any member entry that was manually edited into `project.yaml` without going through `joy project member add`. This runs silently in the background; you only see it when something is wrong.

**Changing your passphrase:**

```sh
joy auth passphrase              # Prompts for current, then new passphrase
```

The wrap of your seed re-encrypts under the new passphrase KEK. Your
identity keypair stays the same: the seed is the long-term secret, the
passphrase is one of two keys that unwrap it. Existing
attestations on your entry and Crypt zone wraps you have been granted
remain valid. Existing sessions are invalidated; run `joy auth` once
with the new passphrase.

**Recovering after a forgotten passphrase:**

```sh
joy auth recover --recovery-key  # Prompts for the recovery key + new passphrase
```

The recovery key (shown once at `joy auth init`) unwraps the seed via
its own KEK and re-wraps it under the new passphrase. Same keypair,
no re-onboarding.

**Non-interactive passphrase entry:**

Two flags let you supply the passphrase without typing it at the
prompt:

```sh
joy auth --passphrase 'correct horse battery staple'   # value on the command line
echo 'correct horse battery staple' | joy auth --passphrase-stdin
```

Use `--passphrase` for ad-hoc scripts and tests. Prefer
`--passphrase-stdin` when a GUI frontend or CI pipeline collects the
secret elsewhere: the value is read from a single stdin line and
never appears in the process listing the way `--passphrase <value>`
would. The flag is rejected together with `--passphrase`; pick one.
Both work on every command that takes a passphrase (`joy auth init`,
`joy auth`, `joy auth token add`, `joy auth recover`,
`joy project member add`, `joy crypt …`, etc.).

To rotate the recovery key from an authenticated session:

```sh
joy auth recover --regenerate-key  # New recovery key; old one becomes useless
```

**Removing a member:**

If the removed member attested others, those attestations transfer automatically to you as the removing admin. No extra step, no ceremony.

```sh
joy project member rm pete@example.com    # Requires your passphrase if there are orphans to re-attest
```

You cannot remove yourself; Joy prints the project's other manage members so you know who to ask.

### Anonymous privacy mode

By default a project is `open`: each member entry in `.joy/project.yaml`
carries the member's e-mail in cleartext. A project can instead run
`anonymous`, where no e-mail or name is written to the versioned files.

```bash
joy init --anonymous                 # start a new project anonymous (asks for a passphrase)
joy project set privacy anonymous    # or switch an existing project (needs auth + manage)
joy project set privacy open         # switch back
```

In anonymous mode each member is keyed by an opaque id (`m-<short>`) and
project.yaml carries a one-way `email_match` verifier instead of the
address; the cleartext e-mail lives only in `.joy/members.yaml`,
encrypted per member. Joy resolves ids back to e-mails for you
automatically while your session is active, never prints a raw id, and
asks a viewer who cannot decrypt to authenticate.

What is and is not covered:

- **Anonymised:** every Joy artifact in the working tree (project.yaml,
  items, logs) and release-note contributor lists.
- **Out of scope:** the Git committer identity (`user.name` /
  `user.email` in each commit) and anything already in history before
  the switch. Joy keeps only its own files free of cleartext PII.

Adding a human member while anonymous is refused (it would write the
e-mail in cleartext); add them in open mode and switch back. To honour a
deletion request (GDPR Art. 17), erase a member's e-mail and name from
`members.yaml` while keeping the opaque id and the audit trail:

```bash
joy project member erase someone@example.com
```

### AI Delegation Tokens

AI members authenticate via short-lived delegation tokens rather than passphrases. A human with manage capability issues a token; the AI redeems it in its own shell.

You run:

```sh
joy auth token add ai:claude@joy             # prints a joy_t_... token string
```

Share the token string with the AI in chat. The AI runs:

```sh
joy auth --token <token> --json
```

The `--json` response carries everything the AI needs in one go: `data.session_env` (the ephemeral session credential to pass on every subsequent call), `data.member` (the AI's own member ID, used for instance in commit metadata), and `data.delegated_by` (your email, recorded as the `Delegated-By:` commit trailer).

The AI then attaches the session to each command, either as a flag (recommended for AI tool runners that spawn a fresh shell per command, and for permission allowlists that prefer flags over env-var patterns):

```sh
joy ls --session <session_env>
joy add task "Investigate failing test" --session <session_env>
```

or as an env var, when the shell persists state between calls:

```sh
export JOY_SESSION=<session_env>
```

When both are set, `--session` wins. Tokens are multi-use within their TTL (default 24h); each redemption produces an independent session so the AI can run from multiple shells against the same delegation.

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

`joy config set` always writes to your personal `.joy/config.yaml` - your preferences never affect teammates. Project defaults in `config.defaults.yaml` set the shared baseline that the whole team inherits.

Key settings:

| Setting | Default | What it does |
|---------|---------|-------------|
| `workflow.auto-assign` | `true` | Auto-assign items on `joy start` |
| `output.color` | `auto` | Color mode: `auto`, `always`, `never` |
| `output.emoji` | `false` | Show emoji indicators in output |
| `output.short` | `true` | Compact list output (abbreviations) |
| `output.fortune` | `true` | Show occasional quotes in output |
| `auto-sync` | `true` | Refresh joy-managed state when the binary version moves ahead of this clone's marker |

---

## 10. Updating joy

Joy keeps two things current side by side: the `joy` binary on your
machine, and the joy-managed artefacts in each clone (`.gitattributes`,
the YAML merge driver registration, the commit-msg hook,
`SECURITY.md`, AI tool instruction files, ...). Both are handled by a
single command:

```sh
joy update                       # Swap binary + refresh in-repo state
joy update --check               # Read-only audit of every joy-managed artefact
joy update --no-binary           # In-repo refresh only
joy update --json                # Same, machine-readable envelope
```

`joy update` is receipt-gated for the binary swap: only builds installed
through the cargo-dist installer carry the receipt that lets joy update
itself in place. Builds installed via `cargo install`, Homebrew, or a
distro package skip the swap with a clear message and ask you to use the
installer that placed the binary. The in-repo refresh runs in either
case (when the binary on disk is at least as new as the repo's marker;
see "Downgrade guard" below).

### Auto-sync: the in-repo half is implicit

You almost never have to run `joy update` for the in-repo refresh.
Every joy invocation cheaply compares the running binary's version
against `joy.last-sync-version` (kept in this clone's local git config)
and silently catches up when they differ. When that happens you see a
single stderr line such as:

```
joy 0.15.0: synced this repo (previous marker: 0.14.2).
```

If the file mentioned alongside that line is your AI tool's instruction
file (CLAUDE.md, COPILOT.md, QWEN.md, AGENTS.md), re-read it; the
in-context copy may now be stale. To opt out per project, set
`auto-sync: false` in `.joy/config.yaml`; `joy update` then becomes
the only path that touches in-repo state.

### Downgrade guard

If the repo was last synced by a newer joy binary than the one you are
running, joy refuses to roll repo state back. The first joy invocation
prints a one-line warning, `joy update --check` reports the version
marker as stale, and `joy update` runs the binary swap (so you can
catch up) but skips the in-repo refresh from this still-running OLD
process. Open a new shell once the new binary is in `$PATH` and the
auto-sync (or another `joy update`) does the rest.

### Two real flows

- **You have an older binary in a freshly synced repo.** `joy update`
  swaps the binary and tells you to re-run from a new shell; the next
  joy invocation in the new shell auto-syncs.
- **You have a current binary in an older repo.** Nothing to do
  explicitly: the very next joy command (any of them) auto-syncs.

---

## 11. Encryption with Crypt

Some Joy items are sensitive: NDA-bound customer information, embargoed
security incidents, multi-tenant work where one client's content must
never be visible to another. Crypt is selective end-to-end encryption:
anything you mark via `joy crypt add` is AES-256-GCM-encrypted on the
spot, and stays ciphertext through the working directory, Git's index,
every commit, every clone, and the forge.

You opt in per item or per file/directory. Unmarked content stays plain.

**Encrypt an item:**

```sh
joy crypt add JOY-0123 --passphrase "..."
# > Added JOY-0123 to zone 'default'.
```

After this call, `.joy/items/JOY-0123-*.yaml` on disk is the
`JOYCRYPT...` blob. Git stores those bytes verbatim. There is no Git
filter, no `.gitattributes` rule, no `.git/config` wiring - Joy
encrypts the file before Git ever sees it.

**Encrypt a file or whole directory:**

```sh
joy crypt add data/customer-x/notes.txt        # one file
joy crypt add data/customer-x/                 # recursive
joy crypt add --all                            # every existing item under the zone
```

**Read encrypted items:**

```sh
joy show JOY-0123 --passphrase "..."           # decrypts transparently
joy ls --passphrase "..."                      # list, with one prompt at the start
```

Once unlocked, every command in the same `joy` invocation uses the
zone keys without re-prompting. Write commands (`joy edit`, `joy
comment`, `joy start`, `joy close`, `joy assign`, `joy deps`,
`joy rm`, `joy milestone link`) prompt for the passphrase the same
way and then go straight through.

`joy ls` always lists every item in the project. Items in a zone you
cannot decrypt appear as a locked row: `***` in the typed columns and
`[encrypted, no access]` as the title. An `ENC` column is added as
soon as at least one item carries a zone, showing the zone name on
both unlocked and locked rows. `joy show` on an item without zone
access prints `no access to zone <name>`.

For non-interactive use (CI, scripts, hooks), set the passphrase via
the `JOY_PASSPHRASE` environment variable; it is consulted whenever
no `--passphrase` flag is on the command line. Treat the variable as
sensitive: it lives only in the shell that exports it.

**Read or edit encrypted free files:**

The default verbs keep plaintext off the local filesystem entirely:

```sh
joy crypt read   data/customer-x/notes.txt | less          # decrypt to stdout
echo "new content" | joy crypt write data/customer-x/notes.txt
joy crypt edit   data/customer-x/notes.txt                 # $EDITOR on a temp; re-encrypted on save
```

For binary files that need a real path on disk (PPT, PDF, images,
video), use the explicit `unlock` / `lock` toggle. **Any process
running as the same OS user, including AI tools, can read the file
while it is unlocked.** Lock as soon as you are done.

```sh
joy crypt unlock data/customer-x/diagram.png   # plaintext on disk; AI on FS can read it
xdg-open data/customer-x/diagram.png            # view in an external app
joy crypt lock   data/customer-x/diagram.png   # back to ciphertext
```

If you forget to lock, the next `joy auth` (or any other `joy crypt`
command that prompts you) walks the zones and re-locks every
plaintext file it finds.

**Inspect what is encrypted:**

```sh
joy crypt status                # zones, items in any zone, your access
joy crypt ls                    # paths, items, members for the addressed zone
joy crypt ls --unlocked         # files currently in plaintext on disk
joy crypt zone ls               # per-zone summary
```

**Multiple confidentiality boundaries (named zones):**

If two clients must never see each other's data, use named zones:

```sh
joy crypt add JOY-0125 --zone customer-x
joy crypt grant alice@team.com --zone customer-x
joy crypt zone ls
```

Each zone has its own key. Granting to zone A says nothing about
zone B even when both wraps live in the same `project.yaml`.

**Grant and revoke access:**

```sh
joy crypt grant bob@team.com    # X25519 ECDH wrap of the zone key for Bob
joy crypt revoke bob@team.com   # Remove Bob's wrap (rotate the zone key after for forward secrecy)
```

The grant uses Bob's `verify_key` from `project.yaml`; Bob just needs
to have run `joy auth` at least once so his key material is published
for the pairwise wrap.

**Grant access to an AI Tool (token-scoped):**

AI Tools (Claude Code, Qwen, Mistral Vibe, Copilot, ...) work
differently. Each operator has their own per-(operator, AI) delegation
keypair, derived deterministically from the operator's seed at
passphrase entry; the public key is registered once in `project.yaml`,
the private key is never persisted. A `joy crypt grant ai:claude@joy
--zone customer-x` writes one wrap per operator-delegation, all in one
commit. Each operator's tokens then pick up the access:

```sh
joy auth token add ai:claude@joy             # auth-only token, default 24h
joy auth token add ai:claude@joy --crypt --ttl 30m   # auth + crypt, 30 min
joy crypt grant   ai:claude@joy --zone customer-x    # per-operator wraps
joy crypt revoke  ai:claude@joy --zone customer-x    # remove all of them
```

The `--crypt` flag embeds your delegation private key in the token
itself. The AI then carries that key in its `JOY_SESSION` env var
(never on disk) and can unwrap zone keys until the token expires. An
auth-only token (no `--crypt`) lets the AI run joy commands but
returns "no access to zone" on any decrypt attempt.

**Per-token expiry is honoured by the session.** A `--ttl 30m` token
produces a 30-minute session, regardless of any longer session TTL
default. The AI cannot extend its own access beyond the token window.

**Rotation when something looks off:**

```sh
joy ai rotate ai:claude@joy   # nuclear: every operator's delegation for claude is gone
```

Per-operator rotation - kills your outstanding tokens for one AI
without touching teammates' delegations - is `joy auth delegation
rotate <ai>`. `joy ai rotate <ai>` is the project-wide nuclear:
every operator's delegation for that AI is gone.

**No separate Crypt recovery.** Crypt access is tied to your Auth
identity. As long as you can recover your identity via
passphrase or the recovery key, every Crypt zone you have a wrap for
remains decryptable. There is no second secret to lose.

**Forge plaintext via Joyint (optional).** Variants A and B:

- *Pure E2E (default).* Forge sees ciphertext. PR diffs / blame / web
  view show binary garbage for marked content. Strongest privacy.
- *Joyint as decrypt gateway.* Grant the platform a per-zone
  platform-wrap from the Joyint Web UI. Joyint can then render
  plaintext to authenticated reviewers in its UI, and decrypt-on-
  mirror to GitHub/GitLab/Gitea per `joy int mirror add ... --decrypt`.
  Audited and revocable. (Web UI lands with MS-03 WebUI Crypt parity.)

Other forges never receive a wrap; there is no Joy-aware decrypt
service running on GitHub. You either accept review tools showing
binary garbage on those forges, or you mirror through Joyint with
`--decrypt`.

---

## 12. Chats

Every project has team chats, and they live where the project lives: in the repository, on `refs/joy/chats`, sealed for their participants (see [8. AI Tool Integration](#chats-sealing-and-the-delegate-button)). The apps and the CLI open the same chats.

```sh
joy chat ls                      # Every chat you are in, newest first
joy chat show 10                 # Read a chat (this marks it read, as opening it in the app does)
joy chat show 10 --unread        # Only what you have not read yet
joy chat show 10 --last 5        # Only the last five messages
joy chat show 10 --since 2h      # Only the last two hours (30m, 2h, 3d)
joy chat send 10 "On my way."    # Say something
joy chat info 10                 # Who is in, who has read what
```

### Chat Ids

A chat has an id like an item: `MPS-CHAT-0010-BB`, the project's acronym, `CHAT`, a number, and a short suffix. In commands the number is enough (`10`, `0010`), as is `MPS-CHAT-0010`, the full id, the chat's name, or `general` for the team-wide chat. `joy chat ls` prints the full id; the apps show the number in the sidebar and the suffix only when two chats share a number.

### Reading and Sending

Reading fetches the chat from the forge first, so `show` and `ls` always print the forge's state. Sending appends locally and pushes at once, one forge contact per message; when someone else pushed first, the CLI fetches, unites and pushes again by itself. A chat created in the app is unknown to the CLI until a read fetched it, so run `joy chat ls` before writing into a chat you have not seen from the terminal yet.

### AI Members in Chats

An AI answers a mention (`@vibe ...` at the start of a message) only in the apps, and only under a delegation from the person addressing it: the app that sends the message starts the turn, on the platform for a platform project and on your machine for a local one, with your delegation, your key and your budget. The CLI has no chat session and no turn host, so `joy chat send` refuses a message that opens with the mention of an AI member and says why. A mention later in the text only refers to the AI and is sent as it is.

An AI can send from the CLI itself: with its session (`--session`, or `JOY_SESSION`) the message is posted as the AI member, marked as delegated by the person whose token the session came from.

## Bonus: Cross-Directory Queries (`-w`)

Joy normally operates on the project containing the current working directory. The global `-w / --working-dir <PATH>` flag runs a command as if you had `cd`'d into PATH first:

```sh
joy ls -w ../platform            # List items of the sibling project
joy roadmap -w ~/repos/jyn       # Roadmap of an unrelated project
joy log -w ../platform --limit 5 # Audit trail of another tree
```

PATH must contain a Joy project (a `.joy/` directory in itself or an ancestor); otherwise the command bails. Tab completion offers directory names after `-w`. This is the supported way to query across multiple projects without `cd` and without per-command repo shortcuts.

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

Tab completion replaces manual ID lookup; pair it with `joy ls` filters for fast navigation.

---

## Bonus: Machine-Readable Output

Every Joy command accepts a global `--json` flag. Default output stays human-readable; `--json` switches to a stable, structured envelope so scripts and CI never have to scrape display text.

```sh
joy ls --json                                # Same as joy --json ls
joy show JOY-0001 --json                     # Single item as JSON
joy --json ls | jq '.data.items[].id'        # Pipe into jq
```

The shape is `{"version": 1, "data": ...}`. Within a major Joy release, fields are added but never removed or repurposed - consumers can rely on the keys they already use. CI scripts should always consume `--json`, not display output.

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
| `joy comment <ID> [TEXT]` | Add comment (opens $EDITOR if TEXT omitted) |
| `joy comment edit <ID> <N> [TEXT]` | Replace comment #N |
| `joy comment rm <ID> <N> [--force]` | Delete comment #N |
| `joy deps <ID>` | Manage dependencies |
| `joy milestone` | Manage milestones |
| `joy roadmap` | Milestone roadmap (tree view) |
| `joy log` | Event log (audit trail) |
| `joy release bump <BUMP>` | Step 1: patch version strings in configured files |
| `joy release record <BUMP>` | Step 2: record, commit, tag (local only) |
| `joy release publish [--forge VALUE]` | Step 3: push + create the forge release (auto-detects forge from git remotes; `--forge` overrides per run) |
| `joy release show [VERSION]` | Show a release or preview the next |
| `joy release ls` | List all releases |
| `joy project` | View/edit project info and members |
| `joy project get/set <KEY> [VALUE]` | Read or write a project field (e.g. `forge`, `language`, `docs.*`) |
| `joy config` | Show or modify configuration |
| `joy ai init` | Set up AI tool integration |
| `joy chat ls` | List your chats |
| `joy chat show <CHAT>` | Read a chat and mark it read (`--unread`, `--last N`, `--since 2h`) |
| `joy chat send <CHAT> <TEXT>` | Send a message |
| `joy chat info <CHAT>` | Participants and read state |
| `joy update` | Update the joy binary and refresh joy-managed state |
| `joy update --check` | Read-only audit of every joy-managed artefact |
| `joy tutorial` | You are here |

Most write commands accept `--author <MEMBER>` to attribute the action to a specific identity. Every command accepts the global `-w / --working-dir <PATH>` flag to run as if started from PATH.

See also: `joy --help`, `joy <command> --help`, `docs/dev/vision/`
