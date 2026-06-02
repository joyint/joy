# Joy AI Tutorial

This is the CLI reference for AI assistants working in a Joy project. It tells you how to talk to the `joy` binary. It does not tell you how to behave in this project. Behavior is governed by [Joy's interaction modes](#session-start), the project's [authoritative docs](#reading-the-project), and the user's direction in chat. "Session" in this tutorial means one conversation between you and the user, from greeting to disconnect.

Re-run `joy ai tutorial` whenever a `joy` invocation prints `joy X.Y.Z: synced this repo (...)` mentioning this file, because operational details may have moved with the version.

## Session start

At session start, run two `joy` commands yourself to pick up project context:

```
joy config get modes.default
```

returns your default interaction mode for this project. Treat the resolved value as authoritative; do not guess. The five levels, from least to most oversight:

- `autonomous` - work independently; only stop at governance gates
- `supervised` - confirm before irreversible actions
- `collaborative` - propose approach, proceed after confirmation
- `interactive` - present options with rationale, wait for user decision
- `pairing` - step by step, question by question

Confirm the resolved mode to the user in one line at session start, e.g. "Working in collaborative mode. Want to change that for this session?". Accept natural language overrides at any time (e.g. "be more autonomous", "let's work through this together", "just do it").

```
joy project get language
```

returns the configured project language. Start the session communicating with the user in that language. Switch to the user's language as soon as they speak a different one. All written artifacts (item titles, descriptions, comments, commit messages) must stay in the project language regardless of the chat language.

Beyond these two checks plus reading the project's authoritative docs (see next section), run further joy commands only when the activity at hand requires them. Do not preemptively browse the backlog, roadmap, member state, or other project state when neither the user nor your current activity asks for it.

## Reading the project

At the start of every session, read the project's three authoritative docs:

- `docs.vision` - product goals and design decisions
- `docs.architecture` - technical stack and structure
- `docs.contributing` - coding conventions and commit messages

Their paths are exposed by the CLI:

```
joy project get docs.vision
joy project get docs.architecture
joy project get docs.contributing
```

If two or more of these paths resolve to the same file, read it once.

Re-read each file on every new session, because the contents may have changed since you last read it. If any file is missing, empty, or contains only template stubs (HTML comments, headings without body), surface it to the user at session start and offer to fill it in by asking targeted questions and writing the answers. These three files are specification, not background reading. Leaving them as templates is not acceptable.

After the three docs, list the project's decisions at session start so you know what policy items exist:

```
joy ls --type decision --all
```

Decisions are project-wide policy choices that apply to all subsequent work unless the decision body restricts its scope. Read the body of a specific decision via `joy show <ID>` when your activity touches its topic. Skip items in status `new`, `open`, or `deferred` (not yet binding).

## Operational loop

After session start:

1. User asks. If purely informational, answer with read-only commands.
2. For changes: obtain a session if missing (see [Authentication](#authentication)), find or create the relevant Joy item.
3. Plan-comment, `joy start <ID>`, do the work.
4. Result-comment, `joy close <ID>` (or `joy submit <ID>` per your mode).
5. Commit with `[JOY-XXXX-XX]` in the subject and the `Co-Authored-By:` + `Delegated-By:` trailers (see [Commit messages](#commit-messages)).

## Authentication

A session is only needed for writes. Read-only commands work without one: `joy ls`, `joy show`, `joy find`, `joy log`, `joy roadmap`, `joy config get`, `joy project`, `joy auth status`, `joy ai tutorial`.

### Get a session

Run this yourself to check if you are already a member:

```
joy project member
```

If you see your `ai:...@joy` row, you are registered. Ask the operator to create a token. The operator runs:

```
joy auth token add ai:<name>@joy
```

If you do not see your row, you are not registered. Ask the operator to register you. The operator runs one of these:

```
joy project member add ai:copilot-chat@joy --with-token
joy ai init
```

`member add --with-token` works for any tool, including chat-only tools like Copilot or Cursor. It registers you and prints a token in one step. `joy ai init` works only for tools Joy can detect. It writes instruction files and registers you. After `joy ai init`, the operator runs `joy auth token add ai:<name>@joy` to create the token.

Suggest a name based on the tool you are, for example `ai:copilot-chat@joy` or `ai:cursor@joy`. This is only a suggestion. The real ID is returned with the token.

The operator gives you a token starting with `joy_t_`. Redeem it yourself, one time:

```
joy auth --token <TOKEN> --json
```

The response contains three values:

```
{ "data": {
    "session_env": "joy_s_...",
    "member": "ai:copilot-chat@joy",
    "delegated_by": "operator@example.com"
} }
```

`session_env` is your session credential. Pass it as `--session` on every write. `member` is your real ID. Use it in commit trailers and item references. `delegated_by` goes into the `Delegated-By:` trailer of every commit.

Reuse the same `session_env` for the whole session. Redeeming the token again creates a new session and makes the old `session_env` invalid. The old one then fails with "guard denied".

### Use the session: always --session

Run writes yourself. Put `--session` at the end of every write:

```
joy add task "Investigate failing test" --session <session_env>
joy start JOY-0042-AB --session <session_env>
joy comment JOY-0042-AB "Plan: ..." --session <session_env>
```

AI tools start a fresh shell for each command. The `--session` flag is the only reliable way to pass the session.

The `JOY_SESSION` environment variable is an alternative. It only works in one shell that keeps its environment across all `joy` calls. Set it with `export JOY_SESSION=<session_env>`. If both are set, `--session` wins. Prefer `--session`.

### When auth fails

Run this yourself to check your identity and session:

```
joy auth status
```

Read the error. Do not retry the same token.

```
guard denied: <human> must authenticate
```

Your `--session` value is missing, old, or from a later redemption. You fell back to the git identity. Pass the current `session_env`.

Other errors:

```
Expired              # ask the operator for a fresh token
Wrong project        # token is from another project; ask for one here, or use -w <path>
Bad signature        # token is corrupted; ask for a new one
encrypted, no access # ask the operator for: joy auth token add <YOUR-ID> --crypt
```

Most joy commands accept `--json` for structured output. Use it to extract specific fields; the human-readable default works well for general reading.

## Capabilities and gates

Your capabilities (what kind of work you are allowed to do) and the per-capability interaction mode are shown in full form by `joy project member show <YOUR-MEMBER-ID>`. The `joy project` member table is the compact overview of the same data for all members. You discover a missing capability when a `joy` command refuses with a capability warning. **A capability warning is a hard stop.** Surface it to the user, do not attempt a workaround.

Status transitions may be restricted by per-project gates. `joy project` shows the workflow diagram and the list of configured gates. When a gate blocks an AI-initiated transition, the CLI refuses with a clear message. Tell the user and stop; do not search for another path.

## Workflow

The status flow diagram and any configured gates are visible in `joy project`. Item-shortcut commands map to status transitions:

- `joy start <ID>`: open or new to in-progress.
- `joy submit <ID>`: in-progress to review.
- `joy close <ID>`: review to closed.
- `joy reopen <ID>`: closed or deferred to open.

## Item lifecycle commands

| Command | Purpose |
|---|---|
| `joy ls` | List items. Filters: `--status`, `--type`, `--mine`, `--blocked`. |
| `joy find <KEYWORD>` | Search items by text across titles, descriptions, comments. |
| `joy show <ID>` | Item details. Read before modifying. |
| `joy add <TYPE> "<TITLE>" [OPTIONS]` | Create an item. Use `--description`, `--priority`, `--effort`. |
| `joy edit <ID> [OPTIONS]` | Modify an item. |
| `joy comment <ID> "<TEXT>"` | Append a comment. |
| `joy deps <ID> --add <OTHER>` | Add a dependency. |
| `joy milestone link <ID> <MILESTONE>` | Link an item to a milestone. |
| `joy roadmap` | Milestone roadmap with progress. |

Run `joy <command> --help` for the full surface.

## Item types, priorities, effort

Types: `epic`, `story`, `task`, `bug`, `rework`, `decision`, `idea`.

Priorities: `critical`, `high`, `medium`, `low`.

Effort, both numeric and t-shirt size accepted: `1=xxs`, `2=xs`, `3=s`, `4=m`, `5=l`, `6=xl`, `7=xxl`.

## Commit messages

Conventional Commits: `type(scope): short imperative description`. Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`, `rework`.

The subject line must reference at least one Joy item, e.g. `[JOY-0042-AB]`. Multiple references are allowed: `[JOY-0042-AB] [JOY-0043-CD]`. `[no-item]` is the explicit escape for infrastructure commits with no logical backlog item, for example release version bumps, regenerated artefacts, or CI workflow setup. If unsure, create an item.

End every commit message with two trailers:

```
Co-Authored-By: <YOUR-TOOL-NAME> <YOUR-TOOL-EMAIL>
Delegated-By: <data.delegated_by from your token redemption>
```

The exact `Co-Authored-By:` line is set in your tool-specific instruction file (e.g. `Co-Authored-By: Claude <noreply@anthropic.com>` for Claude Code). The `Delegated-By:` line names the human operator who delegated to you for this session, taken from `data.delegated_by` of the token redemption JSON.

## Minimum AI hygiene

The rules in `docs.contributing` take precedence over everything below. If `docs.contributing` is empty or template-only, or silent on a particular point, the following minimums apply (in addition to the commit-message rules in [Commit messages](#commit-messages)):

1. **Item before code.** Never write code without first running `joy start <ID>` on an open Joy item. If no item exists for the work, a new one must be added (`joy add <type> "<title>"`) before starting.
2. **Plan before, result after.** A plan comment (`joy comment <ID> "Plan: ..."`) must be added to the item before writing code, and a result comment (`joy comment <ID> "[x] what was done"`) after.
3. **Close before final commit.** The item must be closed with `joy close <ID>` (or submitted with `joy submit <ID>` if your mode keeps you out of the closing step) before the final `git commit`.

## Where Joy data lives

Never read or write files under `.joy/` directly. The `joy` CLI is the only correct interface to project state. If you need an operation that no `joy` command supports, ask the user; never edit YAML by hand.

The CLI may auto-stage files it creates or modifies (controlled by `joy config get workflow.auto-git`). When you run `git commit`, those staged changes are picked up automatically; review `git status` before committing so you know what is going in.

---

AI and human, one team, one goal, joy in every commit.
