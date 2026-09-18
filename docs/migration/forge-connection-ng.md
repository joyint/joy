# Upgrading across the forge connection rebuild

What is on a machine before the upgrade, what the upgrade does with it,
and the handful of cases where a person has to do something. Every
sentence here describes the behaviour on this branch; where nothing
happens, that is said too, because "no migration step" is the useful
answer and only useful when it is stated.

The short version: no data migrates and no script is written. Three
things may want a hand, all of them optional and all of them named
below: removing a stale connector binary, re-pointing a hook path in one
narrow case, and unlocking a Linux login keyring that nobody unlocks.

## The three connector binaries

Before this rebuild a workstation had up to three connectors,
`joy-github`, `joy-gitlab` and `joy-gitea`. They reached a machine
through `cargo install` or a source build. Now one binary, `joy-forge`,
carries every forge.

**joy deletes nothing.** Inside each search directory the name order is
`joy-forge` first and the legacy `joy-<forge>` name second, so a stale
`~/.cargo/bin/joy-github` beside a fresh `joy-forge` is simply not
asked. `joy forge plugins` names it anyway, with the line to run:

    github  /home/s/.local/bin/joy-forge  protocol 2  PATH  joy-forge 0.20.0
      problem: shadowed-legacy
      another binary for this forge is installed and unused; rm /home/s/.cargo/bin/joy-github

Run the `rm` line or leave it; nothing breaks either way. joy never
removes a binary it did not install.

**The installers are the exception, and a narrow one.**
`curl get.joyint.com/joy | sh` and the PowerShell installer ship `joy`
and `joy-forge` in one archive and write both into the install receipt.
On the next run they remove a legacy connector name only when the
PREVIOUS receipt lists that exact path as their own file. A connector
you installed yourself with `cargo install` is never in that receipt and
is never touched. `joy update` re-runs the installer, so joy and the
connector stay in lockstep.

**A machine with only an old connector still works, partly.** A
protocol 1 binary is still asked the six original verbs (`claims`,
`identity`, `resolve`, `store`, `files`, `release`) about a REMOTE
target, so publishing a release keeps working on a machine nobody has
upgraded. Only three of the six are handed a `--remote` argument with
it, the three whose protocol 1 parser knows one: `claims`, `store` and
`files`. Giving `release` one is exactly what would end the publish,
because its old parser exits 2 on an argument it does not know. Asked
anything newer, the connector produces the state `plugin_outdated`,
which names the file that answered and the fix in one sentence. The
detection needs no cooperation from the old binary: its argument parser
exits 2 with usage on stderr and nothing on stdout, and that is the rule
joy reads.

## The server and agent images

`platform/Dockerfile` copied three connectors into `/usr/local/bin` and
the agent image shipped none. Now the server image copies `joy-forge`
alone and the agent image installs it beside `joy`.

Images migrate by rebuild, and there is no mixed state to plan for: the
server binary and the connector travel in one image, so a container is
either wholly before or wholly after. The legacy names are deliberately
NOT in either image; they exist for `cargo install` users during the
deprecation window, and inside an image a legacy name could only ever be
a stale one shadowing the current connector.

The server asks the connector `version` and then `claims` once at boot
and says so when the connector is missing, broken or speaks the old
protocol, with the resolved path, the plugin id and the verb. So a lost
`COPY` line is named at the first boot of the image rather than at the
first forge contact of the first project.

## The status words, and what an older app sees

The engine's classifier now speaks fourteen words where it spoke four:
`needs_sign_in`, `needs_org_approval`, `needs_sso`, `no_push_rights`,
`needs_host_trust`, `scope_missing`, `plugin_missing`,
`plugin_outdated`, `tls_untrusted`, `proxy_auth`, `rate_limited`,
`offline`, `denied` and `error`.

**Nothing new goes on the wire, and no second field was added.** The
platform maps every verdict back to the four words it always sent before
writing the sync status: every refusal of a login reads as `denied`,
every fault of the machine reads as `error`, and `rate_limited` and
`offline` are unchanged. An older app build therefore sees exactly the
vocabulary it was written against, blocks writes on exactly the same
verdicts, and needs no upgrade to stay correct.

The finer words are still available where both halves are new: the
desktop gets them from its own engine, because its forge contacts do not
go through the platform at all. When the platform later starts sending
them, the same field carries them, and the mapping above is what an app
that does not know a word falls back on: an unknown word shows the
neutral banner and blocks nothing.

The rule, said once: one vocabulary, extended; platform and app shipped
together.

## The keyring entries on Linux

**Nothing survives, so there is nothing to migrate and no script to
run.**

