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
  Guard, event log, and audit trail apply. The FORGE connectors are the
  named exception and a class of their own: they authenticate, they
  hold state, and two of their verbs write on the forge. Their section
  below says which, and what each one changes.

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

A FORGE plugin (the connector) is a plugin like any other, with one
addition: besides printing node trees for humans it answers **typed
queries** that joy-core consumes. All forge knowledge (host names, alias
address formats, API access) lives in the connector; joy-core only
speaks this protocol. Every answer is exactly one JSON object on stdout,
except the one streaming verb described under "Two output modes".

### The protocol number and the `version` verb

The protocol has a number, and the first verb of every call sequence
asks for it. The question is about the BINARY and not about one forge
inside it, so no forge id goes in front of it, and the answer names
every forge the file carries:

    joy-forge version
    {"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}

Every legacy `joy-<forge> version` answers the same object for its own
one forge.

joy asks this **once per resolved file per process**, cached by the
file's canonical path and its modification time, under the 5 s deadline
class. It is not asked per verb, so a 1 Hz poll costs no extra process,
and one `joy-forge` is asked once however many registry rows resolve to
it. A handshake that fails (the file cannot be started, or it does not
answer in time) is remembered for a minute, so a connector whose
`version` hangs does not cost a fresh 5 s spawn per verb.

A connector from before the number existed (protocol 1) is recognised
without its cooperation: its argument parser rejects the unknown
subcommand and exits **2 with usage on stderr and nothing on stdout**.
The rule is therefore: exit code 2 with empty stdout, or any answer that
does not parse as the object above, means protocol 1.

A protocol 1 connector is still asked the six original verbs (`claims`,
`identity`, `resolve`, `store`, `files`, `release`) about a REMOTE
target, so an old machine keeps publishing releases. A `--remote`
argument travels with three of the six, the three whose protocol 1
parser knows one: `claims`, `store` and `files`. `release` never gets
one, because its old parser would exit 2 on it and the publish would
end there. Asked anything else, joy does not call it at all and reports
the state `plugin_outdated` with the file that answered and the fix in
one sentence:

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
searched before all three, **in a development build only**. It exists as
a documented **test hook** and not as a product switch: no joy surface
offers it, and a released joy does not read it at all, so no line in a
person's shell profile can redirect a connector call, and with it the
forge token that call carries, to another executable.

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

joy ends the connector's **whole process group** (a job object on
Windows) on every path: when the deadline passes, when the caller
cancels, AND when the connector itself exits normally. The last one is
not politeness. A grandchild inherited the connector's stdout, and a
pipe reaches end of file only when every write end is closed, so a
`gh`, `glab`, `tea` or `curl` left running would hold the call open long
after the connector answered and the deadline would bound nothing.
A connector should therefore treat any step it starts as its own to
clean up, and should not detach one.

### Two output modes

Most verbs print exactly one JSON object and exit. A long running verb
prints **newline delimited JSON, one object per line, each flushed
explicitly**, and joy hands every line to its caller as it arrives. That
is what lets a person read a device code while the connector is still
polling the forge. `login` is the verb that uses it.

### What a connector reads from your machine, and how it reaches the forge

A connector speaks HTTP itself since JOY-0298-E4: no `curl` and no `gh`
sits on any API path. Two consequences a person or an operator can act
on:

**Proxies.** The connector honours the same sources the engine does:
`http.<url>.proxy` from git config (most specific URL first), then
`http.proxy`, then `https_proxy` / `HTTPS_PROXY` (an `http://` target
uses `http_proxy` / `HTTP_PROXY`), then `ALL_PROXY` / `all_proxy`.
`NO_PROXY` is evaluated by joy for every one of them, including the
ones from git config, and its entries are trimmed, so
`NO_PROXY="a.com, b.com"` really does cover `b.com`. A SOCKS proxy is
refused by name: "joy cannot use the SOCKS proxy <url>; it supports
HTTP and HTTPS proxies only." A proxy password never appears in a log
line or an error text.

