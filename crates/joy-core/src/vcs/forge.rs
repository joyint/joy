// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The one headless git engine for forge sync (JOY-0265-D7), over git2
//! (JP-0034-A2: one git engine everywhere; gix cannot push). Grown from
//! the platform's runtime-checkout layer and shared by the platform, the
//! desktop app, and everything else that syncs a checkout with a forge.
//!
//! Two mechanics live under the one vcs roof, each with its reason: the
//! CLI verbs in [`super`] run the git BINARY because user hooks and user
//! config must fire; everything here is HEADLESS work (server worker,
//! app worker) that must never fire hooks and authenticates with tokens
//! or the machine's stored credentials.
//!
//! FETCH_HEAD is never written or read here. It is the one file git
//! updates without a lock, and sharing it tore syncs apart twice: a
//! chats fetch of a ref the forge does not have left it EMPTY (read as
//! "corrupted loose reference", JP-00DB-61), and an app worker racing
//! the user's own `git pull` produced "Cannot rebase onto multiple
//! branches" (JAPP-0198-EA). Fetches update the remote-tracking ref
//! only, and fast-forwards read THAT.

use std::path::{Path, PathBuf};

/// The user name a forge expects beside an access token in HTTP basic auth.
/// Every forge takes the token as the password, but each one wants its own
/// name in front of it, and sending the wrong one is refused as if we could
/// not authenticate at all (JP-00D8-94: Codeberg answered
/// "server requires authentication that we do not support" for a token it
/// would have accepted). Host-based, because that is all the credentials
/// callback is handed.
fn basic_auth_user(url: &str) -> &'static str {
    let lower = url.to_ascii_lowercase();
    if lower.contains("github.com") {
        // GitHub ignores the name but documents this one.
        "x-access-token"
    } else if lower.contains("gitlab") {
        // GitLab: an OAuth token rides under this fixed name.
        "oauth2"
    } else {
        // Gitea and Codeberg: the token IS the name, with no password.
        ""
    }
}

/// How this checkout talks to its forge.
///
/// The platform authenticates with the account's OAuth token; the desktop
/// app with whatever the person's machine holds (ssh-agent, credential
/// helper). One enum instead of two engines, so every caller gets the
/// same retry discipline and the same honest errors.
pub enum Auth {
    /// A forge access token (platform): tried in the shape this forge
    /// expects, then the other common shape, then the machine's helper.
    Token(String),
    /// The machine's own credentials (desktop): ssh-agent for ssh
    /// remotes, the git credential helper for https ones.
    Local,
}

impl Auth {
    pub fn token(token: impl Into<String>) -> Self {
        Auth::Token(token.into())
    }

    fn callbacks(&self, config: Option<git2::Config>) -> git2::RemoteCallbacks<'static> {
        let mut callbacks = git2::RemoteCallbacks::new();
        match self {
            Auth::Token(token) => {
                let token = token.clone();
                // Attempt 1: the account token, in the shape this forge
                // expects. Attempt 2: the other common shape, so a forge we
                // have not met (a self-hosted GitLab behind a custom host
                // name, say) still gets a fair try instead of a
                // wrong-looking refusal. Attempt 3: the machine's git
                // credential helper (gh, osxkeychain) — the dev fake-login
                // has no real token, and local devs DO have helper
                // credentials. Then STOP: retrying the same credentials
                // forever is exactly what libgit2 reports as "too many
                // redirects or authentication replays".
                let attempts = std::cell::Cell::new(0u32);
                callbacks.credentials(move |url, username, _allowed| {
                    attempts.set(attempts.get() + 1);
                    match attempts.get() {
                        1 if !token.is_empty() => match basic_auth_user(url) {
                            "" => git2::Cred::userpass_plaintext(&token, ""),
                            user => git2::Cred::userpass_plaintext(user, &token),
                        },
                        2 if !token.is_empty() => match basic_auth_user(url) {
                            "" => git2::Cred::userpass_plaintext("x-access-token", &token),
                            _ => git2::Cred::userpass_plaintext(&token, ""),
                        },
                        n if n <= 3 => match &config {
                            Some(config) => git2::Cred::credential_helper(config, url, username),
                            None => Err(git2::Error::from_str("no credential config")),
                        },
                        _ => Err(git2::Error::from_str(
                            "authentication failed (token and credential helper both rejected)",
                        )),
                    }
                });
            }
            Auth::Local => {
                let attempts = std::cell::Cell::new(0u32);
                callbacks.credentials(move |url, username, allowed| {
                    attempts.set(attempts.get() + 1);
                    if attempts.get() > 3 {
                        return Err(git2::Error::from_str(
                            "authentication failed (agent and credential helper both rejected)",
                        ));
                    }
                    if allowed.contains(git2::CredentialType::SSH_KEY) {
                        return git2::Cred::ssh_key_from_agent(username.unwrap_or("git"));
                    }
                    if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                        if let Some(config) = &config {
                            return git2::Cred::credential_helper(config, url, username);
                        }
                    }
                    git2::Cred::default()
                });
            }
        }
        callbacks
    }
}

/// `origin`, or the first configured remote — a checkout the product
/// made always has `origin`, but a repo a person wired by hand may not
/// (the desktop opens those too).
fn origin_or_first<'r>(repo: &'r git2::Repository) -> anyhow::Result<git2::Remote<'r>> {
    match repo.find_remote("origin") {
        Ok(remote) => Ok(remote),
        Err(_) => {
            let remotes = repo.remotes().map_err(err)?;
            let name = remotes
                .get(0)
                .map_err(|_| anyhow::anyhow!("no remote configured"))?
                .ok_or_else(|| anyhow::anyhow!("remote name is not utf-8"))?
                .to_string();
            repo.find_remote(&name).map_err(err)
        }
    }
}

/// Open the repository that holds `dir` — `discover`, not `open`: the
/// desktop opens project roots that may sit inside a larger repo, and
/// for an exact root (every platform checkout) discover is the same
/// thing.
fn open(dir: &Path) -> Result<git2::Repository, git2::Error> {
    git2::Repository::discover(dir)
}

/// The credential-helper config for a repo: repo-level settings
/// (insteadOf, per-repo helpers) included; the global config as the
/// fallback when there is no repo yet (clone).
fn cred_config(repo: Option<&git2::Repository>) -> Option<git2::Config> {
    match repo {
        Some(r) => r.config().and_then(|mut c| c.snapshot()).ok(),
        None => git2::Config::open_default().ok(),
    }
}

fn err(e: git2::Error) -> anyhow::Error {
    anyhow::anyhow!("git: {}", e.message())
}

/// Clone a forge URL into `dest` using the account token.
pub fn clone(url: &str, auth: &Auth, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest.parent().expect("checkout dir has a parent"))?;
    let mut fetch = git2::FetchOptions::new();
    fetch.remote_callbacks(auth.callbacks(cred_config(None)));
    git2::build::RepoBuilder::new()
        .fetch_options(fetch)
        .clone(url, dest)
        .map_err(err)?;
    // the clone machinery runs libgit2's update_tips once; nothing here
    // ever reads FETCH_HEAD, so the checkout starts without one
    std::fs::remove_file(dest.join(".git/FETCH_HEAD")).ok();
    Ok(())
}

/// Fetch the current branch and fast-forward to the remote state. Diverged
/// histories are an error (manual merge; the YAML merge driver stays the
/// known follow-up).
pub fn pull_ff(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    let span = tracing::info_span!("git.pull_ff", repo = %repo_dir.display());
    let _s = span.enter();
    let result = pull_ff_inner(repo_dir, auth);
    if let Err(e) = &result {
        // divergence is the NORMAL case under write-behind sync (ADR
        // JAPP-00D8) — callers branch on it; only real failures are errors
        if e.to_string().contains("diverged") {
            tracing::debug!(repo = %repo_dir.display(), "git pull: histories diverged");
        } else {
            tracing::error!(repo = %repo_dir.display(), error = %e, "git pull failed");
        }
    }
    result
}

fn pull_ff_inner(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    fetch_branch(repo_dir, auth)?;
    ff_from_tracking(repo_dir)
}

/// The remote-tracking ref for `branch`: the branch's configured
/// upstream when it has one, else `refs/remotes/<remote>/<branch>` for
/// the resolved remote ([`origin_or_first`]) — never a hardwired
/// "origin", because a hand-wired repo may call its remote differently.
fn tracking_ref_name(repo: &git2::Repository, branch: &str) -> anyhow::Result<String> {
    if let Ok(b) = repo.find_branch(branch, git2::BranchType::Local) {
        if let Ok(upstream) = b.upstream() {
            if let Ok(name) = upstream.get().name() {
                return Ok(name.to_string());
            }
        }
    }
    let remote = origin_or_first(repo)?;
    let remote_name = remote.name().ok().flatten().unwrap_or("origin").to_string();
    Ok(format!("refs/remotes/{remote_name}/{branch}"))
}