The desktop asked the keyring crate for `linux-native`, which selects
the kernel keyutils store. That store is "completely in-memory and will
not persist across reboots". What lay in it was the unlocked identity
seed, and since the app learned to sign in to joyint.com, the platform
claim and the session bearer too, so a Linux desktop user was asked for
their passphrase again after every reboot and signed out of joyint.com
with it, and nobody had written down why. The feature set is now
`linux-native-sync-persistent`, which keeps keyutils as the in memory
cache and puts the persistent Secret Service collection behind it.

Because the old store lost its contents at every reboot, the change has
no upgrade path to write: the first unlock after the upgrade stores the
seed in a place that is still there tomorrow, and that is the whole
migration.

One case changes for the worse and is named rather than hidden. A Linux
session with no Secret Service at all, or with a collection nobody
unlocked, cannot store the entry now, and the refusal is not confined to
the remembering. The app writes the seed AFTER it has already opened the
crypt and passes the keystore's error straight on, so an unlock with
"remember" ticked returns a keystore error for an unlock that succeeded:
the zones are open in this process, and the caller is told the unlock
failed (the `remember` branch of `joy_unlock`, `crypt_ops.rs`). Before,
the write would have succeeded into keyutils and the seed would have
been gone at the next reboot without a word. A refusal a person can see
is the better of the two, and the way out is the same as it always was:
unlock the login keyring, or do not ask the app to remember the seed.
Reporting the failed remembering beside the successful unlock, instead
of in place of it, is the open end of this.

The connector does have a fallback the app does not, because a headless
machine is where it has to work: it writes `forge-tokens.json`, mode
0600 in a 0700 directory, and every answer says `"source": "file"` when
it did.

## Removing a stored forge credential

Before this rebuild there was no way to do it on either surface: the
desktop could only forget the crypt seed and the CLI had nothing. There
are now exactly two paths, and both end in the connector's own `logout`:

- `joy forge logout --host <host>`, or `joy forge logout --all` for
  every host in the set `joy forge status` shows: the hosts of
  `forges.yaml`, the hosts of this project's remotes, and the hosts
  joy's own credential file holds. A credential that lives in the
  operating system's store alone cannot be enumerated, so such a host is
  reached by naming it with `--host`.
- The **Sign out** action on the host row in the Device section of the
  app's start page.

Both remove the stored credential and revoke the token at the forge
where the forge offers revocation. GitHub is revoked through
`DELETE /applications/{client_id}/token`, which ends that one token; the
grant is never deleted, because deleting a grant would end every token
of that application for that person. Where a forge offers no revocation,
both surfaces say so rather than implying the token is gone everywhere:

    Removed the credential for codeberg.org (keychain), and the forge offers no revocation.
    Signed out of codeberg.org; the forge offers no way to revoke it.

The first line is what the CLI prints, the second what the app's row
shows. The wording differs because each surface writes for its own
reader; what they report is the same pair of facts, removed and revoked.

**A credential another program owns is not joy's to remove.** When the
token came from `gh`, `glab` or `tea`, joy removes nothing and names the
command that does:

    the token for github.com comes from gh; run `gh auth logout --hostname github.com` to remove it

    The credential for github.com was stored by another program, so joy
    did not remove it. Remove it there with gh auth logout --hostname github.com.

Again the CLI first, the app second. This is the reason joy treats a
foreign CLI's store as read only: joy never writes it, never refreshes
it and never revokes from it.

## The hooks in your checkout

`joy init` and `joy update` set `core.hooksPath` to `.joy/hooks` as they
always did. What is new is that joy now CHAINS: it records the path it
replaced in `.joy/hooks/chained-path`, and every hook it installs ends
by running the same hook name from there, so husky, lefthook and
pre-commit keep working behind joy's own check. The chained hook's exit
code is the hook's exit code. Where there was no previous value the
chain goes to git's own `$GIT_COMMON_DIR/hooks`, which is what git would
have run.

joy says it once, at the moment it takes the path over:

    joy installed its hooks and kept yours: .husky still runs after joy's.

`.joy/hooks/` is gitignored, so the recorded path travels with the
checkout of the person who has it and never with the team.

**The one case that needs a hand.** A checkout where an OLDER joy
already took `core.hooksPath` has no record of what was there, because
no version before this one wrote one. `joy update` sees its own path
already in place, records nothing, and the previous hooks stay
unreachable. If that is your checkout and you know what the path was,
write it into `.joy/hooks/chained-path` as a single line; from then on
the chain works like everybody else's.

On Windows both hooks are bash and run only under the `sh` that Git for
Windows brings. A machine with no git has no hooks at all, and joy's own
commits are checked in process instead.