**The trust store.** The operating system's own: the Windows
certificate store, the macOS system anchors plus the Keychain trust
settings, and on Linux the OpenSSL style default paths, which is what
`update-ca-certificates` and `SSL_CERT_FILE` write. A corporate CA
installed the normal way is trusted with no joy setting. **On Linux
only**, `ca_bundle` and `ca_dir` in `forges.yaml` and `http.sslCAInfo`
and `http.sslCAPath` in git config are honoured as well; on macOS and
Windows they are refused with the sentence that names the system store,
because the engine cannot honour them there either. joy never turns
certificate verification off and does no client certificate.

### Instances an operator configures: `forges.yaml`

A self hosted forge used to be reachable only when gh, glab or tea was
already signed in to it, which makes the sign in door circular for an
enterprise: nobody can sign in through joy because joy does not claim
the host, and joy does not claim the host because nobody signed in.
An operator cuts that circle with one file, in joy's own configuration
directory (`$XDG_CONFIG_HOME/joy/forges.yaml`, `~/.config/joy/forges.yaml`,
`~/Library/Application Support/joy/forges.yaml` on macOS,
`%APPDATA%\joy\forges.yaml` on Windows):

```yaml
- host: git.acme.test
  kind: github            # github | gitlab | gitea
  api_base: https://git.acme.test/api/v3
  web_base: https://git.acme.test
  client_id: Iv1.the-app-registered-on-this-instance
  device_endpoint: https://git.acme.test/login/device/code
  auth_endpoint: https://git.acme.test/login/oauth/authorize
  token_endpoint: https://git.acme.test/login/oauth/access_token
  scopes: repo user:email
  ca_bundle: /etc/pki/tls/certs/acme-root.pem   # Linux only
```

`claims` consults it, so the internal forge is claimed with no forge
CLI installed at all, and every verb asks the `api_base` named there.
The project level `forge:` override keeps working and wins for that
project.

**The OAuth client id is configuration, not code.** An instance signs
in through the application its own operator registered, which is what
`client_id` and the three endpoints carry; only the defaults for
github.com, gitlab.com and codeberg.org are built in, and those are
still PLACEHOLDERS marked `REPLACE-ME` until joy's public clients are
registered. Where a host has no usable client id, `login` says so in
one sentence and names the two ways forward: store a token with
`--token-stdin`, or put a `client_id` for that host into this file.
Nothing is sent to a forge in the meantime.

### Where a token comes from

joy never reads another CLI's credential store. A foreign credential is
obtained by SPAWNING the CLI, which is also the only way its own
refresh runs, and it is read only for joy:

| forge | command |
| --- | --- |
| GitHub | `gh auth token --hostname <host> [--user <login>]` |
| GitLab | `glab auth credential-helper get` (`glab auth token` does not exist) |
| Gitea, Forgejo | `tea login helper get` (`tea logins list` prints no token) |

A caller that has a token of its own hands it over by NAME
(`--token-env VAR`); the value travels in the child's environment and
reaches the forge in an `Authorization` header. It is never an argument,
so no process list can carry it.

A named variable is the whole answer for that call: when it holds
nothing, the call has no credential, and nothing else is consulted. A
multi-account host must never act as the machine's own account because
its variable happened to be empty.

