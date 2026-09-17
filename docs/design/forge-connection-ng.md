# Forge connection NG: adjusted design (2026-09-17, v5)

Status: v4 replaces v3 after the second critic. v3 replaced v2 (/tmp/appcheck/research/verify-synthesis.md) and folded in the first critic's findings on v2 (10 contradictions, 14 missing topics, 10 unverified load bearing points, 7 journey breaks) and the eight new verdicts R1 to R8. v4 keeps the design body and its section numbers and repairs the plan around it: one owner for the single `certificate_check` slot, one budget table with its milliseconds derived from it, a decided Codeberg poll period, a wave layout whose dependencies hold (J5 and P1a in wave 0, J4h, J4p and P1b in wave 1), the release verb's credential source named per wave, the enumerated flag and feature list of decision 29, decision 30 for the organisation test estate, package J11 for the CLI identity call sites, and four journey grades corrected to partly. Source paths are repo relative to `/home/horst/Work/Joyint/project/.claude/worktrees/forge-connection-ng` unless they name a crate registry version (read under `/home/horst/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`) or a vendored libgit2 path inside `libgit2-sys-0.18.7+1.9.6/libgit2/`.

Binding constraints, unchanged and honoured throughout: git2 only, no git process anywhere, forge knowledge only in the plugins, no new switches beside a broken path (JI-017C-E1 permits planned commands and flags that an approved item names), local projects never need the platform, two identity layers (joyint.com account and Joy identity per project), no background work on project data outside member sessions.

## 1. Verdict table

