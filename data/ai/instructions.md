# Joy AI Instructions

You are working in a project managed with [Joy](https://github.com/joyint/joy), a terminal-native, Git-native product management tool.

## Session start

At the start of each session, briefly confirm the current interaction level. Example: "Working in interactive mode (level 4). Want to change that for this session?" Accept natural language overrides at any time ("let's work through this together", "just do it", etc.).

Read the interaction level from `.joy/config.yaml` under `agents.<role>.interaction-level`. If not configured, default to level 3 (propose approach, then execute after confirmation).

Interaction levels:
- **1-2**: Start working autonomously. Only confirm before irreversible actions.
- **3**: Propose your approach, proceed after confirmation.
- **4**: Propose options with rationale, wait for the user's decision.
- **5**: Work through it step by step, question by question.

The user can override the level at any time during the conversation.

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

Item types: `epic`, `story`, `task`, `bug`, `rework`, `decision`, `idea`.
Priority levels: `critical`, `high`, `medium`, `low`.

## Rules

**Always use the Joy CLI.** Do not read or write `.joy/items/*.yaml` files directly. If a Joy command does not exist for an operation, ask the user or suggest a new command -- do not work around it by editing YAML.

**Track status.** Run `joy start <ID>` before coding, `joy close <ID>` after committing. Never skip status tracking.

**Comment your plan.** Before implementing a backlog item, comment the planned solution into the item using `joy comment`. Confirm with the user before proceeding.

**English only.** All item titles, descriptions, and comments must be in English, regardless of the conversation language.

**Titles are short.** Max 60 characters, actionable ("Add X", "Fix Y", not "X should be added").

**No emoji in docs.** No emoji in documentation, commit messages, or code comments. Emoji are a CLI runtime feature only.

## Working with items

### Creating items

Analyze the user's input and break it into Joy items. Present a numbered list (title, type, priority) for confirmation before creating. Create epics first when there are 3+ related items. Do not over-decompose -- a 1-2 day story is fine as one item.

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

## Commit messages

Use conventional commits: `type(scope): description`
Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`
No emoji in commit messages.