/// The one low-level fetch: download the objects for `src` and point
/// `dst` at the advertised tip OURSELVES. libgit2's own update_tips is
/// never run, because it truncates FETCH_HEAD unconditionally (the
/// UPDATE_FETCHHEAD flag only guards the writing of entries, see
/// libgit2 remote.c truncate_fetch_head) — and that bare truncation is
/// the whole torn-FETCH_HEAD class (JP-00DB-61, JAPP-0198-EA).
/// `Ok(None)` when the forge does not advertise `src`.
fn download_ref(
    repo: &git2::Repository,
    auth: &Auth,
    src: &str,
    dst: &str,
) -> anyhow::Result<Option<git2::Oid>> {
    let mut remote = origin_or_first(repo)?;
    let advertised = {
        let connection = remote
            .connect_auth(
                git2::Direction::Fetch,
                Some(auth.callbacks(cred_config(Some(repo)))),
                None,
            )
            .map_err(|e| anyhow::anyhow!("fetch failed (offline?): {}", e.message()))?;
        // an empty advertisement (freshly created forge) is a plain
        // empty list since git2 0.21 — and an honest "nothing there"
        connection
            .list()
            .map_err(err)?
            .iter()
            .find(|r| r.name() == src)
            .map(|r| r.oid())
    };
    let Some(tip) = advertised else {
        return Ok(None);
    };
    let mut opts = git2::FetchOptions::new();
    opts.remote_callbacks(auth.callbacks(cred_config(Some(repo))));
    let refspec = format!("+{src}:{dst}");
    remote
        .download(&[refspec.as_str()], Some(&mut opts))
        .map_err(|e| anyhow::anyhow!("fetch failed (offline?): {}", e.message()))?;
    let _ = remote.disconnect();
    repo.reference(dst, tip, true, "joy-vcs: fetch")
        .map_err(err)?;
    Ok(Some(tip))
}

/// The NETWORK half of a pull: fetch the working branch into its
/// remote-tracking ref. FETCH_HEAD is not touched (see [`download_ref`]).
/// Touches no working tree and no local branch. Every caller of git work
/// on a shared checkout holds that checkout's gate (JP-00DB-61: one git
/// process per checkout).
///
/// A branch that is gone from the forge (renamed or deleted) is said out
/// loud instead of surfacing as a phantom state.
pub fn fetch_branch(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    let span = tracing::info_span!("git.fetch", repo = %repo_dir.display());
    let _s = span.enter();
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?;
    let branch = head
        .shorthand()
        .map_err(|_| anyhow::anyhow!("detached HEAD"))?
        .to_string();
    let src = format!("refs/heads/{branch}");
    let dst = tracking_ref_name(&repo, &branch)?;
    match download_ref(&repo, auth, &src, &dst)? {
        Some(_) => Ok(()),
        None => anyhow::bail!("branch {branch} not found on the forge (renamed or deleted?)"),
    }
}

/// The LOCAL half of a pull: fast-forward the working branch onto its
/// remote-tracking ref, which [`fetch_branch`] just updated. Fast, no
/// network; the caller holds the project gate.
pub fn ff_from_tracking(repo_dir: &Path) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?;
    let branch = head
        .shorthand()
        .map_err(|_| anyhow::anyhow!("detached HEAD"))?
        .to_string();
    let tracking = repo
        .find_reference(&tracking_ref_name(&repo, &branch)?)
        .map_err(err)?;
    let remote_commit = repo.reference_to_annotated_commit(&tracking).map_err(err)?;
    let (analysis, _) = repo.merge_analysis(&[&remote_commit]).map_err(err)?;
    if analysis.is_fast_forward() {
        let refname = format!("refs/heads/{branch}");
        let mut reference = repo.find_reference(&refname).map_err(err)?;
        reference
            .set_target(remote_commit.id(), "joy-vcs: fast-forward")
            .map_err(err)?;
        repo.set_head(&refname).map_err(err)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(err)?;
    } else if !analysis.is_up_to_date() {
        anyhow::bail!("local and remote histories diverged; resolve on the forge side");
    }
    Ok(())
}

/// Stage `.joy/` changes and commit them as the acting account; returns the
/// commit id, or None when the tree is clean.
pub fn commit_joy(
    repo_dir: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<Option<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let mut status_opts = git2::StatusOptions::new();
    status_opts
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(".joy");
    let dirty = !repo
        .statuses(Some(&mut status_opts))
        .map_err(err)?
        .is_empty();
    if !dirty {
        return Ok(None);
    }
    let mut index = repo.index().map_err(err)?;
    index
        .add_all([".joy"], git2::IndexAddOption::DEFAULT, None)
        .map_err(err)?;
    index.write().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let signature = git2::Signature::now(author_name, author_email).map_err(err)?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(err)?;
    Ok(Some(oid.to_string()))
}

/// Push the current branch back to the forge.
/// Ask the forge whether it would let us WRITE, without writing anything
/// (operator 2026-07-27).
///
/// A public repository can be read by anyone and written by nobody without
/// credentials, so a project can look perfectly healthy until the first
/// push — which may be hours later, in the middle of something. The push
/// side of the protocol answers this question during its handshake: the
/// connection authenticates and the server advertises its refs, and no
/// object and no ref is sent. A refusal here is exactly the refusal a real
/// push would meet.
pub fn probe_write_access(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    let span = tracing::info_span!("git.probe_write", repo = %repo_dir.display());
    let _s = span.enter();
    let repo = open(repo_dir).map_err(err)?;
    let mut remote = origin_or_first(&repo)?;
    remote
        .connect_auth(
            git2::Direction::Push,
            Some(auth.callbacks(cred_config(Some(&repo)))),
            None,
        )
        .map_err(|e| anyhow::anyhow!("push failed: {}", e.message()))?;
    let _ = remote.disconnect();
    Ok(())
}

pub fn push(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    let span = tracing::info_span!("git.push", repo = %repo_dir.display());
    let _s = span.enter();
    let result = (|| -> anyhow::Result<()> {
        let repo = open(repo_dir).map_err(err)?;
        let head = repo.head().map_err(err)?;
        let branch = head
            .shorthand()
            .map_err(|_| anyhow::anyhow!("detached HEAD"))?
            .to_string();
        let mut remote = origin_or_first(&repo)?;
        let mut opts = git2::PushOptions::new();
        opts.remote_callbacks(auth.callbacks(cred_config(Some(&repo))));
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        remote
            .push(&[refspec.as_str()], Some(&mut opts))
            .map_err(|e| anyhow::anyhow!("push failed: {}", e.message()))?;
        Ok(())
    })();
    if let Err(e) = &result {
        // some callers defer a failed push to the write-behind worker; the
        // event still carries the cause with the repo context
        tracing::error!(repo = %repo_dir.display(), error = %e, "git push failed");
    }
    result
}

// ---- generic ref plumbing (chats and other side refs) ------------------
//
// The chat SEMANTICS (adopt / fast-forward / message-union merge) live in
// joy-chat-store; these are the raw verbs it composes. They never write
// FETCH_HEAD either.

/// Fetch one ref into a local destination ref. `Ok(false)` when the
/// forge does not have the source ref (first-ever sync, or the ref was
/// removed) — the stale destination is deleted then, so reconciles run
/// against nothing rather than a stale state.
pub fn fetch_ref(repo_dir: &Path, auth: &Auth, src: &str, dst: &str) -> anyhow::Result<bool> {
    let repo = open(repo_dir).map_err(err)?;
    match download_ref(&repo, auth, src, dst)? {
        Some(_) => Ok(true),
        None => {
            if let Ok(mut stale) = repo.find_reference(dst) {
                stale.delete().ok();
            }
            Ok(false)
        }
    }
}

/// Push one local ref to the same name on the forge.
pub fn push_ref(repo_dir: &Path, auth: &Auth, refname: &str) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    let mut remote = origin_or_first(&repo)?;
    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(auth.callbacks(cred_config(Some(&repo))));
    let refspec = format!("{refname}:{refname}");
    remote
        .push(&[refspec.as_str()], Some(&mut opts))
        .map_err(|e| anyhow::anyhow!("push of {refname} failed: {}", e.message()))?;
    Ok(())
}

/// The oid the forge holds for `refname`, without fetching anything
/// (JP-008B-24: polls compare hashes and fetch only on a change). `None`
/// when the forge does not have the ref. Callers hold a registered
/// project, so the remote always advertises at least its working branch
/// (a fully ref-less remote trips a git2 empty-list edge).
pub fn ls_remote_ref(
    repo_dir: &Path,
    auth: &Auth,
    refname: &str,
) -> anyhow::Result<Option<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let mut remote = origin_or_first(&repo)?;
    let connection = remote
        .connect_auth(
            git2::Direction::Fetch,
            Some(auth.callbacks(cred_config(Some(&repo)))),
            None,
        )
        .map_err(|e| anyhow::anyhow!("ls-remote failed (offline?): {}", e.message()))?;
    let head = connection
        .list()
        .map_err(err)?
        .iter()
        .find(|r| r.name() == refname)
        .map(|r| r.oid().to_string());
    Ok(head)
}

