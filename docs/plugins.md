# Joy Plugins

A Joy plugin is an executable named `joy-<name>` on the PATH. Callers (the
Joyint app's `/name` commands, and shells) run it inside a Joy project;
the plugin computes and prints **one JSON node tree on stdout**. That is
the whole contract.

## The contract

- **Invocation**: `joy-<name> [args...]`, working directory inside the
  project (honor `-w/--working-dir` like joy itself for parity).
- **Output**: exactly one JoyNode as JSON on stdout. Nothing else may go
  to stdout; logs and diagnostics belong on stderr.
- **Errors**: message on stderr, non-zero exit (2 for "no project").
- **Reads, no writes**: plugins compute over the project (joy-core or the
  files). Anything that mutates the project goes through `joy` itself so
  Guard, event log, and audit trail apply.

## The node tree

The canonical shapes live in `crates/joy-bi/src/nodes.rs` (Rust) and are
mirrored for the app in `@joyint/plugin-schema`. Kinds:

| kind    | fields                               | purpose                    |
| ------- | ------------------------------------ | -------------------------- |
| `value` | `label?`, `value`, `unit?`, `view?`  | one KPI                    |
| `table` | `label?`, `columns`, `rows`, `view?` | tabular data               |
| `list`  | `label?`, `items`, `view?`           | flat scalars               |
| `text`  | `text`                               | prose                      |
| `group` | `label?`, `children`, `view?`        | structure (recursive)      |

`view` is a rendering hint (`bar`, `pie`, ...); consumers may ignore it.
Scalars are string, number, boolean, or null. Field names and `kind` tags
are the wire contract: renaming them is a breaking change and moves the
schema major (see `@joyint/plugin-schema`, "Versioning").

## The reference implementation

`joy-bi` (this repo, `crates/joy-bi`) is the reference plugin:

```sh
joy-bi milestone JAPP-MS-01   # progress, status/type breakdown, effort
joy-bi velocity 2w            # closed items per bucket; h, d, w, m
```

Read its `report.rs` for the intended shape of a report and its tests for
how to test against a temp project.

## Making a plugin available

Install the binary on the PATH (`cargo install --path crates/joy-bi` or
your package manager). The Joyint app discovers `/name` commands by
probing `joy-<name>` and renders the node tree with charts; the CLI story
(`joy <name> ...` passthrough) is tracked separately.