Where no caller named a variable, the connector looks in its OWN entry
first (the one `login` and `token-store` wrote, see "The connector's own
credential" below), then at the forge's own variables, which is what a
person or a CI runner exports anyway, and only then does it spawn the
forge CLI. The `source` field of the `token` answer says which of the
six it was: `keychain`, `file`, `gh`, `glab`, `tea` or `env`.

| forge | variables | read for |
| --- | --- | --- |
| GitHub | `GH_TOKEN`, `GITHUB_TOKEN` | github.com only |
| GitHub Enterprise | `GH_ENTERPRISE_TOKEN`, `GITHUB_ENTERPRISE_TOKEN` | every other host |
| GitLab | `GITLAB_TOKEN` | every host |
| Gitea, Forgejo | `GITEA_TOKEN` | every host |

The GitHub split is gh's own: a github.com token is never sent to
somebody's Enterprise Server, and an Enterprise token never to
github.com.

The configuration of those three CLIs is looked for per operating
system: gh in `GH_CONFIG_DIR`, `XDG_CONFIG_HOME/gh`, `%AppData%\GitHub CLI`
and `~/.config/gh`; glab in `GLAB_CONFIG_DIR`, `~/.config/glab-cli` and
the XDG directory of the platform (including `%LOCALAPPDATA%\glab-cli`);
tea in the XDG directory of the platform and then `~/.tea/tea.yml`.

### The connector's own credential

The connector binary owns one entry per host and login, and it is the
only process that touches it: the app and the CLI call this same binary
for `login`, `token`, `token-store` and `logout` instead of reading a
store themselves. On macOS that is the only arrangement without a
confirmation dialog, because a generic password is created with an ACL
that trusts the creating application alone.

- The entry is addressed with `Entry::new(service, user)` and nothing
  else: the service is `joy-forge`, the user is `<host>` or
  `<host>|<login>`. One more entry exists beside the credentials, under
  the service `joy-forge.logins`: `Entry::new` cannot enumerate a store,
  and "the only login this host holds" and the probe order below both
  need the list, so the list of logins per host is an entry of its own.
  It holds names, never a token.
- Where the operating system's credential store cannot answer, the
  connector writes the entry itself, to `<config>/forge-tokens.json`,
  mode 0600 in a 0700 directory, and says `"source": "file"` rather
  than pretending. "Cannot answer" is detected and not guessed: after a
  write the entry is read back through a NEW handle, and a store that
  cannot answer with what was just written did not persist it. That is the honest case on a headless server, in a
  container and on any machine whose store refuses.
- The granted scope set is stored beside the token in the same entry,
  space separated, and is what the local `scope_missing` check below
  reads.
- A token past its lifetime is refreshed under a cross process lock,
  one per host and login, in `<app state>/locks/`. The waiter never
  refreshes anyway: it looks again and, finding nothing usable,
  answers `{"known": false, "reason": "busy"}`. Every call that may
  write takes it: `token`, `login`, `token-store` and `logout`.
- The 0600 file is ONE document for every host and login, while that
  lock is per host and login, so a read-modify-write of the file takes
  one more whole file lock beside the document, and the document is
  written to a staging file and renamed into place. Neither is
  decoration: without the first, two logins of one host signing in at
  once lose an entry; without the second, a crash mid write leaves a
  file that does not parse, which reads as "nothing is stored".
- A `--host-kind delegated` call never opens this person's credential
  store at all. Its credential travels in the variable the caller named
  (`--token-env`), and the interactive verbs are refused there anyway.

#### Where the credential really lies, per operating system

"Never on disk in joy's hands" was the old sentence here, and it was
never the whole truth: the operating system's store IS a file, with the
operating system's rules on it. What those rules are is worth knowing
before a token is put there, so they are written out rather than
implied. The connector asks the `keyring` crate for four store
features, with the crate's own defaults off: `apple-native`,
`windows-native`, `linux-native-sync-persistent` and `crypto-rust`. That
choice is what each line below describes. A fifth feature, `vendored`,
is asked for on top and names no store at all; it decides how the Linux
one is reached, by linking libdbus into the binary so that a minimal
container has no `libdbus-1.so` to install first.

| Operating system | Where the token lies | Who can read it | What it survives |
| --- | --- | --- | --- |
| Windows | Credential Manager, a generic credential | any process of this logon session, with no prompt | a reboot; the entry is written `CRED_PERSIST_ENTERPRISE`, so it roams with a roaming profile |
| macOS | the login keychain, a generic password | the binary that created it, with no prompt; another binary raises the allow-or-deny dialog | a reboot, and the keychain's own lock state decides when it can be read |
| Linux, session bus present | the Secret Service collection (GNOME Keyring, KWallet), with kernel keyutils kept in front of it as an in memory cache | any process of this session that can reach the bus | a reboot, provided the collection is unlocked again; the keyutils half never does |
| Linux, no session bus or locked collection | `<config>/forge-tokens.json`, mode 0600 in a 0700 directory | this user, and root | a reboot |
| Anywhere the store refuses | the same 0600 file | this user, and root | a reboot |

Two consequences a person can act on. On Windows and on Linux with a
session bus, any other program you run is on the same side of the door
as joy is: the store protects the token from other USERS and from a
stolen disk, not from software you started yourself. And the 0600 file
is a real outcome, not a bug report: a headless server, a container and
a desktop whose keyring nobody unlocked all land there, the `source`
field of every answer says `file` when they do, and `joy forge status`
prints it.

The `linux-native` feature alone would be the kernel keyutils store,
which is "completely in-memory and will not persist across reboots"
(keyring 3.6.3, `src/keyutils.rs`). That is why the persistent Secret
Service collection is asked for beside it, and why the desktop app asks
for the same four store features: a machine should have one answer about
where a secret lives, not one per binary. The app leaves `vendored` off,
because it is a GTK and WebKit application that links `libdbus-1.so.3`
on every Linux it can run on at all.

Which login answers for a remote is decided in one order, and every
answer says which step decided (`chose_by`): the device local pin for
that host in that project (`forgeLogin` in the project's app state
file, never in the committed `project.yaml`), then the login the
memory recorded for this remote, then the only login the host holds,
then one probe per candidate (one REST call for `owner/repo`), and
otherwise `{"known": false, "reason": "no-login-for-repo"}`. The same
order runs inside every verb that needs a credential, not only inside
`token`, and one remote is probed at most once per call.

Two rules keep that order honest. The memory records only a login the
forge reported as able to PUSH, because that is what the memory means
and because a later push must not take a read-only login from it for
free. And a forge that could not be ASKED is never reported as "none of
your logins can reach this repository": a transport failure is evidence
about the network and none about any login, so the memory of that
remote stays where it is and the answer says the forge could not be
asked.

### Signing in from the CLI: `joy forge`

The CLI's door to all of this is one command group, and every sentence
in joy that says "sign in to the forge" points at it:

    joy forge login [--host <host>] [--token-stdin] [--for read|write|create|release] [--login <name>]
    joy forge status [--host <host>]
    joy forge logout [--host <host> | --all]
    joy forge plugins

Without `--host` the host comes from this project's remote, the one joy
really contacts (`origin`, or the first configured one). There is no
`--remote <url>` here; that option belongs to the connector protocol.

`login` runs the connector's `login` verb through the streaming runner
and prints the verification URL and the code on stderr while the
connector polls; joy never opens a browser. `--token-stdin` reads ONE
line from stdin instead, hands it to `token-store`, which validates it
before storing, and refuses an empty line. The token is never an
argument, so no process list can carry it, and there is no
`--token <value>`.

`status` prints one row per host: the host, its forge, the login, the
state, where the credential came from, the granted scopes, the expiry,
and which binary answered with its path and protocol. The host set is
the hosts of `forges.yaml`, the hosts of this project's remotes and the
hosts joy's own credential file holds; a credential that lives in the
operating system's store alone cannot be enumerated, so such a host is
shown when `forges.yaml`, a remote or `--host` names it. `status` exits
1 when no host in its set is signed in, and `login` exits 1 on every
state other than `signed-in`.

`logout` calls the connector's `logout`. A credential that came from
gh, glab or tea is removed by nobody but that CLI, so joy removes
nothing and prints the foreign command instead.

`plugins` is the diagnostic, and it contacts no forge: one row per
registry id with the file that answers, the search step that found it,
its protocol and version, a `problem` word (`shadowed-legacy`,
`plugin_outdated`, `plugin_missing`) and the `rm` line for a stale
binary beside the fresh one.

With `--json` each of the four answers is exactly one envelope
`{"version":1,"data":{...}}` on stdout, and every diagnostic stays on
stderr.

### The verbs that need a person, and the builds that do not have them

`login`, `logout` and `token-store` live behind joy-core's `interactive`
cargo feature, which is OFF by default. joy-cli turns it on, because the
CLI is what a person types at. The platform asks for `forge-net` alone,
so the module is not in the server binary at all.

The desktop carries it too (design D3.11), because that is the build
with a window in front of a person: its manifest
(`app/apps/desktop/src-tauri/Cargo.toml`) asks for `ts`, `forge-net` and
`interactive`. That manifest lives in the app repository, so no guard in
THIS repository can see it; the app's own build is what would notice if
the line went away, and the platform's guard is what keeps the feature
out of the server.

Two more layers sit behind the feature:

- the call takes a progress sink and a cancel token, both mandatory, so
  nothing starts a fifteen minute browser flow by accident;
- a build that does carry the feature still refuses `login` when the
  host kind is `background` or `delegated`, instantly and by name: the
  agent image builds joy-cli from source, so a delegated agent has
  `joy forge login` on its PATH and has to be told to sign in on the
  machine that owns the session, or to store a token there with
  `--token-stdin`.

`just guard-interactive` checks what is in reach of this repository:
that joy-core leaves the feature off, that joy-cli is the only crate
here that asks for it, and, where the platform is checked out beside
joy, that
`cargo tree -e features -i joy-core --manifest-path ../platform/Cargo.toml`
carries no `interactive` node. The platform's own pipeline runs that
last check where the platform really is; the recipe says which half it
was able to do.

### Scopes, and the `scope_missing` answer

Every connector knows which scope set each verb group needs, and
answers locally instead of spending a request the forge would refuse:

```
{"state":"scope_missing","host":"gitlab.com","verb":"create-repository",
 "needed":["api"],"have":["read_api","write_repository"],
 "next":"sign in again with wider access"}
```

on exit code 0. It is an ANSWER, not a failure, and a host renders it
as one sentence with one button. The sets are per forge: GitHub covers
everything with `repo user:email` (there is no read only private scope
there); GitLab has three, `read_api read_repository` for a read only
member, `read_api write_repository` for a read write member and
`api write_repository` for everything, because `write_repository`
"Uses Git-over-HTTP. Does not support API authentication."; Gitea and
Forgejo scope per category, `read:user read:repository` to read,
`write:repository` to write and `write:user write:repository` to create
a repository.

Where the local check passes and the forge still refuses, the refusal
is classified from its headers and never from prose, and a scope
problem is never reported as "denied".

### The five outcomes a caller tells apart

`{"known": false}` (exit 0) is an ANSWER: the connector was asked and
had nothing. It is not a failure, and joy keeps it apart from the four
that are: `plugin_missing` (no file with any of the names anywhere in
the search order), `plugin_outdated` (a protocol 1 file was asked a
protocol 2 verb), `plugin_failed` (a non-zero exit, with the
connector's own stderr, or an answer that does not parse) and
`plugin_timed_out` (with whatever the connector wrote on stderr before
it hung). Each one carries the file that answered.

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
  repository (deleted or no access, which forges answer alike) AND the
  credential could have seen a private one: GitHub and GitLab both
  answer 404 rather than 403 for a private repository, so a 404 an
  anonymous caller or a narrow token got is `unknown`, never `gone`;
  `{"state": "unknown"}` when the forge could not be asked. Only the
  last is not a verdict. Without `--token-env` the forge is asked
  anonymously and sees public repositories only.
  One optional field, `"size_bytes": <integer>`, may travel with any
  state that saw the repository. It is normalised to BYTES inside the
  connector because the unit is forge knowledge (GitHub counts
  kilobytes, Gitea KiB, GitLab bytes and only for a caller with the
  right role). Its absence is not an error: the connector reads the
  repository record for it, and where the forge will not show that
  record the answer simply carries no size.

- `joy-<name> files --remote <url> [--token-env <VAR>]`
  (JAPP-0293-A7) Which files does the default branch carry? Answer:
  `{"state": "files", "paths": ["..."], "truncated": true|false}`, where
  `truncated` says the forge or the plugin's own page bound cut the
  listing off; an empty repository lists no paths. `{"state": "unknown"}`
  when the forge could not be asked.

- `joy-<name> repositories --host <h> [--query s] [--limit n] [--page cursor]`
  Which repositories can this account reach? Answer:
  `{"state": "repositories", "repositories": [...], "truncated": bool,
  "next": null|"cursor"}`. The default limit is 200, and the answer is
  paginated so one of them stays well under 64 KiB. Each row carries
  `full_name`, `name`, `private`, `clone_url`, `ssh_url`,
  `default_branch` and `web_url`. Without a credential the answer is
  `{"state": "needs_sign_in", "host": "..."}`: there is no account
  whose repositories could be listed.

- `joy-<name> create-repository --host <h> --name <n> [--owner <o>] [--private]`
  Create a repository. Answer: `{"created": true, "clone_url": ...,
  "ssh_url": ..., "default_branch": ..., "web_url": ...}`, or an error
  object carrying `state` and `message`. A project can only be brought
  to joyint.com when it has a remote repository, so this verb is what
  makes "picks or creates a repo" complete.

- `joy-<name> web-url --remote <url>`
  The https twin of this remote. Answer:
  `{"known": true, "https_url": "https://git.acme.com/team/sub/repo.git"}`
  or `{"known": false}`. Only the connector knows the web base of a
  self hosted instance, which may sit under a nested sub path or behind
  a different ssh domain. No credential and no request.

- `joy-<name> token --remote <url> | --host <h> [--for read|write|create|release] [--login <name>]`
  The credential this machine holds, and which login it belongs to.
  Answer:
  `{"known":true,"host":"github.com","login":"scotty","token":"gho_...",`
  `"username":"x-access-token","source":"keychain|file|gh|glab|tea|env",`
  `"scopes":"repo user:email","expires_at":null,"chose_by":"pin|memory|only|probe"}`,
  or `{"known":false,"reason":"no-login|no-keychain|unsupported-host|no-login-for-repo|busy"}`.
  `--for` is the DIRECTION the credential is wanted for, and it decides
  what the probe accepts: `write`, `create` and `release` take only a
  login the forge reports as able to push, `read` takes the first that
  sees the repository, and without the flag a login that can push wins
  over one that can only read but a reader is still an answer. Without
  it a login with read-only rights answers 200 for a private repository,
  wins the probe, and the push then fails under the wrong account.

- `joy-<name> token-store --host <h> [--login <name>]`
  Read ONE token from **stdin**, validate it against the instance's own
  API, store it, and answer the same object as `token`. This is the
  headless door: a Linux server, a CI runner, a Windows host with no
  browser, and every Gitea family instance whose operator registered no
  OAuth client. The token is never an argument, in either direction.

- `joy-<name> login --remote <url> | --host <h> [--for read|write|create|release] [--login <name>]`
  Sign in. Newline delimited JSON on stdout, one object per line, each
  flushed as it happens:
  `{"event":"verification","host":...,"url":...,"url_complete":...,"code":...,"expires_in":...,"interval":...}`,
  then `{"event":"waiting","seconds_left":...}`,
  `{"event":"slow_down","interval":...}`, and finally one
  `{"event":"result","known":true,"login":...,"user_id":...,"emails":[...],"scopes":...,"stored":"keychain|file","expires_at":...}`
  or
  `{"event":"error","code":"access_denied|expired_token|unsupported|network|device_flow_disabled|invalid_scope","message":"..."}`.
  **The connector never opens a browser and never prints the token.**
  The host decides: the desktop opens the URL, the CLI prints it, and a
  `--host-kind background` or `delegated` call is refused at once with
  the sentence that names `--token-stdin`.

- `joy-<name> logout --host <h> [--login <name>]`
  Answer: `{"removed":bool,"revoked":bool,"source":"keychain|file|gh|glab|tea"}`.
  The token is revoked at the forge where the forge offers it
  (`DELETE /applications/{client_id}/token` on GitHub, never
  `.../grant`, which would kill every token of that app for the
  person). A credential a forge CLI owns is not joy's to remove: the
  answer names the foreign command in `command` and removes nothing.
  This verb WRITES, so it takes the same refresh lock every other
  writing verb takes; a call that cannot take it answers
  `"removed": false` with `"reason": "busy"` rather than deleting an
  entry another process is renewing. `"removed": true` means the entry
  is gone: where the store or the file refused the write, the answer
  says `false` and carries the reason in `message`. On a host that
  holds several of joy's own logins with none of them named, nothing is
  removed and the answer names them in `logins` and asks for `--login`.

- `joy-<name> release --remote <url> --tag <t> --title <t> --notes-file <path>`
  (JOY-0256-64) Create — or complete — the release for this tag on
  your forge; the notes arrive as a file because they are multi-line.
  The remote names the repository: gh used to read that out of the
  working directory, and the connector's own REST call has to be told.
  Answer: `{"url": "..."}` on success, or `{"unsupported": true}` when
  the forge has no release backend yet (joy then keeps its tag-only
  publish). This is the contract's ONE write verb, and unlike the read
  queries it reports failure: the reason goes to stderr, the exit code
  is non-zero, and `joy release publish` fails with it. Idempotence is
  the plugin's duty: a release that already exists (a tag-triggered
  forge workflow may have made it) keeps its URL and gets the notes
  prepended exactly once (JOY-0248-AE). The verb carries NOTES and no
  assets: no argument names one, and joy's own publish never uploaded
  one. The asset upload (on uploads.github.com, from the release's own
  `upload_url`) lands with the argument that carries it.

Rules, in addition to the base contract:

- **Best effort, never blocking**: a missing binary, a timeout, or an
  error answer degrade to "no claim / unknown" in the caller. Identity
  resolution must never fail because a connector is absent. The ANSWER
  degrades; the REASON does not: every failed call leaves one warn line
  naming the connector, the verb and the file, and the typed state
  above is available to every caller that wants to say more than
  "unknown". That includes the two failures that start no process at
  all, a connector nobody installed and a connector that speaks
  protocol 1, which are the two a silent "unknown" hides best.
- **Read-only and side-effect free**, except the verbs that name their
  own side effect: `release` and `create-repository` on the forge, and
  `login`, `token-store` and `logout` on this machine's credential
  store.
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
same archive, so `joy update` keeps them in lockstep. Sign in with joy
itself:

    joy forge login --host github.com

or keep using your forge's own CLI (`gh auth login`, `glab auth login`,
`tea login add`) as you would anyway; joy reads either. From then on joy
resolves alias addresses through it, and a project on a host you are
signed in to is recognized on its own. No environment variable: the
connector reads the CLI's configuration, asks that CLI for a token when
it needs one, and speaks to the forge itself. `joy forge status` says
which of the two answered for a host, and `joy forge plugins` says which
binary answered at all.