/// Pull with a REAL merge (ADR JAPP-00D8): fetch, fast-forward when
/// possible, otherwise three-way-merge the histories. Conflicting
/// `.joy/*.yaml` files merge through joy-core's YAML engine (the same
/// logic as the git merge driver); joycrypt blobs and everything else
/// take the forge side — under write-behind only `.joy` is written
/// locally. Divergence is the NORMAL case under write-behind, not an
/// error. The merge commit carries the acting member (JP-00DE-11): the
/// write that made the checkout dirty is whose work this merge finishes.
pub fn pull_merge(
    repo_dir: &Path,
    auth: &Auth,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<()> {
    // the shared fetch half: honest about a vanished branch, and it
    // never touches FETCH_HEAD (JP-00DB-61)
    fetch_branch(repo_dir, auth)?;
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?;
    let branch = head
        .shorthand()
        .map_err(|_| anyhow::anyhow!("detached HEAD"))?
        .to_string();
    let tracking = repo
        .find_reference(&tracking_ref_name(&repo, &branch)?)
        .map_err(err)?;
    let remote_commit = repo.reference_to_annotated_commit(&tracking).map_err(err)?;
    let (analysis, _) = repo.merge_analysis(&[&remote_commit]).map_err(err)?;
    if analysis.is_up_to_date() {
        return Ok(());
    }
    if analysis.is_fast_forward() {
        let refname = format!("refs/heads/{branch}");
        let mut reference = repo.find_reference(&refname).map_err(err)?;
        reference
            .set_target(remote_commit.id(), "joy-vcs: fast-forward")
            .map_err(err)?;
        repo.set_head(&refname).map_err(err)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(err)?;
        return Ok(());
    }
    // three-way merge
    let local = repo
        .find_commit(
            head.target()
                .ok_or_else(|| anyhow::anyhow!("unborn HEAD"))?,
        )
        .map_err(err)?;
    let theirs = repo.find_commit(remote_commit.id()).map_err(err)?;
    let mut index = repo.merge_commits(&local, &theirs, None).map_err(err)?;
    resolve_conflicts_yaml_aware(&repo, &mut index)?;
    let tree_id = index.write_tree_to(&repo).map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let sig = git2::Signature::now(author_name, author_email).map_err(err)?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "chore: merge forge changes [no-item]",
        &tree,
        &[&local, &theirs],
    )
    .map_err(err)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
        .map_err(err)?;
    repo.cleanup_state().ok();
    Ok(())
}

/// Resolve every conflict of a merged `index` in place: conflicting
/// `.joy/*.yaml` files merge through joy-core's YAML engine (the same
/// logic as the git merge driver); joycrypt blobs and everything else
/// take the `theirs` side. Shared by [`pull_merge`] (theirs = the forge)
/// and [`land_branch_yaml`] (theirs = the joywork branch — which only
/// carries `.joy` changes, so the theirs-wins arm stays theoretical there).
fn resolve_conflicts_yaml_aware(
    repo: &git2::Repository,
    index: &mut git2::Index,
) -> anyhow::Result<()> {
    if index.has_conflicts() {
        let conflicts: Vec<_> = index
            .conflicts()
            .map_err(err)?
            .filter_map(|c| c.ok())
            .collect();
        for conflict in conflicts {
            let path_bytes = conflict
                .our
                .as_ref()
                .or(conflict.their.as_ref())
                .or(conflict.ancestor.as_ref())
                .map(|e| e.path.clone())
                .unwrap_or_default();
            let path = String::from_utf8_lossy(&path_bytes).to_string();
            let read = |entry: &Option<git2::IndexEntry>| -> Vec<u8> {
                entry
                    .as_ref()
                    .and_then(|e| repo.find_blob(e.id).ok())
                    .map(|b| b.content().to_vec())
                    .unwrap_or_default()
            };
            let ours_bytes = read(&conflict.our);
            let theirs_bytes = read(&conflict.their);
            let base_bytes = read(&conflict.ancestor);
            let merged: Vec<u8> = if path.starts_with(".joy/")
                && path.ends_with(".yaml")
                && !crate::merge::is_joycrypt_blob(&ours_bytes)
                && !crate::merge::is_joycrypt_blob(&theirs_bytes)
            {
                let doc = crate::merge::merge_yaml_doc(
                    &String::from_utf8_lossy(&base_bytes),
                    &String::from_utf8_lossy(&ours_bytes),
                    &String::from_utf8_lossy(&theirs_bytes),
                )
                .map_err(|e| anyhow::anyhow!("joy-yaml merge of {path}: {e}"))?;
                doc.into_bytes()
            } else if !theirs_bytes.is_empty() {
                // non-joy or encrypted content: the forge side wins
                theirs_bytes
            } else {
                ours_bytes
            };
            let blob = repo.blob(&merged).map_err(err)?;
            let mut entry = conflict
                .our
                .or(conflict.their)
                .or(conflict.ancestor)
                .ok_or_else(|| anyhow::anyhow!("empty conflict entry"))?;
            entry.id = blob;
            entry.flags &= !0x3000; // clear the stage bits: stage 0 (merged)
            index.add(&entry).map_err(err)?;
            index.remove_path(std::path::Path::new(&path)).ok();
            index.add(&entry).map_err(err)?;
        }
    }
    Ok(())
}

// ---- repo lifecycle (seeding, harnesses, volume facts) ------------------

/// Initialize a bare repository on `branch` (a local stand-in forge for
/// harnesses).
pub fn init_bare(dir: &Path, branch: &str) -> anyhow::Result<()> {
    let repo = git2::Repository::init_bare(dir).map_err(err)?;
    repo.set_head(&format!("refs/heads/{branch}"))
        .map_err(err)?;
    Ok(())
}

/// Initialize a plain repository on `branch`.
pub fn init_repo(dir: &Path, branch: &str) -> anyhow::Result<()> {
    let repo = git2::Repository::init(dir).map_err(err)?;
    repo.set_head(&format!("refs/heads/{branch}"))
        .map_err(err)?;
    Ok(())
}

/// Configure a named remote.
pub fn add_remote(dir: &Path, name: &str, url: &str) -> anyhow::Result<()> {
    let repo = open(dir).map_err(err)?;
    repo.remote(name, url).map_err(err)?;
    Ok(())
}

/// The file's content at `refname` (harness verification).
pub fn blob_at(dir: &Path, refname: &str, path: &str) -> Option<String> {
    let repo = open(dir).ok()?;
    let commit = repo.find_reference(refname).ok()?.peel_to_commit().ok()?;
    let entry = commit.tree().ok()?.get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).to_string())
}

/// Every path `refname` changed relative to its merge-base with
/// `base_refname` (harness verification).
pub fn changed_paths_between(
    dir: &Path,
    base_refname: &str,
    refname: &str,
) -> anyhow::Result<Vec<String>> {
    let repo = open(dir).map_err(err)?;
    let base_tip = repo
        .find_reference(base_refname)
        .map_err(err)?
        .peel_to_commit()
        .map_err(err)?
        .id();
    let tip = repo
        .find_reference(refname)
        .map_err(err)?
        .peel_to_commit()
        .map_err(err)?
        .id();
    let base = repo.merge_base(base_tip, tip).map_err(err)?;
    let base_tree = repo.find_commit(base).map_err(err)?.tree().map_err(err)?;
    let tip_tree = repo.find_commit(tip).map_err(err)?.tree().map_err(err)?;
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&tip_tree), None)
        .map_err(err)?;
    let mut paths = Vec::new();
    for delta in diff.deltas() {
        for f in [delta.new_file().path(), delta.old_file().path()]
            .into_iter()
            .flatten()
        {
            let p = f.display().to_string();
            if !paths.contains(&p) {
                paths.push(p);
            }
        }
    }
    Ok(paths)
}

/// Stage EVERYTHING and commit it (seeding and harness use; product
/// writes go through [`commit_joy`] / [`commit_all`], which respect the
/// `.joy` boundary).
pub fn commit_everything(
    repo_dir: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(err)?;
    let mut index = repo.index().map_err(err)?;
    index
        .add_all(["."], git2::IndexAddOption::DEFAULT, None)
        .map_err(err)?;
    index.write().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let sig = git2::Signature::now(author_name, author_email).map_err(err)?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|o| repo.find_commit(o).ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .map_err(err)?;
    Ok(oid.to_string())
}

/// Whether ANY path (untracked included) differs from HEAD — the volume
/// GC's conservative dirt check. Unreadable answers dirty: never delete
/// on doubt.
pub fn worktree_dirty(repo_dir: &Path) -> bool {
    let Ok(repo) = open(repo_dir) else {
        return true;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    repo.statuses(Some(&mut opts))
        .map(|s| !s.is_empty())
        .unwrap_or(true)
}

// ---- checkout observation (status surfaces) ----------------------------

/// Changed paths under `.joy/` (worktree or index), untracked included.
pub fn joy_dirty_paths(repo_dir: &Path) -> anyhow::Result<Vec<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(".joy");
    let statuses = repo.statuses(Some(&mut opts)).map_err(err)?;
    Ok(statuses
        .iter()
        .filter_map(|entry| entry.path().ok().map(str::to_string))
        .collect())
}

