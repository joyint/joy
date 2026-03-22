# Joy AI Instructions

You are working in a project managed with [Joy](https://github.com/joyint/joy), a terminal-native, Git-native product management tool.

## Session start

At the start of each session:

1. Run `joy ai check` to verify your AI instructions are current. If it exits with
   code 2, tell the user which templates are outdated and suggest `joy ai setup`.
   Do not proceed with outdated instructions.
2. Run `joy config get agents.default.interaction-level` to read the configured level.
   If the key does not exist, default to level 3.
3. Briefly confirm: "Working in interactive mode (level 3). Want to change that for
   this session?" One line, no menu.
4. Accept natural language overrides at any time ("let's work through this together",
   "just do it", "be more autonomous", etc.).

Interaction levels:
- **1-2**: Start working autonomously. Only confirm before irreversible actions.
- **3**: Propose your approach, proceed after confirmation.
- **4**: Propose options with rationale, wait for the user's decision.
- **5**: Work through it step by step, question by question.

The user can set the default level with:
`joy config set agents.default.interaction-level 4`

Per-role levels (e.g. `agents.architect.interaction-level`) override the default
when the AI is acting in that role.

## Core commands

Use these Joy CLI commands for all product management operations:

| Command | Purpose |
|---------|---------|
| `joy ls` | List and filter items |
| `joy show <ID>` | Show item details (always read before modifying) |
| `joy add <TYPE> <TITLE> [OPTIONS]` | Create a new item |
| `joy edit <ID> [OPTIONS]` | Modify an existing item |
| `joy comment <ID> "TEXT"` | Add a comment to an item |
| `joy start <ID>` | Set status to in-progress |
| `joy close <ID>` | Set status to closed |
| `joy roadmap` | Show milestone roadmap with progress |
| `joy milestone show <ID>` | Show milestone details and risks |
| `joy config` | Show current project configuration |
| `joy project` | Show project metadata |

Item types: `epic`, `story`, `task`, `bug`, `rework`, `decision`, `idea`.
Priority levels: `critical`, `high`, `medium`, `low`.
Effort scale (1-7): 1=trivial, 2=small, 3=medium, 4=large, 5=major, 6=heavy, 7=massive.

## Rules

**Always use the Joy CLI.** Never read or write files in `.joy/` directly -- not items, not config, not milestones. Use `joy ls`, `joy show`, `joy config`, etc. If a Joy command does not exist for an operation, ask the user or suggest a new command -- do not work around it by editing YAML.

**Every code change needs a Joy item.** If you discover a bug, identify a rework need, or make any change to the codebase, create a Joy item for it BEFORE implementing the fix. This is non-negotiable -- the event log is the project's audit trail. Ad-hoc fixes without items are invisible to governance and compliance.

**Track status.** Run `joy start <ID>` before coding, `joy close <ID>` after committing. Never skip status tracking.

**Comment everything.** Before implementing, comment the planned solution: `joy comment <ID> "Plan: ..."`. After implementing, comment the result: `joy comment <ID> "[x] what was done"`. This applies to ALL items -- planned work, discovered bugs, and ad-hoc fixes alike. The comments are the audit record of what was decided and why.

**Use the project language for artifacts only.** Run `joy project` to read the configured language (default: `en`). This language strictly governs all written artifacts: Joy item titles, descriptions, comments, commit messages, and documentation. Never deviate from it, even if the conversation is in another language. **Conversation language is separate.** For interactive communication (responses, explanations, questions), detect and follow the user's language. If the user writes in German, respond in German. The project language setting does NOT apply to conversation -- only to artifacts that are persisted in the project.

**Titles are short.** Max 60 characters, actionable ("Add X", "Fix Y", not "X should be added").

**No emoji in docs.** No emoji in documentation, commit messages, or code comments. Emoji are a CLI runtime feature only.

## Working with items

### Creating items

Analyze the user's input and break it into Joy items. Present a numbered list (title, type, priority, effort) for confirmation before creating. Suggest an effort (1-7) based on the scope of each item. Use `--effort` when creating: `joy add task "Fix login" --effort 2`. Create epics first when there are 3+ related items. Do not over-decompose -- a 1-2 day story is fine as one item.

### Implementing items

1. Read the item: `joy show <ID>`
2. Comment your planned solution: `joy comment <ID> "Plan: ..."`
3. Confirm with the user
4. Start the item: `joy start <ID>`
5. Implement the changes
6. Commit the code
7. Comment the result: `joy comment <ID> "[x] done this, [x] done that"`
8. Close the item: `joy close <ID>`

### Suggesting next work

When asked what to work on next, check:
1. Current milestone items: `joy milestone show <MS-ID>`
2. Blocked items that can be unblocked
3. High-priority items without a milestone
Prioritize milestone items over unlinked items.

## Project context

Before starting work, read these documents if they exist:
- `docs/dev/Vision.md` -- product goals and design decisions
- `docs/dev/Architecture.md` -- technical stack and structure
- `CONTRIBUTING.md` -- coding conventions and commit messages

These documents are the source of truth. Do not contradict them.

## First session

At the start of your first session in a project, ALWAYS do these checks
before anything else:

1. Read `docs/dev/Vision.md`, `docs/dev/Architecture.md`, and `CONTRIBUTING.md`
2. If any of these files are missing, empty, or contain only template
   headings (HTML comments like `<!-- ... -->`), tell the user and offer
   to fill them in together
3. Read `.joy/ai/instructions/setup.md` for the checklists to guide the
   conversation

Do not wait for the user to ask. This check is mandatory on first session.

## Commit messages

Use conventional commits: `type(scope): description`
Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`
No emoji in commit messages.

**Every commit must reference a Joy item ID** (e.g. `JOY-0001`). A commit-msg hook
enforces this. For infrastructure commits without an item, use `[no-item]` in the
message. In multi-repo setups, each subproject needs its own items -- a commit in the
Joy repo references `JOY-XXXX`, a commit in the umbrella references `JI-XXXX`.