Two levers exist. Per project, when a project lives on an instance
nobody is signed in to locally (a GitHub Enterprise Server, a
self-hosted GitLab, any Gitea or Forgejo), name its forge once and the
right connector answers for it:

    joy project set forge gitea

Per machine, an operator ships `forges.yaml` (above), and then the
instance is claimed and asked with no forge CLI installed at all.

A server has neither a forge CLI nor a person in front of it, so it is
told the same facts through its own configuration instead; the platform
ships them as environment variables (see its `.env.example`), and hands
the caller's login and token to the connector per call.

### The size of the connector

Decision 6 of the design (one binary rather than three) rests on a
measurement, and the measurement is kept here so it can be checked
rather than assumed. It is to be revisited when any release target
passes 10 MB.

| target | `joy-forge`, release, stripped | measured |
| --- | --- | --- |
| x86_64-unknown-linux-gnu | 5.38 MB (5 642 200 bytes) | 2026-09-17 |
| aarch64-unknown-linux-gnu | open, the release build measures it | |
| x86_64-apple-darwin | open, the release build measures it | |
| aarch64-apple-darwin | open, the release build measures it | |
| x86_64-pc-windows-msvc | open, the release build measures it | |

The four open rows need a cross linker the development host does not
have; the release workflow builds every target anyway and is where they
are filled in.