/// The dirty `.joy/` paths WITH their status flags — the change
/// fingerprint the app's debounce uses (a status change without a path
/// change must still arm it).
pub fn joy_dirty_fingerprint(repo_dir: &Path) -> Vec<String> {
    let Ok(repo) = open(repo_dir) else {
        return Vec::new();
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(".joy");
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return Vec::new();
    };
    statuses
        .iter()
        .filter_map(|e| e.path().ok().map(|p| format!("{}:{:?}", p, e.status())))
        .collect()
}

/// The repo's configured identity (user.name, user.email) — what the CLI
/// would commit as; it must map to a Joy member (Git-Integration
/// concept). An honest error when it is not configured.
pub fn repo_identity(repo_dir: &Path) -> anyhow::Result<(String, String)> {
    let repo = open(repo_dir).map_err(err)?;
    let sig = repo.signature().map_err(|e| {
        anyhow::anyhow!(
            "git identity missing (user.name/user.email): {}",
            e.message()
        )
    })?;
    Ok((
        sig.name().unwrap_or_default().to_string(),
        sig.email().unwrap_or_default().to_string(),
    ))
}

/// Local branch names, plus remote branches without a local counterpart
/// (shown checkout-able in a dropdown).
pub fn branch_names(repo_dir: &Path) -> Vec<String> {
    let Ok(repo) = open(repo_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        for (branch, _) in branches.flatten() {
            if let Ok(Some(name)) = branch.name() {
                names.push(name.to_string());
            }
        }
    }
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Remote)) {
        for (branch, _) in branches.flatten() {
            if let Ok(Some(full)) = branch.name() {
                let short = full.split_once('/').map(|(_, b)| b).unwrap_or(full);
                if short != "HEAD" && !names.iter().any(|n| n == short) {
                    names.push(short.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

/// The first remote's URL (forge detection lives with the caller).
pub fn remote_url(repo_dir: &Path) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    let remotes = repo.remotes().ok()?;
    let name = remotes.get(0).ok()??;
    let remote = repo.find_remote(name).ok()?;
    remote.url().ok().map(|u| u.to_string())
}

/// The oid a ref points at, as hex; `None` when absent or unborn.
pub fn ref_oid(repo_dir: &Path, refname: &str) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    repo.refname_to_id(refname).ok().map(|o| o.to_string())
}

/// HEAD's commit oid; `None` on an unborn branch.
pub fn head_oid(repo_dir: &Path) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    let oid = repo.head().ok()?.target().map(|o| o.to_string());
    oid
}

/// The author e-mail of the checkout's HEAD commit. The fallback merge
/// author for leftover dirt whose writer is no longer known (commits
/// found AHEAD after a restart): the merge finishes THAT member's
/// delivery, so it rides under the same name (JP-00DE-11).
pub fn head_author(repo_dir: &Path) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    let head = repo.head().ok()?.peel_to_commit().ok()?;
    let email = head.author().email().ok().map(|e| e.to_string());
    email
}

/// Local commits not on the remote-tracking branch and vice versa, as of
/// the last fetch (the sync button's honest counters).
pub fn ahead_behind(repo_dir: &Path) -> anyhow::Result<(u32, u32)> {
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?;
    let branch = head
        .shorthand()
        .map_err(|_| anyhow::anyhow!("detached HEAD"))?
        .to_string();
    let local = head
        .target()
        .ok_or_else(|| anyhow::anyhow!("unborn HEAD"))?;
    let upstream = match tracking_ref_name(&repo, &branch)
        .ok()
        .and_then(|name| repo.refname_to_id(&name).ok())
    {
        Some(oid) => oid,
        None => return Ok((0, 0)),
    };
    let (ahead, behind) = repo.graph_ahead_behind(local, upstream).map_err(err)?;
    Ok((ahead as u32, behind as u32))
}

/// Create a linked git worktree of the project checkout on a fresh branch
/// (the Job Container works here; it shares the checkout's object database).
/// Returns the worktree directory.
pub fn create_worktree(
    repo_dir: &Path,
    worktree_name: &str,
    branch: &str,
    worktree_path: &Path,
) -> anyhow::Result<PathBuf> {
    let repo = open(repo_dir).map_err(err)?;
    // Branch off the current HEAD commit.
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    if repo.find_branch(branch, git2::BranchType::Local).is_err() {
        repo.branch(branch, &head, false).map_err(err)?;
    }
    let reference = repo
        .find_reference(&format!("refs/heads/{branch}"))
        .map_err(err)?;
    let mut opts = git2::WorktreeAddOptions::new();
    opts.reference(Some(&reference));
    repo.worktree(worktree_name, worktree_path, Some(&opts))
        .map_err(|e| anyhow::anyhow!("worktree add failed: {}", e.message()))?;
    Ok(worktree_path.to_path_buf())
}

/// Stage every change in the worktree EXCEPT `.joy/` and commit as the AI
/// member. Returns None when that leaves nothing to commit. Unlike
/// `commit_joy` this commits code, and it is the fallback for work an agent
/// left uncommitted — item state never rides a job branch (JP-006D-28), so
/// `.joy` paths are excluded from staging (and any `.joy` change the agent
/// staged itself is unstaged first).
pub fn commit_all(
    worktree_dir: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<Option<String>> {
    let repo = open(worktree_dir).map_err(err)?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    // Unstage anything the agent staged under .joy (git add without commit),
    // so the fallback tree below cannot carry it.
    if let Some(p) = &parent {
        repo.reset_default(Some(p.as_object()), [".joy"]).ok();
    }
    let mut index = repo.index().map_err(err)?;
    index
        .add_all(
            ["*"],
            git2::IndexAddOption::DEFAULT,
            Some(&mut |path: &Path, _spec: &[u8]| -> i32 {
                if path.starts_with(".joy") {
                    1 // skip: item state never rides the job branch
                } else {
                    0
                }
            }),
        )
        .map_err(err)?;
    index.write().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;
    if parent.as_ref().map(|p| p.tree_id()) == Some(tree_id) {
        return Ok(None); // nothing but (excluded) .joy noise changed
    }
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let signature = git2::Signature::now(author_name, author_email).map_err(err)?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(err)?;
    Ok(Some(oid.to_string()))
}

/// Push the worktree's branch to the forge with the account token.
pub fn push_branch(worktree_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    push(worktree_dir, auth)
}

/// Remove a linked worktree and its registration in the checkout.
pub fn prune_worktree(repo_dir: &Path, worktree_name: &str, worktree_path: &Path) {
    std::fs::remove_dir_all(worktree_path).ok();
    if let Ok(repo) = open(repo_dir) {
        if let Ok(wt) = repo.find_worktree(worktree_name) {
            // best effort by contract — a stale registration only blocks
            // the next create_worktree, but the cause belongs in the log
            if let Err(e) = wt.prune(Some(git2::WorktreePruneOptions::new().valid(true))) {
                tracing::warn!(worktree = %worktree_name, error = %e.message(),
                    "worktree registration prune failed");
            }
        }
    }
}

// ---- job sandbox: the joywork checkout (JP-006D-28) ---------------------
//
// jobs/<job-id>/joywork is the agent's item-state surface: a git2 worktree
// of the project checkout at MAIN's tip. refs/heads/main is already checked
// out in the platform checkout and a branch cannot be checked out twice, so
// the joywork worktree sits on a per-job LOCAL branch
// `joy/jobwork/<job-id>` forked at main's tip (never pushed). After the run
// the platform commits joywork's `.joy` changes on that branch and lands
// them on main via [`land_branch_yaml`].

/// The local (never pushed) branch carrying a job's joywork checkout.
pub fn jobwork_branch(job_id: &str) -> String {
    format!("joy/jobwork/{job_id}")
}

fn jobwork_worktree_name(job_id: &str) -> String {
    format!("jobwork-{job_id}")
}

/// Create the joywork worktree for a job: fork `joy/jobwork/<job-id>` at
/// the checkout's current HEAD (main's tip) and check it out at
/// `joywork_path`. A stale registration or branch from a crashed run is
/// replaced.
pub fn create_joywork(
    repo_dir: &Path,
    job_id: &str,
    joywork_path: &Path,
) -> anyhow::Result<PathBuf> {
    let repo = open(repo_dir).map_err(err)?;
    let name = jobwork_worktree_name(job_id);
    if let Ok(wt) = repo.find_worktree(&name) {
        // the working tree is gone (caller checked); drop the registration
        if let Err(e) = wt.prune(Some(git2::WorktreePruneOptions::new().valid(true))) {
            tracing::warn!(worktree = %name, error = %e.message(),
                "stale joywork registration prune failed");
        }
    }
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let refname = format!("refs/heads/{}", jobwork_branch(job_id));
    repo.reference(&refname, head.id(), true, "joy-vcs: joywork fork")
        .map_err(err)?;
    let reference = repo.find_reference(&refname).map_err(err)?;
    let mut opts = git2::WorktreeAddOptions::new();
    opts.reference(Some(&reference));
    repo.worktree(&name, joywork_path, Some(&opts))
        .map_err(|e| anyhow::anyhow!("joywork add failed: {}", e.message()))?;
    Ok(joywork_path.to_path_buf())
}

/// Remove a job's joywork worktree and its local jobwork branch.
pub fn prune_joywork(repo_dir: &Path, job_id: &str, joywork_path: &Path) {
    prune_worktree(repo_dir, &jobwork_worktree_name(job_id), joywork_path);
    if let Ok(repo) = open(repo_dir) {
        if let Ok(mut b) = repo.find_branch(&jobwork_branch(job_id), git2::BranchType::Local) {
            if let Err(e) = b.delete() {
                tracing::warn!(job = %job_id, error = %e.message(),
                    "jobwork branch delete failed; a later round re-points it");
            }
        }
    }
}

/// The commit id a local branch points at.
pub fn branch_tip(repo_dir: &Path, branch: &str) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(err)?;
    Ok(repo
        .refname_to_id(&format!("refs/heads/{branch}"))
        .map_err(err)?
        .to_string())
}

/// Validate a job branch's own commits (fork-point..HEAD of the worktree):
/// returns the first commit whose diff against its first parent touches a
/// path under `.joy/`, as `(commit-description, path)`. Item state never
/// rides a job branch — the agent's `.joy` writes belong in the joywork
/// checkout (JP-006D-28). `None` means the branch is clean.
pub fn joy_commit_on_branch(
    checkout_dir: &Path,
    worktree_dir: &Path,
) -> anyhow::Result<Option<(String, String)>> {
    let main_repo = open(checkout_dir).map_err(err)?;
    let main_oid = main_repo
        .head()
        .map_err(err)?
        .target()
        .ok_or_else(|| anyhow::anyhow!("unborn HEAD in checkout"))?;
    // the linked worktree shares the checkout's object database, so main's
    // oid resolves here too
    let repo = open(worktree_dir).map_err(err)?;
    let tip = repo
        .head()
        .map_err(err)?
        .target()
        .ok_or_else(|| anyhow::anyhow!("unborn HEAD in worktree"))?;
    let fork = repo.merge_base(tip, main_oid).map_err(err)?;
    let mut walk = repo.revwalk().map_err(err)?;
    walk.push(tip).map_err(err)?;
    walk.hide(fork).map_err(err)?;
    for oid in walk {
        let oid = oid.map_err(err)?;
        let commit = repo.find_commit(oid).map_err(err)?;
        let tree = commit.tree().map_err(err)?;
        let parent_tree = match commit.parents().next() {
            Some(p) => Some(p.tree().map_err(err)?),
            None => None,
        };
        let diff = repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .map_err(err)?;
        for delta in diff.deltas() {
            for f in [delta.new_file().path(), delta.old_file().path()]
                .into_iter()
                .flatten()
            {
                if f.starts_with(".joy") {
                    let describe = format!(
                        "{:.8} ({})",
                        oid.to_string(),
                        commit.summary().ok().flatten().unwrap_or_default()
                    );
                    return Ok(Some((describe, f.display().to_string())));
                }
            }
        }
    }
    Ok(None)
}

/// The first path under `.joy/` in the diff of `branch` against its merge
/// base with HEAD (main) — AcceptJob's second line of defense, which also
/// catches direct commits pushed to the branch on the forge.
pub fn branch_touches_joy(repo_dir: &Path, branch: &str) -> anyhow::Result<Option<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let their = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|e| anyhow::anyhow!("branch {branch}: {}", e.message()))?
        .into_reference()
        .peel_to_commit()
        .map_err(err)?;
    let base_oid = repo
        .merge_base(head.id(), their.id())
        .unwrap_or_else(|_| head.id());
    let base_tree = repo
        .find_commit(base_oid)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let their_tree = their.tree().map_err(err)?;
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&their_tree), None)
        .map_err(err)?;
    for delta in diff.deltas() {
        for f in [delta.new_file().path(), delta.old_file().path()]
            .into_iter()
            .flatten()
        {
            if f.starts_with(".joy") {
                return Ok(Some(f.display().to_string()));
            }
        }
    }
    Ok(None)
}

