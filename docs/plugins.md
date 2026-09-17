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

## Forge connectors: the query contract (JOY-0251-AA, protocol 2)

A FORGE plugin - the connector - is a plugin like any other, with one
addition: besides printing node trees for humans it answers **typed
queries** that joy-core consumes. All forge knowledge (host names, alias
address formats, API access) lives in the connector; joy-core only
speaks this protocol. Every answer is exactly one JSON object on stdout,
except the one streaming verb described under "Two output modes".

### The protocol number and the `version` verb

The protocol has a number, and the first verb of every call sequence
asks for it:

    joy-forge github version
    {"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}

joy asks this **once per resolved file per process**, cached by the
file's path and its modification time, under the 5 s deadline class. It
is not asked per verb, so a 1 Hz poll costs no extra process.

A connector from before the number existed (protocol 1) is recognised
without its cooperation: its argument parser rejects the unknown
subcommand and exits **2 with usage on stderr and nothing on stdout**.
The rule is therefore: exit code 2 with empty stdout, or any answer that
does not parse as the object above, means protocol 1.

A protocol 1 connector still answers the six original verbs (`claims`,
`identity`, `resolve`, `store`, `files`, `release`) with `--remote`, so
an old machine keeps publishing releases. Asked anything else, joy does
not call it at all and reports the state `plugin_outdated` with the file
that answered and the fix in one sentence:

    the GitHub connector at /home/s/.cargo/bin/joy-github speaks protocol 1,
    this joy needs protocol 2. Install the new connector (`cargo install
    joy-cli` ships joy-forge), then remove the old one:
    rm /home/s/.cargo/bin/joy-github

joy never deletes a binary it did not install.

### How joy finds a connector

One binary, `joy-forge`, carries every forge; the legacy names
`joy-github`, `joy-gitlab` and `joy-gitea` stay as a fallback for
`cargo install` users until the deprecation window closes. The search
order is:

1. the directories the host registered at startup
   (`joy_core::forge_plugins::set_plugin_dirs`): the desktop registers
   the directory of its own executable, the CLI its install directory;
2. the directory of the current executable;
3. PATH.

**Inside every directory the name order is `joy-forge` first**, then the
legacy `joy-<forge>` name. That order is what makes a stale
`~/.cargo/bin/joy-github` harmless beside a fresh `joy-forge`: nothing is
deleted, the new name simply wins, and `joy forge plugins` prints the
`rm` line for the old one.

`JOY_PLUGIN_DIR` (a list of directories, separated like PATH) is
searched before all three. It exists as a documented **test hook** and
not as a product switch; no joy surface offers it.

When the combined binary answers, the forge id is its first argument
(`joy-forge github claims ...`). When a legacy binary answers, it is not
(`joy-github claims ...`).

### What every call carries

Besides its verb, every protocol 2 call carries:

- the target, either `--remote <url>` or `--host <hostname>`. Every verb
  accepts both, and the connector is invoked **without a project root**:
  `claims --host github.com` answers with nothing on disk;
- `--host-kind interactive|background|delegated`, who is behind the
  process. A `background` or `delegated` host has nobody who could
  answer a question, so the connector must skip any step that can raise
  an operating system dialog;
- `--login <name>` where the caller pinned a login;
- `--token-env <VAR>` where a token is handed over. The token itself
  travels in the child's environment under that name and never in the
  process list.

A protocol 1 connector sees none of these: `--remote` only, plus the
`--login` and `--user-id` that `identity` always took.

### Deadlines, output and cancellation

joy runs a connector with stdin closed and **both output pipes
captured**, and reads stdout **while** the child runs. A connector may
therefore answer with megabytes; an answer is never cut off at the pipe
buffer, and a connector's stderr reaches the caller's error message
instead of a terminal that may not exist.

Every verb has its own deadline:

| verb | deadline |
| --- | --- |
| `claims`, `identity`, `resolve`, `web-url`, `version` | 5 s |
| `store`, `files`, `repositories`, `create-repository`, `token`, `token-store`, `logout` | 30 s |
| `release` | 120 s |
| `login` | 15 s to the first event, then that event's `expires_in`, capped at 900 s |

When the deadline passes, or when the caller cancels, joy ends the
connector's **whole process group** (a job object on Windows), so a
`gh`, `glab`, `tea` or `curl` grandchild does not outlive the call.
A connector should therefore treat any step it starts as its own to
clean up, and should not detach one.

### Two output modes

Most verbs print exactly one JSON object and exit. A long running verb
prints **newline delimited JSON, one object per line, each flushed
explicitly**, and joy hands every line to its caller as it arrives. That
is what lets a person read a device code while the connector is still
polling the forge. `login` is the verb that uses it.

### The five outcomes a caller tells apart

`{"known": false}` (exit 0) is an ANSWER: the connector was asked and
had nothing. It is not a failure, and joy keeps it apart from the four
that are: `plugin_missing` (no file with any of the names anywhere in
the search order), `plugin_outdated` (a protocol 1 file was asked a
protocol 2 verb), `plugin_failed` (a non-zero exit, with the
connector's own stderr, or an answer that does not parse) and
`plugin_timed_out`. Each one carries the file that answered.

### The verbs

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

- `joy-<name> store --remote <url> [--token-env <VAR>]`
  (JP-013C-11) Does this repository hold a joy store, and may the
  caller create one? A multi-account host asks this instead of cloning.
  Answer, by `state`:
  `{"state": "store", "project_yaml": "..."}` when `.joy/project.yaml`
  on the default branch is readable (its content comes along);
  `{"state": "missing", "may_create": true|false, "default_branch": "..."}`
  when the repository is there without a store, with the caller's push
  permission and the branch the forge names as its default;
  `{"state": "gone"}` when the forge does not show the caller the
  repository (deleted or no access, which forges answer alike);
  `{"state": "unknown"}` when the forge could not be asked. Only the
  last is not a verdict. Without `--token-env` the forge is asked
  anonymously and sees public repositories only.
  One optional field, `"size_bytes": <integer>`, may travel with any
  state that saw the repository. It is normalised to BYTES inside the
  connector because the unit is forge knowledge (GitHub counts
  kilobytes, Gitea KiB, GitLab bytes and only for a caller with the
  right role). Its absence is not an error.

- `joy-<name> files --remote <url> [--token-env <VAR>]`
  (JAPP-0293-A7) Which files does the default branch carry? Answer:
  `{"state": "files", "paths": ["..."], "truncated": true|false}`, where
  `truncated` says the forge or the plugin's own page bound cut the
  listing off; an empty repository lists no paths. `{"state": "unknown"}`
  when the forge could not be asked.

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
  resolution must never fail because a connector is absent. The ANSWER
  degrades; the REASON does not: every failed call leaves one warn line
  naming the connector, the verb and the file, and the typed state
  above is available to every caller that wants to say more than
  "unknown".
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

### Forge connectors on a workstation

There is nothing to configure. `cargo install joy-cli` ships the
`joy-forge` connector beside `joy`, and the installers put both in the
same archive, so `joy update` keeps them in lockstep. Sign in with your
forge's own CLI (`gh auth login`, `glab auth login`, `tea login add`) as
you would anyway. From then on joy resolves alias addresses
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