| Claim | Verdict | Decisive fact | Change it forces |
| --- | --- | --- | --- |
| C1 credential helper via git2 | fails | git2 0.21 maps a short helper name to `git credential-<name>` and runs `sh -c "<cmd> get"` first, falling back to spawning the first token directly (git2-0.21.0/src/cred.rs:298-317, :387-398, :400-428) | joy writes its own helper runner (D1.3) and never calls `Cred::credential_helper` |
| C2 libssh2 key and agent matrix | holds with changes | Windows WinCNG has ED25519 0, ECDSA 0, DSA 0 and reads only classic PKCS#1 PEM (wincng.h:74, :87-92, wincng.c:887-942) | per OS ssh step with a Windows carve out, key pre validation, joy parses ssh config (D1.4) |
| C3 anonymous remote twin | holds with changes | `remote_anonymous` still applies `url.*.insteadOf` and carries no refspecs (remote.c:237-256, :273-302) | predict insteadOf, take the twin from the plugin, write the tracking ref after a push (D1.5) |
| C4 device flow availability | holds with changes | GitHub device flow is opt in per app; new OAuth Apps default to 8 h tokens since 2026-08-14; GitLab has the device grant from 17.3; the Gitea family has none (https://docs.gitea.com/development/oauth2-provider) | two doors: device grant, and PKCE S256 on a loopback listener (D2.7) |
| C5 forge CLI tokens | holds with changes | `glab auth token` does not exist; `tea logins list` prints no token | name the exact command per CLI, fix config discovery per OS, own timeout for `token` (D2.4) |
| C6 one keychain entry for app, CLI and plugin | fails | `linux-native` selects kernel keyutils, in memory, gone at reboot (keyring-3.6.3/src/keyutils.rs:19-23, lib.rs:207-216) | `linux-native-sync-persistent`, a 0600 fallback written by joy, one owner per entry (D2.6) |
| C7 git2 commits without git config | holds with changes | only `forge::repo_identity` needs user.name and user.email (forge.rs:1161-1173) | delete or demote it, define the signature rule per privacy mode (D4.5) |
| C8 founder via InitOptions.user | holds with changes | the desktop command passes `user: None` and enrollment still reads git config (init.rs:59-96, crypt_ops.rs:726-773) | plumb the founder through every layer (D4.4, D3.9) |
| C9 distribution of the plugins | holds with changes | every plugin crate is distable by default; installers install one file (joy/dist-workspace.toml, website/public/install/joy.sh:110) | one distributed connector binary, a resolution contract, sidecars (D2.1, D2.2) |
| C10 maintenance without git gc | holds with changes | 72 percent of the measured objects were unreachable (JOY-023C-1E:16); libgit2 exposes no deletion | pack plus a guarded sweep with a keep set and a grace window (D3.7) |
| C11 contact rate unchanged over https | fails | the poll reschedules after the round trip; a private https contact is two HTTP requests (chatTransport.ts:166-186, httpclient.c:566-568) | fixed period poll, one combined ls-remote, a budget in requests (D1.9) |
| C12 agent reachable in every launch mode | fails | the desktop imports only PATH; the Windows agent service ships disabled (lib.rs:261-310) | per OS statement, import the agent variables, own agent probe (D1.4) |
| C13 one shared OAuth app | holds with changes | deleting an app grant deletes every token of that app for the user | a separate public client for desktop and CLI; logout revokes the token, never the grant (D2.4) |
| C14 fallback order under org policy | holds with changes | gh's helper serves gh's own OAuth App token, blocked by the same policy | the twin is a consequence of "no working local ssh credential" (D1.2) |
| C15 hooks and the item rule | holds with changes | `core.hooksPath` replaces the location entirely (https://git-scm.com/docs/git-config); joy sets it today (init.rs:255-271) | joy owns the path and chains to the previous hooks; an in process validator (D3.3, D3.5) |
| C16 plugin timeouts fit a login verb | holds with changes | `run_query_full` reads stdout only after exit and returns `Option` (forge_plugins.rs:316-357) | runner v2 with streaming, per verb timeouts, process group kill (D2.3) |
| C17 basic auth shapes | holds with changes | `basic_auth_user` decides by substring over the whole URL (forge.rs:32-44) | decide by parsed host plus the claimed forge kind; never the empty password shape (D1.6) |
| C18 personas and journeys | fails | four journeys neither served nor named as changed | named in section 3, with the package that closes each or the honest grade |
| R1 proxies and TLS | new, verified | `ProxyOptions` derives `Default`, which is `GIT_PROXY_NONE` (proxy_options.rs:9-16, libgit2-sys lib.rs:1906-1909), and joy passes nothing anywhere (forge.rs:330, :488, :626, :2083) | every contact carries `ProxyOptions` from one function; a trust store statement per OS (D1.11, D1.12) |
| R2 known_hosts | new, verified; v2's premise was false | libgit2 1.9.6 refuses an unknown or changed host key with no callback: `error = GIT_ECERTIFICATE` at ssh_libssh2.c:682, refusal at :764-767 | joy does the whole check in Rust and may accept; the live bug is that git2 ssh contacts already fail (D1.4a) |
| R3 classifier and limits | new, verified | `http.c` is `#ifndef GIT_WINHTTP`, `winhttp.c` is `#ifdef GIT_WINHTTP` (http.c:10, winhttp.c:10); the two vocabularies never coexist | classify on `error.code()` and `error.class()` plus a status regex, never prose (D1.8) |
| R4 CLI surface and versioning | new, verified | joy-cli is one flat `Commands` enum with no forge, plugins or sync variant (lib.rs:140-214); JI-017C-E1 forbids unnamed flags, not new commands | the `joy forge` group, a protocol version verb, a name order for resolution (D3.10, D2.2a) |
| R5 desktop sign in and modes | new, verified | production adds no CORS layer at all (api/mod.rs:144-147) and the cookie is `SameSite=Lax` (auth/mod.rs:428-431) | one claim exchange behind both doors, an ownership rule, a refresh lock (D4.0, D4.1a to D4.1d) |
| R6 sparse, partial, shallow | new, verified | the string "sparse" does not occur in libgit2's implementation or headers; no filter capability (smart.h:26-41); `depth` exists (remote.h:771-778) | the lean shape is a depth 1 clone with a full working tree; the App concept sentence is corrected (D4.3) |
| R7 scopes per verb | new, verified | GitLab `write_repository` "Does not support API authentication" (https://docs.gitlab.com/security/tokens/access_token_scopes/) | three scope sets per forge, a `scope_missing` state, GitLab create needs `api` (D2.7a) |
| R8 maintenance and twin push | new, verified | `git_remote_push` does call `git_remote_update_tips` and rebuilds `active_refspecs` from the configured ones (remote.c:2995-2997, :3045-3073) | only the anonymous twin needs a hand written tracking ref; per ref push statuses must be read (D1.5, D3.7) |

## 2. Adjusted design

**Index of this section.** The numbers below are canonical. Sections 3 to 7 reference these and no others, and every reference there can be checked against this list.

| Header | Title |
| --- | --- |
| D1 | one credential resolver in the engine |
| D1.1 | Shape and inputs |
| D1.2 | Candidate order, with the twin trigger of decision 11 |
| D1.3 | joy's own credential helper runner (replaces C1) |
| D1.4 | ssh chain |
| D1.4a | Host key verification (new, replaces one sentence of v2's D1.4) |
| D1.5 | The https twin, tracking refs and per ref push statuses |
| D1.6 | Credential shapes |
| D1.7 | Token freshness and caching |
| D1.8 | Failure classification |
| D1.8a | The classifier reads class, code and status, never prose |
| D1.8b | The mapping |
| D1.8c | Sentences for the two transport states |
| D1.9 | Contact budget, stated in HTTP requests |
| D1.10 | Prompt rule, narrowed to its mechanism |
| D1.11 | Proxies |
| D1.12 | The trust store and TLS interception |
| D1.13 | What the git binary did that joy must now do itself |
| D2 | forge plugins are the sign in door of local hosts |
| D2.1 | Binary shape and distribution |
| D2.2 | Plugin resolution contract |
| D2.2a | Protocol version and stale binaries |
| D2.3 | Runner v2 |
| D2.4 | Verb catalogue |
| D2.5 | Instance configuration for self hosted forges |
| D2.6 | Token storage, and who may read which entry |
| D2.6a | The refresh lock |
| D2.7 | OAuth per forge |
| D2.7a | Scope sets per forge, per verb group |
| D2.7b | What the person sees on the consent page |
| D2.7c | The read only member and `scope_missing` |
| D2.8 | Plugins get a real HTTP client |
| D2.9 | Plugin class |
| D2.10 | The rate limit oracle, per forge |
| D3 | CLI user and agent (G1, G2) |
| D3.1 | Packaging fact that must land first |
| D3.2 | The git2 only move |
| D3.3 | The item reference rule in process |
| D3.4 | joy's commits touch only joy's paths |
| D3.5 | Hooks, one rule |
| D3.6 | Signing |
| D3.7 | Maintenance replacing `git gc --auto` |
| D3.8 | Agent rules (G2) |
| D3.9 | What stays on git config, and what does not |
| D3.10 | The CLI's door to a forge (new) |
| D3.11 | The interactive gate (adopts the dropped verdict L change) |
| D3.12 | Migration (new) |
| D4 | app user (G3) |
| D4.0 | The platform address is app state, not a build constant |
| D4.1 | Start page |
| D4.1a | The desktop sign in door: one claim, two doors behind it |
| D4.1b | The two mode ownership rule |
| D4.1c | Multi account per host |
| D4.1d | The refresh lock in the app |
| D4.2 | Local sign in |
| D4.3 | Add a project, and what "lean" means |
| D4.4 | Setup mask for a repository without a store |
| D4.5 | Identity for commits |
| D4.6 | Sync worker and poll |
| D4.7 | Banner |
| D4.8 | Sidecars and the agent's environment |
| D4.9 | Mobile, named as changed |
| D5 | platform |
| D6 | docs, website, tests |

### D1: one credential resolver in the engine

#### D1.1 Shape and inputs

`joy_core::vcs::forge::Auth::Local` keeps its name and its place; its body becomes a resolver. It receives, per operation:

- the repository directory and the remote object chosen by `origin_or_first` (forge.rs:178-190). `remote_url` (forge.rs:1226-1233) is changed to the same selection, otherwise the throttle key, the twin source, the ownership join key of D4.1b and the contacted remote disagree in a multi remote checkout.
- the direction (fetch or push).
- the host facts: the forge id the plugin claims for the host and the web base it answers (D2 `claims`, `web-url`), or the engine's fallback table for github.com, gitlab.com and codeberg.org.
- the host kind: `Interactive`, `Background` or `Delegated`.
- the per host state: transport memory, cached credential, strike state, proxy decision.

The host kind is a parameter of the resolver and of every plugin call, and it is set once per process at the entry point of each host, not read again inside the engine:

- joy-cli sets it in `cli_main` before it dispatches: `Delegated` when `JOY_SESSION` names a live delegation session (identity.rs:48-170), otherwise `Interactive` for a command a person typed, otherwise `Background` for `prepare-commit-msg` and any hook invocation. This is the one read of the environment, and it is a read of joy's own session variable, not a new switch.
- the desktop sets `Interactive` for a foreground action the person started and `Background` for the sync worker and the chat poll.
- the platform sets `Background` everywhere, and the interactive verbs are not compiled into its binary at all (D3.11).

That removes the v2 contradiction between "a parameter, not read from the environment" and "JOY_SESSION forces Delegated": the environment is read exactly once, by the host, to choose the parameter.

#### D1.2 Candidate order, with the twin trigger of decision 11

Within one libgit2 contact the credentials callback is re entered while the result is `GIT_EAUTH` (ssh_libssh2.c:855-880), so several credentials of one transport cost one contact. A change of transport needs a new contact and a new throttle allowance (contact.rs:232-240 keys on the host, so ssh and https share the key).

Rule per configured remote:

1. **Configured https remote.** Forge token (D2), then a credential from joy's own helper runner (D1.3), all inside one contact. There is no anonymous step inside a poll (see below).
2. **Configured ssh remote.** The ssh candidates come first: agent, then key files that joy has validated, inside one contact. This is the correction the critic demanded, and it follows verdict row C14 and decision 11 rather than v2's D1.2: the twin is a consequence of "no working local ssh credential", never of "a token exists".
3. **When the twin is used.** Exactly two triggers, and both are recorded per host in the transport memory:
   - (a) joy establishes before the contact that there is no usable ssh credential for this host: the agent probe finds no agent or no identity for the host (D1.4), and no key candidate survives pre validation (on Windows this is the normal state, because WinCNG reads no openssh-key-v1 file at all, wincng.c:887-942). The memory entry is `no-ssh-credential` with a 24 hour TTL, and it is dropped as soon as `SSH_AUTH_SOCK` appears, an agent identity appears, or a candidate key file's mtime changes.
   - (b) the ssh contact failed with an authentication class failure (`class == Ssh` with `code == Auth`, D1.8). The memory entry is `ssh-failed` with the same TTL.
   A host whose memory says `ssh-worked` never goes to the twin, whatever tokens exist.
4. **Never** when an `insteadOf` or `pushInsteadOf` rule matches the twin (D1.5), and never for a host no plugin claims and that is not one of the three known hosts.
5. **Windows.** The key file step is skipped unless the file is a classic PKCS#1 RSA PEM. The skip is reported by name ("this key file is in the OpenSSH format, which joy cannot read on Windows"), not silently.

At most two contacts per operation. A third attempt happens only after a person acted. The transport that authenticated, together with the credential source, is written to joy's own state file (`<app_state_dir>/forge-state.json`, mode 0600, `joy_core::auth::session::app_state_dir()`, session.rs:257-271), never into the person's `.git/config`.

**No anonymous polling.** The v2 sentence "then anonymous for a fetch when the repository is public, all inside one contact" is removed. Unauthenticated https on github.com is 60 requests per hour per IP (https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api), which a 1 Hz chat poll exhausts in about a minute. The rule:

- A person initiated one off operation (a clone, an explicit "check now") may contact an https remote with no credential.
- A poll or a worker tick may not. For an https remote where the resolver finds no token and no helper credential, the host's poll interval becomes the unauthenticated interval, 15 minutes per host shared by every project on that host, and the surface says: "Not signed in to github.com. joy checks for changes every 15 minutes. Sign in for live updates." with the sign in action.
- Where the configured remote is ssh and an ssh credential works, the poll stays on ssh at the normal interval; the unauthenticated interval applies only to the credential free https case.

#### D1.3 joy's own credential helper runner (replaces C1)

`git2::Cred::credential_helper` is not used anywhere. joy-core gains `vcs::credential_helper`:

- Key lookup through `git2::Config` in this order: `credential.<exact remote url>.helper`, `credential.<proto>://<host>[:<port>].helper` (git2 omits the port, cred.rs:323-330, joy includes it), `credential.helper`. Empty values reset the chain as git does; joy reads all values of a multivar, not only the last one (config_list.c:160-201 returns the last).
- Value mapping: a leading `!` means a shell shaped value, an absolute path is used as is, anything else is a short name resolved to a binary named `git-credential-<name>` (plus `.exe`): on Windows through `HKLM`/`HKCU` `Software\GitForWindows` `InstallPath` and `LibexecPath` (install.iss:157-162), probing `mingw64\bin` and `mingw64\libexec\git-core`; on unix through `$(dirname $(which git))/../libexec/git-core` and PATH. joy never builds the string `git credential-<name>` and never spawns `git`.
- Shell shaped values are split with an argv splitter that honours single quotes, because gh always single quotes its Windows path (`!'C:\Program Files\GitHub CLI\gh.exe' auth git-credential`). A genuinely shell shaped value runs under the Git for Windows `usr\bin\sh.exe` found through the registry on Windows and under `/bin/sh` on unix, never `sh` from PATH.
- Per spawn environment through `Command::env`, never `std::env::set_var`: `GCM_INTERACTIVE=never`, `GCM_GUI_PROMPT=0`, `GIT_TERMINAL_PROMPT=0` for `Background` and `Delegated` hosts. For `Interactive` hosts these variables are not set at all. **The v2 clause "for Interactive hosts only when the person opted out of prompts" is deleted**: there was no setting behind it, and inventing one would be a new switch. The host kind is the whole rule.
- Input lines: `protocol=`, `host=` with `:port` when the URL carries one, `path=` when `useHttpPath` is set, `username=` when known. IP literal hosts are written as `host=` too, unlike git2 (cred.rs:214-220).
- Operations: `get`, plus `store` after a credential was accepted and `erase` after a credential was refused. git2 runs only `get` (cred.rs:395, :415), which is why a revoked GCM entry is replayed today on every contact.
- The helper's stderr is captured and quoted in the failure detail ("helper 'manager': fatal: Cannot prompt because user interactivity has been disabled."), the exact text GCM produces.
- The answer is cached per host in the resolver for the process, with the TTL rule of D1.7.

#### D1.4 ssh chain

- **Agent probe.** joy probes the agent itself before it calls libgit2, so that "no agent", "agent has no identity for this host" and "agent rejected" are three sentences. libgit2 collapses all three into `GIT_EAUTH` with "error authenticating" (ssh_libssh2.c:236-290).
- The desktop imports `SSH_AUTH_SOCK` and `SSH_AGENT_PID` in `adopt_login_shell_path` the way it imports PATH today (lib.rs:261-310), and passes them to the spawned joy CLI and to the agent.
- **ssh config** is parsed by joy (ssh2-config 0.8.0, MIT), because libgit2 reads none of it (zero hits for `ssh_config`, `IdentityFile`, `SSH_AUTH_SOCK` in libgit2-sys-0.18.7/libgit2/src). joy honours `Host`, `HostName`, `Port`, `User`, `IdentityFile`, and now also `UserKnownHostsFile`, `GlobalKnownHostsFile`, `StrictHostKeyChecking` and `HashKnownHosts` (D1.4a). `IdentityAgent` is read by joy's parser and applied by setting `SSH_AUTH_SOCK` in joy's own process before the contact, which is the only way to reach 1Password, Secretive or KeePassXC.
- `ProxyCommand` and `ProxyJump` are refused by name, not with a DNS error. This is now confirmed by absence: the string `proxy` does not occur anywhere in libssh2-sys-0.3.2's bundled sources or headers, libssh2 is handed an already connected socket, and libgit2 opens that socket itself. Sentence: "This host uses ProxyCommand in your ssh config. joy cannot run a proxy helper; use the https remote for this host or remove the rule."
- **Key files are pre validated** by joy before they reach libgit2: read the PEM header, reject openssh-key-v1 on Windows, detect an encrypted key (`Proc-Type: 4,ENCRYPTED`, or a cipher other than `none` in openssh-key-v1) and skip it when no passphrase is available. Mandatory, because a key file read error is `LIBSSH2_ERROR_FILE`, which libgit2 turns into -1 and which ends the whole operation instead of trying the next candidate (ssh_libssh2.c:366-380).
- The user name stays constant across callback invocations (ssh_libssh2.c:865), and the three attempt cap (forge.rs:154) is replaced by "as many candidates as the chain has".
- **Passphrase policy.** A prompt happens only for `Interactive` hosts, at most once per process per key, and the unlocked key stays in memory for the run. `Background` and `Delegated` hosts never prompt and report "key <path>: passphrase needed, skipped".

#### D1.4a Host key verification (new, replaces one sentence of v2's D1.4)

Correction to the record first: the claim that libgit2 accepts an unknown host key without a `certificate_check` callback is **false** for the version joy links. `check_certificate` starts with `int cert_type, cert_valid = 0, error = GIT_ECERTIFICATE;` (ssh_libssh2.c:682) and refuses whenever its own check did not return MATCH: `if (!cert_valid) { git_error_set(GIT_ERROR_SSH, "invalid or unknown remote ssh hostkey"); return (error == GIT_PASSTHROUGH) ? GIT_ECERTIFICATE : error; }` (:764-767). The real defect is the opposite one and it exists today: joy sets no callback anywhere (no hit for `certificate_check` in joy/crates, app/apps or platform/src; the only `RemoteCallbacks` is forge.rs:114), so every git2 ssh contact already fails with "invalid or unknown remote ssh hostkey" whenever `$HOME/.ssh/known_hosts` holds no plain, exactly matching line, and joy offers no way to accept.

libgit2 reads one file and nothing else (`SSH_DIR ".ssh"`, `KNOWN_HOSTS_FILE "known_hosts"`, ssh_libssh2.c:426-427, :454-462): no `known_hosts2`, no `/etc/ssh/ssh_known_hosts`, no `UserKnownHostsFile`, no `StrictHostKeyChecking`, no wildcard patterns (libssh2 matches plain names with `strcmp`, knownhost.c:407-414), no `@cert-authority` and no `@revoked` (knownhost.c:744-845 has no marker branch, so such a line is stored under the host name "@cert-authority" and never matches anything).

**joy decides alone, in the one callback slot there is.** git2 0.21 holds exactly one slot per contact (`certificate_check: Option<Box<CertificateCheck<'a>>>`, remote_callbacks.rs:27), so exactly one module owns it: `joy_core::vcs::certificates`, package J4h. Every `RemoteCallbacks` joy builds, for ssh and for https alike, installs that one closure, and the closure dispatches on the certificate kind. The two kinds have opposite rules, which is the only way both can hold:

- **Host key (`CertificateType::Hostkey`, an ssh contact).** joy performs the whole check of this section in Rust and answers either `CertificateOk` or an error. This branch never returns `CertificatePassthrough`, because passthrough hands the verdict back to a check that reads one file. joy may override libgit2 in both directions (ssh_libssh2.c:752-762).
- **x509 (an https contact).** This branch returns `CertificatePassthrough`, so libgit2's own verdict stands: joy never accepts a certificate libgit2 refused and never refuses one it accepted. The branch reads no verdict, because git2 0.21 drops libgit2's valid flag before the closure runs (remote_callbacks.rs:413-418); it only stashes the certificate's issuer and subject (parsed from `CertX509::data()`) in the callback state for the detail line. The `tls_untrusted` state is decided by the classifier when the operation then fails with `code == Certificate` or `class == Ssl` (D1.8b, D1.8c). Package J4p adds that branch's stash and sentence into the module J4h owns; it does not add a second callback.

joy cannot read libgit2's verdict on the host key branch: git2 0.21 drops the `valid` flag (`certificate_check_cb(cert, _valid, hostname, data)`, remote_callbacks.rs:413-418) and passes no port, so joy carries host and port in its own callback state, and the x509 branch never sees the flag either: it reads only the certificate data and leaves the verdict to libgit2, which surfaces as the error the operation returns. joy also cannot let libgit2's message reach the caller (it is overwritten at ssh_libssh2.c:765), so joy stores its refusal sentence in a cell captured by the closure and substitutes it when the operation returns.

**Which files joy reads.** For the host and port being contacted: every file named by `UserKnownHostsFile`, defaulting to `~/.ssh/known_hosts` and `~/.ssh/known_hosts2`, then every file named by `GlobalKnownHostsFile`, defaulting to `/etc/ssh/ssh_known_hosts` and `/etc/ssh/ssh_known_hosts2` (https://man.openbsd.org/ssh_config.5). joy writes to exactly one file, the first entry of `UserKnownHostsFile`, creating `~/.ssh` with mode 0700 and the file with 0600 when missing. Because libgit2 still reads `~/.ssh/known_hosts` itself before joy's callback runs, and because one unparsable line makes `libssh2_knownhost_readfile` discard the whole file with a negative return that kills the connection ("Failed to parse known hosts file", knownhost.c:960-985, surfaced as "error loading known_hosts" at ssh_libssh2.c:555-559), joy validates that file line by line before the first ssh contact of a process and reports it by name with the offending line number.

**Matching rule.** A line matches when its host field matches the contacted host and its key field equals the base64 of the raw host key blob (`CertHostkey::hostkey()`, cert.rs:140-152). The host field matches when a plain pattern matches with `*`, `?` and `!` negation as man sshd defines them, or the bracketed form `[host]:port` matches for a port other than 22, or a hashed entry `|1|<base64 salt>|<base64 hash>` matches, computed as `HMAC-SHA1(base64decode(salt), hostname)` (knownhost.c:416-441). Hashed entries are the normal case: Ubuntu ships `HashKnownHosts yes` in /etc/ssh/ssh_config. A `@revoked` line that matches host and key refuses unconditionally in every host kind, because man sshd says revoked keys "are never accepted for authentication". `@cert-authority` is out of scope for the first version: a certificate host key arrives as `SshHostKeyType::Unknown` with a blob whose first field ends in `-cert-v01@openssh.com` (libssh2 hostkey.c:1414-1470), and joy refuses with "joy cannot verify certificate host keys yet, this host presents one".

**The rule per host kind.**

- `Interactive`: on an unknown host joy shows host, port, key type name (`SshHostKeyType::name()`, cert.rs:52-62, which is the known_hosts spelling) and the fingerprint `SHA256:` plus unpadded base64 of `hash_sha256()`, the exact string the forges publish, and offers to trust it once. On yes, joy appends one line to the first `UserKnownHostsFile`: hashed when `HashKnownHosts` is yes, bracketed when the port is not 22, under joy's cross process advisory file lock (`joy_core::util::file_lock` on fs4, landed by J4a in wave 0 beside the per process `checkout_gate`, forge.rs:558-584), never rewriting an existing line. The refresh lock of D2.6a reuses the same primitive. With an empty known_hosts libssh2 offers ECDSA before ed25519 (hostkey.c:1346-1374), so the stored line is often `ecdsa-sha2-nistp256` where OpenSSH would have stored `ssh-ed25519`; both are valid. `StrictHostKeyChecking yes` means refuse without asking; `accept-new` means append without asking; `no` and `off` are treated as `accept-new`. joy never accepts a changed key.
- `Background` and `Delegated`: never ask, never write. An unknown host is refused with the classifier state `needs_host_trust`, and the sentence names host, port, fingerprint, the file that would need the line and the exact line to paste.
- **Mismatch** (a line exists for this host and this key type with a different key): always refuse, in all three host kinds, never write. The sentence names host and port, the presented and the expected fingerprint, the file and the one based line number, and says the line has to be removed by hand. A host that has lines only for other key types is not a mismatch; it is unknown for that key type, and the sentence says so.

**Pinned fingerprints.** joy ships the published host keys of github.com (and ssh.github.com), gitlab.com (and altssh.gitlab.com) and codeberg.org as full key blobs for every published type, in a data file inside the release, not as code. The pin is consulted only when no file read above contains any line for that host, and only for these hosts. `Interactive` still shows the fingerprint and adds "this is the key <forge> publishes at <URL>"; `Background` and `Delegated` accept the pinned key and write nothing. A known_hosts line always wins over the pin. Stated plainly: pinning replaces the person's first contact decision with trust in the joy release, and it makes joy refuse a legitimate key rotation at those three hosts until a new pin ships, so the mismatch sentence for a pinned host names the joy version and the forge's fingerprint page, and the pin file can be replaced without rebuilding. joy never fetches fingerprints at contact time. Codeberg publishes fingerprints only (https://docs.codeberg.org/security/ssh-fingerprint/), so its pin is built from a blob taken once and checked against the published fingerprints at build time. That build step is J4h's work and is named in J4h's acceptance. Until decision 23 is answered, the `Background` and `Delegated` half of this section rests on the pin, which is why the journey row in section 3 reads partly and not yes.

**Where the pin file lives, and who puts it there.** The loader reads two paths in this order: `<directory of the running binary>/host-keys.json`, then `<prefix>/share/joy/host-keys.json`, where the prefix is the parent of that directory (`joy_core::vcs::known_hosts::pins::candidates`). A release archive carries the file in its root beside `joy` and `joy-forge` (J8, the `include` key of joy/dist-workspace.toml), so a person who only unpacks an archive and runs joy out of it has the pins. An install has a prefix, so an installer writes the file to `<prefix>/share/joy/host-keys.json`, which is `$HOME/.local/share/joy/host-keys.json` for the install directory both hand written installers default to. Placing it is W1's work and is named in W1's acceptance, together with naming it in the install receipt, so that a second install run replaces it and a key rotation reaches installed machines as a replaced file. It is never written into the install directory itself: a copy there wins over the share copy for good and would shadow every later rotation.

**What no package places yet, named rather than discovered at the first rotation.** The installer scripts cargo-dist generates (`joy-cli-installer.sh`, `joy-cli-installer.ps1`) copy the binaries of the archive and nothing else, and dist 0.31 has no setting for a data file, so two paths onto a machine carry no pin file: an install through the generated script, and every `joy update`, which downloads and runs that same script. Those installs read the pins compiled into the binary, which is the empty set. A winget install extracts the whole archive into its package directory, so the file sits beside `joy.exe` there, but what komac writes into the manifest once a second executable is in that directory is unverified and the first tag after J8 has to be watched. The answers open to us, to be settled together with decision 23 because an empty pin set makes none of it urgent: joy's own `update` command copies the pin file out of the downloaded archive once axoupdater returns; get.joyint.com keeps serving the hand written scripts and the generated ones leave the release; or the pins go back into the binary and a rotation becomes a release. Until then the sentence "the pin file can be replaced without rebuilding" holds for an archive and for an install made by the hand written installers, and for nothing else.

#### D1.5 The https twin, tracking refs and per ref push statuses

The twin is computed only for a contact; the configured remote is never rewritten.

- Source of truth is the plugin: `web-url --remote <url>` (D2.4). Only the plugin knows the web base of a self hosted instance, which may sit under a nested sub path (https://docs.gitlab.com/18.9/install/relative_url/) or behind a different `SSH_DOMAIN` and `SSH_PORT` (https://docs.gitea.com/administration/config-cheat-sheet).
- Engine fallback table for github.com, gitlab.com and codeberg.org only, with ssh.github.com mapping to github.com and altssh.gitlab.com to gitlab.com. Parsing mirrors `git_net_url_parse_standard_or_scp`: the scp form yields a path without a leading slash, the `ssh://` form with one (net.c:522-530, :661-806); the bracketed port form `[git@host:2222]:owner/repo.git` is not part of the host.
- Refusal conditions (fall back to the configured remote and say so): no plugin claims the host and it is not one of the three known hosts; fewer than two path segments; the host has no dot or does not resolve; an `insteadOf` or `pushInsteadOf` rule matches. The last one is mandatory because `git_remote_create_anonymous` applies those rules and git2 0.21 binds neither `git_remote_create_with_opts` nor `GIT_REMOTE_CREATE_SKIP_INSTEADOF` (remote.c:237-256, :363-370). joy reimplements libgit2's longest prefix rule over `url.*.insteadof` (remote.c:3079-3140).
- ssh config aliases are rescued: the parse of D1.4 supplies `HostName`, so `git@work:owner/repo.git` maps to the real host for the twin while the configured ssh URL stays the ssh candidate.

**Tracking refs, corrected.** v2 said libgit2 updates the tracking ref only as a side effect of origin's fetch refspec. That is half wrong, and the wrong half was the dangerous one. `git_remote_push` does call `git_remote_update_tips` (remote.c:3045-3073), and `git_remote_upload` rebuilds `active_refspecs` from the remote's **configured** refspecs, not from the explicit push refspecs (remote.c:2995-2997, in contrast to the fetch path at remote.c:1279-1290). joy pushes over `origin_or_first`, a named remote, so `refs/remotes/origin/<branch>` is already written today and the ahead and behind counter does not freeze after an ordinary push. It freezes only over the anonymous twin, which carries zero refspecs (remote.c:273-302) and where `git_remote__matching_refspec` skips push specs (remote.c:2631-2649).

Therefore:

- After a successful push over the twin, the engine writes the branch's configured upstream, or `refs/remotes/<remote of the configured URL>/<branch>`, using `tracking_ref_name` (forge.rs:301-312) so the name matches what `ahead_behind` reads (forge.rs:1258-1280). Force is not a special case: libgit2's own `git_push_update_tips` creates the ref with force 1 and the message "update by push" (push.c:200-212), and joy does the same.
- **Per ref rejections are read.** `push` and `push_ref` set `RemoteCallbacks::push_update_reference` (remote_callbacks.rs:74, :205). Without it a push whose every ref was rejected returns `Ok(())`: `git_push_finish` fails only when the pack could not be unpacked (push.c:537-540), and the per ref status is delivered only through that callback (remote.c:3034-3038). joy collects every `Some(reason)`, fails the operation, and puts the server's sentence into the detail line. The tracking ref is written only for refs whose status was `None`.
- A server that does not advertise `report-status` leaves the status list empty (smart_protocol.c:1246-1250). joy treats that as "unconfirmed", writes no tracking ref, and lets the next `ls-remote` establish the truth.
- `refs/joy/chats` has no libgit2 tracking ref on either remote and needs none. joy keeps its own, `refs/joy/chats-remote` (chat_ref.rs:54-59), written by `download_ref` on the fetch side (forge.rs:352-356); after a successful push of the chat ref the engine sets it to the pushed oid as well, so the union merge reconciles against what the forge holds.
- `probe_write_access_raw` (forge.rs:482-496) runs on **the transport that carries the credential for this operation**, not always on the configured remote. If the operation would push over the twin, the probe uses the twin. If neither transport has a credential, the probe is not run at all and the state is `needs_sign_in`, never `no_push_rights`. This closes the v2 contradiction where the Windows case could not produce the state the banner needs.

#### D1.6 Credential shapes

`basic_auth_user` (forge.rs:32-44) is replaced by a decision on the parsed host plus the forge kind the plugin claims:

- GitHub and GitHub Enterprise Server: user `x-access-token`, token as password.
- GitLab: user `oauth2` (gitlab doc/api/oauth2.md:409-417).
- Gitea, Forgejo and Codeberg: user `oauth2`, token as password (forgejo services/auth/method/util.go:60-67).
- The shape "token as user name with an empty password" is never sent. On GitLab it always fails and is counted by the failed authentication ban (D1.8b).
- The `Auth::Token` callback honours the `allowed` credential mask, otherwise an insteadOf rewrite to ssh produces "authentication callback returned unsupported credentials type" (ssh_libssh2.c:415-418).
- The shape that worked is remembered per host next to the transport memory.

#### D1.7 Token freshness and caching

The forge token is asked from the plugin per host, not per contact, and cached in memory with a TTL of `min(expires_at - 60 s, 5 minutes)`. The plugin refreshes on its side (GitLab 7200 s, Gitea and Forgejo 3600 s, gh's `gho_` tokens long lived but revoked after a year without use). A 401 invalidates the cache immediately and triggers one re ask. A 1 Hz chat poll must not spawn a plugin or a .NET GCM process per contact.

#### D1.8 Failure classification

##### D1.8a The classifier reads class, code and status, never prose

`vcs::contact::classify(&str)` is deleted. Today it is a lowercase substring scan in three ordered groups (contact.rs:83-109), every string of which is a libgit2 literal from the non Windows build only, and joy destroys the evidence before the classifier sees it: `contact_error` returns `e.message()` alone (forge.rs:94-105) and `forge.rs` prefixes `"(offline?)"` onto every connect failure (forge.rs:335, :353, :630), which makes a 404 over https read as "no connection to github.com". Both stop.

The replacement is `classify(evidence: &ContactEvidence) -> Failure`, where `ContactEvidence` is `{ error: git2::Error, transport: Https | Ssh, direction: Fetch | Push, credential: NonePresented | TokenPresented | HelperPresented | AgentPresented, token_worked_before: bool, host: String }`. The classifier reads, in this order:

1. `error.code()` (error.rs:94-131): `Auth` is `GIT_EAUTH`, `Certificate` is `GIT_ECERTIFICATE`, `Timeout` is `GIT_TIMEOUT`.
2. `error.class()` (error.rs:173-215): `Http`, `Net`, `Ssh`, `Ssl`, `Os`.
3. The HTTP status number, extracted by one regex over exactly two libgit2 formats and nothing else: `unexpected http status code: (\d+)` (http.c:282, every non Windows build) and `request failed with status code: (\d+)` (winhttp.c:1274, every Windows build). Both are `%d`/`%lu` of a C int, so the digits are ASCII and locale independent.
4. Text, only as a last resort and only against libgit2's own English literal prefixes, never against anything the operating system appended: `failed to send request` and `failed to connect to host` (winhttp.c:950, :870), `too many redirects or authentication replays` (http.c:439, winhttp.c:1066).

This is a correctness fix, not a refactor. `http.c` is compiled under `#ifndef GIT_WINHTTP` and `winhttp.c` under `#ifdef GIT_WINHTTP` (http.c:10, winhttp.c:10), and libgit2-sys selects `GIT_WINHTTP` for every windows target, Secure Transport for apple, OpenSSL elsewhere (build.rs:257-269). The two producers share no sentence. On Windows a 401 that no credential satisfies is not `GIT_EAUTH` at all: `acquire_credentials` returns a positive pass through and the flow falls into "request failed with status code: 401" with code -1 (winhttp.c:998-1046, :1274). Every `GIT_ERROR_OS` message has `FormatMessageW` output for `GetLastError` appended by `git_error_vset` (errors.c:176-199, win32/error.c:16-52) with `MAKELANGID(LANG_NEUTRAL, SUBLANG_DEFAULT)`, the user default language, so on a German Windows the tail is German; on Linux the same holds for `strerror` and `gai_strerror` (socket.c:52-54, :186-188). Matching "connection", "timed out" or "could not resolve" therefore fails on every non English host. Two named acceptance tests are required, one against a WinHTTP error corpus and one against an OpenSSL corpus, both with a non English operating system message tail.

##### D1.8b The mapping

| Evidence | State |
| --- | --- |
| Plugin binary not found or spawn failed | `plugin_missing` |
| Plugin answers protocol 1 to a protocol 2 verb | `plugin_outdated` |
| Plugin answers `scope_missing` | `scope_missing` |
| Plugin answers `needs_sso` (GitHub 403 with an `X-GitHub-SSO` header, D2.7c) | `needs_sso`, the header's URL is the action |
| `code == Auth` on https, or status 401, and no credential source answered | `needs_sign_in` |
| `code == Auth` on https, or status 401, with a token that was presented | `needs_sign_in`, after one cache invalidation and one re ask of the plugin (D1.7) |
| Status 429 | `rate_limited`, wait time from the plugin on GitHub only, otherwise from the strike table |
| Status 403, direction Fetch, `token_worked_before`, GitHub host | ask the oracle (D2.10): `rate_limited`, `needs_org_approval` or `denied` |
| Status 403, direction Fetch, `token_worked_before`, GitLab host | `rate_limited` with the documented wait per instance kind of D2.10 (gitlab.com 15 minutes, self managed default 1 hour), without asking the plugin (the failed authentication ban). There is one such rule and it lives in D2.10 |
| Status 403 or 404, direction Push, after a read on the same host succeeded | `no_push_rights` |
| Status 404, direction Fetch, GitHub, token otherwise valid | `needs_org_approval` |
| Status 404, direction Fetch, otherwise | `error`, "github.com does not have this repository (renamed, deleted or not visible to this login)". Never `offline` |
| Status 502, 503, 504 | `offline`, "github.com is not answering right now" |
| Status 5xx other | `error` |
| `code == Timeout`, or `class == Net` with no status, or `class == Os` with prefix `failed to send request` / `failed to connect to host` | `offline` |
| `code == Certificate` on https, or `class == Ssl`, or `class == Http` with one of the seven WinHTTP certificate sentences (winhttp.c:718-740) | `tls_untrusted` |
| Proxy 407 texts (http.c:165-169, :206-208) | `proxy_auth` |
| `class == Ssh` with `code == Auth` | `needs_sign_in` with the ssh detail line of D1.4 |
| `class == Ssh` with `code == Certificate` ("invalid or unknown remote ssh hostkey", ssh_libssh2.c:765-766) or joy's own host key refusal | `needs_host_trust` |
| `class == Ssh`, generic code, message is the remote's own stderr (ssh_libssh2.c:138) | direction Push: `no_push_rights`; direction Fetch: `denied`. The forge's sentence goes to the detail line, never to the banner |
| `too many redirects or authentication replays` | `needs_sign_in`, not `denied`; libgit2 sets this as `GIT_ERROR` with the comment "not GIT_EAUTH, because the exact cause is unclear" (http.c:439-440) |
| Everything else | `error` |

`denied` survives only as the oracle's answer on GitHub and as the ssh fetch refusal. No state is decided by the presence of the digits "401" or "403" anywhere in a string, which is what happens today for any branch or path that contains them (contact.rs:88-96).

Status word mapping for platform readers that do not know the new words: `needs_sign_in`, `needs_org_approval`, `needs_sso`, `no_push_rights`, `needs_host_trust` and `scope_missing` read as `denied`; `plugin_missing`, `plugin_outdated`, `tls_untrusted` and `proxy_auth` read as `error`; `rate_limited` and `offline` are unchanged. An older app build compares only against "denied" and "rate_limited" (createForgeSync.ts:98-118) and defaults to "offline" only for an empty word (App.tsx:1144), so an unknown word shows the neutral banner and does not block writes.

**Wording rules, binding for every host.** The surface shows one plain sentence and at most one action. The list of sources tried goes to the log and to a details view the person opens deliberately; it never goes into a tooltip and never onto the banner (today the raw libgit2 sentence sits in the banner's title attribute, createForgeSync.ts:88-120). The detail line has a fixed grammar: `agent: no identities; key ~/.ssh/id_ed25519: passphrase needed (skipped, background); helper 'manager': fatal: Cannot prompt because user interactivity has been disabled.; no forge login for github.com`. Every failure sentence names one next step.

##### D1.8c Sentences for the two transport states

- `tls_untrusted`. An https contact goes through the same single `certificate_check` slot as an ssh contact and through the same one closure of `vcs::certificates` (D1.4a, owned by package J4h). libgit2 hands the raw callback `is_valid` plus the host name and restores its own message on passthrough (httpclient.c:784-841), but git2 0.21's safe wrapper drops `is_valid` (remote_callbacks.rs:413-418), so joy never reads it. On the x509 branch the closure returns `CertificatePassthrough`, stashes issuer and subject from `CertX509::data()` for the detail line, and never accepts or refuses anything itself; the state `tls_untrusted` is decided by the classifier from the error the operation returns (`code == Certificate` or `class == Ssl`, D1.8b). Package J4p writes this branch's sentence and state into the module J4h owns. Sentence: "The certificate for github.com is not trusted by this machine's certificate store." Detail: "issued by 'Acme Corporate Root CA', chain not trusted; libgit2: the SSL certificate is invalid". One next step per OS: "add your organisation's CA with update-ca-certificates" (Linux), "add your organisation's CA to the login or System keychain and mark it trusted" (macOS), "your administrator must install the CA in the Windows certificate store" (Windows). The detail line carries the platform specific libgit2 text so the classifier can be proven on all three: "the SSL certificate is invalid" (streams/openssl.c:381-384), "untrusted connection error" (stransport.c:117-120), "SSL certificate signed by unknown CA" (winhttp.c:718-740).
- `proxy_auth`. Sentence: "The proxy proxy.acme.example:8080 needs a user name and a password." Next step: sign in to the proxy (joy stores the answer through its credential helper runner under host `proxy.acme.example`), with the alternative in the detail line: "or set http.proxy to http://user@proxy.acme.example:8080". The two libgit2 texts that map here are "proxy authentication required but no callback set" (http.c:165-169, server type "proxy" at http.c:98) and "proxy requires authentication that we do not support" (http.c:206-208). The second one on Linux and macOS means the proxy offered only NTLM or Negotiate and gets its own next step: "this proxy requires Windows integrated authentication, which joy cannot do on this system; ask for a proxy password or a bypass rule for <host>."

#### D1.9 Contact budget, stated in HTTP requests

Contacts are the wrong unit, and v2's acceptance test inherited the error. Two facts fix the arithmetic.

First, credentials follow a 401. `apply_credentials` sends no `Authorization` header when there is neither an auth context nor a stored challenge (httpclient.c:566-568), so the first request of every new connection to a private repository is unauthenticated and is answered 401; the credential callback then runs and the request is replayed. Within one connection the Basic context persists (auth.c `basic_context`), so every further request of that connection carries the header on the first try.

Second, `download_ref` connects twice. The `RemoteConnection` returned by `connect_auth` disconnects on drop (git2 remote.rs:803-807) and joy drops it before `remote.download` (forge.rs:328-353); `git_remote_download` then reconnects (remote.c:1251-1261, :1338-1345), and `http_close` has already freed the credential (http.c:719-733), so the 401 dance repeats.

| Verb | Connections | requests, anonymous | requests, private with a good token | requests, private with a dead token |
| --- | --- | --- | --- | --- |
| `ls_remote_ref` / `ls_remote_refs` | 1 | 1 | 2 | up to 4, three of them 401 |
| `probe_write_access` | 1 | 1 | 2 | up to 4 |
| `fetch_branch` / `fetch_ref` today | 2 | 3 | 5 | up to 5 |
| `fetch_branch` / `fetch_ref` with the connection held | 1 | 2 | 3 | up to 4 |
| `push` / `push_ref` | 1 | n/a | 3 | up to 4 |

The dead token column is joy's own doing: `Auth::Token` tries the forge shape, the other shape and then the credential helper inside one contact (forge.rs:124-148), and libgit2 allows fifteen replays per stream (http.h:14).

Changes:

- `download_ref` performs its `download` through `connection.remote()` while the connection is alive (git2 remote.rs:797-801), which removes one handshake and one challenge per fetch.
- `contact::run` charges the throttle per request, using the verb it already receives, the transport and whether a credential is in play.
- Per host budgets, in requests per second per host per machine. **This table is canonical.** Every millisecond figure in this design is `1000 / budget` computed from it, and no millisecond figure is stated independently anywhere else:

| Host | Budget, requests per second | Milliseconds per request |
| --- | --- | --- |
| codeberg.org | 0.9 (a measured ceiling of about 1.1 requests per second, JP-00EF-CC) | 1111 |
| github.com | 1.0 | 1000 |
| gitlab.com | 5.0 | 200 |
| unknown self hosted | 1.0 | 1000 |

  The gap for a given verb is `requests(verb) / budget`. At Codeberg one authenticated `ls-remote` costs 2.2 s of budget, so a 1 Hz chat poll over authenticated git https on Codeberg is not achievable, and the design says so instead of shipping a poll that cannot run.
- **The poll period, decided, one rule.** The period per host is `requests(verb) / budget`, rounded up to the next whole second. On codeberg.org a private https chat poll is two requests, so it runs at most every 3 s (2 / 0.9 = 2.2 s, rounded up). There is no second door on that host: the Gitea family offers no conditional REST door joy may use, and D2.10 forbids the rate limit oracle call on Codeberg, Forgejo and Gitea after a 403 or 429, because it rides the same per IP bucket as the git request (the one off login probe of D4.1c and the `identity` validation of `token-store` are not limiter reactions and stay allowed). J5 computes the period from the table above; A5 and P2 use the number J5 computes and never a number of their own.
- N open projects on one host divide the budget: the poll interval is N times the host gap.
- The 429 brake is fixed: the strike is not cleared by the next success (contact.rs:322-330 clears the entry today), `strikes` becomes the exponent it is documented to be, and the gap is `gap * 2^strikes` capped, until `STRIKE_LASTS` (600 s) has elapsed.
- `ls_remote_refs(root, auth, &[...])` replaces two contacts by one, because `connection.list()` already downloads the full advertisement (forge.rs:608-638).
- The acceptance criterion is written in HTTP requests per minute per host, derived from the budget table above and measured with a counting proxy or `GIT_TRACE`, not in contacts. J5's and A5's acceptance criteria are stated in those units and in no other.

#### D1.10 Prompt rule, narrowed to its mechanism

`Background` and `Delegated` hosts raise no prompt that joy controls. The mechanisms, each of which is verified:

1. The host kind is a parameter set once at the entry point (D1.1), not a TTY guess. `joy_process::headless()` is not the test, because it answers false on every unix host.
2. joy's own ssh passphrase prompt and joy's own host key question are gated on `Interactive` (D1.4, D1.4a).
3. The credential helper runner passes `GCM_INTERACTIVE=never`, `GCM_GUI_PROMPT=0` and `GIT_TERMINAL_PROMPT=0` per spawn for those host kinds (D1.3).
4. The interactive plugin verbs (`login`, `logout`, the token paste) are compiled out of the platform binary and refused at runtime on `Background` and `Delegated` hosts (D3.11).
5. Every plugin call carries the host kind as a protocol field, and the plugin uses it to skip any step that can raise an operating system dialog.

What the mechanism does **not** cover, stated rather than promised: an operating system store that decides on its own to show a dialog (a macOS keychain item whose ACL no longer matches the calling binary, a locked Secret Service collection with a prompt agent). For those, the guard is the verb deadline plus the process group kill of D2.3: the call is bounded, the dialog is closed with the child, and the answer is the named failure "the credential store on this machine did not answer without asking a person". The exact error mapping (`errSecInteractionNotAllowed`) stays in section 6 as unverified, and no state is built on it silently.

#### D1.11 Proxies

Until now the git binary read the proxy configuration and joy never had to. With git2 only, joy must ask for it: `git2::ProxyOptions` derives `Default`, which is `GIT_PROXY_NONE` (proxy_options.rs:9-16, libgit2-sys lib.rs:1906-1909), and every joy call site passes nothing (`connect_auth(dir, callbacks, None)` at forge.rs:330, :488, :626, :2083; `FetchOptions`/`PushOptions` without `proxy_options` at forge.rs:260, :348, :513, :594, :1349, :2108). A grep for proxy over joy-core/src/vcs and platform/src finds only prose comments. Today joy behind a corporate proxy simply times out on Linux and macOS.

Rule: every `FetchOptions`, every `PushOptions` and every `connect_auth` in joy-core carries a `ProxyOptions` built by one function, `vcs::proxy::options_for(url, repo)`, which decides between three outcomes.

1. **Bypassed.** If the host matches joy's own NO_PROXY evaluation, the options stay `GIT_PROXY_NONE`. joy evaluates NO_PROXY itself and applies it to proxies from git config as well, because libgit2 applies it only to the environment branch (`http_proxy_config` never looks at no_proxy, remote.c:1085-1133) while git applies it always. joy's matcher follows libgit2's grammar (net.c:1070-1117: comma separated, `*`, `*.domain`, `.domain`, `host:port`, no CIDR) and additionally trims whitespace around each entry, because libgit2 does not and `NO_PROXY="a.com, b.com"` therefore silently loses `b.com`.
2. **Auto.** `ProxyOptions::auto()`, which lets libgit2 walk its own order: `remote.<name>.proxy`, then `http.<url>.proxy` from the full URL down the path to the bare host, then `http.proxy`, then `https_proxy`/`http_proxy`, then `HTTPS_PROXY`/`HTTP_PROXY` (remote.c:1085-1194). joy does not reimplement this.
3. **Specified.** `ProxyOptions::url(...)` in two cases: when only `ALL_PROXY`/`all_proxy` is set, because libgit2 never reads it and git does (git-config http.proxy: "normally configured using the 'http_proxy', 'https_proxy', and 'all_proxy' environment variables"), and when joy has proxy credentials to inject.

**Proxy credentials.** git2 0.21 hardwires `credentials: None` and `certificate_check: None` in `ProxyOptions::raw` (proxy_options.rs:44-53) and offers no setter, so joy can never answer a 407 through a callback. It answers it the only way that works: libgit2 presents a proxy URL's own userinfo before it consults any callback (http.c:141-152), so joy resolves the proxy credential through the helper runner of D1.3 (`protocol=http`, `host=<proxyhost>[:port]`), builds `http://user:pass@proxy:port` in memory and passes it as `ProxyOptions::url`. The credential never touches the person's git config and never appears in a log line or an error text. On Windows this is also the only correct mode: under `GIT_PROXY_AUTO` the 407 path passes a NULL URL into `acquire_credentials` (winhttp.c:1255-1270), so joy uses `GIT_PROXY_SPECIFIED` whenever a proxy is known.

Which proxy sources apply per OS, stated plainly so support can read it:

- Linux and macOS: git config (`remote.<name>.proxy`, `http.<url>.proxy`, `http.proxy`) and the environment (`https_proxy`, `http_proxy`, the uppercase variants, plus `ALL_PROXY` through joy). The macOS System Settings network proxy and the GNOME proxy settings are read by nothing in this stack; a person who set only those is offline as far as joy is concerned and gets the offline sentence with that fact in the detail line.
- Windows: the same sources, plus the machine wide WinHTTP proxy from the registry, which applies even when joy passes nothing, because the session is opened with `WINHTTP_ACCESS_TYPE_DEFAULT_PROXY` (winhttp.c:828-833). Microsoft's note is decisive: it "Retrieves the static proxy or direct configuration from the registry" and "does not inherit browser proxy settings". Per user Internet Explorer or Edge settings, PAC files and WPAD are never used. When joy computes a proxy, WinHTTP switches to `WINHTTP_ACCESS_TYPE_NAMED_PROXY` and the registry proxy is replaced, not merged (winhttp.c:439-493).
- The desktop inherits no shell environment: `adopt_login_shell_path` imports `PATH` and nothing else (lib.rs:261-310). `HTTPS_PROXY` in a .zshrc is therefore invisible to the app while it works in the CLI. joy extends that import to `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY`, `SSL_CERT_FILE` and `SSL_CERT_DIR` (lower and upper case), which is the only way the app and the CLI behave the same on one machine.

Not supported, and said so instead of failing obscurely: SOCKS proxies. libgit2 parses any proxy URL as an HTTP proxy and always speaks HTTP CONNECT (httpclient.c:686-700); a `socks5://host` without an explicit port is rejected with libgit2's own "invalid URL" because no default port exists for that scheme (net.c:110-123, http.c:340-342). joy detects a non http/https proxy scheme before the contact and says "joy cannot use the SOCKS proxy <url>; it supports HTTP and HTTPS proxies only." Off Windows only Basic reaches a proxy, because libgit2-sys defines neither `GIT_NTLM` nor `GIT_GSSAPI` (build.rs:256-269) and both aliases resolve to `git_http_auth_dummy` (auth_ntlm.h:15, auth_negotiate.h:15, auth.c:65-71); on Windows WinHTTP does Negotiate, NTLM, Digest and Basic (winhttp.c:140-180). This asymmetry is stated in the failure text, not hidden.

#### D1.12 The trust store and TLS interception

The one `certificate_check` closure of D1.4a is installed on https contacts as well. Its x509 branch returns `CertificatePassthrough` and reads no verdict (D1.8c), so the trust decision below is libgit2's alone; joy only names the issuer in the detail line.

There are three trust stores and joy has no single CA setting. What joy honours, per OS:

- **Linux**: OpenSSL's default verify paths, which are `SSL_CERT_FILE` and `SSL_CERT_DIR` when set (`SSL_CTX_set_default_verify_paths`, streams/openssl.c:147-148). joy builds with `git2/vendored-openssl` (joy-core/Cargo.toml:89), so the compiled in OPENSSLDIR is useless and the working source is git2's init, which runs openssl-probe (git2 lib.rs:766-771, :894-895) and finds `/etc/ssl/certs/ca-certificates.crt` and `/etc/ssl/certs` (openssl-probe-0.1.6 src/lib.rs:29-53, :162-200). A corporate CA installed with `update-ca-certificates` is therefore trusted without any joy setting.
- **macOS**: the system anchors plus the Keychain Access trust settings. libgit2 calls `SecTrustEvaluate` and never `SecTrustSetAnchorCertificates` (stransport.c:101-121), and it rejects only Invalid, OtherError, Deny, RecoverableTrustFailure and FatalTrustFailure, so the `kSecTrustResultProceed` that an administrator trust setting produces (Apple TN2232) is accepted. There is no file based alternative.
- **Windows**: the Windows certificate store through WinHTTP. A CA pushed by group policy works, and there is no file based alternative.
- **One joy owned escape hatch, Linux only**: a `ca_bundle` or `ca_dir` entry in `forges.yaml` (D2.5), applied once at process start with `git2::opts::set_ssl_cert_file` / `set_ssl_cert_dir` (opts.rs:255-291). It is process global and must run before the first contact. On macOS and Windows joy refuses the entry with a sentence, because `GIT_OPT_SET_SSL_CERT_LOCATIONS` is compiled only for OpenSSL and mbedTLS (settings.c:207-223).

What joy will not do, listed so nobody expects it:

- joy does not read `http.sslCAInfo`, `http.sslCAPath`, `http.sslVerify`, `http.sslBackend`, `http.schannelUseSSLCAInfo`, `http.proxyAuthMethod`, `http.proxySSLCAInfo`, `http.sslCert` or `http.sslKey`, and it reads no `GIT_SSL_CAINFO`, `GIT_SSL_CAPATH` or `GIT_SSL_NO_VERIFY`. libgit2 reads none of them (zero hits in the whole 1.9.6 tree; the only http.* keys are `http.followRedirects`, `http.<url>.proxy` and `http.proxy`). One named exception: on Linux only, joy reads `http.sslCAInfo` and `http.sslCAPath` from git config at start and feeds them into the same `git2::opts` call, because that is the setting a corporate workstation image already carries. On macOS and Windows those two keys, and the two `forges.yaml` keys beside them, are reported as ignored with one sentence per entry that names the entry, where it came from and the step that does work on this operating system: "joy ignores http.sslCAInfo from git config: it does not apply here, because this system checks certificates against its own store. To trust an internal CA, add your organisation's CA to the login or System keychain and mark it trusted." (macOS), and "... your administrator must install the CA in the Windows certificate store." (Windows). The per OS step is the same wording as the `tls_untrusted` next step of D1.8c, so a person reads one instruction and not two.
- joy never disables certificate verification. There is no joy equivalent of `http.sslVerify=false`.
- joy does not do mutual TLS. libgit2 has no client certificate support (no `SSL_CTX_use_certificate` in streams/openssl.c or stransport.c; winhttp.c:983-991 sets `WINHTTP_NO_CLIENT_CERT_CONTEXT` with the comment "Client certificates are not supported"). A forge or proxy that demands a client certificate fails with a sentence that names this.
- An https proxy's own certificate is checked with `proxy_certificate_check_cb` (httpclient.c:985-993), which git2 hardwires to None, so an untrusted https proxy certificate surfaces as the raw libgit2 text with no host context. joy prefers an `http://` proxy URL and documents the degraded message.

#### D1.13 What the git binary did that joy must now do itself

For the record and for the migration note: the git binary read `all_proxy`, applied `no_proxy` to config sourced proxies too, supported `http.proxyAuthMethod` with basic, digest, negotiate and ntlm, looked the proxy password up through the credential helper, honoured `http.sslCAInfo`, `http.sslCAPath`, `http.sslVerify`, `http.sslBackend`, `GIT_SSL_*`, client certificates and SOCKS proxies, read `~/.ssh/config` including `ProxyCommand`, and verified host keys against `/etc/ssh/ssh_known_hosts` and hashed `known_hosts` entries. Of that list joy takes over `all_proxy`, `no_proxy` everywhere, the proxy password through the helper (injected as URL userinfo), `http.sslCAInfo`/`http.sslCAPath` on Linux, the ssh config keys of D1.4, and the whole known_hosts semantics of D1.4a. Everything else is dropped by decision and named in the documentation, not discovered by a person at a failed push. The plugins are the one place where this repeats: they call the curl binary today (joy-github/src/github.rs:159, :404), which honours all of the above, so the move of plugin HTTP in process (D2.8) must answer this question a second time for that stack, and the platform's own `reqwest` is the bad example, since `rustls-tls` maps to `rustls-tls-webpki-roots` (reqwest 0.12.28 Cargo.toml:148) and trusts the bundled Mozilla roots only.

### D2: forge plugins are the sign in door of local hosts

#### D2.1 Binary shape and distribution

One cargo package produces the connector binary. Recommendation: `joy-forge`, a single binary carrying all three forges, declared as a `[[bin]]` of the package that already produces `joy`, so one archive, one installer change, one receipt and one sidecar per platform cover everything. Measured today: three separate plugins cost 3.3 MB stripped of which about 3 MB is a duplicated std, clap and serde floor; one binary is about 1.2 MB. The names `joy-github`, `joy-gitlab` and `joy-gitea` stay as a PATH fallback for `cargo install` users until the deprecation window of D2.2a closes. Without this, the next `v*` tag ships four extra cargo-dist Apps by itself (joy/dist-workspace.toml has no `dist = false`) and makes the winget `installers-regex` ambiguous (joy/.github/workflows/release.yml:385-395).

#### D2.2 Plugin resolution contract

`ForgePluginSpec` (forge_plugins.rs:26-47) stops carrying `binary: &'static str` as the whole truth and becomes `{id, binary_names, resolved_path, found_in, protocol, plugin_version}`. Resolution order, applied in joy-core and in app/apps/desktop/src-tauri/src/plugin_ops.rs alike:

1. Directories the host registered at startup (`forge_plugins::set_plugin_dirs(Vec<PathBuf>)`): the desktop passes `tauri_utils::platform::current_exe()`'s parent, the CLI passes its own install directory.
2. The directory of the current executable.
3. PATH.

**Inside every directory, the name order is `joy-forge` first, then `joy-github`, `joy-gitlab`, `joy-gitea`.** Without this the cargo install case loses deterministically: Rust resolves a bare program name through the directory of the current executable on Windows and through PATH only on unix (https://doc.rust-lang.org/std/process/struct.Command.html), `~/.cargo/bin` is normally on PATH, `cargo install` writes "all executables ... into the installation root's `bin` folder" and removes nothing another package installed (https://doc.rust-lang.org/cargo/commands/cargo-install.html), so a stale `joy-github` sits beside a fresh `joy` forever.

`JOY_PLUGIN_DIR` exists only as a documented test hook, not as a product switch, and decision 29 names it in the same item as every flag and feature this design adds.

#### D2.2a Protocol version and stale binaries

The protocol gets a number and a first verb. `joy-forge version` (and every legacy `joy-<forge> version`) answers exactly one object:

```
{"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}
```

The handshake is asked once per resolved path per process and cached in a map keyed by the canonical path and its mtime, under the 5 s query timeout class. It is not asked per verb, so the 1 Hz chat poll costs no extra process.

Detecting a protocol 1 binary needs no cooperation from it: its clap parser rejects the unknown subcommand and exits 2 with usage on stderr and empty stdout (joy-github/src/main.rs:88 `Cli::parse()`; clap_builder `USAGE_CODE = 2`). The rule is: exit code 2 with empty stdout, or any answer that does not parse as the object above, means protocol 1.

A protocol 1 plugin may still answer the six old verbs (`claims`, `identity`, `resolve`, `store`, `files`, `release`) with `--remote` only, so an old machine keeps publishing releases. It may not answer any verb of D2.4. Asking one produces the state `plugin_outdated` with the resolved path and the fix in one sentence: "the GitHub connector at /home/s/.cargo/bin/joy-github speaks protocol 1, this joy needs protocol 2. Install the new connector (`cargo install joy-cli` ships joy-forge), then remove the old one: rm /home/s/.cargo/bin/joy-github". The legacy names leave the registry one release after the deprecation window announced in the release notes. Until then `joy forge plugins` reports a shadowing legacy binary with `"problem":"shadowed-legacy"` and prints the exact `rm` line. joy never deletes a binary it did not install; the installers and `joy update` remove the three legacy names only when the install receipt lists them as their own files.

#### D2.3 Runner v2

`run_query_full` (forge_plugins.rs:316-357) is split:

- `run_once(spec, args, env, deadline) -> PluginOutcome` reads stdout concurrently with waiting, pipes stderr, and returns `{ exit_code, stdout_json, stderr_text, timed_out, spawn_error }`. Today stdout is read only after exit, which deadlocks any answer above the 64 KiB pipe buffer; this already threatens the existing `files` verb (joy-github/src/github.rs:501-544).
- `run_stream(spec, args, env, on_event, cancel) -> PluginOutcome` reads stdout line by line while the child runs.
- Cancellation kills the process group (unix: own process group, Windows: a job object), because `child.kill()` leaves curl or gh grandchildren behind.
- Timeouts per verb: `claims`, `identity`, `resolve`, `web-url`, `version` 5 s; `store`, `files`, `repositories`, `create-repository` 30 s; `token` 30 s (gh alone allows 60 s per keyring read); `release` 120 s; `login` two bounds, 15 s until the first `verification` event and then that event's `expires_in` capped at 900 s.
- Every caller distinguishes `{"known":false}` (exit 0), plugin missing, plugin outdated, plugin failed and timed out.
- All verbs accept `--host <hostname>` in addition to `--remote <url>`, and the runner no longer requires a project root.
- Every call carries two new protocol fields: `--host-kind interactive|background|delegated` (D1.10) and, where a login is pinned, `--login <name>` (D4.1c).

#### D2.4 Verb catalogue

Existing verbs keep their shapes, with one change: `release` moves off gh onto the plugin's own HTTP client (D2.8, and contradiction 8 in section 7). New verbs:

`token --remote <url> | --host <h> [--login <name>]`, exit 0 with one JSON object:

```
{"known":true,"host":"github.com","login":"scotty","token":"gho_...","username":"x-access-token",
 "source":"keychain|gh|glab|tea|env","scopes":"repo user:email","expires_at":null,"chose_by":"pin|memory|only|probe"}
{"known":false,"reason":"no-login|no-keychain|unsupported-host|no-login-for-repo|busy"}
```

The `login` and `chose_by` fields are part of the shape, not decoration: D4.1c requires every plugin answer that names a token to say which login it belongs to and which step of the login order chose it, and `logout`, `joy forge status` and the Device host row all read them. J3 builds these verbs with the shapes of D2.4 as amended by D4.1c.

Sources per forge: the plugin's own keychain entry, then `gh auth token [--hostname H] [--user U]`, `glab auth credential-helper` (hidden, JSON with `expiry_timestamp`), `tea login helper get` over stdin (it refreshes OAuth tokens on the way). `glab auth token` does not exist and `tea logins list` prints no token. The config discovery of all three must be fixed first: gh (`GH_CONFIG_DIR`, `XDG_CONFIG_HOME/gh`, `%AppData%\GitHub CLI`, `~/.config/gh`), glab (`GLAB_CONFIG_DIR`, `~/.config/glab-cli`, then XDG per platform including `%LOCALAPPDATA%\glab-cli`), tea (XDG per platform, then `~/.tea/tea.yml`). Today joy looks only in `~/.config/...` (github.rs:102-110, gitlab.rs:86-93, gitea.rs:86-95).

`login --remote <url> | --host <h> [--for read|write|create|release] [--login <name>]`, newline delimited JSON on stdout, one object per line, each flushed explicitly:

```
{"event":"verification","host":"github.com","url":"https://github.com/login/device","url_complete":null,"code":"WDJB-MJHT","expires_in":900,"interval":5}
{"event":"waiting","seconds_left":870}
{"event":"slow_down","interval":10}
{"event":"result","known":true,"login":"scotty","user_id":"12345","emails":["s@example.com"],"scopes":"repo user:email","stored":"keychain|file","expires_at":"2026-09-16T18:00:00Z"}
{"event":"error","code":"access_denied|expired_token|unsupported|network|device_flow_disabled|invalid_scope","message":"..."}
```

The plugin never opens a browser and never prints the token during login. The host decides: the desktop opens the URL, the CLI prints it, a delegated session never opens anything and the verb is refused before it starts (D3.11).

`token-store --host <h> [--login <name>]` reads one token from **stdin**, validates it with `identity`, stores it and answers the same object as `token`. This is the headless door named in D2.7 and missing from v2: a Linux server, a CI runner, a Windows host with no browser, and every Gitea family instance whose operator registered no OAuth client. The token is never an argument.

`logout --host <h> [--login <name>]` answers `{"removed":true|false,"revoked":true|false,"source":"keychain|file|gh|glab|tea"}` and revokes the token at the forge where the forge offers it (`DELETE /applications/{client_id}/token` for GitHub, never `.../grant`). When the credential came from a foreign CLI, the plugin removes nothing and names the foreign command.

`repositories --host <h> [--query s] [--limit n] [--page cursor]` answers `{"state":"repositories","repositories":[{...}],"truncated":bool,"next":null|"cursor"}`. Default limit 200, paginated so one answer stays well under 64 KiB.

`web-url --remote <url>` answers `{"known":true,"https_url":"https://git.acme.com/team/sub/repo.git"}` or `{"known":false}`. This is the twin source of D1.5.

`create-repository --host <h> --name <n> [--owner <o>] [--private]` answers `{"created":true,"clone_url":..,"ssh_url":..,"default_branch":..}` or an error object. A project can only be brought to joyint.com when it has a remote repository, so this verb is what makes "picks or creates a repo" complete.

`store` gains one optional field, `"size_bytes": <integer>`, normalised to bytes inside the plugin because the unit is forge knowledge: GitHub's REST repository `size` is kilobytes and "Size is calculated hourly. When a repository is initially created, the size is 0."; GitLab returns `statistics.repository_size` in bytes only when `statistics=true` is passed and the caller holds at least the Reporter role; Gitea's API `size` is KiB (services/convert/repository.go:205). The field is optional and its absence is not an error.

#### D2.5 Instance configuration for self hosted forges

Plugins keep no instance in their code. Today a plugin claims a self hosted host only when gh, glab or tea is already signed in to it (github.rs:24, :31, gitea.rs:21), which makes D2 circular for an enterprise. Two configuration sources are added:

- `~/.config/joy/forges.yaml` (and the same file under the platform's config directory): a list of `{host, kind, api_base, web_base, client_id, device_endpoint?, auth_endpoint?, token_endpoint?, ca_bundle?, ca_dir?, scopes?}`. An operator ships it with the workstation image.
- The existing project level `forge:` override in project.yaml (forge_plugins.rs:221) keeps working and wins for that project.

`claims` consults both, so an internal forge is claimed without gh, glab or tea. The plugin's help text stops naming `joy forge setup` and names `joy forge login`, which now exists (D3.10).

#### D2.6 Token storage, and who may read which entry

The keychain entry is owned by the plugin binary alone. The app never reads or writes it; it calls the same signed binary for `login`, `token`, `token-store` and `logout`. On macOS this is the only arrangement that avoids a confirmation dialog, because a generic password is created with an ACL that trusts only the creating application ("By default, the application which creates an item is trusted to access its data without warning", https://keith.github.io/xcode-man-pages/security.1.html).

- keyring features: `["apple-native", "windows-native", "linux-native-sync-persistent", "crypto-rust"]`. The shipped desktop manifest uses `linux-native` (app/apps/desktop/src-tauri/Cargo.toml:32), which is the kernel keyutils store: in memory, "will not persist across reboots" (keyring-3.6.3/src/keyutils.rs:19-23).
- **Entry addressing, decided.** `Entry::new(service, user)` only, never `new_with_target`, and **joy does not read a foreign CLI's store at all**. This resolves the v2 contradiction: reading `glab:<host>:token` or gh's item would need `new_with_target` on Windows, and on macOS a direct read from a different binary risks an allow or deny dialog (cli/cli docs/macos-keyring.md). Instead the plugin **spawns the CLI** (`gh auth token --hostname H --user U`, `glab auth credential-helper`, `tea login helper get`), which is also the only way the CLI's own refresh runs. A foreign CLI credential is therefore read only for joy: joy never refreshes it, never writes it and never revokes it, and `logout` names the foreign command instead. The service is the plugin's own name, the user is `<host>` or `<host>|<login>`.
- Fallback file, written by joy because the crate has none: `~/.config/joy/forge-tokens.json`, mode 0600, directory 0700, used on `NoStorageAccess`, on `PlatformFailure` and on any target where the crate would degrade to its in process mock. The UI and the docs say so, with gh's sentence as the model ("If a credential store is not found or there is an issue using it gh will fallback to writing the token to a plain text file").
- The platform does not link keyring at all. The default Docker seccomp profile blocks `add_key`, `keyctl` and `request_key`, and a container has no session bus.

#### D2.6a The refresh lock

Nothing cross process exists today. `checkout_gate` is a per process map of mutexes and says so itself: "Per process; cross-process safety is git's own ref locking plus the store's compare-and-swap" (forge.rs:559-583). A grep for fs2, fd_lock, flock and lockfile over joy/crates finds only release prose, and joy's Cargo.lock carries no file lock crate, so the crate itself (fs4 or fd-lock) is new. It lands exactly once, in wave 0 with J4a, as `joy_core::util::file_lock` (a cross process advisory whole file lock on fs4) beside the per process map of `checkout_gate` (forge.rs:558-584); J4h takes it for the known_hosts append and J3 reuses it here for the refresh lock, so no package after J4a carries a lock dependency of its own for locking.

Because D2.6 makes the plugin the only process that touches joy's own entry, the lock lives in the plugin and is taken by every `token`, `login`, `token-store` and `logout` call that may write. One exclusive whole file advisory lock per host and login. The file is `<app_state_dir>/locks/forge-<first 16 hex of SHA256(host|login)>.lock` under `joy_core::auth::session::app_state_dir()` (session.rs:257-271), not under the config directory, because joy's config base is `%APPDATA%` on Windows (store.rs:127-139), which roams.

Protocol: open or create; take the exclusive lock with a bounded wait (a non blocking attempt in a 50 ms backoff loop, 10 s total); re read the entry; refresh only if it still carries the same access token fingerprint (the first twelve hex digits of its SHA-256, the shape platform/src/auth/mod.rs:571-577 already uses) and is still expired against a 60 s skew; write; release by dropping the handle; never unlink the lock file. On a timeout do not refresh: re read once, use the entry if it is now valid, else answer `{"known":false,"reason":"busy"}`.

Windows: fs4 calls `LockFileEx` with `LOCKFILE_EXCLUSIVE_LOCK` over the whole range. Such a lock is mandatory for the region ("Locking a portion of a file for exclusive access denies all other processes both read and write access to the specified region of the file."), it is released when the handle closes or the process dies, but "the time it takes for the operating system to unlock these locks depends upon available system resources", which the bounded wait plus the re read absorbs. `LockFileEx` is supported over SMB 3.0, so a redirected `%LOCALAPPDATA%` still works.

macOS and Linux: `flock(2)`, which is advisory ("processes may still access files without using advisory locks possibly resulting in inconsistencies") and whose "Locks are on files, not file descriptors ... If a process holding a lock on a file forks and the child explicitly unlocks the file, the parent will lose its lock". The plugin therefore reads gh, glab and tea answers **before** it takes the lock and never spawns them while holding it. Where the lock cannot be taken at all, the plugin degrades to "do not refresh, use the entry as it stands, report busy", never to "refresh anyway".

The honest limit: no file lock binds gh, glab or tea, which write their own stores from their own processes. That is the second reason joy treats foreign entries as read only. The platform learned this the hard way and the reason belongs here: a rotating refresh token refreshed twice at once killed Codeberg tokens for an hour on 2026-08-29, and ten thousand refresh attempts against one dead GitHub refresh token got the whole OAuth app throttled on 2026-09-07 (platform/src/auth/mod.rs:585-600).

#### D2.7 OAuth per forge

GitHub (github.com): device authorization grant. `POST https://github.com/login/device/code` with `client_id` and the scope set of D2.7a, `Accept: application/json`; poll `POST https://github.com/login/oauth/access_token` with `grant_type=urn:ietf:params:oauth:grant-type:device_code`, no client secret; honour `interval`, `slow_down` (plus 5 s), `expired_token`, `access_denied`, `device_flow_disabled`. Device flow must be enabled on the app registration. Since 2026-08-14 a newly registered OAuth App defaults to 8 h access tokens with a 6 month refresh token, so either "Expire user access tokens" is unchecked at registration or the plugin implements refresh; `offline_access` is never requested unless refresh exists.

GitHub Enterprise Server: the endpoints are instance local (`https://HOSTNAME/login/device/code`), the client id is instance local, and the app must be registered on the instance. This is what `forges.yaml` carries.

GitLab: device grant from 17.3 (generally available 17.9). `POST {base}/oauth/authorize_device`, poll `POST {base}/oauth/token`. gitlab.com's OIDC discovery does not advertise a device endpoint, so the path is hardcoded. The application must be registered with Confidential off and **with the union of every scope set joy may request**, because since the fix for issue 543138 (merge request 200352, merged 5 August 2025) a device request may narrow but never widen: a scope outside the application's set is refused with `{"error":"invalid_scope","error_description":"The requested scope is invalid, unknown, or malformed."}` before any verification code is shown.

Gitea, Forgejo, Codeberg: no device grant exists in any released version. The door is authorization code with PKCE S256 on a loopback listener. The redirect URI must be registered as exactly `http://127.0.0.1` (no port, no path) and bound to an ephemeral port at runtime. Access tokens live 3600 s, refresh tokens 730 h, and Forgejo rotates the refresh token on each use. The login request must always name at least one non OIDC scope: a grant carrying only openid, profile or email produces `AccessTokenScopeAll`, a token with the person's full rights (services/oauth2_provider/access_token.go `GrantAdditionalScopes`).

Headless hosts on the Gitea family sign in with `token-store` (D2.4) or with the machine's credential helper.

#### D2.7a Scope sets per forge, per verb group

The verbs fall into seven groups: A identity (verified emails), B repository facts (`store`, `files`), C git read over https, D git write over https, E list repositories, F create repository, G release with assets. `claims`, `resolve`, `web-url` and `version` make no network call and need no scope.

**GitHub (github.com and GHES, identical names).** One set covers A to G: `repo user:email`. A needs `user:email`; B, C, D, E and G need `repo` for private repositories; F needs "public_repo or repo scope to create a public repository, and repo scope to create a private repository"; G needs `workflow` only "when the resolved target commit modifies workflow files", which joy never does. `read:org` is not in the set, because `GET /user/repos` already covers "repositories that they can access through an organization membership". A public only variant `public_repo user:email` exists. There is no read only private scope on GitHub, so the minimal scope demand cannot be met with an OAuth App; it is met later by a GitHub App or a fine grained token with Contents read, and the design says so instead of promising it now.

**GitLab.** v2's set was wrong in both directions and is replaced by three sets:

| Set | Scopes | Covers |
| --- | --- | --- |
| read only member | `read_api read_repository` | A, B, C, E |
| read write member | `read_api write_repository` | A to E |
| full | `api write_repository` | A to G |

`write_repository` "Uses Git-over-HTTP. Does not support API authentication.", so `create-repository` (POST /projects) and the Releases API need `api`. This is the resolution of the v2 contradiction: with the read write set, `create-repository` is answered locally with `scope_missing`, and `joy forge login --for create` widens it. `read_repository` covers C and the raw file read of B; `GET /projects/:id`, the tree and `protected_branches` are plain API GETs covered by `allow_access_with_scope :read_api, if: ->(request) { request.get? || request.head? }` (lib/api/api.rb). `read_user` is redundant beside `read_api`, and `read_repository` is redundant beside `write_repository`. The registered application carries the union `api write_repository`.

**Gitea, Forgejo, Codeberg.** Scopes are per category and the HTTP method picks the level ("use the http method to determine the access level", routers/api/v1/api.go): read only `read:user read:repository` (A, B, C, E); read write `read:user write:repository` (A to E and G, because releases live under /repos); create repository `write:user write:repository`, because `POST /user/repos` is checked twice, by the /user group (`CategoryUser`) and by the route (`CategoryRepository`). git over https uses the same categories (`CheckRepoScopedToken` with `GetScopeLevelFromAccessMode`: fetch read, push write).

The platform's own Gitea authorize URL sends no scope at all today (platform/src/auth/providers.rs:331-339) and therefore holds full rights tokens; it is corrected in the same package to `read:user write:repository` (plus `write:user` only where the platform creates repositories).

#### D2.7b What the person sees on the consent page

GitHub publishes no verbatim strings. The docs state "Requested scopes are displayed to the user on the authorization form", that the page shows "the app's developer contact information and a list of the specific data that's being requested", and that the person "will also see how the authorization will affect each organization you're a member of". The design does not promise particular on screen words for GitHub; it promises that the requested set is exactly `repo user:email` and that the product explains both lines in its own words before the browser opens.

GitLab shows the scope descriptions verbatim, one per requested scope (config/locales/doorkeeper.en.yml). A read only member sees "Grants read access to the API, including all groups and projects, the container registry, and the package registry." and "Grants read-only access to repositories on private projects using Git-over-HTTP or the Repository Files API."

Gitea and Forgejo show `Authorize "<app>" to access your account?`, then `With scopes: <scopes>`, then `You will be redirected to <domain> if you authorize this application.` When no non OIDC scope is requested they instead show "If you grant the access, it will be able to access and write to all your account information, including private repos and organisations." Seeing that sentence is the signal that the request was built wrong.

#### D2.7c The read only member and `scope_missing`

A read only member gets the read only set of their forge and keeps every verb except push, `create-repository` and `release`. On GitHub there is no read only set, so a read only member on GitHub is told that GitHub grants read and write in one scope and that joy still never pushes without an explicit action.

The plugin records the granted scope set beside the token in the same entry, as a space separated string. GitHub returns it in the device token response, GitLab in the doorkeeper token response; Gitea's `AccessTokenResponse` has no scope field, so for Gitea the plugin stores the set it requested. Before any verb that needs a scope the set does not carry, the plugin answers locally, without spending a request:

```
{"state":"scope_missing","host":"gitlab.com","verb":"create-repository","needed":["api"],
 "have":["read_api","write_repository"],"next":"sign in again with wider access"}
```

on exit code 0. This is an answer, not a failure, and the host renders it as one sentence with one button. When the local pre check passes and the forge still refuses, the plugin classifies and never reports `denied` for a scope problem:

- GitHub: 403 whose `X-Accepted-OAuth-Scopes` names a scope missing from `X-OAuth-Scopes` is `scope_missing`; 403 with an `X-GitHub-SSO` header is `needs_sso` and the header carries the URL to follow; 403 naming OAuth App access restrictions is `needs_org_approval`; everything else is `denied`. One extra rule is mandatory: GitHub answers 404 rather than 403 for private resources ("GitHub uses a 404 Not Found response instead of a 403 Forbidden response to avoid confirming the existence of private repositories"), so `store` must not map 404 to `gone` when the request carried no token or a token without `repo`. Today `store_verdict` maps that 404 straight to `{"state":"gone"}`, which tells the person their repository does not exist.
- GitLab: 403 whose WWW-Authenticate carries `error="insufficient_scope"` is `scope_missing`; any other 403 is `denied`; 404 is `gone` only when the set contains `read_api` or `api`.
- Gitea and Forgejo: 403 whose body reads "token does not have at least one of required scope(s), required=..., token scope=..." is `scope_missing`, and the `required=` list is parsed into `needed`.

#### D2.8 Plugins get a real HTTP client

The device and PKCE flows, `repositories`, `create-repository` and now `release` need an in process HTTP client with TLS (ureq or reqwest with rustls). Today every forge call is a `curl` or `gh` subprocess (github.rs:159-171, :398-438), and `release` reaches the API entirely through gh (`gh --version`, `gh auth status`, `gh release view/edit/create`, github.rs:279-346). **`release` moves to REST in the same package** (`GET /repos/{o}/{r}/releases/tags/{tag}`, `POST /repos/{o}/{r}/releases`, `PATCH /repos/{o}/{r}/releases/{id}`, plus the asset upload on uploads.github.com), which is what turns "Scotty publishes a release from the CLI" from a journey that still needs gh into one that does not. Two leaks are fixed in the same package: `verified_emails` passes the bearer token as a curl argument (github.rs:153, gitlab.rs:152), and the GitLab plugin asks `https://gitlab.com/api/v4/user/emails` even for a self hosted instance (gitlab.rs:153); the GitHub plugin has the same bug against `https://api.github.com/user/emails` on GHES (github.rs:159-171).

Because the client is new, D1.11 and D1.12 apply to it too: the client honours the same proxy sources and the same OS trust store, and the plugin refuses a SOCKS proxy with the same sentence.

#### D2.9 Plugin class

The forge plugins become connectors: own authentication, own state (one keychain entry per forge host and login), named side effects per verb. docs/plugins.md's three sentences ("Reads, no writes", "read only and side effect free except release", "no token in joy's hands") are replaced by a connector section. ADR JOY-0251-AA needs no change: it never forbade plugin state.

#### D2.10 The rate limit oracle, per forge

The plugin is consulted after a git contact only when all of these hold: the status was exactly 403 or 429; the transport was https; the host is a GitHub host; and no oracle call has been made for this host inside the current strike window (600 s). The call is `GET /rate_limit` and nothing else, because GitHub documents it as free of the primary limit ("Calling this endpoint does not count against your primary rate limit, but it can count against your secondary rate limit"). The answer chooses between `rate_limited` with a wait time from `x-ratelimit-reset` or `retry-after`, `needs_org_approval`, and `denied`.

The oracle is **not** consulted on GitLab hosts. GitLab's API bucket is separate from the Git HTTP bucket ("General user and IP rate limits aren't applied to Git HTTP requests"), so a REST call is harmless for the bucket but cannot answer the question: what produces a GitLab 403 on git with a valid token is the failed authentication ban, which applies only to git and container registry requests, "Cannot be cleared by authenticating once a ban has started", and provides "No response headers". Defaults to quote in the sentence: 10 failed authentication requests in a 1 minute period from one IP produce 403 for 1 hour, all three values configurable and disabled by default self managed; gitlab.com "responds with HTTP status code 403 for 15 minutes when a single IP address sends 300 failed authentication requests in a 1-minute period". The plugin is asked at most once for token validity (`GET /user`), not for a limit verdict.

The oracle is **never** consulted on Codeberg, Forgejo or Gitea hosts after a 403 or 429. Forgejo ships no HTTP rate limit key at all (its configuration cheat sheet has no "rate limit", "ratelimit", "throttle" or "429" entry), and Codeberg's limiter is a per IP reverse proxy limiter in front of the whole site, quoted by an admin as "Currently: 2000 requests / 300s". A REST probe rides the same per IP bucket that just refused the git request.

Why this matters for D1.9: joy's own amplification is real. `Auth::Token` tries three credential shapes per contact (forge.rs:124-148) and libgit2 allows fifteen replays per stream (http.h:14), so one contact with a dead token is three or four 401s. At the desktop's 1 Hz poll a dead GitLab token reaches 10 failures in about 3 seconds on a self managed instance with the ban enabled, and 300 failures in about 100 seconds on gitlab.com. D1.7's "one re ask, then stop" is what keeps this bounded.

### D3: CLI user and agent (G1, G2)

#### D3.1 Packaging fact that must land first

joy-cli does not enable joy-core's `forge-net` feature today; only the desktop does (joy-core/Cargo.toml:89, app/apps/desktop/src-tauri/Cargo.toml:18). Moving the CLI to git2 for chat transfer, push and tags is therefore also a packaging change: joy-cli turns the feature on, which pulls vendored OpenSSL on unix into every CLI build and into `cargo install joy-cli`. Without the flip, every CLI forge contact fails with an unsupported protocol message that the classifier reads as `error`.

#### D3.2 The git2 only move

The calls that leave the git binary (vcs/mod.rs:250-283, chat_ref.rs:164-196): `add -A`, `commit`, `tag -a`, `push`, `push_tag`, `push_with_tags`, `status --porcelain`, `describe`, `ls-files`, `rm --cached`, `rm`, `log -1 --format=%ct`, `fetch`/`push` of one refspec, and `gc --auto`. Their replacements are the existing engine functions plus the maintenance of D3.7. The `gc --auto` spawn in `chat_ref::maintain_occasionally` is both a constraint violation and a hazard (D3.7), and it goes first.

#### D3.3 The item reference rule in process

`joy_core::commit_msg::validate(message, acronym) -> Result<(), CommitMsgError>` reproduces the bash rule exactly: merge commits exempt, `[no-item]` bypass, the pattern `ACRONYM-[0-9A-Fa-f]{4}(-[0-9A-Fa-f]{2})?`, the ADR-015 diagnostic text. It is called by every joy commit path: `git_ops::auto_git_post_command`, joy-cli `release.rs:295`, `forge::commit_joy`, `commit_all`, `commit_everything`, `commit_paths`. The bash hook stays for the person's own `git commit` and shares one definition with the validator so the two cannot drift. For joy's own automatic commits the validator warns and proceeds (a refusal would strand the write behind worker with uncommitted .joy changes); for a person's `joy` command it refuses. The `Delegated-By` and `Co-Authored-By` trailers that the prepare-commit-msg hook adds for an agent are produced by joy's own commit path too.

#### D3.4 joy's commits touch only joy's paths

`commit_index` commits the whole index and moves HEAD (forge.rs:991-1020), so today a joy commit can sweep a half finished `git add -p` into a "joy: ..." commit, and after D3.2 no pre-commit hook stands in the way. joy's own commit paths stage and commit only the paths joy wrote. `commit_all` and `commit_everything` keep their semantics for the platform's own checkouts, which joy owns, and are not used in a person's checkout. The same rule bounds the `.gitattributes` clean filter risk: libgit2 runs no external filter, so a joy commit of a path under git-lfs would write content instead of a pointer, and joy never commits such a path in a person's checkout. `commit_all` and `commit_everything` are audited for this in J6.

#### D3.5 Hooks, one rule

**Decided: joy owns `core.hooksPath` and chains.** The v2 paragraph contradicted itself (do not set the value, and chain from it) and the chain is only possible if joy owns the path, so the rule is:

- `joy init` and `joy update` set `core.hooksPath` to `.joy/hooks` as they do today (init.rs:255-271, update.rs:330-365), and they record the value that was there before in `.joy/hooks/chained-path` (a file joy writes, not a git config key, so it travels with the checkout of the person who has it and never with the team).
- Every hook joy installs ends by executing the same hook name from the chained path, and where there was no previous value, from `$GIT_DIR/hooks`. The chained hook's exit code is the hook's exit code when joy's own check passed.
- joy says this once, at `joy init` and at the first `joy update` that finds a foreign path: "joy installed its hooks and kept yours: <path> still runs after joy's."
- Reason: `core.hooksPath` replaces the location entirely (https://git-scm.com/docs/git-config), so the alternative (not setting it) means joy's commit-msg check is absent from the person's active hook path and the item rule is enforced for nobody on exactly the team repositories D3.3 is written for. Chaining keeps husky, lefthook and pre-commit alive, which is what G1 asks for.
- On Windows both hooks are bash and run only under Git for Windows' bundled sh; a machine without git has no hooks at all and gets the in process validator of D3.3 instead.

#### D3.6 Signing

libgit2 1.9.6 has no signing and reads no `commit.gpgsign`. joy's own commits are therefore unsigned, on every host, from the day the CLI moves. Consequences for the docs and for Picard: a branch with "require signed commits" refuses joy's commits and every platform job branch. A later signing path exists (`Repository::commit_create_buffer` plus `commit_signed`, repo.rs:1376-1453) but reintroduces a passphrase prompt on headless hosts, so it is a separate item. Counterweight: today joy runs `git commit` headless with `GIT_TERMINAL_PROMPT=0`, which does not disarm gpg's pinentry, so a signed automatic commit can already hang.

#### D3.7 Maintenance replacing `git gc --auto`

Packing alone is not maintenance. On the measured store, 6140 loose objects and 38.89 MiB held 1703 reachable objects worth 0.7 MiB and 4437 unreachable objects worth 22 MiB (JOY-023C-1E:16). joy packs and sweeps, and the sweep is safe beside another git or libgit2 process for reasons read out of libgit2, not assumed.

**Trigger.** Per canonical checkout, not the process global counter that fires on the first write of every process (chat_ref.rs:164-195). The gate is a cheap loose object estimate (count one fanout directory, multiply by 256, as git does), a wall clock floor of ten minutes per checkout, and git's own threshold of 6700 loose objects. Maintenance runs inside `forge::checkout_gate(root)` (forge.rs:558-584).

**Step 1, keep set.** The closure of: every reference under `refs/*` and `HEAD`; every `id_old` and `id_new` of the reflogs of `HEAD` and `refs/heads/*` (`Repository::reflog`, `Reflog::iter`, `ReflogEntry::id_old`/`id_new`); every index entry oid. This is git's own list ("objects referenced by the index, remote-tracking branches, reflogs ... and anything else in the refs/* namespace"). git2 offers no reachability query (only `graph_ahead_behind` and `graph_descendant_of`; `git_graph_reachable_from_any` is unbound), and `Odb::foreach` returns loose and packed oids mixed with no mtime, so the keep set is built explicitly and the sweep readdirs `.git/objects/xx` itself. The consequence that makes this worthwhile: `refs/joy/chats` gets no reflog (refdb.c:303-338 logs only `refs/heads/*`, `refs/remotes/*`, `refs/notes/*` and `HEAD`), so the chat store's lost compare and swap commits are not pinned by any reflog and are exactly the 72 percent the sweep reclaims, while joy's own HEAD commits stay alive for the reflog window. If a customer sets `core.logAllRefUpdates=always`, the keep set grows to include them and the sweep reclaims nothing; joy detects that config and says so once rather than silently doing nothing.

**Step 2, pack.** `git2::PackBuilder`, `insert_commit` for every keep set commit and `insert_recursive` for the rest, then `write(repo.path().join("objects/pack"), 0)`, then `Odb::refresh()` on every live odb in the process. The indexer publishes the `.idx` before renaming the `.pack` (indexer.c:1387-1430); both git and libgit2 tolerate that window because the pack backend enumerates `.idx` files (odb_pack.c:235-247).

**Step 3, sweep, two classes.**
- Class A, an object present in the pack joy just wrote and whose `.pack` exists: unlink at any age.
- Class B, an object not in the keep set: unlink only when its mtime is older than the grace window.
- Never delete a pack, ever, not even one joy wrote: `pack_window_open` reopens a pack by name when the mwindow LRU has closed its descriptor (pack.c:338-372, :1099), so a pack removed under a live odb becomes a hard read error. Pack consolidation is a later item; git's own `gc.autoPackLimit` is 50.

**Why this is safe next to another process.** The ordinary loose read is stat, open, one full read, close, with no lasting handle and no mmap (odb_loose.c:618-640, futils.c:221-243). `locate_object` checks existence and `read_loose` opens separately, and a file that vanished in between maps to `GIT_ENOTFOUND` (fs_path.c:727-733), which `git_odb_read` answers with `git_odb_refresh` and a second pass restricted to backends that have a refresh function (odb.c:1398-1421). The loose backend has none and the pack backend does (odb_loose.c:1195-1213, odb_pack.c:912), so the retry resolves out of joy's new pack. A write by another process is equally correct: `git_odb_write` short circuits through `git_odb__freshen`, whose loose branch is a `utimes` that now fails, so it falls through to `pack_backend__freshen`, which touches the pack instead (odb.c:1628-1629, odb_loose.c:1128-1143, odb_pack.c:598-610). What this does not cover is the person's own `git gc --prune=now` and an agent's `git commit` in a container that shares the object store; those carry the residual risk git itself admits to ("these features fall short of a complete solution, so users who run commands concurrently have to live with some risk of corruption (which seems to be low in practice)").

**What "age" means.** mtime, and nothing else. relatime and noatime are atime policies and never suppress an mtime update on a write or on `utimes`. Two cases make the sweep skip rather than proceed: a clock skewed mount (an object whose mtime is in the future) and a `utimes` the mount or the ownership refuses, because then another process cannot freshen and its protection is gone.

**Grace window.** 14 days, git's default, in any checkout joy does not own. 24 hours in the platform's project clones and in a desktop only store where joy is the sole writer. Never `now`, on any host.

**Windows.** Every unlink is best effort per file. libgit2 opens files with `FILE_SHARE_READ | FILE_SHARE_WRITE` and never `FILE_SHARE_DELETE` (posix_w32.c:43-44), and "The DeleteFile function fails if an application attempts to delete a file that has other handles open for normal I/O or as a memory-mapped file". joy treats raw OS error 32 (ERROR_SHARING_VIOLATION) and 5 (ERROR_ACCESS_DENIED) as "leave it, next run", counts them and never aborts, which is what git's own prune does (`unlink_or_warn`; `mingw_unlink` retries at 0, 1, 10, 20, 40 ms).

**Cost, measured.** On a probe store built to the JOY-023C-1E shape (1000 reachable of 3500 loose objects, 14 MB of `.git`), the keep set walk takes under 10 ms warm with 847 file opens, enumerating and stat-ing every loose file takes under 10 ms, and packing the keep set produces 130 KB in 0.04 s; at 3500 reachable it is 0.02 s, 2850 opens and 0.15 s. Scaled to the real 1703 reachable objects that is roughly 1400 opens and well under 100 ms, dominated by the pack write. The ten minute floor is two orders of magnitude more conservative than the cost requires.

**Two hazards to close while here.** No process may run `git gc` against a shallow joy checkout, because an externally written commit-graph bypasses the shallow grafts (commit_list.c:179-204); and `chat_ref::maintain_occasionally` spawns `joy_process::command("git")` with `gc --auto` today, which is both the constraint violation and this hazard. The root cause of the garbage is also fixed where it is cheap: `commit_root` writes the commit object before the compare and swap (chat_ref.rs:209-232), so every lost race of up to eight attempts leaves a commit and its trees behind.

**Acceptance.** A store like the operator's sandbox ends below 1 MB with every chat intact and stays under 6700 loose objects; a second process holding an object open does not break the sweep; no object younger than the grace window is removed.

#### D3.8 Agent rules (G2)

- Non interactive is decided by the host kind of D1.1 and by `JOY_SESSION`, not by a TTY.
- `GIT_TERMINAL_PROMPT=0`, pinned into the agent environment today (agent_ops.rs:594-624), stops steering joy's own contacts and keeps steering the agent tool's own git calls. The same holds for `GIT_AUTHOR_*` and `GIT_COMMITTER_*`.
- The CLI gets one failure vocabulary an agent can read: the `contact::Failure` words plus the new states of D1.8, available as `--json` on every command that contacts a forge. The private classifier in joy-cli commands/chat.rs (`classify_sync_error`) is retired.
- A CLI command may now wait for the throttle of D1.9 before it contacts a forge (up to 2.2 s on Codeberg for an authenticated ls-remote). That is a visible latency change for a terminal user and is stated in the docs.

#### D3.9 What stays on git config, and what does not

Twenty call sites of `user_email()` remain in joy-cli (measured on 2026-09-17: 20 hits in 9 files, auth, crypt, chat, board, project, ai, event log, enroll). "joy needs no git config" is therefore not yet a true sentence for the CLI, and the docs may not claim it. Two parts move now, because they are the ones the level 1 journey stands on:

- `joy init` takes `--user <address>` (it already does, init.rs:59-96), and on an `Interactive` host with no git config it asks for the address instead of failing with `NoFounderIdentity`. On a `Background` or `Delegated` host it refuses with the named sentence "this project does not know who you are; run joy init --user <address>".
- `joy auth init` and the enrollment path take the member explicitly (`redeem_with_passphrase`, enroll.rs:199), so a founder created without git config can enrol.

The remaining call sites are package J11 (wave 3, depends on J9): they move onto `joy_core::identity::resolve_identity`, which reads the session first, then the project's member pin, and git config only as a prefill. The journey map grades level 1 partly until J11 lands, and the row names J11 rather than an unnamed follow up.

#### D3.10 The CLI's door to a forge (new)

The CLI persona gets one command group, `joy forge`, built as a nested clap `Subcommand` inside a `ForgeArgs` struct exactly like `joy auth` (auth.rs:20-66) and registered as one variant in `Commands` (lib.rs:140-214). Nothing collides: `forge` exists today only as the `--forge` option of `joy release publish` (release.rs:75-80) and as the `forge:` key in project.yaml. The word is in fact already promised by texts that point at nothing: joy-github prints "run `joy forge setup`" twice (github.rs:297, :310) and docs/dev/vision/ForgeSync.md names `joy forge setup` and `joy sync` throughout, while joy-cli has neither. The group retires both lies; the two help strings become `joy forge login`, and the vision document is marked superseded.

- `joy forge login [--host <H> | --remote <URL>] [--token-stdin] [--for read|write|create|release] [--login <name>]`. With neither `--host` nor `--remote`, the host comes from the current project's remote through the plugin's `claims`. Without `--token-stdin` it runs the plugin's `login` through the streaming runner: the first `verification` event is printed to stderr as two lines (the URL and the code) plus a countdown from `expires_in`; the CLI never opens a browser; the last event decides the outcome. With `--token-stdin` it reads one line from stdin, strips the trailing CR/LF, refuses an empty line, and hands the token to `token-store`, which validates it with `identity` before storing. The token never appears in argv, which is the rule and the wording `--passphrase-stdin` already carries (auth.rs:232-266, JOY-018E-21). When stdin is a terminal and `--token-stdin` was given, the line is read without echo through rpassword. There is no `--token <value>` and there will not be one.
- `joy forge status [--host <H>]`. One row per host: host, forge id, login, credential source, expiry, granted scopes, and the plugin that answered with its resolved path and protocol. The host set is the union of the hosts in `forges.yaml`, the hosts of the current project's remotes, and the hosts that hold a stored credential.
- `joy forge logout [--host <H> | --all]`. Calls the plugin's `logout`. When the source is a foreign CLI, joy names the foreign command: "the token for github.com comes from gh; run `gh auth logout --hostname github.com` to remove it".
- `joy forge plugins`. Diagnostics, no network: one row per registry id with the resolved binary path, the search step that found it, the protocol and version it answers, and a `problem` word when a binary is shadowing or outdated.

`joy forge token set` is rejected as a second door for the same result: it duplicates `joy forge login` and "token" already means a delegation token in this CLI (`joy auth token`, auth.rs:50-66). One door, two sources.

Output and exit codes follow what exists. Human mode prints to stdout, all progress and diagnostics to stderr. `--json` is the global flag (lib.rs:403-407) and every answer is exactly one envelope `{"version":1,"data":{...}}` through `output::emit` (output.rs:27-88), so stdout carries one object and nothing else. Exit codes: 0 on success; 2 stays reserved for clap usage errors (`USAGE_CODE = 2`) and for `joy update`'s "stale" meaning (update.rs:237); every forge failure is 1, and the distinction is carried by the `state` word, which is the one failure vocabulary D3.8 promises an agent. In `--json` mode the envelope is printed first and the process then exits with the code, exactly as `joy auth status` does (auth.rs:720-731).

The `--json` shapes:

```
login   {"host":"github.com","state":"signed-in","login":"scotty","user_id":"12345",
         "emails":["s@example.com"],"source":"device|pkce|token","stored":"keychain|file",
         "scopes":"repo user:email","expires_at":"2026-09-16T18:00:00Z"}
        {"host":"codeberg.org","state":"needs_sign_in|cancelled|expired|denied|unsupported|
         plugin_missing|plugin_outdated|scope_missing|offline|rate_limited|error","message":"...","action":"..."}
status  {"hosts":[{"host":"github.com","forge":"github","login":"scotty","state":"signed-in|expired|none",
         "source":"keychain|file|gh|glab|tea|env|none","scopes":"repo user:email","expires_at":null,
         "plugin":{"id":"github","binary":"joy-forge","path":"/home/s/.local/bin/joy-forge","protocol":2}}]}
logout  {"host":"github.com","removed":true,"revoked":true,"source":"keychain"}
plugins {"plugins":[{"id":"github","binary":"joy-forge","path":"/home/s/.local/bin/joy-forge",
         "found_in":"exe-dir","protocol":2,"version":"joy-forge 0.21.0","problem":null}]}
```

`joy forge status` exits 1 when no host in its set is signed in. `joy forge login` exits 1 on every state other than `signed-in`.

Four failure sentences change to name the new door, and they are the only ones: joy-cli/src/forge.rs:52-56 (release, plugin failed) gains "run `joy forge plugins` to see which binary answered"; forge.rs:198-203 ("no supported forge detected") gains "or run `joy forge login --host <host>`"; commands/chat.rs:138-142 and :168-172 (chat push and fetch) print the classifier's action sentence, and for `needs_sign_in` that sentence is "run `joy forge login --host <host>`". There is no `joy sync` command to change (the CLI's sync path is `sync_ref` inside chat.rs:149).

Under JI-017C-E1 this group is planned work, not a switch beside a broken path, because no CLI sign in path exists to stand beside. Every flag, option and feature this design adds is enumerated in decision 29 and named by one approved JI item with the working title "forge connection NG: new commands, flags and features", which has to exist before the first commit of J1 and of J10. That is exactly what the item's enforcement clause asks for.

#### D3.11 The interactive gate (adopts the dropped verdict L change)

`login`, `logout` and `token-store` are compiled out of every build that must not perform them. Three layers, because a cargo feature alone is a compile time guard and the CLI itself ships inside the agent image.

1. **Cargo feature.** joy-core gains `interactive = []`, off by default, in the style of `forge-net` (joy-core/Cargo.toml:89). The module `joy_core::forge_plugins::interactive` is `#[cfg(feature = "interactive")]`. joy-cli becomes `features = ["tutorial", "interactive"]` (joy-cli/Cargo.toml:17), the desktop becomes `features = ["ts", "forge-net", "interactive"]` (app/apps/desktop/src-tauri/Cargo.toml:18), and platform/Cargo.toml:14 stays `features = ["forge-net"]`. This is structurally real: the platform is its own cargo workspace with its own lock (`[workspace] members = [".", "crates/joyint-agent"]`, platform/Cargo.lock), joy-cli is not in it, the other three crates that pull joy-core into the platform's graph (joy-chat-store, joy-ai, jyn-core) request default features only, and joyint-agent does not depend on joy-core at all. No feature unification can turn the flag on in the server binary.
2. **No silent call path.** The signature requires a progress sink and a cancel token: `pub fn login(spec: &ResolvedPlugin, host: &Host, progress: &mut dyn LoginProgress, cancel: &CancelToken) -> Result<LoginOutcome>`. There is no argument free wrapper and no `Default` sink. The platform's token channel is untouched: `--token-env JOY_FORGE_TOKEN` stays byte identical (platform/src/auth/repo_store.rs:26).
3. **Runtime refusal for the builds that do carry the feature.** The agent image builds joy-cli from source (platform/docker/agent/Dockerfile:22), so a delegated agent has `joy forge login` on its PATH. `login` returns an error when the host kind is `Background` or `Delegated`. Sentence: "joy forge login needs a person at this machine; this process runs under a delegation session. Sign in on the machine that owns the session, or store a token there with joy forge login --token-stdin". The refusal is instant, not a fifteen minute wait.
4. **A CI guard** so the manifest cannot drift back: the platform's pipeline runs `cargo tree -e features -i joy-core --manifest-path platform/Cargo.toml` and fails when the output contains `interactive`.

The alternative considered and rejected: a type level `PluginHost` enum whose `Server` variant has no `login` method. It leaves the code in the server binary, and nothing prevents server code from constructing the interactive variant.

#### D3.12 Migration (new)

What exists on machines today and what happens on upgrade:

- **Three plugin binaries.** They reach a workstation only through `cargo install` or a source build. On upgrade nothing is deleted: the name order plus the handshake of D2.2a make the stale binaries harmless, `joy forge plugins` names them, and the person removes them with the printed `rm` line. The installers gain the new binary in the same archive and the same receipt, so `joy update` keeps joy and the connector in lockstep.
- **The platform and agent images.** platform/Dockerfile:85-88 copies joy-github, joy-gitlab and joy-gitea into /usr/local/bin and platform/docker/agent/Dockerfile ships no plugin at all. Images migrate by rebuild; the only work is replacing three COPY lines with one and adding the connector to the agent image. There is no mixed state, because server binary and plugins travel in one image.
- **The status words.** One producer (platform/src/sync.rs:90-101) and five documented readers (joy/src/tauri.ts:656, SyncStatusDto.ts:6, platform-client/src/client.ts:94, joy_pb.ts:2834, TeamChat.tsx:26-27). The new states are added to the same field, not to a second field. An older app degrades safely and provably (D1.8b). The rule is "one vocabulary, extended, platform and app shipped together", and the release notes say what an old client sees.
- **Keyring entries on Linux.** The desktop's features are `["apple-native","windows-native","linux-native"]` (Cargo.toml:32) and keyring 3.6.3 declares no default features, so on Linux the selected store is plain keyutils (lib.rs:207-216), which "is completely in-memory and will not persist across reboots" (keyutils.rs:19-23). What is stored there is the unlocked identity seed in hex (crypt_ops.rs:467-489, entry at :212-216). Finding, to be fixed with its own item: Linux desktop users lose the remembered identity at every reboot today and are asked for the passphrase again, which is a defect that exists before any forge work. The migration consequence is the pleasant one: moving to `linux-native-sync-persistent` needs no data migration, because nothing survives to migrate. No migration script is to be written for this.
- **Removing a stored forge credential.** Today no path exists on either surface: the desktop can only forget the crypt seed (crypt_ops.rs:537, :551) and the CLI has nothing. After the rebuild there are exactly two paths, both ending in the plugin's `logout`: `joy forge logout [--host H | --all]` and the Sign out action on the host row of the start page's Device section. Both remove the stored credential and revoke at the forge where revocation exists; when the credential came from gh, glab or tea, joy removes nothing and names the foreign command. The docs get the same two sentences per operating system.

### D4: app user (G3)

#### D4.0 The platform address is app state, not a build constant

Nothing in D4.1 exists in a release build until this lands. `SERVER_URL` comes from `import.meta.env.VITE_JOYINT_SERVER` and `platform` is `undefined` without it (app/apps/desktop/src/main.tsx:132, :319-325), so an unbaked build shows only "This build is not connected to joyint.com" (StartScreen.tsx:516-519). The platform address moves into the person's app state (ADR JAPP-02BD-56, `app_state_dir()`), defaults to `app.joyint.com`, is editable on the start page, and gets `https://` prepended when no protocol is given. This is point 1 of JAPP-02CA-F8 and a precondition of every sentence below.

#### D4.1 Start page

Two visually separate sections, because D4 adds a local forge sign in to a page that already lists the platform account's connected forges, and Geordi's journey forbids silent mixing:

- **Account (joyint.com)**: sign in with a magic link or with a forge, the account's connected forges, the platform project list. This is the door Troi uses and it is first, because the forge sign in is never the identity anchor.
- **Device (this machine)**: local forge sign ins, one row per host, each naming the login it holds, the credential source, the granted scopes and a Sign out action.

#### D4.1a The desktop sign in door: one claim, two doors behind it

The desktop never relies on a webview cookie. Production adds no CORS layer at all (`apply_dev_cors` returns the router unchanged when `dev_origin` is unset, platform/src/api/mod.rs:144-147) and `mint_session` issues `SameSite=Lax` (platform/src/auth/mod.rs:428-431), so the sign in window's cookie cannot travel with the main window's gRPC-web fetches, which are a foreign origin. The desktop session is a bearer token, as JAPP-02CA-F8 decided, and the mechanism is the same for the forge door and for the magic link door.

**The claim.** Before it opens anything, the app mints a one time secret `S` (the house shape: two v4 UUIDs, 244 bits, `new_token()`, email.rs:52-58), keeps `S` in the OS keychain under the platform address, and opens the sign in window at `<base>/signin?claim=<SHA256(S)>&device=<label>`. The platform's own sign in page loads there, so the claim hash is same origin data from the first byte.

**Door one, forge OAuth.** `/auth/<provider>` gains a `claim` parameter which rides the OAuth `state` through the redirect. `oauth_callback` mints the session as today and parks it under the claim hash instead of only setting the cookie.

**Door two, magic link.** The same page hosts the address field the web build already has (app/apps/web/src/SignIn.tsx:28-47) and posts `{email, claim_hash, device_label, next}` to `POST /auth/email/start`. `issue_link` stores `claim_hash`, `device_label` and a freshly generated `user_code` on the `magic_links` row beside `token_hash` (migrations/0022:44-54, 0035:11). The mail body gains one line: "The Joy app on <device_label> shows the code WDJB-MJHT." The person clicks the link in whatever browser their mail client opens. `GET /auth/email/callback` behaves exactly as today (single use consumption, `confirm` applies the address, `mint_session`, `Set-Cookie`, redirect) and, when the row carries a claim hash, redirects to a same origin confirmation page instead of `next`. That page names the device label, prints the `user_code` and carries one Confirm button. Only the confirm POST parks the session under the claim hash. This step is not decoration: RFC 8628 section 5.4 names exactly this attack ("an attacker might send an email instructing the target user to visit the verification URL and enter the user code"), and a claim created by one machine and redeemed by another is a device flow whether it is called one or not.

**Redemption.** `POST /auth/session/claim` with `{"secret": S}` answers `{"token":"<bearer>","expires_at":"..."}` once and deletes the claim row, or 404 while nothing is parked. The answer is identical in shape whether nothing is parked, the claim never existed or it was already redeemed. The app polls every 2 s; the poll is owned by the Rust side, survives a restart (the secret is in the keychain until redeemed or expired) and does not die when the person closes the sign in window, because "check your mail" takes minutes. The current poll does the opposite (`if (!(await invoke<boolean>("oauth_window_is_open", {}))) return;`, main.tsx:150), and that line goes.

**Lifetimes.** The claim row expires with the link it rides on: 15 minutes for a `login` link, 24 hours for a `confirm` link (`LOGIN_TTL_MINUTES`, `CONFIRM_TTL_MINUTES`, email.rs:30-32). The five minutes of JAPP-02CA-F8 stays as a second clock that starts when the session is parked. The claim is single use. The bearer is a new 244 bit secret, stored only as its SHA-256 beside the session row (the rule magic links already follow, email.rs:18), never the session uuid. It inherits the session's thirty days (`SESSION_DAYS`, auth/mod.rs:31) and is re-minted when presented past half its life, handed back in a `joy-session-token` response header the app writes to the keychain. Sign out deletes the session row and the keychain entry. `MAX_OPEN_LINKS = 5` per address and purpose (email.rs:35) already brakes mail flooding; the claim gets the same brake, one open claim per app instance.

**Two places change, not eight.** On the client, `platformTransport` sets the Authorization header beside its hardcoded `credentials: "include"` (platform-client/src/client.ts:186-217). On the server, `session_id_from_headers` learns the Authorization header beside the cookie (auth/mod.rs:560-565). CORS admits the desktop origins without credentials. The web keeps its cookie and changes nothing.

**Rejected, with the reason.** A webview hosted redeem page alone fails because a mail client opens the system browser, and fails a second time even on a webview click because of the CORS and SameSite facts above. A custom `joyint://` scheme is not the mechanism: it needs a Tauri plugin and a bundle key that do not exist today (tauri.conf.json declares no scheme; Cargo.toml:44-46 carries only `tauri` and `tauri-plugin-dialog`), "Deep links are only triggered for installed applications on desktop", macOS cannot register at runtime, and a moved AppImage invalidates its own registration (https://v2.tauri.app/plugin/deep-linking/). It may later be an accelerator that shortens the poll, never the path the design depends on.

#### D4.1b The two mode ownership rule

**What identifies a project.** A local project is its canonical checkout path; joy already uses exactly that, hashed, to name a project's app state (session.rs:273-284), and two clones of one repository stay two projects on purpose. A platform project is its project id plus the forge remote it names (`PlatformProjectRef{id, origin}`, StartScreen.tsx:51-62). The join key is the normalized origin remote of the local checkout: lowercase host, the scp form `git@host:owner/repo` rewritten to `host/owner/repo`, userinfo and a default port dropped, a trailing `.git` and slash stripped, read through the same `origin_or_first` selection the resolver uses (forge.rs:178-190). `RecentEntry` (recent_ops.rs:25-29) gains `origin`, filled on record and lazily on the next listing; an entry whose directory is gone never joins.

**Who owns the row.** The Device section owns a repository whenever a non missing local clone exists on this machine, because that mode is the only one that works offline and without an account, the only one the app can open without asking the platform, and the machine has already paid for it with a checkout. A platform project with no local clone stays in the Account section. Never two rows for one origin, on either side.

**The other mode is an action, not a row.** A joined row sits in Device with the badge "also on joyint.com" and a secondary action "Open on joyint.com". An Account row with no clone gets "Clone to this device". A Device row no platform project references gets "Add to joyint.com". The Add dialog already refuses a duplicate registration by origin (StartScreen.tsx:1107-1112).

**Is it a conflict.** Not for the data. The working branch is pushed and, on rejection, merged with the joy-yaml engine and pushed again on both sides (platform/src/sync.rs:1382-1400; the desktop worker is the same write behind shape, sync_ops.rs:20-22, :204-213). Chats are merged by a keyless, content addressed union which is "Conflict-free by construction" and "key-free, so the forge, a seedless peer and the platform all produce the identical merge" (chat_ref.rs:302-308). What two live workers do cost is two rows, two write behind committers on one branch, and two poll budgets charged against one host, which D1.9 counts per host and not per mode. Therefore: one row, and the joined row's `auto_sync` (app_state.rs:22-33) defaults to off, so exactly one write behind committer is live per origin unless the person turns the second one on.

**The sentences.** On the joined row: "On this device and on joyint.com. The same repository, kept in step by joyint.com, so commits from the platform show up here." Beside the Device open button: "Open on joyint.com" with the tooltip "Opens the copy joyint.com keeps. Your clone stays where it is." When the person switches on auto sync for a joined row: "joyint.com already syncs this repository. Switching this on means both write to it. That is safe but noisy."

#### D4.1c Multi account per host

The plumbing exists and is unused on the device: `CallerFacts{login, user_id, token_env, token_value}` already says what it is for ("the platform's session knows who acts ... while a single-person device passes none and the plugin finds its own facts", forge_plugins.rs:59-77). The device side is filled in now, and every plugin answer that names a token gains a `login` field, because D4.1's promise of "one row per host, each naming the login it holds" cannot be kept otherwise.

**How a login is chosen for a remote.** `token` and `login` accept `--login <name>`. Without one, the plugin picks in this order and reports which step decided:

1. the device local pin for that host on that project;
2. the login the transport memory recorded as the last one that pushed successfully to this remote;
3. the only login the host holds, with no probe at all;
4. a probe: one REST call for `owner/repo` per candidate (GitHub `GET /repos/{owner}/{repo}` reading `permissions.push`, GitLab `GET /projects/{urlencoded}`, Gitea `GET /repos/{owner}/{repo}`), candidates in the order plugin keychain login, then the forge CLI's active login, then the rest in a stable order; the first that answers 200, and for a push direction reports write, wins;
5. otherwise `{"known":false,"reason":"no-login-for-repo"}`, and the banner reads "None of your GitHub logins (work, scotty) can reach acme/widgets. Sign in with the login that can."

The winner is cached per normalized remote (the same key as the ownership join) in the device state and thrown away on a 401, 403 or 404 from that remote and on logout. The probe is one request per remote, allowed on a `Background` host because it raises no prompt, and never per contact.

**This is not academic.** gh keeps several accounts per host and documents the trap: "Without the --user flag, the active account for the host is chosen." (https://cli.github.com/manual/gh_auth_token). A plugin that calls `gh auth token --hostname H` and nothing else hands back whichever account the person last switched to, which is how a private repository gets pushed under a work login. tea is multi login per host too; glab holds one token per host block, so there the rule collapses to step 3.

**Where the pin lives.** Not in `project.yaml`: that file is the project's shared, committed file (model/project.rs:37-47) and joy syncs it to the forge, so a pin there publishes one person's work account to the whole team and to the forge. The pin lives in the per project app state file joy-core computes and which the CLI and the app both reach (`app_state_project_file`, session.rs:273-284), as `forgeLogin: {"github.com": "scotty-work"}`.

**Which identity the forge sees.** Said plainly in the design and in the app: the forge attributes a push to the account whose token authenticated it; the commit author is the acting Joy member (D4.5), and GitHub "links a commit to a user by matching the email address in the commit header to an email address on a GitHub account", so pusher and author may legitimately differ. Chat refs carry neither: every `refs/joy/chats` commit is signed with the fixed neutral identity `joy <joy@localhost>` and a day coarsened time (chat_ref.rs:78-90), so only the push is attributable there. The Device host row reads: "GitHub, signed in as scotty-work. Pushes appear on GitHub as scotty-work. Your commits stay signed as <member>."

#### D4.1d The refresh lock in the app

Same lock, same owner, same file as D2.6a. The app never takes it, because the app never touches the entry.

#### D4.2 Local sign in

The button runs the plugin's `login` through the streaming runner (D2.3). The first event carries the URL and the code; the app opens the URL itself and shows the code with a copy action and a countdown from `expires_in`. Progress travels as a Tauri event (the mechanism chat_ops already uses). Every desktop command that can touch a plugin is `async` or `#[tauri::command(async)]` and runs the blocking work in `spawn_blocking`; `joy_release_publish` is the counter example that blocks the main thread for up to 120 s today (release_ops.rs:439, :498). Where the forge supports PKCE with a loopback redirect (the Gitea family), the desktop uses it instead of the device flow. A successful login clears the sync banner and triggers one immediate retry.

#### D4.3 Add a project, and what "lean" means

- Pick from `repositories` (paginated, searchable) or paste a URL or open a folder.
- Before cloning, the app asks the plugin for the repository size (`store`'s `size_bytes`, D2.4) and warns above **250 MB** reported size while letting the person continue; above **1 GB** it pre-selects the platform project while still allowing the clone. The upper anchor is the forge's own guidance: "We recommend repositories remain small, ideally less than 1 GB, and less than 5 GB is strongly recommended." A missing size is not an error: an anonymous or Guest caller, or a repository created minutes ago, has no usable figure, and the app clones without a warning. The warning says "up to about N MB", because the reported figure covers the full history while a depth 1 clone downloads roughly one snapshot.
- Clone with `forge::clone` plus a progress callback (bytes and objects), a cancel action and cleanup of a partial destination directory. There is no clone call in the desktop today.
- The shared Add dialog forks per host: the desktop offers clone with a destination folder, the web offers registration only.

**Footprint, corrected against the engine.** Three capability statements, each with its decisive source:

- **Sparse checkout: no.** The string "sparse" does not occur once in libgit2 1.9.6's implementation or public headers (the only tarball hits are in libgit2/script/api-docs/generate, a shell script that calls the git CLI). checkout.c never reads `GIT_INDEX_ENTRY_SKIP_WORKTREE`; the only place that honours the bit is diff_generate.c:843-845, where such an entry is reported as `GIT_DELTA_UNMODIFIED`. git2 0.21 exposes only the raw index flag (lib.rs:531). Setting the bit by hand does not survive, because the next `checkout_head` (forge.rs:411, :705, :1825) materialises the files again. A forge cannot help, because sparse checkout is purely client side over `$GIT_DIR/info/sparse-checkout`.
- **Partial clone: no**, on both the request and the read side. `git_pkt_buffer_wants` writes want lines, shallow lines and an optional `deepen %d` line and nothing else (transports/smart_pkt.c), and the capability table has no `filter` entry (transports/smart.h:26-41). "promisor", "blob:none" and "partial" do not occur in src or include, so a promisor pack would be read as an ordinary pack and a missing object would be a hard failure.
- **Shallow clone: yes**, and it is the only footprint reducer git2 offers. `GIT_FETCH_DEPTH_FULL = 0` and `GIT_FETCH_DEPTH_UNSHALLOW = 2147483647` (include/git2/remote.h:771-778); depth reaches a clone through `git_clone_options.fetch_opts` (clone.h:126); git2 exposes `FetchOptions::depth(i32)` (remote.rs:593). Caveats that belong in the design: the local transport refuses any depth ("shallow fetch is not supported by the local transport", transports/local.c:310), which is why the joy test rigs that clone from local paths cannot exercise it; a shallow clone gets no tags (clone.c:627-629); ahead and behind become approximate, because the revwalk applies the shallow grafts (commit.c:560-566 via commit_list.c:214) and a merge base below the cutoff is invisible to `git_graph_ahead_behind` (forge.rs:1277, :2018); and no process may run `git gc` against a shallow joy checkout (D3.7).
- **Joy refs only, a lean repository with no working tree: not possible with the current store layout.** `refs/joy/chats` is an independent history of chat trees (chat_ref.rs:54, :337) and carries no items; items live under `.joy/` on the working branch, and `commit_joy` builds its tree from the repository index with `index.add_all([".joy"])` followed by `index.write_tree()` (forge.rs:441-446), so the branch commit's tree is the full project tree. The local half is work tree bound as well: `commit_joy` needs `repo.statuses` and the index, `ff_from_tracking` calls `checkout_head`, `is_worktree` rejects a bare repository by definition (forge.rs:806-811), and the store reads `.joy` YAML from the filesystem.

**The recommended lean shape** for the desktop and the non technical member is a normal working tree clone of the default branch at `depth = 1`, with `refs/joy/chats` fetched afterwards at full depth by the existing `download_ref` (which sends no depth, and a depth 0 fetch into a shallow repository is protocol correct). History is the part that is saved; the tip tree is not, and the design says so in that order. `forge::clone` grows a depth argument that the desktop sets to 1 and the CLI leaves at 0. Deepening is a later fetch with a larger depth and must be offered, not required, when a joy operation needs older history. Because the local transport refuses depth, the shallow path is covered by an integration test against a real forge.

**The App concept sentence is corrected.** Chapter 6's sentence that the sparse `.joy` checkout is "der schlanke Standard fuer die App und das nicht-technische Mitglied" is not implementable with the single git engine the same concept mandates in chapter 7, and it is repeated in app/VISION.md:63-66, app/ARCHITECTURE.md:174-176 and ADR JAPP-0024-64, while JAPP-0053-B0 already records that sparse checkout was left out. All four change together to: Joy work needs only `.joy`, but the engine cannot restrict the working tree or the download by path; the lean default is a shallow clone of the default branch at depth 1 with the full working tree on disk; there is neither a download side partial clone nor a sparse working tree; before cloning, the app asks the forge for the repository size and above the threshold offers the platform project instead. ADR JAPP-0024-64 keeps its decision (footprint follows the use case) and loses its mechanism; its consequence sentence becomes "A member doing only Joy work sees the whole working tree but downloads only the latest revision; the app never promises a .joy-only checkout." Mobile keeps the same statement, with the note that a genuinely large repository is a platform project there.

#### D4.4 Setup mask for a repository without a store

`joy_init_project` gains `user`, `acronym`, `language` and `description`, and app/packages/tools/joy/src/tauri.ts:74 is widened. Founder sources, in order: the verified addresses the plugin's `identity` answers for the signed in login, then a typed address validated by the mask, then git config as a prefill. The mask never shows an empty picker. Forge noreply aliases are filtered out before display, because `resolve_founder_email` refuses them even as an explicit override (init.rs:181-196). For a local folder with no remote and no account the same mask appears with the typed address as the only source, which removes `NoFounderIdentity` for Scotty's weakest case. The second step takes the member too: `joy_auth_setup` (crypt_ops.rs:726-773) and `redeem_with_passphrase` (enroll.rs:199) get an explicit member parameter.

#### D4.5 Identity for commits

`forge::repo_identity` (forge.rs:1161-1173) is deleted or demoted to a prefill helper, and its two desktop callers (git_ops.rs:157, :202) take the identity from the acting member, recorded per project by the app and fed into `joy_core::identity::resolve_identity`.

Signature rule, binding:

- Open mode: name is git config `user.name` when it maps to the acting member, else the member id; email is the member id.
- Anonymous mode: the opaque `m-<hex>` id for both fields, never the email, so the git2 commit does not undo ADR-042 (privacy.rs:583, members_file.rs:38-60).
- Before every `Signature::now`, joy guarantees non empty strings. `git_signature_new` refuses an empty name or email (signature.c:68-99); the typed error is "this project does not know who you are, pick your member".
- The desktop release path (release_ops.rs:341-349) moves onto `forge::commit_index` and `forge::tag_annotated`, or it is excluded from G3 in writing.

#### D4.6 Sync worker and poll

- The chat poll schedules at a fixed period: measure before the call and schedule the next tick at `max(0, interval - elapsed)` (chatTransport.ts:166-186).
- One combined `ls_remote_refs` per host per tick answers the chat ref and the working branch.
- The interval is derived from the host's request budget (D1.9) and multiplied by the number of open projects on that host. An https remote with no credential polls at the unauthenticated interval (D1.2).
- Transient failures back off (3 s, 10 s, 30 s, 2 min) instead of retrying every 3 s forever (sync_ops.rs:20-22), and the worker consults `contact::limited_until` before it tries again.
- The worker never raises a prompt of any kind (D1.10).
- Unattended behaviour: after the app window has been hidden or unfocused for a configured idle period the worker keeps committing locally and stops contacting the forge until the window returns. A machine that talks to a forge with a stored token while nobody is there needs a stated stop, and this is it.

#### D4.7 Banner

| State | Desktop sentence and action | Web sentence and action |
| --- | --- | --- |
| needs_sign_in | "Sign in to GitHub to sync." Button: sign in. | "Connect GitHub to your Joyint account." Button: connect. |
| needs_host_trust | "This machine has never seen the host key of github.com." Button: show the fingerprint. | not shown |
| needs_org_approval | "Your organisation must approve Joy for this repository." Button: open the approval page. | same |
| needs_sso | "Your organisation requires single sign-on for this login." Button: open the sign-on page. | same |
| scope_missing | "Your GitLab sign in does not allow creating projects." Button: sign in again with wider access. | same |
| no_push_rights | "You can read this repository but not write to it." No button. | same |
| rate_limited | "GitHub is rate limiting us, retrying in N minutes." No button. | same |
| tls_untrusted | "The connection to github.com could not be trusted." Button: show what to do. | same |
| proxy_auth | "The proxy proxy.acme.example:8080 needs a user name and a password." Button: sign in to the proxy. | not shown |
| offline | "No connection to github.com." Button: retry. | same |
| plugin_missing | "The GitHub connector is missing on this machine." Button: repair. | not shown |
| plugin_outdated | "The GitHub connector on this machine is too old." Button: show how to replace it. | not shown |

The raw libgit2 text never appears on the surface or in a tooltip.

#### D4.8 Sidecars and the agent's environment

`bundle.externalBin` in tauri.conf.json carries the connector binary and the joy CLI, both built with a `-<target-triple>` suffix for every release target. The app resolves them with `tauri_utils::platform::current_exe()` (never `tauri::process::current_binary`, which returns the AppImage path) and keeps spawning through `joy_process::command`, so joy-process keeps owning the Windows console window rule; tauri-plugin-shell is not added. The sidecar directory is registered with `forge_plugins::set_plugin_dirs` and prepended to the PATH of the local agent. macOS signing covers sidecars inside out; on Windows the sidecar must be signed before `just build-desktop` bundles it, because the release workflow signs only the finished .msi and .exe (app/.github/workflows/release.yml:195-213).

#### D4.9 Mobile, named as changed

Mobile is out of scope for this rebuild. Reason: every plugin verb is a subprocess and iOS forbids spawning, which is the same fact that forces git2. What a mobile host would need, listed so the later item is short: the forge logic as a library crate linked into the app (in process `token`, `login`, `repositories`), device key storage in the Secure Enclave or Keystore deposited at the forge, and a checkout shape decision, which D4.3 now answers for every host: not sparse, shallow, and a large repository is a platform project.

### D5: platform

The model is unchanged: OAuth token per identity over https, renewal, write behind sync inside member sessions. The additions and the corrections:

- Befunde 5 is restated correctly, and the correction runs the other way from v2. The **server** image already installs curl and copies joy-github, joy-gitlab and joy-gitea into /usr/local/bin (platform/Dockerfile:69-89); the image that has **git** is the **agent** image (platform/docker/agent/Dockerfile:45-51 installs `git ca-certificates curl` and purges only curl) and it carries no plugin. The job container bind mounts the long lived clone read write (`--mount type=bind,source=<repo>,target=<repo>`, agent/container.rs:537-548, :860-872), so an agent's `git commit` can fire `git gc --auto` on exactly the object store the server sweeps, from another process in another namespace, which the per process `checkout_gate` does not cover. That is the concurrency case the grace window exists for.
- Silent failure is fixed at the source: `run_query_full` logs spawn failure, non zero exit and timeout at warn level with the plugin id and the verb, and each image gets a startup probe that runs `claims` and `version` once.
- The token stays out of argv. The protocol passes only the variable name (forge_plugins.rs:66-76, :324-326); the remaining leaks are inside the plugins (github.rs:153, gitlab.rs:152) and are fixed in D2.8.
- `forge_facts` selects the identity row by the project's forge, not the account's first row by `created_at` (joy_service.rs:708-722).
- The platform adopts the shared engine changes: the twin push tracking ref write and the per ref push statuses (D1.5), the fixed period poll in platform/src/api/chat_poll.rs:38-46, :160-175, the per host request budget (overridable through `JOYINT_FORGE_MIN_GAP_MS`), the classifier of D1.8, the proxy and CA handling of D1.11 and D1.12 (a self hosted operator behind interception bakes the CA into the image or sets `SSL_CERT_FILE` in the container; the runtime image installs ca-certificates, Dockerfile:69-71, so the probe succeeds).
- The platform's own REST client cannot be fixed by an OS CA: platform/Cargo.toml:58 uses reqwest with `rustls-tls`, and reqwest 0.12.28 maps `rustls-tls = ["rustls-tls-webpki-roots"]` (Cargo.toml:148), that is the bundled Mozilla roots only. Switching to `rustls-tls-native-roots` is the only way a corporate CA is honoured there, and it is part of P4.
- **Maintenance has an owner.** The clone at `data_dir/projects/<id>/repo` is the GDPR deletion unit and lives indefinitely for any project used at least daily; the hourly volume pass removes it only after `JOYINT_PROJECT_REMOVE_SECS` (default 86400) of inactivity and only when the volume is fully synced (git/mod.rs:20-56, control_plane/mod.rs:528-539, :541-612, :659-661, config.rs:587-589). So there is a real store that grows for months and that no git binary has ever touched. Owner: the sync worker. Maintenance is a lane of `SyncWorker::tick` beside fetch and push. It runs for a project only while `active(now)` holds against `ACTIVE_WINDOW` (60 s, sync.rs:263-265), which keeps it inside the rule that forbids background work on project data outside member sessions (sync.rs:7-16), and it takes `gate_for(project)` (sync.rs:849-853), the same gate the apply half of a fetch takes. It obeys the same loose object estimate and ten minute floor as every other host, with the 24 hour grace window.
- **`create-repository` exists on the platform too**, because Troi is a web user with no plugin on her host: a platform endpoint that calls the responsible plugin's `create-repository` with the account's identity and registers the result as a project in one step.
- JAPP-02CA-F8 moves before the app work: `session_id_from_headers` learns the `Authorization` header, CORS for desktop origins stops being dev only, and the claim exchange of D4.1a is specified with issuance, lifetime, storage (OS keychain), renewal, revocation, and what the person sees when it dies.
- Multi instance forges: `registrable_bases` (joy_service.rs:7138) takes a list per forge kind, and GitHub Enterprise Server becomes a kind of its own with an instance local client id. The desktop's `forge::detect` (app/apps/desktop/src-tauri/src/forge.rs:27) learns hosts from the same configuration instead of matching substrings.
- Job identity and audit: the author of a job commit is the member id from the store, not the login shaped prefix (runner.rs:364 strips `ai:` and cuts at `@`, so `ai:claude@joy` becomes `claude`, used as name and email at runner.rs:418). The delegating human goes into a trailer. The design states which forge actor a push presents (the project account's OAuth token, watch.rs:259, runner.rs:422) and whether that satisfies the audit requirement of level 4.
- Session windows are reconciled: the sync worker's `ACTIVE_WINDOW` is 60 s (sync.rs:77) and the job watcher keeps a project for 3600 s and indefinitely while a job runs (job/watch.rs:73, :218). One definition of "inside a member session" is written down, and either the job watcher comes under it or the design records why a running job is an extension of the session that started it.
- A member without a forge identity is decided, not left open (decision 12). Today `Account::token_for` returns an empty string (auth/mod.rs:494-510) and a private repository refuses with "No write access. Read-only mode active." plus a retry button that cannot work.
- SSO stays out of scope and is named as such (no SAML, OIDC or SCIM anywhere in platform/src).
- Per repository scope (JP-00C9-D4, GitHub App) stays the structural answer to the org policy problem and gets a backlog place, with the honest interim statement of D2.7a: level 3 and 4 ship with `repo user:email` on GitHub, `api write_repository` or the narrower sets on GitLab, and the Gitea authorize URL stops requesting no scope at all (providers.rs:331-339).

### D6: docs, website, tests

- C4 pages (Joy, joy-core, Produktebene, Desktop) follow the decisions: the git binary path is gone, the resolver and the connector class are described, the deviations that are now intended are removed.
- docs/plugins.md gets a connector section: own authentication, own state (one keychain entry per forge host and login), side effects per verb, the newline delimited event stream as a second output mode, the protocol number and the `version` verb, the exact commands per forge CLI, and the storage sentence per operating system instead of "never on disk in joy's hands": Windows Credential Manager (any process of the logon session may read it, and keyring hardcodes `CRED_PERSIST_ENTERPRISE`, so the entry roams with a roaming profile), macOS login keychain (only the creating binary without a prompt), Linux Secret Service where a session bus and an unlocked collection exist, kernel keyutils as an in memory cache, and a 0600 file everywhere else.
- A new documentation chapter, "What joy reads from your machine", listing the ssh config keys, the known_hosts files, the proxy sources and the trust store per OS (D1.11 to D1.13), because that list is what support will be asked for.
- The joy docs identity chapter is corrected: "Members are added automatically during joy init (from git config user.email)" (website/content/joy/docs/tutorial.mdx:462) becomes "from your Joy member, prefilled from git config where it exists".
- The App concept, app/VISION.md:63-66, app/ARCHITECTURE.md:174-176 and ADR JAPP-0024-64 are changed together per D4.3.
- Website wording is tied to conditions: "the desktop needs neither git nor gh" may be published when no desktop path reaches the git binary (including the release path) and when release builds can sign in (D4.0, P3, P7).
- New documentation cases that do not exist today: a Windows case (default Git for Windows install, `credential.helper=manager`, no sh on PATH, no agent service, WinHTTP proxy from the registry), a self hosted case (IP literal host, non default port, `forges.yaml`, an internal CA), an organisation case (OAuth app policy, SAML key authorization, an intercepting proxy), a headless Codeberg case (no device flow, `joy forge login --token-stdin`), and a first contact case (the host key question and what the fingerprint means).
- tests/validation/cx: the `joyint-connect` fiction is removed and replaced by five journeys, each with a named harness: CLI user with git and a remote (bats), CLI user signing in to a forge and pasting a token (bats, with a fake device endpoint), agent under a delegation session against a remote (bats, `JOY_SESSION` set, asserts no prompt and stable failure words), app user without a terminal (Playwright for the web shell, the existing desktop e2e setup for Tauri), and plugin distribution (after each installer, a forge question is answered without a further install step).
- Regression tests that guard the new promises: a **request** rate test that counts HTTP requests per minute per host for a focused chat session and fails when the count rises; a maintenance test that drives a long lived chat store and asserts `.git` stays below 6700 loose objects and a size bound; a classifier test against a WinHTTP corpus and an OpenSSL corpus with non English operating system message tails; a known_hosts test with a hashed entry, a mismatch and an unparsable line.

## 3. Persona and journey map

Grades are re graded honestly against v3. "yes" means the design closes the journey and a package carries acceptance for it; "partly" means the design closes part of it and names what stays open; "changed" means the journey is deliberately different and the change is written down; "no" means not served and named as such.

| Journey | Persona | Served | The step that changed, and the package |
| --- | --- | --- | --- |
| Level 1 solo, everything local, no account | Scotty | partly | The founding step is closed: `joy init` asks for the address on an interactive terminal and refuses by name on a background host (D3.9, J9). The remaining `user_email` call sites in joy-cli (20 in 9 files at the time of writing, minus the two J9 moves) are package J11, which moves them onto `resolve_identity`; until J11 lands a person with no git config still meets git config in auth, chat and board commands, and the row stays partly for that one reason (D3.9, J9, J11) |
| Scotty installs with one command | Scotty | yes | The installer ships the connector beside joy and records both in the receipt (D2.1, J8, W1) |
| Scotty signs in to a forge from the terminal | Scotty | yes | New: `joy forge login`, `--token-stdin`, `status`, `logout`, `plugins` (D3.10, J10). This is the door every "sign in to the forge" sentence now points at |
| Scotty delegates to his AI tool locally | Scotty, Data | partly | Credentials and identity work headless (D1.10, D3.8, D3.11); installing the agent CLI itself is still a manual step outside the app |
| Scotty publishes a release from the CLI | Scotty | yes | `release` moves off gh onto the plugin's own HTTP client (D2.8, J2). In wave 1 the token still comes from `CallerFacts.token_env` or from spawning gh, so J2's acceptance is a machine without curl; the machine with neither gh nor curl is J3's acceptance, once the connector holds its own credential (D2.8, J2, J3) |
| Scotty opens a local repo in the desktop app | Scotty | yes | Setup mask replaces `NoFounderIdentity`, the desktop write path leaves the git binary (D4.4, D3.2, A3) |
| Scotty uses a local repo from the mobile app | Scotty | changed | Out of scope with the reason (no subprocess, therefore no connector) and with the list of what a mobile host needs; the checkout shape question is answered for every host (D4.9, D4.3) |
| Level 2 team on a shared repo, no platform | Geordi | yes | joy's commits touch only joy's paths, joy owns `core.hooksPath` and chains to the team's hooks, maintenance is joy's own and safe beside another git (D3.4, D3.5, D3.7, J6, J7) |
| Geordi brings his private forge project to joyint.com | Geordi | partly | The rights bundle is stated per forge and per verb group, and a read only member has a working set on GitLab and Gitea (D2.7a, J3). Per repository scope on GitHub is still a backlog item, so the enterprise concern is named, not solved |
| Geordi signs in across his clients | Geordi | changed | The CLI has no door to joyint.com; CLI and platform meet at the forge, and the CLI now has a forge door of its own (D3.10). The changed journey is written down instead of implied |
| Geordi opens a platform project in the desktop app | Geordi | partly | The claim exchange replaces the webview cookie and the start page separates Account from Device (D4.0, D4.1a, P3, P7, A2). Partly, because the claim exchange has never run against a running platform (section 6), and because D4.0, the precondition of every D4.1 sentence, is built in A2 and in no earlier package |
| Geordi switches between projects | Geordi | yes | Unchanged |
| Geordi has the same repository locally and on joyint.com | Geordi | yes | New: the ownership rule, the join key, one row, `auto_sync` off by default on a joined row, and the three sentences the person reads (D4.1b, A2) |
| Geordi job: assistant implements a task, result as a branch | Geordi, Data | partly | The job commit gets a real member address and a delegating trailer, the push actor is documented, per ref rejections are read so a protected branch refusal is no longer reported as success (D1.5, P5). Signing stays unsupported |
| Level 3 team with Joyint, Troi gets in without the CLI | Troi | partly | The magic link door works in the desktop through the claim exchange (D4.1a, P7). A member without a forge identity still depends on decision 12 and package P6 |
| Troi opens a platform project in the web app | Troi | partly | Banner wording and action per state in the shared component (D4.7, A5); a private repository without her own forge token is answered by decision 12, not by a retry button (P6) |
| Troi joins from the app (magic link, identity in the app) | Troi | partly | The mechanism is specified end to end: claim hash, confirmation page with a user code, parked session, single use redemption, bearer in the keychain (D4.1a, P7, A2). Partly for the same reason as Geordi's row: the claim exchange has never run, and the platform address of D4.0 is A2's work |
| Troi starts a new project from a forge | Troi | yes | `create-repository` as a plugin verb (J3) and as a platform endpoint that registers the result in one step (P9), so her half of the journey has an implementation |
| Troi web: assistant in a shared platform project | Troi, Data | yes | Unchanged |
| Level 4 enterprise governance | Picard | partly | Multi instance forges, GHES as a kind, `forges.yaml` for operator registered clients and an internal CA; SSO is named as out of scope (D2.5, D5, P4) |
| Picard connects an internal forge | Picard | yes | The plugin claims a host from configuration, not only from a signed in forge CLI, and the instance's own CA and proxy are handled (D2.5, D1.11, D1.12, J2, J4p) |
| Picard's machines sit behind a proxy with TLS interception | Picard, Geordi | partly | New: one `ProxyOptions` function, NO_PROXY applied to config sourced proxies too, `ALL_PROXY` through joy, proxy credentials as URL userinfo, the trust store per OS, SOCKS refused by name (D1.11, D1.12, D1.13, J4p). Partly, because NTLM and Negotiate to a proxy are refused by name on Linux and macOS, which is the ordinary corporate proxy on that estate: Windows reaches such a proxy through WinHTTP, and everywhere else joy offers Basic and URL userinfo and says so in the failure text |
| Picard connects SSO | Picard | no | Named as out of scope with the code fact (no SAML, OIDC or SCIM in platform/src) so procurement is not misled (D5) |
| Picard enterprise AI permissions and audit | Picard, Data | partly | Job author address, delegating trailer, push actor documented, session windows reconciled, per ref rejections read; signed commits remain unsupported (D5, D3.6, P5) |
| Data acts under a delegation session | Data | partly | Non interactive is a parameter set once at the entry point, the interactive verbs are compiled out or refused instantly, and the helper environment is set per spawn (D1.1, D1.10, D3.11, J10). What stays open is an operating system store that raises its own dialog: the call is bounded by the deadline and the process group kill, and the error mapping is unverified (section 6) |
| A machine that has never seen the host key | all | partly | New, and it fixes a live bug: libgit2 1.9.6 already refuses and joy had no way to accept. joy does the whole check in Rust, over every known_hosts file ssh would read, with hashed entries, markers and ports (D1.4a, J4h). Yes for an `Interactive` host. Partly for `Background` and `Delegated` hosts, whose only way onto a fresh machine is the pin, which depends on decision 23; the Codeberg pin's build step is now J4h's work and J4h's acceptance |
| A person with two accounts on one host | Scotty, Geordi | yes | New: a deterministic login order per remote, a device local pin outside the committed project file, and the sentence that says which account the forge will show (D4.1c, J3, A2) |
| A large repository on the desktop | Troi, Geordi | partly | New and corrected: size warning from the forge, a depth 1 clone, no sparse and no partial clone, and the four documents that promised otherwise are changed together (D4.3, A4, W2). Partly, because ahead and behind are approximate under depth 1 (the revwalk applies the shallow grafts, forge.rs:1277, :2018): A4 records the shallow state of a clone and A5 shows no numbers on a shallow clone, only "in sync" or "changes to sync" (A4, A5) |
| The sign in bar: one browser login | all app users | yes | Streaming `login` with NDJSON events, PKCE loopback preferred on the desktop, a present gh login reused by spawning gh (D2.3, D2.4, D2.6, D4.2; packages J1, J3, A2) |
| The banner when sync fails | all app users | yes | One sentence, one action, per state, decided from error class, code and a status number rather than prose, and proven on a WinHTTP corpus (D1.8, D4.7, A5) |
| Windows host with no agent and no sh | Scotty, Geordi | yes | Token path first because "no working local ssh credential" is the Windows norm, an own helper runner that never spawns git, key files declared dead on WinCNG by name, a CLI sign in door, and the registry proxy stated (D1.2, D1.3, D3.10, D1.11; packages J4a, J4p, J4b, J10, A2) |
| macOS with sidecar plugins and a keychain | all desktop users | partly | The connector alone owns the entry and the app never reads it (D2.6), and foreign CLI stores are no longer read at all, which removes most of the risk. What is still unconfirmed is whether the app sidecar and a separately installed CLI copy carry the same code signing identifier; until a signed build is tested, a CLI update may re prompt (section 6, T4) |
| The website promise "never opened a terminal" | visitor | changed | The wording is published only when both conditions hold (no git binary in any desktop path, sign in in release builds) (D6, W2) |

## 4. Work packages

Each package names its repository, its files, acceptance criteria as observable behaviour, and its dependencies. Packages in the same wave may overlap in time. Where a package names a dependency inside its own wave, it starts when that dependency is green and runs beside the rest of the wave; a "Parallel with" list therefore means "may overlap", never "independent of". The in wave dependencies are: P1b on J2 and J4p on J4h (wave 1); J4b, J10 and P9 on J3, A1 on J8, P7 on P3 (wave 2); M1 on A2 and A5 on A4 for the shallow criterion (wave 3).

### Wave 0 (no dependencies, start together)

**J1 (joy): plugin runner v2, protocol v2, resolution.**
Files: joy/crates/joy-core/src/forge_plugins.rs, joy/docs/plugins.md.
Work: concurrent stdout read with a deadline, piped stderr, structured `PluginOutcome`, streaming runner, process group kill, per verb timeouts, rootless invocation, `--host` and `--host-kind` accepted everywhere, the resolution contract (`set_plugin_dirs`, executable directory, PATH) with the `joy-forge` first name order, the `version` verb and the protocol 1 detector (exit 2, empty stdout), a warn log on spawn failure, non zero exit and timeout (the same log P1a needs). The approved JI item of decision 29, working title "forge connection NG: new commands, flags and features", exists before this package's first commit, because `--host` and `--host-kind` are part of it.
Acceptance: a plugin answer of 1 MB is returned intact instead of timing out; a plugin that exits 3 with text on stderr produces an error naming the plugin and the text, not `None`; a binary placed next to the calling executable is found on macOS and Linux without PATH; `claims --host github.com` works with no project on disk; a protocol 1 `joy-github` placed in `~/.cargo/bin` is reported as protocol 1 and a protocol 2 binary named `joy-forge` in the same directory (a test stub in J1; the real binary is J2's) wins the name order.
Parallel with: J4a, J5, J7, J9, P1a.

**J4a (joy): credential helper runner, ssh chain, credential shapes.**
Files: joy/crates/joy-core/src/vcs/credential_helper.rs (new), vcs/ssh_config.rs (new), vcs/forge.rs:32-44, :116-190.
Work: the helper runner of D1.3 including `store` and `erase` and the per spawn environment, the ssh chain of D1.4 (agent probe, ssh config parse, key pre validation, constant user name, `ProxyCommand` refused by name), the shape decision of D1.6, the `allowed` mask fix, `remote_url` aligned with `origin_or_first`, and the cross process advisory file lock primitive `joy_core::util::file_lock` (fs4; taken by J4h for known_hosts appends and by J3 for the refresh lock of D2.6a).
Acceptance: on a machine with `credential.helper=manager` and no git on PATH, joy obtains a credential and a process trace shows no `git` process; a passphrase protected key file is skipped with "passphrase needed" instead of aborting the whole fetch; a GitHub Enterprise host without "github.com" in its name authenticates on the first attempt; on Windows an openssh-key-v1 file is reported as unreadable by name; a remote whose ssh config carries `ProxyCommand` fails with the named sentence and not with a DNS error.
Parallel with: J1, J5, J7, J9, P1a.

**J5 (joy): contact discipline in requests, and the classifier vocabulary.**
Depends on: none (the oracle hook of D2.10 lands behind an interface J3 fills later).
Files: joy/crates/joy-core/src/vcs/contact.rs, vcs/forge.rs:321-357, :608-638.
Work: the request weight table per verb, the per host budget table of D1.9 with the poll period computed as `requests(verb) / budget` rounded up to the next whole second, the strike exponent and strike survival, `ls_remote_refs`, the single connection fetch (`connection.remote()`), the no anonymous polling rule with the unauthenticated interval, the 403 rules of D1.8b including the GitLab wait per instance kind of D2.10, and the oracle hook of D2.10. **J5 also owns the classifier vocabulary**: the state enum carrying every new name of D1.8a and D1.8b, `classify(evidence: &ContactEvidence)`, the mapping table, the status word mapping for older readers and the detail line grammar. It sits in wave 0 so that every later package can be accepted against state names that already exist; J4b assembles the evidence and no longer defines the classifier.
Acceptance: a counting proxy shows that one `fetch_ref` against a private https remote costs three requests, not five; the measured HTTP requests per minute per host stay inside the budget table of D1.9, which for a private chat poll on codeberg.org is at most 40 requests per minute (twenty ticks of two requests at the 3 s period) against the 54 per minute the 0.9 budget allows, and at most 60 per minute on github.com; after a 429 the next contact to that host waits at least twice the gap and the strike survives the next success; one poll tick makes one contact for two refs; an https remote with no credential is polled at most once per 15 minutes per host and the surface says why; the classifier test passes against both the WinHTTP and the OpenSSL corpus with a non English operating system message tail.
Parallel with: J1, J4a, J7, J9, P1a.

**J7 (joy): repository maintenance.**
Files: joy/crates/joy-core/src/vcs/maintenance.rs (new), joy/crates/joy-chat-store/src/chat_ref.rs:164-232, joy/tests/integration/chat_store_maintenance.bats.
Work: the whole of D3.7: the per checkout trigger with the loose object estimate and the time floor, the keep set from refs, reflogs and the index, the pack, the two class sweep, the never delete a pack rule, the mtime rule and its two skip cases, the Windows best effort unlink, the removal of the `git gc --auto` spawn, the `core.logAllRefUpdates=always` detection, the CAS ordering fix.
Acceptance: a store built from 20 chats of 10 messages ends below 1 MB with all chats intact and under 6700 loose objects; a second process holding an object open does not crash the sweep and the object is still readable afterwards; no object younger than the grace window is removed; no `git` process is spawned anywhere in the chat write path.
Parallel with: J1, J4a, J5, J9, P1a.

**J9 (joy): init, identity and the CLI founder.**
Files: joy/crates/joy-core/src/init.rs:59-196, vcs/forge.rs:1161-1173, identity.rs, auth/enroll.rs:199, joy/crates/joy-cli/src/commands (init and auth).
Work: keep `InitOptions.user`, add an explicit member parameter to enrollment, demote `repo_identity`, the signature rule of D4.5 including anonymous mode, the non empty guarantee and the typed error, the alias guard for an explicit override, and the CLI founder path of D3.9.
Acceptance: `joy init --user a@b.c` in a repository with no git config succeeds and the following enrollment enrols `a@b.c` without reading git config; `joy init` with no git config on a terminal asks for the address and completes; the same command under `JOY_SESSION` refuses with the named sentence; a commit in an anonymous mode project carries `m-<hex>` in both signature fields.
Parallel with: J1, J4a, J5, J7, P1a.

**P1a (platform): logging, identity selection, startup probe.**
Depends on: none.
Files: joy/crates/joy-core/src/forge_plugins.rs (the warn log, shared with J1), platform/src/api/joy_service.rs:708-722, platform/Dockerfile (the probe step).
Work: the warn level log in `run_query_full` on spawn failure, non zero exit and timeout, with the plugin id and the verb (one change, shared with J1, landed by whichever commit is first), the identity row selected by the project's forge instead of the account's first row by `created_at`, and a startup probe in the server image that runs `claims` once against the plugins that image already carries.
Acceptance: a deliberately removed plugin produces a warn log line and a failing startup probe instead of a silent "unknown"; a multi forge account contacting a GitLab project uses the GitLab token.
Parallel with: all of wave 0.

### Wave 1

**J2 (joy): connector consolidation, HTTP client, instance configuration, scopes.**
Depends on: J1 (protocol frozen).
Files: joy/crates/joy-github, joy-gitlab, joy-gitea (to libraries), the new `joy-forge` binary, joy/crates/joy-*/src/*.rs API paths.
Work: one binary, in process HTTP with rustls honouring D1.11 and D1.12, **the release verb moved to REST**, curl and gh removed from every API path, the three argv token leaks and the two hardcoded api base bugs fixed, `forges.yaml` plus the project override in `claims`, config discovery per OS, the scope sets and the `scope_missing` answer of D2.7a and D2.7c, `size_bytes` on `store`. **Where the token comes from in this wave:** the connector has no credential store of its own until J3, so `release` and every other verb take the token from `CallerFacts.token_env` or from spawning `gh auth token`, which decision 19 allows and which is the only device side source that exists in wave 1. Removing gh and curl from the API paths does not remove gh as a credential source.
Acceptance: `joy-forge github identity --remote <url>` answers on a machine without curl; **`joy release publish` against a github.com remote succeeds on a machine without curl, with `GH_TOKEN` set or with gh signed in** (the machine with neither gh nor curl is J3's acceptance, because that is where the connector gets its own credential); `ps` during any call shows no token; an internal host listed in `forges.yaml` is claimed with no forge CLI installed; a GHES host asks its own `/api/v3/user/emails`, not api.github.com; a token with the GitLab read write set answers `store`, `files` and `repositories` and answers `create-repository` with `scope_missing` naming `api`; the stripped size of `joy-forge` is measured and recorded per release target, and decision 6 is revisited when any target passes 10 MB.
Parallel with: J4h, J4p, A3, A6, P1b.

**J4h (joy): host key verification and the one certificate callback (new).**
Depends on: J4a (the ssh config parser that supplies its file list), J5 (the state names its refusal reports).
Files: joy/crates/joy-core/src/vcs/known_hosts.rs (new), joy/crates/joy-core/src/vcs/certificates.rs (new, the one `certificate_check` closure), vcs/forge.rs (callback wiring), joy/crates/joy-core/data/host-keys.json (new pin file), the pin build step in joy's build recipes.
Work: the whole of D1.4a: the single `certificate_check` closure of `vcs::certificates`, installed in every `RemoteCallbacks` joy builds, carrying joy's own state for host and port and dispatching on the certificate kind (the host key branch is this package's work, the x509 branch's slot is filled by J4p and stays one closure), the file list from the ssh config parse of J4a, plain, bracketed and hashed matching, `@revoked`, certificate host keys refused by name, the pre validation of `~/.ssh/known_hosts` for libssh2 parseability, the append under the cross process file lock J4a landed in wave 0 (`joy_core::util::file_lock`), the per host kind rule, the pin file for the three public forges behind decision 23 (until the operator decides, the pin file ships empty and `Background` and `Delegated` hosts refuse an unknown host with `needs_host_trust`), **the Codeberg pin build step** (a blob taken once, checked against Codeberg's published fingerprints at build time and again in CI), and the substituted refusal sentence.
Acceptance: a fresh machine with an empty known_hosts contacts github.com over ssh and is offered the fingerprint, which matches the one GitHub publishes; accepting writes one hashed line and the next run is silent; a changed key is refused in all three host kinds with both fingerprints and the file and line number; a hashed entry written by OpenSSH matches; a `Background` host refuses with `needs_host_trust`, the state name J5 already defines, and prints the line to paste; a known_hosts file with one unparsable line is reported by line number instead of killing the connection; once decision 23 is taken, the Codeberg pin file is produced by the build step and its fingerprints equal the ones Codeberg publishes, and until then the pin file is empty and a `Background` host refuses an unknown host by name; an https contact in the same process goes through the same closure and no second `certificate_check` is installed anywhere in the tree.
Parallel with: J2, J4p, A3, A6, P1b.

**J4p (joy): proxies and the trust store (new).**
Depends on: J5 (the `tls_untrusted` and `proxy_auth` state names), J4h (the `certificate_check` module whose x509 branch this package fills).
Files: joy/crates/joy-core/src/vcs/proxy.rs (new), vcs/forge.rs (every `FetchOptions`, `PushOptions` and `connect_auth` call site listed in D1.11), joy/crates/joy-core/src/lib.rs (the one time `git2::opts` call), app/apps/desktop/src-tauri/src/lib.rs:261-310.
Work: `options_for(url, repo)` with the three outcomes, joy's own NO_PROXY matcher with whitespace trimming, `ALL_PROXY` through `ProxyOptions::url`, proxy credentials through the helper runner as URL userinfo, `GIT_PROXY_SPECIFIED` whenever a proxy is known, the SOCKS refusal, the Linux only CA escape hatch from `forges.yaml` and `http.sslCAInfo`, the x509 branch of the one `certificate_check` closure J4h owns, with its `tls_untrusted` sentence and state and its `CertificatePassthrough` return (never a second callback), and the extended environment import in the desktop.
Acceptance: with `HTTPS_PROXY` set and a proxy that requires Basic, a fetch succeeds and no proxy password appears in any log line or error text; with `NO_PROXY="a.com, b.com"` a contact to b.com bypasses the proxy; a `socks5://` proxy produces the named sentence and no attempt; an intercepting CA installed with `update-ca-certificates` is trusted on Linux with no joy setting; the same CA not installed produces `tls_untrusted` with the issuer in the detail line and libgit2's own verdict is what refused it; the desktop app and the CLI behave identically on one machine with a proxy set only in the shell profile.
Parallel with: J2, J4h, A3, A6, P1b.

**A3 (app): identity and setup mask.**
Depends on: J9.
Files: app/apps/desktop/src-tauri/src/lib.rs:237-250, crypt_ops.rs:718-773, app/packages/tools/joy/src/tauri.ts:74, app/packages/app/shell/src/shell/ProjectSetup.tsx, StartScreen.tsx.
Acceptance: opening a folder with no git config and no account leads to a working project through the mask only, with no terminal instruction anywhere in the flow.
Parallel with: J2, J4h, J4p, A6, P1b.

**A6 (app): agent environment.**
Depends on: none.
Files: app/apps/desktop/src-tauri/src/lib.rs:261-310, agent_ops.rs:594-624.
Work: import `SSH_AUTH_SOCK`, `SSH_AGENT_PID` and the proxy and CA variables of D1.11, export the plugin directory, ship and expose the joy CLI sidecar on the agent's PATH, scope `GIT_TERMINAL_PROMPT` to the tool.
Acceptance: an agent started by the app finds `joy` and the connector without the person having installed the CLI; an ssh agent started in the person's shell session is visible to the app on Linux.
Parallel with: J2, J4h, J4p, A3, P1b.

**P1b (platform): the connector in both images.**
Depends on: J2 (the one binary has to exist first).
Files: platform/Dockerfile:85-88, platform/docker/agent/Dockerfile.
Work: replace the three COPY lines with the one connector binary in the server image, add the connector to the agent image, and extend P1a's startup probe to `version`, so a stale or missing connector is named at boot instead of at the first verb.
Acceptance: the agent image answers `claims` and `version` for its own connector at startup; the server image carries one connector binary and no legacy name; an image built with a protocol 1 binary fails its startup probe with the resolved path and the `rm` line.
Parallel with: J4h, J4p, A3, A6 (it starts when J2 is green).

### Wave 2

**J3 (joy): sign in verbs, keychain, OAuth, refresh lock.**
Depends on: J1, J2.
Files: the connector crate, a new `joy-forge-auth` module, joy/crates/joy-core/src/forge_plugins.rs (verb wiring).
Work: `token`, `token-store`, `login`, `logout`, `repositories`, `web-url`, `create-repository` with the shapes of D2.4 as amended by D4.1c (every answer that names a token carries `login` and `chose_by`); keychain storage with the corrected feature set, `Entry::new` only, the 0600 fallback and the foreign CLI read by spawning; the device grant for GitHub and GitLab; PKCE loopback for the Gitea family; the scope sets and `--for`; the login choice order and the pin of D4.1c; the refresh lock of D2.6a, taken on the cross process file lock primitive J4a landed in wave 0 (`joy_core::util::file_lock`).
Acceptance: `joy-forge github login --host github.com` prints the verification line within 15 s and the caller sees it before the process exits; after a successful login `token --host github.com` answers `"source":"keychain"` with the granted scopes; on a machine with a signed in gh, `token` answers `"source":"gh"` with zero clicks and zero dialogs; `token-store` reads a token from stdin, validates it and stores it, and `ps` during the run shows no token; `logout` removes the entry and revokes the token while the platform's token for the same user still works; two joy processes refreshing the same Codeberg entry at once produce one refresh and one `busy`, and the token still works afterwards; a host with two gh accounts answers `token` for a repository only the second account can reach, and reports `"chose_by":"probe"`; **`joy release publish` against a github.com remote succeeds on a machine with neither gh nor curl**, which is the acceptance J2 could not carry because the connector had no credential of its own in wave 1.
Parallel with: J4b, J10, A1, J8, P3, P7, P4, P5, P9.

**J4b (joy): resolver assembly.**
Depends on: J3 (token and web-url), J4a, J4h, J4p, J5.
Files: joy/crates/joy-core/src/vcs/forge.rs:116-190, the new resolver module.
Work: the candidate order of D1.2 with the twin trigger of decision 11 and the transport memory, twin computation and refusal conditions, insteadOf prediction, the push side tracking ref write and `push_update_reference`, the token cache with TTL, the `ContactEvidence` assembly handed to the classifier **J5 owns** (this package no longer defines the classifier, it feeds it), and the host kind parameter.
Acceptance: an ssh remote on a machine with a working agent stays on ssh even when a gh token exists, and `.git/config` is unchanged; the same remote on a machine with no agent and no readable key goes to the twin on the first contact and says which credential it used; a repository with a `pushInsteadOf` rule stays on ssh and says why; after a push over the twin, ahead and behind reads 0; a push whose ref the server rejected fails with the server's sentence instead of returning success; a 403 on a private organisation repository produces `needs_org_approval` with the approval URL; a 404 over https is never reported as offline, and each of these reads the state names J5 defined in wave 0.
Parallel with: J3, J10, A1, J8, P3, P7, P4, P5, P9.

**J10 (joy): the CLI forge command group and the interactive gate (new).**
Depends on: J1, J3.
Files: joy/crates/joy-cli/src/lib.rs (one `Commands` variant), joy/crates/joy-cli/src/commands/forge.rs (new), joy/crates/joy-core/Cargo.toml, joy/crates/joy-cli/Cargo.toml, app/apps/desktop/src-tauri/Cargo.toml, joy/crates/joy-github/src/github.rs:297,310, joy/docs/plugins.md, the platform CI pipeline.
Work: the whole of D3.10 and D3.11, the host kind set once in `cli_main`, the two help strings, and the approved JI item of decision 29, working title "forge connection NG: new commands, flags and features", which names every flag, option and feature the design adds and which exists before this package's first commit.
Acceptance: `joy forge login --host github.com` on a machine with no gh prints a URL and a code and ends with a stored credential; `joy forge login --token-stdin --host codeberg.org < token` stores a validated token and `ps` during the run shows no token; `joy forge status --json` lists the host with source, scopes and expiry and exits 1 when nothing is signed in; a protocol 1 `joy-github` in the executable's directory makes `joy forge login` print the binary's path and the `rm` line while `joy release publish` still works; `cargo tree -e features -i joy-core --manifest-path platform/Cargo.toml` contains no `interactive` and the CI guard fails when it does; `JOY_SESSION=... joy forge login --host github.com` refuses immediately with the delegation sentence.
Parallel with: J3, J4b, A1, J8, P3, P7, P4, P5, P9.

**J8 (joy): distribution shape.**
Depends on: J2.
Files: joy/dist-workspace.toml, the crate manifests, joy/.github/workflows/release.yml.
Acceptance: a tag produces exactly one archive set per target containing joy, the connector and the pin file of D1.4a, and winget matches exactly one asset; the Windows binaries are signed before bundling. J8 puts the pin file into the archive; placing it on an installed machine belongs to W1, and the install paths that carry no pin file at all are named in D1.4a.
Parallel with: J3, J4b, J10, A1.

**A1 (app): sidecars and bundling.**
Depends on: J2, J8.
Files: app/apps/desktop/src-tauri/tauri.conf.json, the build recipes, app/apps/desktop/src-tauri/src/plugin_ops.rs.
Acceptance: a freshly installed .dmg, .msi and .deb answers a forge question with no connector on PATH; the Windows sidecar is signed.
Parallel with: J3, J4b, J10, P3, P7, P4, P5, P9.

**P3 (platform): bearer sessions and CORS.**
Depends on: none.
Files: platform/src/auth/mod.rs:560-565, platform/src/api/mod.rs:144-147, the session issuance path, a migration for the bearer hash column.
Work: Authorization header parsing beside the cookie, CORS for desktop origins without credentials, the bearer token (issuance, hash only storage, thirty day life, re mint past half life through the `joy-session-token` header, revocation).
Acceptance: a release desktop build reaches the platform with a bearer and no cookie; revoking the desktop session from the web signs the desktop out within one poll; the web build is unchanged.
Parallel with: J3, J4b, J10, A1, P7, P4, P5, P9.

**P7 (platform): the claim exchange and the magic link device door (new).**
Depends on: P3.
Files: platform/src/auth/email.rs:39-42 (routes), the `magic_links` migration (claim_hash, user_code, device_label, parked_session_id), platform/src/auth/mod.rs (oauth callback parking), the sign in page and the new confirmation page, `POST /auth/session/claim` registered before the `{provider}` route.
Work: the whole of D4.1a on the server side, including the mail line with the user code, the confirmation page, the single use redemption with an answer that leaks nothing, one open claim per app instance and the `MAX_OPEN_LINKS` brake.
Acceptance: a magic link clicked in a different browser on a different machine shows the device label and the code, and only the Confirm button hands the session to the waiting desktop; the desktop redeems once and a second redemption answers 404; a claim that was never parked answers 404 in the same shape and the same time; the claim expires with its link.
Parallel with: J3, J4b, J10, A1, P4, P5, P9.

**P4 (platform): multi instance forges and the platform's own trust.**
Depends on: J2 (`forges.yaml` shape).
Files: platform/src/api/joy_service.rs:7138-7160, platform/src/auth/providers.rs:98-140 and :331-339, platform/Cargo.toml:58, app/apps/desktop/src-tauri/src/forge.rs:27.
Work: a list of bases per forge kind, GHES as its own kind, the Gitea authorize URL scopes, `reqwest` moved to `rustls-tls-native-roots`.
Acceptance: a GHES instance and two GitLab instances can be registered on one platform instance; the desktop shows the right sign in row for a corporate host; the Gitea sign in page lists `read:user write:repository`; the platform's own REST calls succeed behind an interception CA installed in the image.
Parallel with: J3, J4b, J10, A1, P3, P7, P5, P9.

**P5 (platform): job identity, audit, session windows.**
Depends on: none.
Files: platform/src/job/runner.rs:364-422, platform/src/job/watch.rs:73-259, platform/src/sync.rs:77-260.
Acceptance: a job commit carries the member id as the author email and a trailer naming the delegating human; the docs state which forge actor the push presents; one definition of "inside a member session" is implemented and tested.
Parallel with: J3, J4b, J10, A1, P3, P7, P4, P9.

**P9 (platform): create a repository from the platform (new).**
Depends on: J3 (the plugin verb).
Files: platform/src/api/joy_service.rs (new endpoint), platform/src/auth/repo_store.rs, the web Add dialog.
Work: one endpoint that calls the responsible connector's `create-repository` with the account's identity and registers the result as a project in the same call, with the `scope_missing` answer surfaced as "your GitLab sign in does not allow creating projects".
Acceptance: Troi, signed in with a magic link and one connected forge, creates a repository from the web app and lands in a registered project without leaving the browser; the same attempt with a read only GitLab set shows the scope sentence and a sign in again action.
Parallel with: J3, J4b, J10, A1, P3, P7, P4, P5.

### Wave 3

**J6 (joy): JOY-01FD-ED, git2 only in the CLI.**
Depends on: J4b, J4h, J7, J10. The `forge-net` flip of D3.1 is this package's own first commit (joy/crates/joy-cli/Cargo.toml).
Files: joy/crates/joy-core/src/vcs/mod.rs, joy/crates/joy-cli/**, joy/crates/joy-core/src/commit_msg.rs, joy/crates/joy-core/src/init.rs:255-271, joy/crates/joy-cli/Cargo.toml.
Work: the call list of D3.2, the in process validator of D3.3, path scoped commits of D3.4 including the `commit_all` and `commit_everything` audit for clean filters, the hook chaining of D3.5, one failure vocabulary with `--json`, the non interactive rule.
Acceptance: `joy` works end to end on a machine with no git binary, including chat push and release tag; a commit without an item reference is refused by joy itself; a repository with husky keeps its hooks and joy's commit-msg check runs before them; an agent under `JOY_SESSION` never prompts and prints stable status words; no ssh contact fails with "invalid or unknown remote ssh hostkey" without a way to accept.
Note on ordering: J10 is a hard dependency, because J6 removes the last `git push` call sites (vcs/mod.rs:270-283) and with them the last place where OpenSSH's own trust on first use applies. J4h must be green before J6 for the same reason.

**A2 (app): the platform address, the sign in surface and the two mode rule.**
Depends on: J3, A1, P3, P7.
Files: app/apps/desktop/src/main.tsx:132 and :319-325 (the platform address), app/packages/app/shell/src/shell/StartScreen.tsx, the desktop sign in window, app/apps/desktop/src-tauri/src/plugin_ops.rs, recent_ops.rs, app_state.rs.
Work: **D4.0 belongs to this package**, because nothing in D4.1 exists in a release build until it lands: the platform address moves out of `import.meta.env.VITE_JOYINT_SERVER` into the person's app state, defaults to `app.joyint.com`, is editable on the start page and gets `https://` prepended when no protocol is given. Then the rest: the two sections, the claim exchange client side, the two mode ownership rule of D4.1b and the Device host rows of D4.1c.
Acceptance: a release build with no baked `VITE_JOYINT_SERVER` reaches `app.joyint.com`, and a changed address in the app state is what the next sign in uses, with no rebuild (D4.0); the start page shows Account and Device separately; the magic link door works in the desktop; a device code appears in the window while the plugin is still running; cancelling closes the plugin and its children; a repository that is both a local clone and a platform project appears exactly once, in Device, with "Open on joyint.com" beside it and `auto_sync` off; each Device host row names its login and offers Sign out.

**A4 (app): add a project and clone.**
Depends on: J3, A1.
Files: app/packages/app/shell/src/shell/AddProjectDialog.tsx, app/apps/desktop/src-tauri/src/git_ops.rs (new clone command), joy/crates/joy-core/src/vcs/forge.rs:254-270 (depth parameter), app/apps/desktop/src-tauri/src/app_state.rs (the shallow flag).
Work: the clone path of D4.3, and **A4 records the shallow state of a clone** in the project's app state, because ahead and behind are approximate under depth 1 and A5 needs to know.
Acceptance: picking a repository, the size warning above 250 MB and the platform suggestion above 1 GB, a depth 1 clone with progress, cancelling mid clone and finding no leftover directory; a repository whose forge reports no size clones with no warning; a depth 1 clone is recorded as shallow and a deepened clone stops being recorded as shallow.

**A5 (app): worker, poll and banner.**
Depends on: J4b, J5, A4 (the shallow flag it reads).
Files: app/apps/desktop/src-tauri/src/sync_ops.rs, app/packages/tools/joy/src/chatTransport.ts, app/packages/app/shell/src/app/createForgeSync.ts, App.tsx.
Work: the fixed period poll on the period **J5 computes** from the budget table of D1.9 (never a number of A5's own), the banner sentences of D4.7 against the state names J5 defined, and the shallow rule: **on a clone A4 recorded as shallow, A5 shows no ahead and behind numbers**, only "in sync" or "changes to sync", because the revwalk applies the shallow grafts and the numbers would be approximate.
Acceptance: stated in HTTP requests per minute per host derived from the budget table of D1.9 (at most 40 per minute for a private chat poll on codeberg.org, at most 60 on github.com), the measured figure does not rise when a remote moves from ssh to the twin; a rate limited project stops hammering; each classifier state shows its sentence and its action and no raw libgit2 text is reachable from the surface; a shallow clone shows "in sync" or "changes to sync" and no numbers, while a full clone still shows both counts; the worker stops contacting the forge after the configured idle period and keeps committing locally.

**P2 (platform): adopt the engine changes.**
Depends on: J4b, J5.
Files: platform/src/api/chat_poll.rs, platform/src/git/mod.rs.
Work: the fixed period poll on the period **J5 computes** from the budget table of D1.9, the classifier J5 owns, and the engine changes of D1.5.
Acceptance: the platform's poll period is independent of the round trip time and equals the period J5 computes for the host (3 s on codeberg.org for a private chat poll); a platform push updates the tracking ref and fails on a per ref rejection.

**P6 (platform): member without a forge identity.**
Depends on: P3, and decision 12 (built on its recommendation, the registering account's grant as a project scoped credential, unless the operator decides otherwise).
Acceptance: Troi with a magic link account opens a private platform project, or is told at invitation time that a forge account is required, with the chosen sentence.

**P8 (platform): maintenance on the platform's clones (new).**
Depends on: J7.
Files: platform/src/sync.rs (a maintenance lane in `tick`), platform/src/git/mod.rs.
Acceptance: a long lived project clone that has accumulated chat commits for a month is packed and swept during an active member session and never outside one; the loose object count stays below 6700; a job container writing into the same clone does not lose an object (the grace window is honoured); nothing runs when no member session is active.

**J11 (joy): the remaining identity call sites in the CLI (new).**
Depends on: J9.
Files: joy/crates/joy-cli/src (the auth, crypt, chat, board, project, ai, event log and enroll commands), joy/crates/joy-core/src/identity.rs.
Work: the remaining `user_email()` call sites of D3.9 (20 in 9 files at the time of writing, minus the two J9 moves) move onto `joy_core::identity::resolve_identity`, which reads the delegation session first, then the project's member pin, and git config only as a prefill where a person is asked for an address. No command reads git config as its identity source afterwards.
Acceptance: every joy command that needs an identity works in a repository with no git config once the member is known; removing `user.email` from git config in a joy project changes no joy command's behaviour. Until this package is green, the "Level 1 solo" journey row stays partly and names J11.

**M1 (cross repo): migration and removal paths (new).**
Depends on: J8, J10, A2.
Files: the release notes, website/public/install/joy.sh and joy.ps1 (receipt), app/apps/desktop/src-tauri/Cargo.toml (keyring features), joy/docs.
Work: the whole of D3.12, including the deprecation window for the three legacy binary names, the keyring feature change with the statement that nothing survives to migrate, and the two removal paths for a stored credential.
Acceptance: an upgrade on a machine with three stale plugin binaries keeps working, names them in `joy forge plugins` and prints the exact `rm` line; a Linux desktop that previously lost its remembered seed at every reboot keeps it after the feature change; `joy forge logout --all` and the Sign out action both remove the credential and say what they could not revoke.
Parallel with: J6, A4, A5, P2, P6, P8 (it starts when A2 is green).

### Wave 4

**W1 (website): installer scripts.**
Depends on: J8.
Files: website/public/install/joy.sh:110-120, website/public/install/joy.ps1:80-95.
Work: install both binaries out of the one archive, write the archive's `host-keys.json` to `<prefix>/share/joy/host-keys.json` (D1.4a), and name all of it in the install receipt.
Acceptance: after the one command install, `joy release publish` on a github.com remote reaches the connector with no further install step; the receipt lists every installed binary and `joy update` updates all of them; the pin file of D1.4a lies at `$HOME/.local/share/joy/host-keys.json` and nowhere inside the install directory, a second run of the installer replaces it, and the receipt names it.

**W2 (website and docs): copy and documentation.**
Depends on: J6, A2, A4, A5.
Files: website/src/i18n/text/app.ts:52-57, website/content/joy/docs/tutorial.mdx:462, joy/docs/plugins.md, the C4 pages, app/VISION.md:63-66, app/ARCHITECTURE.md:174-176, ADR JAPP-0024-64, ADR JAPP-0053-B0, docs/dev/vision/ForgeSync.md.
Acceptance: the "no git, no gh" sentence is published only after J6, J2 and A2 are green; the tutorial no longer teaches `git config` as the identity source; the four documents that promised a sparse `.joy` checkout say what the engine can do; "What joy reads from your machine" exists and lists the ssh, known_hosts, proxy and CA sources per operating system.

**T1 to T5 (umbrella tests).**
Depends on: the behaviour each guards.
Files: tests/validation/cx/*, joy/tests/integration/*, app e2e.
Acceptance: the five journeys of D6 run in CI; the request rate test fails on a regression; the maintenance test fails when a store grows past the bound; the classifier test covers both error corpora; the known_hosts test covers hashed, mismatched and unparsable cases; the Windows, proxy and self hosted matrix is either automated or documented as a manual release check. T4 additionally covers the macOS question that is still open: a signed app sidecar and a separately installed CLI copy read the same keychain entry without a dialog, or the result is recorded and decision 6 becomes mandatory.

## 5. Decisions for the operator

Decisions 1 to 18 are carried over from v2 with their recommendations unchanged unless marked; 19 to 29 are new or newly decided in v3; 30 is new in v4.

1. **Own credential helper runner.** Implement it (D1.3) and never call `git2::Cred::credential_helper`, which spawns a git process on every platform.
2. **Shell shaped helper values on Windows.** Run them under the Git for Windows `usr\bin\sh.exe` found through the registry, never `sh` from PATH and never git.
3. **One OAuth app or two.** Register a separate public client for desktop and CLI. Reusing the platform's app couples revocation, token expiry and audit.
4. **GitHub token expiry.** Implement refresh in the connector for the new public client and leave the platform's app with expiry off until the platform is ready.
5. **Connectors hold state.** Accept it and rewrite docs/plugins.md into a connector section. The alternative means the app cannot sign in locally at all.
6. **Connector binary shape.** One binary shipped in the joy archive, with `joy-<forge>` names kept as a PATH fallback for one deprecation window (D2.2a). The size this rests on is measured, not assumed: J2's acceptance records the stripped size of `joy-forge` per release target, and this decision is revisited when any target passes 10 MB.
7. **Keychain fallback.** A 0600 file in joy's own config directory where no secret service is available, labelled in the UI and the docs exactly as gh labels its own fallback.
8. **`core.hooksPath`. Decided differently from v2.** joy owns the value and chains to the person's previous path or `.git/hooks` from joy's own hook (D3.5). The v2 formulation was self contradictory: chaining requires owning the path, and not owning it means the item rule is enforced for nobody.
9. **Commit signing.** joy's commits are unsigned, documented per forge, with signing as a later item.
10. **Prune grace window.** 14 days in a checkout joy does not own, 24 hours in the platform's own clones, never `now` anywhere.
11. **Twin policy. Now implemented as decided.** The twin is used when there is no working local ssh credential, or after an ssh failure, remembered per host, and never when an insteadOf rule matches (D1.2). The v2 rule ("a token exists") is removed.
12. **Platform members without a forge identity.** A project scoped credential (short term the registering account's grant, named in the UI and the audit; long term the GitHub App of JP-00C9-D4).
13. **The desktop worker while unattended.** Keep committing locally, stop contacting the forge after a configured idle period (D4.6).
14. **SSO for level 4.** Out of scope for this rebuild, in the backlog with a date.
15. **Mobile.** Named as a changed journey with the reason and the list of what a mobile host needs (D4.9).
16. **Windows Credential Manager persistence.** Accept `CRED_PERSIST_ENTERPRISE` for now and note the roaming in the docs.
17. **`create-repository`.** Include it now, as a connector verb (J3) and as a platform endpoint (P9).
18. **Classifier vocabulary.** Add the new states with the documented mapping to the old four words (D1.8b).
19. **Keychain addressing and foreign CLI stores. New, decided.** `Entry::new(service, user)` only, and joy never reads gh's, glab's or tea's own store. Foreign credentials are obtained by spawning the CLI, which is also the only way their refresh runs, and they are read only for joy. Alternative: read them directly with `new_with_target`, which needs three different target semantics per platform and risks a macOS dialog from a foreign binary.
20. **Anonymous polling. New, decided.** Never. An https remote with no credential is polled at most once per 15 minutes per host, with the sentence that names the reason and offers sign in; person initiated one off contacts stay allowed. Alternative: poll anonymously, which exhausts GitHub's 60 requests per hour per IP in about a minute.
21. **How `no_push_rights` is decided. New, decided.** From a real push failure with status 403 or 404 on the push direction, or from `probe_write_access` run on the transport that carries the credential. Never from a probe on a transport that has no credential; that case is `needs_sign_in`.
22. **The release verb. New, decided.** It moves to the connector's own HTTP client in J2, so that publishing a release needs neither gh nor curl. Alternative: keep gh and mark the CLI release journey as partly served.
23. **Host key pinning. New, needs a word from the operator.** Recommendation: ship the published host keys of github.com, gitlab.com and codeberg.org in a replaceable data file, consulted only when no known_hosts file has any line for that host. The trade off stated plainly: pinning replaces the person's first contact decision with trust in the joy release, removes the one moment where a man in the middle could be caught on a fresh machine, and makes joy refuse a legitimate key rotation at those hosts until a new pin ships. The alternative (no pin) is honest too, but then a container or a fresh CI machine can never use an ssh remote, which is the platform's situation today.
24. **Proxy credentials as URL userinfo. New, decided.** git2 hardwires `ProxyOptions::raw`'s credentials callback to None (proxy_options.rs:44-53), so the only channel that works is userinfo in the proxy URL, resolved through joy's helper runner and held in memory. Alternative: no proxy authentication at all, which fails every corporate proxy that asks.
25. **The Linux CA escape hatch. New, decided.** On Linux only, joy honours `ca_bundle`/`ca_dir` from `forges.yaml` and `http.sslCAInfo`/`http.sslCAPath` from git config, applied once at start through `git2::opts`. On macOS and Windows those are refused with the sentence that names the system store, because `GIT_OPT_SET_SSL_CERT_LOCATIONS` is compiled only for OpenSSL and mbedTLS.
26. **The lean clone shape. New, decided.** Depth 1 with a full working tree on the desktop, depth 0 for the CLI. There is no sparse and no partial clone with this engine, and the four documents that promised one are changed (D4.3, W2).
27. **The interactive cargo feature and its CI guard. New, decided.** joy-core gains `interactive`, enabled by joy-cli and the desktop and not by the platform, with a runtime refusal for `Background` and `Delegated` hosts and a pipeline check that the platform's feature graph never contains it.
28. **GitLab application scopes. New, decided.** Register the gitlab.com application (and every operator registered one in `forges.yaml`) with the union `api write_repository`, because a device request may narrow but never widen. The login then asks for the narrowest set the verb group needs.
29. **The new commands, flags and features. New, needs an approved item.** JI-017C-E1 asks that any commit adding a flag, option, scope or mode be covered by an approved item that names it, so one item names every one of them. The complete list this design adds: on the CLI `--token-stdin`, `--all`, `--for <read|write|create|release>`, `--login <name>`, `--host <hostname>` and `--json` on each new `joy forge` command (the existing global flag applied to a new surface); on the connector protocol `--host`, `--host-kind <interactive|background|delegated>`, `--login`, `--for`, and the verb options `--query`, `--limit`, `--page` (repositories) and `--name`, `--owner`, `--private` (create-repository); the `interactive` cargo feature of D3.11; the `JOY_PLUGIN_DIR` test hook of D2.2 for as long as it is kept; the `forges.yaml` file and its keys (D2.5, including `ca_bundle`, `ca_dir` and `scopes`); the configured idle period of the desktop worker (D4.6, decision 13); the `forgeLogin` pin in the per project app state (D4.1c). `JOYINT_FORGE_MIN_GAP_MS` exists today (platform/src/config.rs:554) and is not new. The item's working title is **"forge connection NG: new commands, flags and features"**, and it has to be approved before the first commit of J10 and before the first commit of J1, because J1 is where `--host` and `--host-kind` first appear. The `joy forge` command group itself is planned work, not a switch beside a broken path, because no CLI sign in path exists to stand beside; the item names the group as well, so that one item covers the whole surface.
30. **The organisation test estate. New in v4, needs a word from the operator.** `needs_org_approval` and `needs_sso` cannot be tested without a GitHub organisation with OAuth app access restrictions switched on and a second organisation with SAML enforced. Either the operator provides both, or both states ship marked untested in the docs, in the release notes and in Picard's journey wording. There is no third option: D1.8b, D2.7c and D2.10 branch on these two states, and a branch nobody has taken is a paper promise.

## 6. Risks and unverified points

### Settled by the new verdicts, and therefore no longer risks

Recorded here so nobody re opens them: libgit2 refuses an unknown ssh host key by default (R2, the opposite of v2's assumption); libgit2 has no sparse and no partial clone but does have depth (R6); the Windows error vocabulary and the localisation mechanism (R3); the safety of the loose object sweep beside another process, with a measurement of the keep set walk (R8); the twin push tracking ref question, which turned out to be an anonymous remote question only (R8); the contact arithmetic in requests (R8); whether a REST oracle call is safe per forge (R3, answered per forge); the proxy and trust store behaviour per OS (R1); whether keyring can read go-keyring items, which is now moot because joy no longer reads foreign stores (decision 19).

### Verified as open

- **macOS keychain identity.** Whether the app sidecar and a separately installed CLI copy of the connector carry the same code signing identifier. tauri.conf.json names no `signingIdentity`. If they differ, every CLI update re prompts and decision 6 (one binary, one identity) becomes mandatory rather than preferred. T4 tests it on a signed build.
- **`errSecInteractionNotAllowed` and a locked Secret Service.** How keyring 3.6.3 surfaces the headless macOS case, and what happens when a default collection exists but is locked with no prompt agent. D1.10's promise is narrowed to what the mechanism covers and the bounded deadline is the guard, but the classification of "locked or no interaction" is still built on a numeric mapping from search results.
- **Windows integrated proxy authentication.** That `acquire_fallback_cred` fails with a NULL URL under `GIT_PROXY_AUTO` is marked inferred in the verdict, not executed. The design avoids the case by always using `GIT_PROXY_SPECIFIED` when a proxy is known, so nothing depends on it.
- **The connector's size after rustls, OAuth and keyring.** Decision 6 rests on a 1.2 MB measurement taken on today's plugins, which shell out to curl and gh. J2's acceptance now measures the stripped size per release target and records it, and decision 6 is revisited when any target passes 10 MB.
- **Organisation cases.** OAuth app policy and SAML cannot be tested without a restricted organisation. Without one, `needs_org_approval` and `needs_sso` ship untested and Picard's journey stays a paper promise. It is also still unsettled whether GitHub's organisation restriction blocks git over https or only the REST API, and whether a device flow token behaves identically to a web flow token under such a policy. Decision 30 asks the operator for that estate and names the alternative in the same sentence.
- **GitHub's git protocol limits** are undocumented, so the 1.0 requests per second budget for github.com is a judgement and the request rate test is what guards it. Whether GitHub and Codeberg meter git over ssh separately is also open.
- **gitlab.com's failed authentication ban numbers** are documented but not observed. The design avoids the wrong credential shape entirely and caps re asks at one, so this bounds damage already done rather than adding a new risk.
- **Windows ssh reachability.** Whether gitlab.com, Codeberg and typical self hosted Gitea and Forgejo still offer an RSA host key and a kex the WinCNG build supports. If any of them drops RSA host keys, the Windows carve out of D1.2 becomes total and the token path is the only path there.
- **macOS under the hardened runtime.** Whether a notarized, Gatekeeper launched bundle still inherits the launchd provided `SSH_AUTH_SOCK`. Expected yes, not verified against a signed build.
- **Foreign CLI stability.** `glab auth credential-helper` is a hidden cobra command and may change without notice (a CI smoke test is the mitigation); `tea login helper get` exists at v0.16.0 and older installs need the yaml fallback; whether the builtin Gitea and Forgejo OAuth applications are enabled on codeberg.org could not be probed anonymously.
- **The platform's existing OAuth App settings.** Whether "Expire user access tokens" is off has to be read in the app settings, not assumed.
- **Codeberg pinning.** Codeberg publishes fingerprints and no key blobs, so its pin has to be built from a blob taken once and checked against the published fingerprints at build time. Until that step runs, the Codeberg pin is the weakest of the three. The step now has an owner: it is J4h's work and part of J4h's acceptance.
- **Whether joy is willing to diverge from git on insteadOf.** Honouring it defeats the twin exactly in the corporate setups that need it most; ignoring it makes joy behave unlike git. v3 honours it and refuses the twin, which is the conservative side.
- **`keyctl show @s` on the operator's Linux machines.** Whether the existing seed entry already dies at reboot today. The migration in D3.12 assumes it does and therefore writes no migration script.
- **Packaging details.** Whether `azure/artifact-signing-action@v2` recurses into subdirectories, and the RPM bundler's path for external binaries. Both decide whether the connector is signed and where it lands.
- **Flatpak or Snap.** Not a current bundle target; a sandboxed build would additionally need the agent socket held inside the sandbox.

### Asserted and still to be checked

- That the claim exchange of D4.1a behaves as designed against a running platform. It is built from verified facts (no production CORS, `SameSite=Lax`, the existing `magic_links` table and its TTLs, `safe_next`), but JAPP-02CA-F8 has no platform side code today, so P3 and P7 are the first time it runs.
- That one `GET /rate_limit` call per host per strike window stays inside GitHub's secondary limit. The documentation says the endpoint "does not count against your primary rate limit, but it can count against your secondary rate limit", and the design's once per 600 s rule is a judgement against that sentence.
- That the request budgets of D1.9 hold in practice for Codeberg. The 1.1 requests per second ceiling is one measurement (JP-00EF-CC) against a limiter an admin describes as "Currently: 2000 requests / 300s" with no documentation page.
- That a GitLab or Gitea token with the read only set answers `store` and `files` on every instance. The scope rules are read from the servers' own source, but no plugin token has been driven against those endpoints yet; J2's acceptance is where it is proven.

### What could still break the plan

- J6 sits at the end of a long chain (J4b, which needs J3, which needs J1 and J2, plus J4h and J10). If the connector work slips, the CLI keeps the git binary and the website wording cannot be published. Mitigation: J1, J4a, J5, J7, J9 and P1a are independent and land first in wave 0, and J4h follows immediately in wave 1 on J4a and J5, so the helper runner, the contact discipline with the classifier vocabulary, the host key check, the maintenance and the identity plumbing are ready before the resolver assembly.
- J4h is now a hard blocker for J6 rather than a nicety, because the last `git push` call sites are the last place where OpenSSH's trust on first use applies, and because the desktop already has the live bug this package fixes.
- The `forge-net` flip pulls vendored OpenSSL into every CLI build and into `cargo install joy-cli`. If that is unacceptable for the from source path, a second feature shape is needed and J6 grows.
- The `interactive` feature is only as good as its CI guard. Without the `cargo tree` check in the platform pipeline, a later `features = ["forge-net", "interactive"]` compiles silently.
- Mobile and SSO are out of scope. If either is promised during this rebuild, the plan does not hold: mobile needs the connector logic as a library, SSO needs a new provider kind on the platform.

## 7. Changes from v2 and v3

### The ten contradictions

1. **Twin trigger.** v2's D1.2 rule 2 fired the twin because "a forge token for the host is known", against its own verdict row C14 and decision 11. v3 D1.2 fires it on exactly two triggers, both recorded per host: joy established before the contact that there is no usable ssh credential (no agent identity, no readable unlocked key, which is the Windows norm), or the ssh contact failed with an authentication class failure. A host whose memory says `ssh-worked` never goes to the twin.
2. **Hooks.** v2's D3.5 refused to set `core.hooksPath` and chained from it in consecutive sentences. v3 D3.5 decides: joy sets the value, records the previous path in `.joy/hooks/chained-path`, and every joy hook ends by executing the same hook from that path or from `$GIT_DIR/hooks`, with one sentence at install time.
3. **Keychain addressing.** v2 forbade `new_with_target` while requiring reads of `glab:<host>:token`. v3 D2.6 drops the foreign read entirely: joy spawns gh, glab and tea (which is also what runs their refresh), uses `Entry::new` for its own entry only, and treats every foreign credential as read only, naming the foreign command at logout.
4. **Anonymous polling.** v2 allowed an anonymous fetch inside the candidate order. v3 D1.2 removes it from every poll: an https remote with no credential is polled at most once per 15 minutes per host with the sentence "Not signed in to github.com. joy checks for changes every 15 minutes. Sign in for live updates." Person initiated contacts stay allowed.
5. **`no_push_rights`.** v2 decided it from a probe pinned to the configured remote, which on the Windows case can carry no credential at all. v3 D1.5 runs the probe on the transport that carries the credential and, with `push_update_reference` now set, decides the state equally from a real push; with no credential on either transport the state is `needs_sign_in`.
6. **Host kind.** v2 called it a parameter and then read `JOY_SESSION` inside the rule. v3 D1.1 makes each host set the parameter once at its entry point: joy-cli reads `JOY_SESSION` in `cli_main`, the desktop and the platform set it explicitly, and the engine never reads the environment again.
7. **GitLab scopes.** v2's set could not serve `create-repository`, because `write_repository` "Does not support API authentication". v3 D2.7a defines three sets, puts `api` behind create and release, answers the verb locally with `scope_missing` when the set is too narrow, and registers the application with the union so a device request can narrow.
8. **The release verb and gh.** v2 declared the plugins free of gh and curl and left `release` unchanged, while github.rs:279-346 reaches the release API entirely through gh. v3 D2.8 moves `release` to REST in J2. v4 splits the acceptance by wave, because the connector has no credential store before J3: J2 proves `joy release publish` on a machine without curl, with `GH_TOKEN` set or with gh signed in, and J3 proves it on a machine with neither gh nor curl.
9. **The macOS row.** v2 graded it "yes" while section 6 named its mechanism as the single unconfirmed dependency. v3 grades it "partly", states what was removed from the risk (no foreign store reads any more) and what remains (the signing identifier), and T4 tests it.
10. **The no prompt promise.** v2 promised "no macOS keychain dialog" with no mechanism and dropped the verdict change that proposed one. v3 D1.10 lists the five mechanisms that are verified (host kind parameter, joy's own prompts gated, per spawn helper environment, compiled out interactive verbs with a runtime refusal, host kind on the wire) and narrows the promise to "joy raises no prompt of its own", with the verb deadline and process group kill as the bound on an operating system dialog and the error mapping left in section 6.

### The fourteen missing topics

1. **Proxies.** D1.11: one `options_for` function on every call site, libgit2's `GIT_PROXY_AUTO` order quoted, joy's own NO_PROXY applied to config sourced proxies too and with whitespace trimming, `ALL_PROXY` through joy, proxy credentials as URL userinfo, the per OS source list including the Windows registry only rule, SOCKS and NTLM refused by name, and the desktop's environment import extended.
2. **TLS and CA bundles.** D1.12: the three trust stores with their decisive source lines, the Linux only escape hatch through `git2::opts`, the list of git keys joy does not read, no mTLS, no verification switch, and the `tls_untrusted` state with a next step per OS.
3. **known_hosts.** D1.4a: every file ssh would read, plain, bracketed and hashed matching, `@revoked`, certificate host keys, the pre validation that stops one bad line from killing every connection, the rule per host kind, and the pins with their trade off stated.
4. **A CLI sign in surface.** D3.10: `joy forge login | status | logout | plugins`, with `--json` shapes, exit codes and the four failure sentences that now point at it.
5. **A token paste path.** D3.10 and D2.4: `joy forge login --token-stdin` reading one line from stdin into the plugin's `token-store`, never argv, which covers a Linux server, a CI runner, a browserless Windows host and every Gitea family instance with no registered client.
6. **Protocol versioning and stale binaries.** D2.2a: the `version` verb, the protocol 1 detector that needs no cooperation, the `joy-forge` first name order, `plugin_outdated` with the resolved path and the `rm` line, and a deprecation window.
7. **Migration.** D3.12 and package M1: the three binaries, the two images, the status words, the Linux keyring entries (nothing survives, so no script), and the two removal paths for a stored credential.
8. **Platform maintenance owner.** D5 and package P8: a lane of the sync worker's tick, inside an active member session, under the project gate, with the 24 hour window, plus the correction that the agent image is the one with git and that its job container shares the clone read write.
9. **The interactive gate.** D3.11: the `interactive` cargo feature that the platform's separate workspace cannot turn on, a signature that forces a progress sink, a runtime refusal for `Background` and `Delegated`, and a CI guard.
10. **The classifier by class and code, Windows included.** D1.8a and D1.8b: `error.code()`, `error.class()`, one status regex over exactly two formats, English literal prefixes only as a last resort, the reason (the two transports are mutually exclusive per build and Windows loses `GIT_EAUTH` on a 401), and two acceptance tests with non English message tails.
11. **The two mode ownership rule.** D4.1b: the normalized origin as the join key, Device owns a repository whenever a local clone exists, the other mode is an action, `auto_sync` off on a joined row, and the three sentences the person reads.
12. **Sparse, partial and shallow.** D4.3: sparse impossible, partial impossible, shallow available with its caveats, the lean shape defined as depth 1 with a full working tree, and the four documents that promised a sparse `.joy` checkout corrected together.
13. **The undefined prompt opt out.** Removed. D1.3 sets the no prompt environment for `Background` and `Delegated` and for nobody else; there is no setting, no flag and no config key with that meaning.
14. **Multi account per host and the refresh lock.** D4.1c: a deterministic login order with a device local pin outside the committed project file, and the sentence naming which forge account a push shows. D2.6a: one exclusive advisory lock per host and login, owned by the connector, in the state directory, with the compare and swap after re reading, the bounded wait, the Windows and unix behaviour, and the honest limit that no lock binds gh, glab or tea.

### The ten unverified load bearing points

- The oracle question is answered per forge (D2.10): GitHub only, with `GET /rate_limit`; GitLab gets a local verdict with the documented ban times; the Gitea family is never asked.
- The sweep is answered with source and a measurement (D3.7): the loose read holds no handle, `git_odb_read` refreshes and retries out of the new pack, freshening falls through to the pack, and the keep set walk costs about 1400 file opens and under 100 ms on the real store.
- The tracking ref question is answered (D1.5): the named remote already gets one, only the anonymous twin does not, force is handled by the same rule libgit2 uses, and per ref rejections are now read instead of being reported as success.
- The locked Secret Service case stays open and is named in section 6; D1.10 no longer promises more than the mechanism covers.
- The magic link question is answered with a mechanism (D4.1a) and a package (P7), and the remaining risk is that it has never run against the platform.
- The `certificate_check` matching side is answered (D1.4a): the callback yields the raw blob, whose base64 is literally the known_hosts key field, and `SHA256:` plus unpadded base64 of `hash_sha256()` is the string the forges publish.
- The connector size question stays open and is named in section 6.
- The contact arithmetic is restated in requests (D1.9) with a per verb table, the two connection fetch fixed, and the acceptance criterion rewritten in requests per minute per host.
- The GitLab and Gitea scope question is answered (D2.7a) and is proven by J2's acceptance rather than assumed.
- The `cargo install` layout is answered (D2.2a): the name order inside every directory, the version handshake, and the removal of the legacy names one release later.

### The seven journey breaks

- **Scotty level 1** drops to "partly": the founding step is closed by J9, the remaining `user_email` call sites are named as a follow up item instead of being covered by a grade.
- **Scotty publishes a release** becomes "yes" only because `release` moves to REST in J2 and the acceptance says so (v4: the gh free half of that acceptance sits in J3, where the connector's own credential arrives).
- **Windows host with no agent and no sh** becomes "yes" for the CLI half too, because `joy forge login` and `--token-stdin` exist (J10) and the registry proxy is stated (J4p).
- **Troi starts a new project** becomes "yes" because the platform half now has a package (P9), not only the connector verb.
- **macOS sidecar and keychain** drops to "partly" with the named unconfirmed property and a test in T4.
- **Troi joins from the app** becomes "yes" because the claim exchange is specified end to end and has a package (P7), including the confirmation step that RFC 8628 section 5.4 requires (v4 regrades it partly: the exchange has never run and D4.0 lands in A2, see the v4 fixes).
- **Data under a delegation session** drops to "partly": the mechanisms that exist are listed, the one that does not (suppressing a foreign operating system dialog) is bounded rather than promised, and the error mapping stays in section 6.

### v4 fixes

1. **Numbering.** The body's numbers are canonical and unchanged. Section 2 gains an index table of every `Dx.y` header with its title, and every reference in sections 3 to 7 was checked against it: all of them resolve to a header in section 2, including `D1.8a`, `D1.8b` and `D1.8c`, which are level five headers under D1.8.
2. **One `certificate_check` owner.** git2 0.21 has one slot, so one module owns it: `vcs::certificates`, package J4h. The one closure dispatches on the certificate kind: the host key branch runs the known_hosts check of D1.4a and answers `CertificateOk` or an error and never passthrough; the x509 branch returns `CertificatePassthrough` so libgit2's verdict stands and stashes only issuer and subject for the detail line, because git2 0.21 drops the valid flag; the state comes from the classifier. Written in D1.4a, in D1.8c, in D1.12, in J4h and in J4p, and J4p adds a branch rather than a callback.
3. **One budget table.** D1.9's requests per second table (codeberg.org 0.9, github.com 1.0, gitlab.com 5.0, unknown self hosted 1.0) is marked canonical and carries the derived milliseconds per request (1111, 1000, 200, 1000), computed as `1000 / budget`. No independent millisecond figure survives anywhere, and J5's and A5's acceptance criteria are stated in HTTP requests per minute per host derived from that table.
4. **The Codeberg poll, decided.** The period per host is `requests(verb) / budget` rounded up to the next whole second, so a private https chat poll on codeberg.org runs at most every 3 s. The either or sentence is gone: there is no REST door on the Gitea family and D2.10 forbids the probe on that host anyway. J5 computes the period, A5 and P2 use it (D1.9, J5, A5, P2).
5. **The wave layout.** J5 moves to wave 0 and additionally owns the classifier enum with every new state name of D1.8a and D1.8b; J4b keeps the resolver assembly and no longer defines the classifier. J4h moves to wave 1 and depends on J4a and J5, with its known_hosts append on the cross process file lock J4a lands in wave 0 and J3's refresh lock reusing it. J4p stays in wave 1 and depends on J5 and J4h. P1 is split into P1a (wave 0: the warn log shared with J1, the identity row by the project's forge, a startup probe that runs `claims`) and P1b (wave 1, depends on J2: the connector in both images and the `version` probe). Every "Parallel with" list is rewritten to match.
6. **J2 and the release verb.** J2 still moves `release` to REST and removes gh and curl from every API path, and it now says where the token comes from in wave 1: `CallerFacts.token_env` or spawning `gh auth token`, which decision 19 allows. J2's acceptance is "without curl, with `GH_TOKEN` or gh signed in"; the "with neither gh nor curl" acceptance moved to J3.
7. **The GitLab 403 wait.** One rule, and it lives in D2.10: the documented wait per instance kind, 15 minutes on gitlab.com and a default of 1 hour self managed. The classifier row in D1.8b repeats the same two numbers and points at D2.10 as the one rule.
8. **Decision 29.** It enumerates every new flag, option and feature: `--token-stdin`, `--all`, `--for`, `--login`, `--host`, `--host-kind`, `--json` on the new commands, the `interactive` cargo feature and the `JOY_PLUGIN_DIR` test hook. One JI item with the working title "forge connection NG: new commands, flags and features" names all of them before the first commit of J10 and of J1. D3.10, D2.2, J1 and J10 point at it.
9. **Journey grades.** Five rows drop to partly with their reason and their owner: Geordi opens a platform project and Troi joins from the app (the claim exchange has never run, and D4.0 is now in A2's scope with main.tsx:132 and :319-325 in its file list and its own acceptance line); a machine that has never seen the host key (partly for `Background` and `Delegated`, which depend on decision 23, with the Codeberg pin build step added to J4h's work and acceptance); a large repository on the desktop (ahead and behind are approximate under depth 1, so A4 records the shallow state and A5 shows no numbers on a shallow clone, only "in sync" or "changes to sync"); Picard behind an intercepting proxy (NTLM and Negotiate to a proxy are refused on Linux and macOS, Windows goes through WinHTTP, Basic and URL userinfo elsewhere).
10. **The still open points.** Scotty level 1 gains package J11 (wave 3, depends on J9): the remaining `default_vcs().user_email()` call sites in joy-cli move onto `resolve_identity` (session, then the member pin, then git config as a prefill only), and the journey row stays partly until J11 and names it. The connector size is measured in J2's acceptance per target, with decision 6 revisited above 10 MB. Organisation policy testing becomes decision 30. The headless macOS keychain mapping stays in section 6 unchanged. The desktop platform address (D4.0) is owned by A2.
11. **The login field.** D2.4's token answer states that `login` and `chose_by` are part of the shape because D4.1c requires them, and J3 now reads "with the shapes of D2.4 as amended by D4.1c".
12. **Everything else is unchanged.** No section was dropped or shortened, no prose outside these twelve points was rewritten, section 7 is renamed "Changes from v2 and v3", and this list is its v4 half.

### v5 fixes (after the third critic)

1. **The lock primitive has an owner.** The cross process advisory file lock (`joy_core::util::file_lock`, fs4) lands in wave 0 with J4a; J4h's known_hosts append and J3's refresh lock reuse it. The sentence that called `checkout_gate` (a per process mutex map) that primitive is gone (D1.4a, D2.6a, J4a, J4h, J3).
2. **The x509 branch reads no flag.** git2 0.21 drops libgit2's `is_valid` before the closure runs, so the branch stashes only issuer and subject for the detail line and returns passthrough; `tls_untrusted` is decided by the classifier from `code == Certificate` or `class == Ssl` (D1.4a, D1.8c, D1.12).
3. **The pin is behind decision 23.** Until the operator decides, the pin file ships empty and `Background` and `Delegated` hosts refuse an unknown host with `needs_host_trust` (J4h).
4. **In wave dependencies are listed once** in the section 4 intro, "Parallel with" means "may overlap", A5 depends on A4 for the shallow criterion, and M1 has a parallel line.
5. **`needs_sso` has a home**: a classifier row, the old word mapping (reads as `denied`), a banner row, and decision 30 names D2.7c.
6. **Decision 29 lists every new option**, including the connector verb options, the `forges.yaml` keys, the idle period and the `forgeLogin` pin, and states that `JOYINT_FORGE_MIN_GAP_MS` exists today.
7. **Two journey rows name their packages** (the sign in bar; the Windows host), the duplicated Geordi row is removed, and the `user_email` count is the measured one (20 hits in 9 files).
8. **Small reference fixes**: J4b's file list no longer names contact.rs; J6 owns the `forge-net` flip; P6 depends on P3 and builds on decision 12's recommendation; J1's `joy-forge` acceptance names a test stub; J11's acceptance no longer leans on W2; the v4 fix 7 self description is corrected; D1.9 no longer overstates D2.10.