/// Land a jobwork branch's `.joy` commit(s) onto the current branch (main)
/// of the checkout: fast-forward when main has not moved since the fork,
/// otherwise a three-way merge whose conflicting `.joy/*.yaml` files
/// resolve through joy-core's YAML engine (same policy as [`pull_merge`]).
/// Does NOT push — the caller lands this in the same gate-locked phase as
/// the attempt write and pushes main once.
pub fn land_branch_yaml(
    repo_dir: &Path,
    branch: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let span = tracing::info_span!("git.land_branch_yaml", repo = %repo_dir.display(), %branch);
    let _s = span.enter();
    let result = land_branch_yaml_inner(repo_dir, branch, message, author_name, author_email);
    if let Err(e) = &result {
        tracing::error!(repo = %repo_dir.display(), %branch, error = %e,
            "landing joywork changes on main failed");
    }
    result
}

fn land_branch_yaml_inner(
    repo_dir: &Path,
    branch: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(err)?;
    let head_refname = repo
        .head()
        .map_err(err)?
        .name()
        .map_err(|_| anyhow::anyhow!("unnamed HEAD"))?
        .to_string();
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let their = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|e| anyhow::anyhow!("branch {branch}: {}", e.message()))?
        .into_reference()
        .peel_to_commit()
        .map_err(err)?;
    let base = repo.merge_base(head.id(), their.id()).map_err(err)?;
    if their.id() == head.id() || base == their.id() {
        return Ok(head.id().to_string()); // nothing new on the branch
    }
    if base == head.id() {
        // main has not moved since the fork: fast-forward
        let mut reference = repo.find_reference(&head_refname).map_err(err)?;
        reference
            .set_target(their.id(), "joy-vcs: land joywork (ff)")
            .map_err(err)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(err)?;
        return Ok(their.id().to_string());
    }
    let mut index = repo.merge_commits(&head, &their, None).map_err(err)?;
    resolve_conflicts_yaml_aware(&repo, &mut index)?;
    let tree_id = index.write_tree_to(&repo).map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let sig = git2::Signature::now(author_name, author_email).map_err(err)?;
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head, &their])
        .map_err(err)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
        .map_err(err)?;
    repo.cleanup_state().ok();
    Ok(oid.to_string())
}

/// Best-effort refresh of a local branch from the forge before AcceptJob's
/// checks: fetch `refs/heads/<branch>` and point the local ref at the
/// remote tip (the forge is the source of truth — this is how direct
/// commits pushed to the branch become visible to the `.joy` freeze and
/// the merge). Errors (offline, unborn remote ref) leave the local state.
pub fn refresh_branch_from_forge(repo_dir: &Path, branch: &str, auth: &Auth) {
    let refresh = || -> anyhow::Result<()> {
        let repo = open(repo_dir).map_err(err)?;
        let tracking = format!("refs/joy/branch-refresh/{branch}");
        let src = format!("refs/heads/{branch}");
        // download_ref, like every fetch here: libgit2's update_tips (and
        // its unconditional FETCH_HEAD truncation) never runs
        let Some(tip) = download_ref(&repo, auth, &src, &tracking)? else {
            anyhow::bail!("branch {branch} not on the forge");
        };
        repo.reference(
            &format!("refs/heads/{branch}"),
            tip,
            true,
            "joy-vcs: refresh job branch from forge",
        )
        .map_err(err)?;
        if let Ok(mut done) = repo.find_reference(&tracking) {
            done.delete().ok();
        }
        Ok(())
    };
    if let Err(e) = refresh() {
        tracing::debug!(%branch, error = %e, "job branch refresh skipped; using local state");
    }
}

/// AcceptJob's merge (JP-006D-28): refuse when the branch diff (merge
/// base..branch) touches `.joy/` — job branches must not carry item state;
/// `.joy` changes ride main via the joywork landing — then merge the
/// branch into main.
pub fn merge_job_branch(
    repo_dir: &Path,
    branch: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let span = tracing::info_span!("git.merge_job_branch", repo = %repo_dir.display(), %branch);
    let _s = span.enter();
    let result = (|| -> anyhow::Result<String> {
        if let Some(path) = branch_touches_joy(repo_dir, branch)? {
            anyhow::bail!(
                "branch {branch} touches {path}: job branches must not carry item state; \
                 .joy changes ride main (JP-006D-28)"
            );
        }
        merge_branch(repo_dir, branch, message, author_name, author_email)
    })();
    if let Err(e) = &result {
        tracing::error!(repo = %repo_dir.display(), %branch, error = %e,
            "job branch merge refused or failed");
    }
    result
}

