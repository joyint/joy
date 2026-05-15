---
name: joy
description: Joy product management assistant - use when the user asks about backlog, items, milestones, planning, or status tracking
---

# /joy - Joy product management assistant

This is the slash-command companion to Joy, the terminal-native and Git-native product management tool. The `joy` binary is installed and available.

## Before doing anything

Run `joy ai tutorial` if you have not already this session. It is the canonical operational guide and covers session start, authentication, item lifecycle, commit messages, capabilities and gates, and project conventions. Everything below assumes you have read it.

## What `/joy` adds on top

The slash command is a natural-language entry point: the user phrases what they want and you map it to the right `joy` command. Confirm before any write per your current interaction mode.

### Viewing and navigating

- "What's the backlog?" / "Show me the board" -> `joy ls` or `joy`
- "What's open?" -> `joy ls --status open`
- "Show me bugs" -> `joy ls --type bug`
- "What am I working on?" -> `joy ls --mine`
- "What's blocked?" -> `joy ls --blocked`
- "Show JI-0003" -> `joy show JI-0003`
- "Find login" -> `joy find login`
- "What's in the milestone?" -> `joy milestone show JI-MS-01`
- Summarize the output for the user in a readable way.

### Status changes

- "Start JI-0003" -> `joy start JI-0003`
- "Submit JI-0003 for review" -> `joy submit JI-0003`
- "Close JI-0003" -> `joy close JI-0003`
- "Reopen JI-0003" -> `joy reopen JI-0003`

### Editing and organizing

- "Change priority of JI-0003 to critical" -> `joy edit JI-0003 --priority critical`
- "Assign JI-0003 to me" -> `joy assign JI-0003`
- "Add a comment to JI-0003" -> `joy comment JI-0003 "..."`
- "JI-0003 depends on JI-0001" -> `joy deps JI-0003 --add JI-0001`
- "Link JI-0003 to JI-MS-01" -> `joy milestone link JI-0003 JI-MS-01`

### Planning and creating items

When the user describes features, ideas, problems, or requirements:

1. Break it down into items using the types `epic`, `story`, `task`, `bug`, `rework`, `decision`, `idea`.
2. Present a short numbered list (title, type, priority, effort, description) and ask if it looks right. Suggest an effort (1-7 or t-shirt size xxs/xs/s/m/l/xl/xxl) per item based on scope.
3. Create items one by one with `joy add --effort <N> --description "..."`. Ask "Create this item? (y/n/edit)" before each.
4. After all items are processed, run `joy ls` to show the result.

Do not over-decompose. Item rules (title length, language, audit-trail discipline) come from `joy ai tutorial` and `docs.contributing`.

### Questions and analysis

- Summarize progress toward a milestone.
- Identify risks (blocked items, unassigned critical items).
- Suggest what to work on next based on milestones, priorities, and dependencies.
