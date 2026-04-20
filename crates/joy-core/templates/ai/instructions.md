# Joy AI Instructions

You are working in a project managed with [Joy](https://github.com/joyint/joy), a terminal-native, Git-native product management tool.

## Session start

At the start of each session:

1. Run `joy ai update --check` to verify your AI instructions are current. If it exits with
   code 2, tell the user which templates are outdated and suggest `joy ai update`.
   Do not proceed with outdated instructions.
2. Run `joy config get modes.default` to read the configured level.
   If the key does not exist, default to `collaborative`.
3. Briefly confirm: "Working in collaborative mode. Want to change that for this
   session?" One line, no menu.
4. Accept natural language overrides at any time ("let's work through this together",
   "just do it", "be more autonomous", etc.).

Interaction levels:
- **autonomous**: Work independently. Only stop at governance gates.
- **supervised**: Confirm before irreversible actions.
- **collaborative**: Propose approach, proceed after confirmation.
- **interactive**: Present options with rationale, wait for user decision.
- **pairing**: Step by step, question by question.

Per-capability levels in `project.yaml` override the default.

## Identity and capabilities

Your member ID is defined in the tool-specific configuration file
(e.g. CLAUDE.md, QWEN.md). At the start of each session:

1. Read your member ID from the tool configuration.
2. Run `joy project member show <YOUR-ID>` to verify your member entry
   exists and to read your current capabilities and limits.
3. If your member entry does not exist, tell the user and suggest
   `joy ai init` or `joy project member add <YOUR-ID>`.

### Authentication

**You must authenticate before executing Joy write commands.**
Read-only commands (`joy ls`, `joy show`, `joy roadmap`, `joy config`, `joy project`)
are always allowed without authentication.

1. Run `joy auth status` to check if you already have an active session.
2. If not authenticated, ask the user for a delegation token:
   "I need a delegation token. Please run `joy auth token add <YOUR-MEMBER-ID>` and share the token."
3. Run: `joy auth --token <TOKEN>` and capture the `export JOY_SESSION=...`
   value from its stdout. That value is the ephemeral session credential; it
   lives only in the shell's environment and is not written to any file.

Tokens are multi-use within their TTL (default 24h); the same token string
may be redeemed in multiple shells on the same machine, and each redemption
produces an independent session. The AI tool is responsible for propagating
the captured `JOY_SESSION` value into every subshell it spawns for joy
commands.

If `joy auth --token` fails, read the error and react accordingly:
- **Token expired** -- the token's TTL has passed. Do not retry with the
  same token. Ask the user to run `joy auth token add <YOUR-ID>` again and
  pass you the new token.
- **Token for wrong project** -- the token was issued for a different
  project. Ask the user to issue a token in the current project.

If a later Joy command fails with a session error, your 24 hour session
has expired or the delegation has been rotated; ask the user for a fresh
delegation token and re-run `joy auth --token`.

Respect your configured capabilities and `max-mode` limits.
**Capability warnings are mandatory stops** -- if a Joy command prints one, stop and ask the user.

## Workflow

Shortcuts: `joy start <ID>` (begin work), `joy submit <ID>` (request review), `joy close <ID>` (done), `joy reopen <ID>` (reopen).

Gates: projects can restrict transitions via `status_rules` in project.yaml. When `allow_ai: false`, inform the user.

## Core commands

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
| `joy config` | Show current project configuration |
| `joy project` | Show project metadata |

Item types: `epic`, `story`, `task`, `bug`, `rework`, `decision`, `idea`.
Priority levels: `critical`, `high`, `medium`, `low`.
Effort scale (1-7): 1=trivial, 2=small, 3=medium, 4=large, 5=major, 6=heavy, 7=massive.

## Rules

**Use the project language for all artifacts.** Run `joy project` to read the configured language (default: `en`). This language strictly governs all written artifacts: Joy item titles, descriptions, comments, commit messages, and documentation. Never deviate, even if the conversation is in another language. Conversation language is separate -- follow the user's language for responses.

**Always use the Joy CLI.** Never read or write files in `.joy/` directly. If a Joy command does not exist for an operation, ask the user -- do not work around it by editing YAML.

**Every code change needs a Joy item.** Create a Joy item BEFORE implementing. Ad-hoc fixes without items are invisible to governance.

**Track status.** Run `joy start <ID>` before coding, `joy close <ID>` after committing.

**Comment everything.** Before implementing: `joy comment <ID> "Plan: ..."`. After: `joy comment <ID> "[x] what was done"`.

**Confirm before changing Joy data.** At mode `collaborative` and above, never create, edit, or close items without explicitly confirming with the user first.

**Titles are short.** Max 60 characters, actionable ("Add X", "Fix Y").

**No emoji in docs.** No emoji in documentation, commit messages, or code comments.

## Project context

On first session, read your project's docs - run `joy project get docs.architecture`, `joy project get docs.vision`, and `joy project get docs.contributing` to get the paths. If a file does not exist or is template-only, offer to fill it in. These documents are the source of truth.

## Commit messages

Use conventional commits: `type(scope): description`
Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`

**Every commit must reference a Joy item ID** (e.g. `JOY-0001`). A commit-msg hook enforces this. For infrastructure commits without an item, use `[no-item]`.

## Working style

These rules apply to every AI assistant working in this project. They
override tool-specific defaults where those conflict. Be explicit about
intent; do not rely on implicit generalization.

### Response style
- Concise and focused. Skip non-essential context, minimal examples.
- Length matches task complexity. No preamble, recap, or closing summary unless asked.
- Lists only for genuinely enumerable content, not for prose inflation.

### Task specification
- Delegate, do not pair-program. State intent, constraints, acceptance criteria, file locations upfront.
- Respecify the whole task if scope shifts; do not clarify vague prompts across many turns.

### Corrections
- A correction is scoped to the point raised, not a mandate to rearchitect.
- Preserve existing approach, structure, and files unless told otherwise.
- Ask before pivoting if a correction seems to imply a larger change. Never discard working code to "clean up".

### Complexity
- Simplest solution that meets the criteria. No unrequested abstractions, config, error layers, or future-proofing.
- No new dependencies without asking. Edit existing code before creating new files.

### Plan-first
- For changes touching >1 file or ~30 lines: produce a <15-line plan (files, functions/types, expected diff shape). Wait for confirmation.

### Tool use
- Do not re-read files or re-run searches already in context.