/// Merge `branch` into the current branch (main) with a merge commit and
/// return the new commit id. Code changes (branch) and record changes
/// (main) touch different files, so the merge is clean; a genuine conflict
/// is surfaced as an error rather than a fake success.
pub fn merge_branch(
    repo_dir: &std::path::Path,
    branch: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let their = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|e| anyhow::anyhow!("branch {branch}: {}", e.message()))?
        .into_reference()
        .peel_to_commit()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let head = repo.head()?.peel_to_commit()?;
    let annotated = repo.find_annotated_commit(their.id())?;
    repo.merge(&[&annotated], None, None)
        .map_err(|e| anyhow::anyhow!("merge {branch}: {}", e.message()))?;
    if repo.index()?.has_conflicts() {
        repo.cleanup_state().ok();
        anyhow::bail!("merge of {branch} has conflicts");
    }
    let mut index = repo.index()?;
    let tree = repo.find_tree(index.write_tree()?)?;
    let sig = git2::Signature::now(author_name, author_email)?;
    let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head, &their])?;
    repo.cleanup_state().ok();
    // reset the working tree to the merged commit
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
    Ok(oid.to_string())
}

/// One changed file in a job branch vs its base.
pub struct DiffFile {
    pub path: String,
    pub patch: String,
    pub additions: u32,
    pub deletions: u32,
}

/// The diff of `branch` against its merge-base with HEAD (main): what the AI
/// proposes. Returns one entry per changed file with a unified patch.
pub fn branch_diff(repo_dir: &std::path::Path, branch: &str) -> anyhow::Result<Vec<DiffFile>> {
    let repo = open(repo_dir).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let head = repo.head()?.peel_to_commit()?;
    let their = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|e| anyhow::anyhow!("branch {branch}: {}", e.message()))?
        .into_reference()
        .peel_to_commit()?;
    let base_oid = repo
        .merge_base(head.id(), their.id())
        .unwrap_or_else(|_| head.id());
    let base_tree = repo.find_commit(base_oid)?.tree()?;
    let their_tree = their.tree()?;
    let mut opts = git2::DiffOptions::new();
    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&their_tree), Some(&mut opts))?;

    // Collect per-file patches by walking the diff.
    let mut files: Vec<DiffFile> = Vec::new();
    let num_deltas = diff.deltas().len();
    for i in 0..num_deltas {
        if let Some(mut patch) = git2::Patch::from_diff(&diff, i)? {
            let delta = patch
                .delta()
                .new_file()
                .path()
                .or_else(|| patch.delta().old_file().path())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let (_ctx, adds, dels) = patch.line_stats()?;
            let buf = patch.to_buf()?;
            files.push(DiffFile {
                path: delta,
                patch: String::from_utf8_lossy(&buf).to_string(),
                additions: adds as u32,
                deletions: dels as u32,
            });
        }
    }
    Ok(files)
}

/// Branch state of a checkout for the web header (JP-0062): current branch,
/// all local branches, ahead/behind vs the remote counterpart.
pub struct BranchState {
    pub branch: String,
    pub branches: Vec<String>,
    pub ahead: u32,
    pub behind: u32,
    pub has_remote: bool,
}

pub fn branch_state(repo_dir: &Path) -> anyhow::Result<BranchState> {
    let repo = open(repo_dir).map_err(err)?;
    // an unborn HEAD (fresh init, no commit yet) is a state, not an error
    let head = repo.head().ok();
    let branch = head
        .as_ref()
        .and_then(|h| h.shorthand().ok())
        .unwrap_or("HEAD")
        .to_string();
    let mut branches = Vec::new();
    for b in repo.branches(Some(git2::BranchType::Local)).map_err(err)? {
        let (b, _) = b.map_err(err)?;
        if let Some(name) = b.name().map_err(err)? {
            branches.push(name.to_string());
        }
    }
    branches.sort();
    let (mut ahead, mut behind, mut has_remote) = (0u32, 0u32, false);
    if let (Some(local), Ok(upstream)) = (
        head.as_ref().and_then(|h| h.target()),
        repo.find_branch(&branch, git2::BranchType::Local)
            .and_then(|b| b.upstream())
            .and_then(|u| {
                u.into_reference()
                    .target()
                    .ok_or_else(|| git2::Error::from_str("no upstream target"))
            }),
    ) {
        has_remote = true;
        if let Ok((a, b)) = repo.graph_ahead_behind(local, upstream) {
            ahead = a as u32;
            behind = b as u32;
        }
    }
    Ok(BranchState {
        branch,
        branches,
        ahead,
        behind,
        has_remote,
    })
}

