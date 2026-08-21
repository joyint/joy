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

## Forge plugins: the query contract (JOY-0251-AA)

A FORGE plugin (`joy-github`, `joy-gitlab`, `joy-gitea`, ...) is a plugin like any
other, with one addition: besides printing node trees for humans it
answers **typed queries** that joy-core consumes. All forge knowledge
(host names, alias address formats, API access) lives in the plugin;
joy-core only speaks this protocol. Every answer is exactly one JSON
object on stdout.

- `joy-<name> claims --remote <url>`
  Does this remote belong to your forge? Answer:
  `{"claims": true}` or `{"claims": false}`.
  joy-core asks this instead of ever parsing forge URLs itself.
  A forge product may own a domain (github.com, gitlab.com), and a
  plugin may claim it. NO INSTANCE belongs in a plugin's code: a
  GitHub Enterprise Server, a self-hosted GitLab and every Gitea or
  Forgejo run on their operator's own domain. A plugin recognizes
  those the honest way, by asking its own CLI which hosts this person
  is signed in to (gh's hosts.yml, glab's config.yml, tea's
  config.yml); an instance nobody is signed in to is reached through
  the project.yaml `forge:` override.

- `joy-<name> identity [--login <l> --user-id <id>] [--token-env <VAR>]`
  Who is ACTING on your forge? Answer: `{"known": false}` — or
  `{"known": true, "login": "...", "user_id": "...", "emails": ["..."]}`
  where `emails` are the verified addresses the plugin can vouch for
  (possibly empty when the source cannot list them).
  Locally the plugin finds its own facts (e.g. the forge CLI's config);
  a multi-account host (the platform) hands the caller's facts in via
  the flags, `--token-env` naming an environment variable so a token
  never appears in the process list.

- `joy-<name> resolve --email <addr>`
  Whose address is this? PURE: the answer derives from the address
  alone (e.g. an alias form encodes login and account id), never from
  ambient state — an address the plugin cannot attribute is
  `{"known": false}`, even when someone is signed in locally. Answer
  shape as above, `emails` usually empty.

- `joy-<name> release --tag <t> --title <t> --notes-file <path>`
  (JOY-0256-64) Create — or complete — the release for this tag on
  your forge; the notes arrive as a file because they are multi-line.
  Answer: `{"url": "..."}` on success, or `{"unsupported": true}` when
  the forge has no release backend yet (joy then keeps its tag-only
  publish). This is the contract's ONE write verb, and unlike the read
  queries it reports failure: the reason goes to stderr, the exit code
  is non-zero, and `joy release publish` fails with it. Idempotence is
  the plugin's duty: a release that already exists (a tag-triggered
  forge workflow may have made it) keeps its URL and gets the notes
  prepended exactly once (JOY-0248-AE).

Rules, in addition to the base contract:

- **Best effort, never blocking**: a missing binary, a timeout, or an
  error answer degrade to "no claim / unknown" in the caller. Identity
  resolution must never fail because a plugin is absent.
- **Read-only and side-effect free** — except the explicit `release`
  verb, whose one side effect is the release it names.
- **No forge knowledge outside the plugin**: joy-core selects the
  responsible plugin purely by asking `claims` over the project's
  remotes (the registry in `joy_core::forge_plugins` lists the known
  plugin names; `project.yaml`'s `forge:` stays the operator override).
- A project without remotes, without installed forge plugins, or whose
  remotes nobody claims behaves exactly as if this contract did not
  exist.

## Making a plugin available

Install the binary on the PATH (`cargo install joy-bi`, `cargo install
--path crates/joy-bi`, or your package manager). The Joyint app
discovers `/name` commands by probing `joy-<name>` and renders the node
tree with charts; the CLI story (`joy <name> ...` passthrough) is
tracked separately.

### Forge plugins on a workstation

There is nothing to configure. Install the plugin for your forge
(`cargo install joy-github`, `joy-gitlab`, `joy-gitea`) and sign in with
that forge's own CLI (`gh auth login`, `glab auth login`, `tea login
add`) as you would anyway. From then on joy resolves alias addresses
through it, and a project on a host you are signed in to is recognized
on its own. No environment variable, no token in joy's hands: the
plugin reads the CLI's configuration and asks the API with it.

One lever exists, per project rather than per machine: when a project
lives on an instance nobody is signed in to locally (a GitHub
Enterprise Server, a self-hosted GitLab, any Gitea or Forgejo), name
its forge once and the right plugin answers for it:

    joy project set forge gitea

A server has neither a forge CLI nor a person in front of it, so it is
told the same facts through its own configuration instead; the platform
ships them as environment variables (see its `.env.example`), and hands
the caller's login and token to the plugin per call.