How to reproduce one row:

    cargo build --release --bin joy-forge
    strip -s target/release/joy-forge -o /tmp/joy-forge.stripped
    ls -l /tmp/joy-forge.stripped

The credential store of the sign in verbs is what grew the Linux row
from 3.79 MB to 5.38 MB: the keyring crate with the persistent Secret
Service backing of D2.6, and libdbus vendored so the connector keeps
linking nothing but libc, the way joy already vendors libgit2 and
OpenSSL. The engine is not in there: the connector reaches joy-core for
the file lock and the app state paths, and the linker drops the rest,
libgit2 included.

For comparison, the three separate protocol 1 plugins were 3.3 MB
stripped together, of which about 3 MB was a duplicated std, clap and
serde floor; the connector now carries rustls, its own HTTP stack and
the three forges in one file for 3.79 MB.

Said plainly, because it is the operator's decision and not the
measurement's: decision 6 was argued with "three plugins cost 3.3 MB
and one binary is about 1.2 MB", and that half of it did not survive
contact with the build. The consolidation did not shrink the total, it
grew it by about half a megabyte, because the three old plugins shelled out to
curl and gh while this one brings its own TLS stack. What the decision
still buys is what the rest of it named: one archive, one installer
change, one receipt, one sidecar per platform, and no ambiguity in the
winget `installers-regex`. The 10 MB revisit threshold is untouched and
far away; the premise is what changed.

`joy` itself grows by nothing worth measuring: `axoupdater`, which the
self-update path already needs, brings `ureq` and `rustls` into that
binary anyway.