/// Switch the checkout to `branch` — creating a local branch from the
/// remote-tracking ref when only the remote has it. Tree first, HEAD
/// second: the reverse order leaves HEAD on the new branch with the old
/// worktree when checkout fails or is partial, which reads as dirty and
/// blocks every further switch (JAPP-0097).
pub fn checkout_branch(repo_dir: &Path, branch: &str) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    let refname = format!("refs/heads/{branch}");
    if repo.find_reference(&refname).is_err() {
        let remote_ref = repo
            .branches(Some(git2::BranchType::Remote))
            .map_err(err)?
            .flatten()
            .find(|(b, _)| {
                b.name()
                    .ok()
                    .flatten()
                    .map(|full| full.split_once('/').map(|(_, s)| s).unwrap_or(full) == branch)
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow::anyhow!("unknown branch: {branch}"))?;
        let target = remote_ref.0.get().peel_to_commit().map_err(err)?;
        repo.branch(branch, &target, false).map_err(err)?;
    }
    let target = repo
        .find_reference(&refname)
        .map_err(err)?
        .peel_to_commit()
        .map_err(err)?;
    repo.checkout_tree(
        target.as_object(),
        Some(git2::build::CheckoutBuilder::new().safe()),
    )
    .map_err(|e| anyhow::anyhow!("checkout: {}", e.message()))?;
    repo.set_head(&refname).map_err(err)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Local end-to-end against a bare "forge" repo: clone, commit, push,
    /// pull. No network, real git2 semantics.
    #[test]
    fn clone_commit_push_pull_roundtrip() {
        let base = std::env::temp_dir().join(format!("jp-git-test-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();

        // Seed the forge with an initial commit holding a .joy marker.
        let seed = base.join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/marker"), "hello").unwrap();
        let mut index = seed_repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = seed_repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        seed_repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        let head = seed_repo.head().unwrap();
        let branch = head.shorthand().unwrap().to_string();
        seed_repo
            .find_remote("origin")
            .unwrap()
            .push(
                &[format!("refs/heads/{branch}:refs/heads/{branch}").as_str()],
                None,
            )
            .unwrap();

        // Clone like the server does (file URLs ignore the token callback).
        let checkout = base.join("checkout");
        clone(
            forge.to_str().unwrap(),
            &Auth::token("irrelevant"),
            &checkout,
        )
        .expect("clone");
        assert!(checkout.join(".joy/marker").exists());

        // Write, commit, push, and see it arrive via a second pull.
        std::fs::write(checkout.join(".joy/item.yaml"), "id: X-1").unwrap();
        let committed =
            commit_joy(&checkout, "test: item", "Tester", "tester@example.com").expect("commit");
        assert!(committed.is_some());
        push(&checkout, &Auth::token("irrelevant")).expect("push");
        assert!(commit_joy(&checkout, "again", "T", "t@e.c")
            .unwrap()
            .is_none());

        let second = base.join("second");
        clone(forge.to_str().unwrap(), &Auth::token("irrelevant"), &second).expect("clone 2");
        assert!(second.join(".joy/item.yaml").exists());
        pull_ff(&checkout, &Auth::token("irrelevant")).expect("pull up-to-date");

        std::fs::remove_dir_all(&base).ok();
    }

    /// A job worktree: branch off the checkout, change code (not just .joy),
    /// commit all, push the branch to the bare forge, verify it landed —
    /// and verify `.joy` changes never reach the branch commit
    /// (JP-006D-28: item state rides main, not job branches).
    #[test]
    fn worktree_branch_commit_push_roundtrip() {
        let base = std::env::temp_dir().join(format!("jp-wt-test-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();
        let seed = base.join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::write(seed.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/marker.yaml"), "state: original\n").unwrap();
        let mut index = seed_repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = seed_repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        seed_repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        let branch0 = seed_repo.head().unwrap().shorthand().unwrap().to_string();
        seed_repo
            .find_remote("origin")
            .unwrap()
            .push(
                &[format!("refs/heads/{branch0}:refs/heads/{branch0}").as_str()],
                None,
            )
            .unwrap();

        let checkout = base.join("checkout");
        clone(forge.to_str().unwrap(), &Auth::token("x"), &checkout).expect("clone");

        // job worktree on a fresh branch
        let wt = base.join("wt");
        let job_branch = "joy/claude/X-1-abc";
        create_worktree(&checkout, "job-abc", job_branch, &wt).expect("worktree");
        assert!(wt.join("main.rs").exists());

        // the agent changes code AND (illegitimately) item state; the
        // fallback commit stages the code only
        std::fs::write(wt.join("main.rs"), "fn main() { println!(\"hi\"); }\n").unwrap();
        std::fs::write(wt.join("NEW.txt"), "added").unwrap();
        std::fs::write(wt.join(".joy/marker.yaml"), "state: tampered\n").unwrap();
        std::fs::write(wt.join(".joy/new-item.yaml"), "id: nope\n").unwrap();
        let committed = commit_all(&wt, "feat: work", "claude", "ai:claude@joy").expect("commit");
        assert!(committed.is_some());
        push_branch(&wt, &Auth::token("x")).expect("push");

        // the branch is on the forge with the code change, .joy untouched
        let forge_repo = git2::Repository::open_bare(&forge).unwrap();
        let branch_ref = forge_repo
            .find_reference(&format!("refs/heads/{job_branch}"))
            .expect("job branch on forge");
        let commit = branch_ref.peel_to_commit().unwrap();
        assert!(commit.message().unwrap().contains("feat: work"));
        let tree = commit.tree().unwrap();
        assert!(tree.get_name("NEW.txt").is_some());
        let joy = tree
            .get_name(".joy")
            .unwrap()
            .to_object(&forge_repo)
            .unwrap()
            .peel_to_tree()
            .unwrap();
        assert!(joy.get_name("new-item.yaml").is_none(), "no new .joy file");
        let marker = joy
            .get_name("marker.yaml")
            .unwrap()
            .to_object(&forge_repo)
            .unwrap()
            .peel_to_blob()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(marker.content()),
            "state: original\n",
            ".joy edit did not ride the fallback commit"
        );

        // a worktree with ONLY .joy noise yields no commit at all
        assert!(commit_all(&wt, "noise", "c", "c@e").unwrap().is_none());

        prune_worktree(&checkout, "job-abc", &wt);
        std::fs::remove_dir_all(&base).ok();
    }

    /// AcceptJob's freeze (JP-006D-28): a job branch whose diff touches
    /// `.joy/` is refused; a clean branch merges.
    #[test]
    fn merge_job_branch_refuses_item_state_on_the_branch() {
        let base = std::env::temp_dir().join(format!("jp-freeze-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();
        let seed = base.join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::write(seed.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/item.yaml"), "id: X-1\n").unwrap();
        let mut index = seed_repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = seed_repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        seed_repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        let b = seed_repo.head().unwrap().shorthand().unwrap().to_string();
        seed_repo
            .find_remote("origin")
            .unwrap()
            .push(&[format!("refs/heads/{b}:refs/heads/{b}").as_str()], None)
            .unwrap();
        let checkout = base.join("checkout");
        clone(forge.to_str().unwrap(), &Auth::token("x"), &checkout).unwrap();

        // a dirty branch: code plus a direct .joy commit
        let wt = base.join("wt-dirty");
        create_worktree(&checkout, "wt-dirty", "joy/claude/dirty", &wt).unwrap();
        std::fs::write(wt.join("ok.txt"), "fine\n").unwrap();
        std::fs::write(wt.join(".joy/item.yaml"), "id: X-1\nstatus: closed\n").unwrap();
        {
            let repo = open(&wt).unwrap();
            let mut idx = repo.index().unwrap();
            idx.add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            idx.write().unwrap();
            let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("agent", "a@e.c").unwrap();
            let parent = repo.head().unwrap().peel_to_commit().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "sneaky", &tree, &[&parent])
                .unwrap();
        }
        assert_eq!(
            branch_touches_joy(&checkout, "joy/claude/dirty").unwrap(),
            Some(".joy/item.yaml".to_string())
        );
        let err = merge_job_branch(&checkout, "joy/claude/dirty", "merge", "h", "h@e.c")
            .expect_err("dirty branch refused");
        assert!(
            err.to_string().contains("must not carry item state"),
            "clear refusal: {err}"
        );

        // a clean branch passes the freeze and merges
        let wt2 = base.join("wt-clean");
        create_worktree(&checkout, "wt-clean", "joy/claude/clean", &wt2).unwrap();
        std::fs::write(wt2.join("ok.txt"), "fine\n").unwrap();
        commit_all(&wt2, "feat: clean", "c", "c@e.c").unwrap();
        assert_eq!(
            branch_touches_joy(&checkout, "joy/claude/clean").unwrap(),
            None
        );
        merge_job_branch(&checkout, "joy/claude/clean", "merge", "h", "h@e.c")
            .expect("clean branch merges");
        assert!(checkout.join("ok.txt").is_file());

        std::fs::remove_dir_all(&base).ok();
    }
}

#[cfg(test)]
mod engine_invariant_tests {
    use super::*;

    struct Rig {
        _tmp: tempfile::TempDir,
        forge: std::path::PathBuf,
        clone_dir: std::path::PathBuf,
        seed: std::path::PathBuf,
    }

    /// A bare "forge" with one commit on main, plus a clone the way the
    /// product clones. No network, real git2 semantics.
    fn rig() -> Rig {
        let tmp = tempfile::tempdir().expect("tempdir");
        let forge = tmp.path().join("forge.git");
        git2::Repository::init_bare(&forge).unwrap();
        let seed = tmp.path().join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/item.yaml"), "id: X-1\ntitle: one\n").unwrap();
        commit_everything(&seed_repo, "seed");
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        push_current_branch(&seed_repo);
        let clone_dir = tmp.path().join("clone");
        clone(forge.to_str().unwrap(), &Auth::token(""), &clone_dir).expect("clone");
        Rig {
            _tmp: tmp,
            forge,
            clone_dir,
            seed,
        }
    }

    fn commit_everything(repo: &git2::Repository, message: &str) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        let parent = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .and_then(|o| repo.find_commit(o).ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap();
    }

    fn push_current_branch(repo: &git2::Repository) {
        let branch = repo.head().unwrap().shorthand().unwrap().to_string();
        repo.find_remote("origin")
            .unwrap()
            .push(
                &[format!("refs/heads/{branch}:refs/heads/{branch}").as_str()],
                None,
            )
            .unwrap();
    }

    /// THE invariant this engine exists for: no verb ever writes
    /// FETCH_HEAD. It is the one file git updates without a lock, and
    /// sharing it tore syncs apart twice (JP-00DB-61, JAPP-0198-EA).
    #[test]
    fn no_verb_ever_writes_fetch_head() {
        let rig = rig();
        let auth = Auth::token("");
        let fetch_head = rig.clone_dir.join(".git/FETCH_HEAD");
        std::fs::remove_file(&fetch_head).ok();
        fetch_branch(&rig.clone_dir, &auth).unwrap();
        if fetch_head.exists() {
            panic!(
                "fetch_branch wrote FETCH_HEAD: {:?}",
                std::fs::read_to_string(&fetch_head)
            );
        }
        ff_from_tracking(&rig.clone_dir).unwrap();
        assert!(!fetch_head.exists(), "ff wrote FETCH_HEAD");
        // an absent side ref (chats of a project that never pushed them)
        assert!(!fetch_ref(
            &rig.clone_dir,
            &auth,
            "refs/joy/chats",
            "refs/joy/chats-tracking"
        )
        .unwrap());
        assert!(!fetch_head.exists(), "fetch_ref wrote FETCH_HEAD");
        pull_merge(&rig.clone_dir, &auth, "T", "t@example.com").unwrap();
        assert!(!fetch_head.exists(), "pull_merge wrote FETCH_HEAD");
        push(&rig.clone_dir, &auth).unwrap();
        assert!(!fetch_head.exists(), "push wrote FETCH_HEAD");
        let branch = open(&rig.clone_dir)
            .unwrap()
            .head()
            .unwrap()
            .shorthand()
            .unwrap()
            .to_string();
        refresh_branch_from_forge(&rig.clone_dir, &branch, &auth);
        assert!(
            !fetch_head.exists(),
            "refresh_branch_from_forge wrote FETCH_HEAD"
        );
    }

    /// The 30-second re-clone loop of 2026-08-25: a forge WITHOUT
    /// refs/joy/chats must not poison anything — every round lands, and
    /// a stale tracking ref from earlier days is cleaned up.
    #[test]
    fn a_forge_without_a_chats_ref_is_harmless() {
        let rig = rig();
        let auth = Auth::token("");
        let repo = git2::Repository::open(&rig.clone_dir).unwrap();
        // a stale tracking ref from an earlier sync
        let head = repo.head().unwrap().target().unwrap();
        repo.reference("refs/joy/chats-tracking", head, true, "stale")
            .unwrap();
        for _ in 0..2 {
            fetch_branch(&rig.clone_dir, &auth).unwrap();
            ff_from_tracking(&rig.clone_dir).unwrap();
            assert!(!fetch_ref(
                &rig.clone_dir,
                &auth,
                "refs/joy/chats",
                "refs/joy/chats-tracking"
            )
            .unwrap());
        }
        assert!(
            repo.find_reference("refs/joy/chats-tracking").is_err(),
            "the stale tracking ref must be gone"
        );
    }

    /// A branch renamed or deleted on the forge is said out loud
    /// (JP-00DB-61 follow-up de0d186), not reported as corruption.
    #[test]
    fn a_vanished_branch_is_named() {
        let rig = rig();
        let forge_repo = git2::Repository::open_bare(&rig.forge).unwrap();
        forge_repo
            .find_branch("main", git2::BranchType::Local)
            .or_else(|_| forge_repo.find_branch("master", git2::BranchType::Local))
            .unwrap()
            .rename("trunk", true)
            .unwrap();
        forge_repo.set_head("refs/heads/trunk").unwrap();
        let err = fetch_branch(&rig.clone_dir, &Auth::token("")).expect_err("branch is gone");
        let msg = err.to_string();
        assert!(msg.contains("not found on the forge"), "{msg}");
        assert!(!msg.contains("corrupted"), "{msg}");
    }

    /// Divergence is the NORMAL case under write-behind: the remote moves,
    /// the local side has its own `.joy` commit, and pull_merge produces a
    /// YAML-merged commit carrying the ACTING MEMBER as author
    /// (JP-00DE-11), never the server.
    #[test]
    fn diverged_histories_merge_yaml_aware_under_the_members_name() {
        let rig = rig();
        let auth = Auth::token("");
        // remote side: title changes
        std::fs::write(rig.seed.join(".joy/item.yaml"), "id: X-1\ntitle: forge\n").unwrap();
        let seed_repo = git2::Repository::open(&rig.seed).unwrap();
        commit_everything(&seed_repo, "remote change");
        push_current_branch(&seed_repo);
        // local side: a new field
        std::fs::write(
            rig.clone_dir.join(".joy/item.yaml"),
            "id: X-1\ntitle: one\npriority: high\n",
        )
        .unwrap();
        let committed = commit_joy(
            &rig.clone_dir,
            "local change",
            "Member",
            "member@example.com",
        )
        .unwrap();
        assert!(committed.is_some());
        pull_merge(&rig.clone_dir, &auth, "Member", "member@example.com").unwrap();
        let merged = std::fs::read_to_string(rig.clone_dir.join(".joy/item.yaml")).unwrap();
        assert!(merged.contains("title: forge"), "{merged}");
        assert!(merged.contains("priority: high"), "{merged}");
        let repo = git2::Repository::open(&rig.clone_dir).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.parents().len(), 2, "a real merge commit");
        assert_eq!(head.author().email().ok(), Some("member@example.com"));
        // and the result pushes: nothing left ahead or behind afterwards
        push(&rig.clone_dir, &auth).unwrap();
        let (ahead, behind) = ahead_behind(&rig.clone_dir).unwrap();
        assert_eq!((ahead, behind), (0, 0));
    }

    /// A hand-wired repo may call its remote anything: the tracking ref
    /// follows the RESOLVED remote's name, never a hardwired "origin".
    #[test]
    fn a_remote_by_any_other_name_works() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let forge = tmp.path().join("forge.git");
        git2::Repository::init_bare(&forge).unwrap();
        let work = tmp.path().join("work");
        init_repo(&work, "main").unwrap();
        std::fs::write(work.join("a.txt"), "a").unwrap();
        super::commit_everything(&work, "seed", "t", "t@example.com").unwrap();
        add_remote(&work, "upstream", forge.to_str().unwrap()).unwrap();
        let auth = Auth::token("");
        push(&work, &auth).unwrap();
        fetch_branch(&work, &auth).unwrap();
        ff_from_tracking(&work).unwrap();
        let repo = open(&work).unwrap();
        assert!(
            repo.find_reference("refs/remotes/upstream/main").is_ok(),
            "the tracking ref lives under the remote's real name"
        );
        assert!(repo.find_reference("refs/remotes/origin/main").is_err());
        assert_eq!(ahead_behind(&work).unwrap(), (0, 0));
    }

    /// The plain catch-up: remote moved, local clean, the fetch + ff pair
    /// brings the clone current without inventing commits.
    #[test]
    fn a_clean_clone_fast_forwards() {
        let rig = rig();
        let auth = Auth::token("");
        std::fs::write(rig.seed.join("code.txt"), "v2").unwrap();
        let seed_repo = git2::Repository::open(&rig.seed).unwrap();
        commit_everything(&seed_repo, "remote moves");
        push_current_branch(&seed_repo);
        fetch_branch(&rig.clone_dir, &auth).unwrap();
        ff_from_tracking(&rig.clone_dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(rig.clone_dir.join("code.txt")).unwrap(),
            "v2"
        );
        let (ahead, behind) = ahead_behind(&rig.clone_dir).unwrap();
        assert_eq!((ahead, behind), (0, 0));
    }
}

#[cfg(test)]
mod pull_merge_tests {
    use super::*;

    fn commit_all(repo: &git2::Repository, message: &str) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@example.com").unwrap();
        let parents: Vec<git2::Commit> = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .and_then(|o| repo.find_commit(o).ok())
            .into_iter()
            .collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .unwrap();
    }

    #[test]
    fn diverged_chat_writes_merge_and_push() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("forge.git");
        git2::Repository::init_bare(&bare).unwrap();
        let url = bare.to_str().unwrap().to_string();

        let a_dir = tmp.path().join("a");
        let a = git2::Repository::clone(&url, &a_dir).unwrap();
        std::fs::create_dir_all(a_dir.join(".joy/chats")).unwrap();
        std::fs::write(
            a_dir.join(".joy/chats/c.yaml"),
            "id: c\ntitle: T\ncreated: 2026-07-05T10:00:00Z\nupdated: 2026-07-05T10:00:00Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T10:00:01Z\n  author: a@x\n  text: hello\n",
        )
        .unwrap();
        commit_all(&a, "seed");
        push(&a_dir, &Auth::token("")).unwrap();

        let b_dir = tmp.path().join("b");
        let _b = git2::Repository::clone(&url, &b_dir).unwrap();

        // A appends m2 and pushes; B appends m3 without knowing about m2
        std::fs::write(
            a_dir.join(".joy/chats/c.yaml"),
            "id: c\ntitle: T\ncreated: 2026-07-05T10:00:00Z\nupdated: 2026-07-05T10:00:02Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T10:00:01Z\n  author: a@x\n  text: hello\n- id: m2\n  at: 2026-07-05T10:00:02Z\n  author: a@x\n  text: from A\n",
        )
        .unwrap();
        commit_all(&a, "a: m2");
        push(&a_dir, &Auth::token("")).unwrap();

        let b = open(&b_dir).unwrap();
        std::fs::write(
            b_dir.join(".joy/chats/c.yaml"),
            "id: c\ntitle: T\ncreated: 2026-07-05T10:00:00Z\nupdated: 2026-07-05T10:00:03Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T10:00:01Z\n  author: a@x\n  text: hello\n- id: m3\n  at: 2026-07-05T10:00:03Z\n  author: b@x\n  text: from B\n",
        )
        .unwrap();
        commit_all(&b, "b: m3");

        // push rejects (non-ff), merge unites, push succeeds
        assert!(push(&b_dir, &Auth::token("")).is_err());
        pull_merge(&b_dir, &Auth::token(""), "t", "t@example.com").unwrap();
        push(&b_dir, &Auth::token("")).unwrap();

        let merged = std::fs::read_to_string(b_dir.join(".joy/chats/c.yaml")).unwrap();
        assert!(merged.contains("from A"), "missing A's message: {merged}");
        assert!(merged.contains("from B"), "missing B's message: {merged}");
        let (ahead, behind) = ahead_behind(&b_dir).unwrap();
        assert_eq!((ahead, behind), (0, 0));
    }
}

#[cfg(test)]
mod basic_auth_tests {
    /// Each forge wants its own name in front of the token. Sending
    /// GitHub's convention to Codeberg is what made a valid token look
    /// like an unsupported authentication method.
    #[test]
    fn every_forge_gets_the_name_it_expects() {
        assert_eq!(
            super::basic_auth_user("https://github.com/owner/repo.git"),
            "x-access-token"
        );
        assert_eq!(
            super::basic_auth_user("https://gitlab.com/owner/repo.git"),
            "oauth2"
        );
        assert_eq!(
            super::basic_auth_user("https://gitlab.self-hosted.example/o/r.git"),
            "oauth2"
        );
        // Gitea and Codeberg take the token AS the name, with no password.
        assert_eq!(
            super::basic_auth_user("https://codeberg.org/joyint/forge-test.git"),
            ""
        );
        assert_eq!(
            super::basic_auth_user("https://gitea.int.joydev.com/joyint/joy.git"),
            ""
        );
    }
}
