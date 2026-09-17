# Release notes draft: the forge connection rebuild

Draft. Every behaviour change a person meets is in here, with what it
looks like from the outside. The upgrade itself is in
[the migration chapter](../migration/forge-connection-ng.md); this file
is about what is different afterwards.

## joy can sign you in to a forge

There was no way to do this from a terminal before. Every sentence that
said "sign in to the forge" pointed at `gh`, `glab` or `tea`, or at a
`joy forge setup` that never existed. Now there is one command group:

    joy forge login  [--host <host>] [--token-stdin] [--for read|write|create|release] [--login <name>]
    joy forge status [--host <host>]
    joy forge logout [--host <host> | --all]
    joy forge plugins

Without `--host` the host comes from this project's remote, the one joy
really contacts.

`login` prints the verification URL and the code on stderr and waits
while the forge is polled. **joy never opens a browser**; you open the
URL yourself. Where a forge has no device flow, or where nobody
registered an OAuth client for the instance, `--token-stdin` reads one
token from standard input instead, validates it against the instance's
own API and stores it. The token is never an argument, so no process
list can carry it, and there is no `--token <value>`.

`status` prints one row per host: the host, its forge, the login, the
state, where the credential came from, the granted scopes, the expiry,
and which binary answered. It exits 1 when no host in its set is signed
in, which makes it usable in a script.

`plugins` contacts no forge. It says which connector binary answers for
which forge, where it was found, which protocol it speaks, and it prints
the `rm` line for a stale binary sitting beside a fresh one.

The app has the same door: the start page now has a **Device** section
with one row per host, naming the login it holds, where the credential
comes from and the scopes it was granted, with **Sign in** and
**Sign out** on the row.

## joy needs no git binary

joy used to run `git` for `add`, `commit`, `tag -a`, `push`,
`status --porcelain`, `describe`, `ls-files`, `rm --cached`, `log` and
`gc --auto`. It does not any more: there is one engine, libgit2 through
the git2 crate, and no joy path starts a git process. The desktop
release path was the last exception and it is gone too, so recording and
publishing a release from the app works on a machine that has never had
git installed.

Three consequences that are easy to meet:

- **joy's own commits are not signed.** libgit2 cannot sign and does not
  read `commit.gpgsign`. A branch protected with "require signed
  commits" will refuse joy's commits, and every platform job branch with
  them. If you need that branch protected, joy's automatic commits have
  to go to a branch that is not.
- **No hook runs on a commit joy makes.** libgit2 runs no hooks. The
  item reference rule that `.joy/hooks/commit-msg` enforces for your own
  `git commit` is checked in process for joy's commits instead. A commit
  joy makes by itself warns and proceeds, because refusing would strand
  a write that already happened; `joy release record`, which you typed,
  refuses.
- **A path with a clean or smudge filter is refused by name.** libgit2
  runs no filter program, so committing a `filter=lfs` path through joy
  would write the file's bytes where the pointer belongs. joy refuses
  such a path and says which one instead of writing it wrong.

## Your own hooks keep running

`joy init` and `joy update` still point `core.hooksPath` at
`.joy/hooks`, because git's `core.hooksPath` REPLACES the hook location
entirely and not setting it would leave joy's check out of your active
hook path. What is new is that joy now records the path it replaced and
every hook it installs ends by running the same hook from there. husky,
lefthook and pre-commit keep working behind joy's own check, and the
chained hook's exit code is the hook's exit code.

joy says it once, when it takes the path over:

    joy installed its hooks and kept yours: .husky still runs after joy's.

Where there was no previous value the chain goes to git's own hooks
directory, which is what git would have run.

## The first ssh contact with a host asks you

Before, an ssh contact with a host this machine had never seen failed
with libgit2's own words and no way forward. Now joy does the whole
known_hosts check itself and asks:

    The authenticity of git.acme.test:22 cannot be established.
    ssh-ed25519 key fingerprint is SHA256:abcdef...
    This host already has lines for ssh-rsa, and none for ssh-ed25519.
    joy would add one line to /home/s/.ssh/known_hosts.
    Trust this host key? (y/N)

The default is no, and a closed standard input is no: joy never trusts a
key because nobody was there to refuse it. On yes, one line is added to
your own `known_hosts` in the form your ssh config asks for, hashed
where `HashKnownHosts` is on.

Three cases do not ask:

- `StrictHostKeyChecking yes` in your ssh config refuses before anything
  may accept, and joy neither asks nor adds.
- `StrictHostKeyChecking accept-new` adds without asking. joy still does
  the check; it only skips the question.
- A background worker, and any process running under a delegation
  session, never asks and never writes. It refuses with the fingerprint
  and the exact line to add, so the person who owns the machine can do
  it once.

This release pins no host keys of its own. The pin file ships empty, so
github.com, gitlab.com and codeberg.org are first contacts like every
other host.

