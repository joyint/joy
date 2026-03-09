# Joy -- Tutorial

This tutorial walks through Joy's core workflow using a small example project: a personal recipe manager called **Cookbox**.

---

## 1. Initialize a project

```sh
mkdir cookbox && cd cookbox
git init
joy init --name "Cookbox"
```

This creates a `.joy/` directory inside the repo:

```
.joy/
├── config.yaml
├── project.yaml
├── items/
├── milestones/
├── ai/
└── log/
```

Everything Joy knows about your project lives here. It is plain YAML, versioned with Git.

---

## 2. Add items

Create an epic to group related work, then add stories and tasks beneath it:

```sh
joy add "Recipe Management" --type epic
```

Joy assigns ID `EP-0001` and creates `.joy/items/EP-0001-recipe-management.yaml`.

```sh
joy add "Add a recipe" --type story --epic EP-0001 --priority high
joy add "Edit a recipe" --type story --epic EP-0001 --priority high
joy add "List recipes with filters" --type story --epic EP-0001 --priority medium
joy add "Set up SQLite database" --type task --epic EP-0001 --priority critical
```

Items are created with status `new`. Without any flags, `joy add` opens an interactive prompt.

---

## 3. View the backlog

```sh
joy ls
```

```
ID       Type   Priority  Status  Title
IT-0001  story  high      new     Add a recipe
IT-0002  story  high      new     Edit a recipe
IT-0003  story  medium    new     List recipes with filters
IT-0004  task   critical  new     Set up SQLite database
```

Filter by type, status, priority, or epic:

```sh
joy ls --type story
joy ls --priority critical
joy ls --epic EP-0001
```

See the full project overview (board-style):

```sh
joy
```

---

## 4. Work on items

Move items through the workflow:

```sh
joy status IT-0004 open          # approve for work
joy start IT-0004                # shortcut for: joy status IT-0004 in-progress
```

Assign the task to yourself:

```sh
joy assign IT-0004 orchidee@joyint.com
```

The database task should be done before recipes can be added. Add a dependency:

```sh
joy deps IT-0001 --add IT-0004
```

Now starting IT-0001 while IT-0004 is still open triggers a warning:

```sh
joy status IT-0001 open
joy status IT-0001 in-progress
# warning: IT-0001 depends on IT-0004 (in-progress)
```

Joy warns but does not block. You decide.

---

## 5. Submit and close

When work is done, move through review to closed:

```sh
joy submit IT-0004               # shortcut for: joy status IT-0004 review
joy close IT-0004                # shortcut for: joy status IT-0004 closed
```

Add a comment before closing:

```sh
joy comment IT-0004 "Database schema looks good, all migrations pass."
```

Check what is still open:

```sh
joy ls --status open
joy ls --blocked
```

---

## 6. Use milestones

Group items into time-boxed goals:

```sh
joy milestone add "MVP" --date 2026-04-01
joy milestone link IT-0001 MS-01
joy milestone link IT-0002 MS-01
joy milestone link IT-0004 MS-01
```

See milestone progress:

```sh
joy milestone show MS-01
```

---

## 7. View details and history

Inspect a single item in full detail:

```sh
joy show IT-0001
```

This displays all fields, dependencies, comments, and change history.

See the project changelog:

```sh
joy log
joy log --since 7d
joy log --item IT-0004
```

---

## 8. Adjust the process

Joy has one workflow with adjustable strictness. By default every status transition is open to everyone. You control strictness by adding rules.

### No rules (solo / prototype)

Out of the box, anyone (including AI agents) can move items to any status. No ceremony.

### Add a triage gate

You want new items to be reviewed before they enter the backlog. Edit `.joy/project.yaml`:

```yaml
roles:
  approver: [orchidee@joyint.com]

status_rules:
  new -> open:
    requires_role: approver
```

Now only `orchidee@joyint.com` can approve items. Joy matches this against `git config user.email` locally and against the OAuth-provided e-mail on the server. Everyone else can still create items (`new`) and work on approved ones.

### Add an acceptance gate

You also want only approvers to close items, and only when CI is green:

```yaml
roles:
  approver: [orchidee@joyint.com]

status_rules:
  new -> open:
    requires_role: approver
  review -> closed:
    requires_role: approver
    requires_ci: true
    allow_ai: false
```

`allow_ai: false` means AI agents cannot close items, even if they could otherwise act as the assigned role.

### Remove rules

To loosen the process, remove rules from `status_rules`. Delete the entire section to go back to zero ceremony. There are no templates, no modes, no workflow engine. Just rules you add or remove.

---

## 9. AI assistance

Set up an AI tool:

```sh
joy ai setup claude-code
joy ai setup mistral-vibe --model devstral-small
```

Joy detects installed CLI tools and configures the chosen one with a model (or `auto` for the tool's default).

Estimate effort for an item:

```sh
joy ai estimate IT-0003
```

Break an epic into detailed items:

```sh
joy ai plan EP-0001
```

Dispatch an implementation to the configured AI tool:

```sh
joy ai implement IT-0001
joy ai implement IT-0001 --budget 5.00
```

Joy prepares the context (item description, relevant code, branch name), invokes the tool, and tracks the result.

Review the result:

```sh
joy ai review IT-0001
```

Track costs:

```sh
joy ai status --costs
```

AI agents are tracked as team members. Their work goes through the same workflow, the same status transitions, and the same rules.

---

## 10. Sync (optional)

For collaboration, add a remote:

```sh
joy sync --push                # push to joyint.com or self-hosted server
joy sync --pull                # pull changes from others
joy clone joyint.com/orchidee/cookbox  # clone a remote project
```

In v1, sync uses HTTPS with authenticated connections. End-to-end encryption for item content is planned for v2.

---

## Summary

| Command            | What it does                        |
| ------------------ | ----------------------------------- |
| `joy init`         | Initialize a project                |
| `joy add`          | Create an item                      |
| `joy ls`           | List and filter items               |
| `joy`              | Project overview                    |
| `joy status`       | Change item status                  |
| `joy start/submit/close` | Status shortcuts              |
| `joy assign`       | Assign item to person or agent      |
| `joy comment`      | Add comment to item                 |
| `joy show`         | Item detail view                    |
| `joy edit`         | Modify an item                      |
| `joy rm`           | Delete an item                      |
| `joy deps`         | Manage dependencies                 |
| `joy milestone`    | Manage milestones                   |
| `joy log`          | Change history                      |
| `joy ai`           | AI estimation, planning, dispatch   |
| `joy sync`         | Push/pull to remote                 |
| `joy clone`        | Clone a remote project              |
| `joy project`      | View/edit project info              |
| `joy serve`        | Start server for sync and web UI    |
| `joy app`          | Launch TUI                          |
| `joy completions`  | Generate shell completions          |

For developer documentation see [docs/dev/](../dev/).
