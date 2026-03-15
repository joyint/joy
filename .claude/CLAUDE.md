# CLAUDE.md

This file provides context for Claude Code sessions working on joy.git standalone. For full ecosystem context, use the umbrella repo [joyint/project](https://github.com/joyint/project).

## Project

Joy is a terminal-native product management tool. Binary: `joy`. Package: `joyint`. Repo: [joyint/joy](https://github.com/joyint/joy).

## Required Reading

- `docs/dev/vision/` -- product vision (README.md, joy.md, jot.md, joyint-com.md)
- `docs/dev/Architecture.md` -- tech stack, repo structure, security, ADRs
- `docs/dev/CONTRIBUTING.md` -- coding conventions, testing, CI/CD, commit messages

## Backlog

The backlog lives in `.joy/items/`. Use the `joy` CLI -- do not edit YAML files directly. Run `joy` for a board overview, `joy ls` to list items, `joy show <ID>` for details.

## Rules

- Do not reference Claude, Anthropic, or AI assistants in code comments, git commits, documentation, or any generated content. No exceptions.
- No emoji in documentation, commit messages, or code comments
- Use Mermaid for all diagrams, never ASCII art
- IDs use the project acronym as prefix (JOY-0001, JOY-MS-01)
- All item titles, descriptions, and comments in English
- When implementing a backlog item: comment planned solution first, then `joy start <ID>` before coding, `joy close <ID>` after committing
- Use "todo" (not "task") for checklist items inside descriptions