Two files that git reads and libgit2 never did are now read: joy reads
every `UserKnownHostsFile` and `GlobalKnownHostsFile` your ssh config
names, hashed entries included, and it validates `~/.ssh/known_hosts`
with libssh2's own rules before the first ssh contact, naming the line
number. One unparsable line used to kill the whole file and produce
"error reading known_hosts" about a file with four hundred lines.

## A repository nobody is signed in for is polled every 15 minutes

An https remote with no credential is now checked once every fifteen
minutes per host, whatever the budget would allow, and the surface says
why rather than leaving you to wonder:

    Nobody is signed in for github.com, so it is checked once every 15
    minutes. Sign in to sync at full speed.

Everywhere else the poll period is computed from the host's request
budget and the number of open projects on that host, so chat in a signed
in project is as quick as it was, and ten open projects on one forge do
not multiply the requests that forge sees.

## A throttling forge names its wait

A forge that answers 429 used to produce "retrying later". Now the
sentence carries a number:

    Codeberg is rate limiting us, retrying in 10 minutes.

On GitHub the number comes from the forge itself where the connector can
ask `GET /rate_limit`, which GitHub documents as free of the primary
limit. On Codeberg, Forgejo and Gitea no probe is made at all: their
limiter sits per IP in front of the whole site, so a probe would ride
the same bucket that just refused the request, and the wait comes from
joy's own strike window. On GitLab a 403 on git with a working token is
the failed authentication ban, which sends no header and cannot be
cleared by signing in, so the wait is GitLab's own documented one:
fifteen minutes on gitlab.com, an hour on a self managed default.

A strike is not cleared by the next success. A forge that limited us a
second ago has not changed its mind because one request got through.

## Failures say what they are

Where joy reported four states it now reports fourteen:
`needs_sign_in`, `needs_org_approval`, `needs_sso`, `no_push_rights`,
`needs_host_trust`, `scope_missing`, `plugin_missing`,
`plugin_outdated`, `tls_untrusted`, `proxy_auth`, `rate_limited`,
`offline`, `denied` and `error`. Each carries one plain sentence and at
most one action, and the raw libgit2 text moves to the detail line.

None of this is decided by finding "403" in a string any more. The state
comes from the error's class and code and from the HTTP status, so a
branch whose name contains 403 is no longer read as a refusal, and an
operating system message in a language other than English is no longer
read as a network outage.

## The app clones one revision, not the whole history

Adding a project in the app clones the default branch at `depth = 1`
with the full working tree on disk. libgit2 offers no sparse checkout
and no partial clone, so depth is the only footprint reducer there is:
history is what is saved, the tip is not.

Before cloning, the app asks the forge how large the repository is. It
warns above 250 MB and pre-selects the platform project above 1 GB, and
you may continue in both cases. The warning reads "up to about N MB",
because the forge's figure covers the full history while a depth 1 clone
downloads roughly one snapshot. A forge that reports no size is not an
error and the clone goes ahead without a warning.

What a shallow clone costs is visible in the app rather than hidden: the
header shows "changes to sync" or "in sync" instead of the ahead and
behind counters, because a merge base below the cutoff is invisible to
the graph walk and both numbers would be approximate. A clone from a
path on your own machine is never shallow: libgit2's local transport
refuses any depth, and a depth would save nothing there anyway.

## One connector binary

`joy-github`, `joy-gitlab` and `joy-gitea` are replaced by one
`joy-forge` that carries every forge. `cargo install joy-cli` ships it
beside `joy`, and the installers put both in one archive so `joy update`
keeps them in lockstep.

The old names keep working for one deprecation window and nothing is
deleted from your machine: `joy-forge` simply wins inside every search
directory, `joy forge plugins` names the stale binary, and you remove it
with the printed `rm` line. An old connector asked a new question
answers `plugin_outdated` and names the file and the fix.

## Linux: what the app remembers now survives a reboot

The app stored its secrets in the kernel keyutils store, which is in
memory and, in its own documentation, "will not persist across reboots".
A Linux desktop user was therefore asked for the passphrase again after
every reboot. The app now uses the persistent Secret Service collection
with keyutils in front of it as a cache, so the remembered seed, the
platform sign in and the session are there tomorrow.

There is nothing to migrate: nothing survived to migrate. One case is
worse than before and is named rather than hidden: a Linux session with
no Secret Service, or with a collection nobody unlocked, cannot store
the entry at all and the app says so, where it used to succeed and lose
the secret at the next reboot without a word.

## You can remove a stored forge credential

There was no way to do this before on either surface.
`joy forge logout --host <host>` and the **Sign out** action on the
app's Device row both remove the stored credential and revoke the token
at the forge where the forge offers revocation. GitHub's revocation ends
that one token and never the grant, which would have ended every token
of the application for you.

A credential that came from `gh`, `glab` or `tea` is not joy's to
remove: joy removes nothing and prints the command that does.
