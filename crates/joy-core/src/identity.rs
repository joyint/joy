// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Identity resolution for Joy CLI operations.
//!
//! Resolves the acting user's identity from, in this order (operator
//! decision 2026-09-19, item JOY-02AE-1A, and the operator's decisions of
//! 2026-09-27, which added the name on the call and put the person's
//! own session in front of git config):
//! 0. The name given to this very call: the global `--user <address>`,
//!    carried as `JOY_USER`. This call acts as that member and nothing
//!    is remembered; unproven like git config, so the guard asks for
//!    the passphrase where it must. (`joy auth --user` is the one
//!    command where the name makes a session instead.)
//! 1. The delegation session in `JOY_SESSION` (unchanged: an AI's
//!    identity has never come from anywhere else).
//! 2. The person who signed in at THIS terminal: the live human session
//!    of this project whose terminal binding matches, verified against
//!    that member's key in `project.yaml`. There is one per project and
//!    device, the one the last `joy auth` made. This is what makes
//!    "sign in once" true on a machine with no git identity at all, and
//!    what lets a person act as somebody other than the one git config
//!    names, for as long as the session lasts.
//! 3. `git config user.email` of the repository (`--local`).
//! 4. `git config user.email` global. One git2 read answers both: the
//!    config git2 opens through the repository already tries local
//!    before global before system (see
//!    [`crate::vcs::forge::user_identity`]), so a value only the global
//!    file carries is found the moment the local file has none.
//! 5. The account the forge tool for the remote's host reports (`gh`,
//!    `glab`, `tea`), matched by an address its own API vouches for.
//!
//! That is the whole list. Steps 3 and 4 are held against `project.yaml`
//! through [`crate::privacy::member_key_for_email`]; step 5 through
//! [`crate::privacy::member_key_for_any`], because a forge account can
//! vouch for more than one address. A match is the member.
//!
//! The device pin that used to stand at step 2 before the 2026-09-19
//! correction is gone from the whole module: nothing in joy writes or
//! reads one any more (a later addition to the same item, JOY-02AE-1A).
//! The session step is not a pin in disguise: a pin was a name written
//! once and read forever, a session is a signature this member made with
//! their passphrase, bound to a terminal and gone after 24 hours.
//! [`acting_member`] follows this same order too, see its own doc for
//! the one way it still differs from this function.
//!
//! A machine that answers none of these, a fresh clone with no
//! session, no git config and no forge login, or a checkout whose git
//! config and forge account both name nobody the project knows, is told
//! so instead of being guessed at: [`acting_member_key`] and
//! [`acting_human_key`] answer [`JoyError::UnknownActingMember`], whose
//! one line names the two remedies (`joy auth`, or `--user` with the
//! passphrase). A read-only command keeps working with an EMPTY member.
//!
//! AI members authenticate via `joy auth --token`, which creates a
//! session. There is no self-declared identity override.

use std::path::Path;

use crate::error::JoyError;
use crate::member_ref::MemberRef;
use crate::model::project::{is_ai_member, Project};
use crate::store;
use crate::vcs::Vcs;

/// The resolved identity of the acting user.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    /// The acting member. Resolves to name/e-mail on display and in `--json`
    /// (ADR-042); the raw at-rest value is the e-mail (open) or opaque id (anon).
    pub member: MemberRef,
    /// If the member is an AI, the human who delegated the action.
    pub delegated_by: Option<MemberRef>,
    /// Whether this identity was cryptographically authenticated (session or token).
    pub authenticated: bool,
}

impl Identity {
    /// Format for event log entries.
    /// Returns `"member"` or `"member delegated-by:human"`.
    ///
    /// This string is written to the on-disk event log and item `created_by` /
    /// `updated_by`, so it must carry the raw id, never the resolved value: use
    /// [`MemberRef::id`]. Resolution happens only when the log is read back.
    pub fn log_user(&self) -> String {
        match &self.delegated_by {
            Some(human) => format!("{} delegated-by:{}", self.member.id(), human.id()),
            None => self.member.id().to_string(),
        }
    }
}

/// Resolve the acting identity for the current operation.
///
/// Priority (operator decision 2026-09-19, item JOY-02AE-1A;
/// the name on the call and the
/// session step added by the operator on 2026-09-27): the name given to
/// this call first, then the delegation session, then the person who
/// signed in at this terminal, then git config, then the forge account,
/// and nothing else.
/// 0. `JOY_USER`, the global `--user` of this call (see [`named_user`]);
///    unauthenticated, nothing remembered
/// 1. JOY_SESSION -- ephemeral-key-bound AI session handle (ADR-033)
/// 2. The live human session of this project bound to this terminal,
///    verified against the member's key (see
///    [`member_key_from_session`]); authenticated
/// 3. The member key: `git config user.email`, repository then global
///    (one read; see [`member_key_from_git_config`])
/// 4. Failing that, the forge account for the remote's host (see
///    [`member_key_from_forge_account`])
/// 5. Fallback: that key, unauthenticated
///
/// (Steps 3 and 4 are the module doc's steps 3 to 5; they are numbered
/// together here because they fill the same variable below.)
///
/// A machine whose git config and forge account both name nobody the
/// project knows, and where nobody signed in, answers with an EMPTY
/// member, and the callers that need a name ([`acting_member_key`],
/// [`acting_human_key`]) turn that into
/// [`JoyError::UnknownActingMember`]; a read-only command keeps working
/// with no member, exactly as it does on a machine with no git identity
/// at all. The prefill a person is OFFERED lives in
/// [`git_config_prefill`], which only `--user`-shaped paths call.
pub fn resolve_identity(root: &Path) -> Result<Identity, JoyError> {
    let project = load_project_optional(root);
    let project_id = crate::auth::session::project_id(root).ok();

    // 0. A name given to this very call (`--user`, carried as JOY_USER):
    //    this call acts as that member and nothing is remembered. As
    //    unproven as git config; the guard asks for the passphrase where
    //    it must. The name is held against the member map the way an
    //    address from git config is, so an anonymous project answers
    //    with its opaque key.
    if let Some(named) = named_user() {
        let member = project
            .as_ref()
            .and_then(|p| crate::privacy::member_key_for_email_or_forge(p, root, &named, None))
            .unwrap_or(named);
        return Ok(Identity {
            member: member.into(),
            delegated_by: None,
            authenticated: false,
        });
    }

    // 1. JOY_SESSION: env var carries the ephemeral private key bound to
    //    the session (ADR-033). We derive the public key from it and match
    //    against `session_public_key` stored in the session file. Without
    //    possession of the env var a sibling terminal cannot reuse a
    //    session file it can read.
    //
    //    A process that carries the variable is an AI's, whether or not
    //    the session behind it still stands. It never falls back to a
    //    person's session (step 2): a delegation that expired, was
    //    rotated or lost its file would otherwise turn the agent into
    //    the person who signed in on this machine, authenticated and
    //    with their chat seed. It goes on unproven instead, like git
    //    config, and the guard refuses what needs proof.
    let ai_session_env = std::env::var("JOY_SESSION").ok().filter(|s| !s.is_empty());
    let carries_ai_session = ai_session_env.is_some();
    let mut said_why = false;
    if let Some(env_value) = ai_session_env {
        if let Some((sid, ephemeral_private, delegation_private)) =
            crate::auth::session::parse_session_env_full(&env_value)
        {
            if let Ok(Some(sess)) = crate::auth::session::load_session_by_id(&sid) {
                if sess.claims.expires > chrono::Utc::now() && is_ai_member(&sess.claims.member) {
                    let session_matches_project = project_id
                        .as_ref()
                        .map(|pid| sess.claims.project_id == *pid)
                        .unwrap_or(false);
                    if session_matches_project {
                        if let Some(ref project) = project {
                            if project.has_member_key(&sess.claims.member)
                                && ephemeral_public_matches(&sess, &ephemeral_private)
                            {
                                // Every AI session here is redeemed from a
                                // delegation token. The second kind that used
                                // to sit next to it, a session a server signed
                                // for itself and bound to a job, is gone with
                                // the platform key model (JI-0174 family).
                                if let Some(reason) = token_session_rejection(
                                    project,
                                    &sess,
                                    delegation_private.as_ref(),
                                ) {
                                    // F3 (JI-0175-B0): a token-redeemed AI
                                    // session must still trace to a LIVE
                                    // delegation. `delegation_key` is the
                                    // delegation_verifier bound at redemption;
                                    // if no member's ai_delegations still
                                    // carries it, the delegation was rotated or
                                    // removed and the session is dead now, not
                                    // at its TTL. When the session carries the
                                    // delegation private key (crypt scope), we
                                    // additionally require it to derive that
                                    // verifier — possession of the delegation
                                    // key, not just of a session file anyone
                                    // with state-dir write could author. Emit a
                                    // hint and fall through unauthenticated, as
                                    // the job and cross-project paths do.
                                    hint_once(&reason);
                                    said_why = true;
                                } else {
                                    return Ok(Identity {
                                        member: sess.claims.member.clone().into(),
                                        // F2 (JI-0175-B0): the delegating
                                        // operator is recorded in the signed
                                        // session claims at redemption; the
                                        // binding check above guarantees the
                                        // claim exists on every accepted
                                        // session, so there is nothing to
                                        // fall back to.
                                        delegated_by: sess
                                            .claims
                                            .delegated_by
                                            .clone()
                                            .map(Into::into),
                                        authenticated: true,
                                    });
                                }
                            }
                        }
                    } else if let Some(ref current_pid) = project_id {
                        // JOY_SESSION is a valid live AI session, but for a
                        // different project. Silently falling back to the
                        // identity below (git config or the forge account)
                        // would confuse the caller when the subsequent guard
                        // denial names the human instead of the AI they
                        // thought they were acting as. Emit a one-line
                        // stderr hint and continue with the fallback so
                        // read-only commands still work.
                        hint_once(&cross_project_session_warning(
                            &sess.claims.project_id,
                            &sess.claims.member,
                            current_pid,
                        ));
                        said_why = true;
                    }
                }
            }
        }
        if !said_why {
            hint_once(DEAD_AI_SESSION);
        }
    }

    // 2. The person who signed in at this terminal. Asked before git
    //    config on purpose (operator, 2026-09-27): a passphrase this
    //    member typed outranks an address a config file carries, which
    //    is how somebody works with no git identity at all, and how a
    //    second person acts at a checkout whose config names the first.
    //    Never for a process that carries JOY_SESSION (see step 1).
    if let Some(p) = project.as_ref().filter(|_| !carries_ai_session) {
        if let Some(member) = member_key_from_session(p) {
            return Ok(Identity {
                member: member.into(),
                delegated_by: None,
                authenticated: true,
            });
        }
    }

    // 3 and 4. git config, then the forge account, tried only now: a
    // session already returned above, so this never reads a config file
    // or spawns a plugin for a command a session already answered.
    // `member_key_from_git_config` is one git2 read that answers the
    // repository and the global step together (see the module doc);
    // `member_key_from_forge_account` is asked only when that read names
    // nobody the project knows.
    let member_key = project
        .as_ref()
        .and_then(|p| {
            member_key_from_git_config(root, p).or_else(|| member_key_from_forge_account(root, p))
        })
        .unwrap_or_default();

    // 5. Fallback: the resolved key, not authenticated (a live session
    //    for it would have answered at step 2). Empty when nothing
    //    answered at all: nobody signed in here, no git config names a
    //    member and no forge account does either, which is a fresh
    //    clone, a second machine, or an address the project does not
    //    (yet) know. The callers that need a name say so (see the note
    //    on this function); a read-only command keeps working with no
    //    member.
    Ok(Identity {
        member: member_key.into(),
        delegated_by: None,
        authenticated: false,
    })
}

/// Step 2 of [`resolve_identity`]'s order: the person who signed in at
/// this terminal, without knowing their name in advance.
///
/// Every session file of this project is a candidate; one counts when it
/// names a human (an AI authenticates through `JOY_SESSION` alone,
/// ADR-033), has not expired, was made at this terminal (human sessions
/// are bound to the terminal they were made at; `None == None` for CI,
/// test harnesses and AI tools), names a member the project knows, and
/// carries that member's own signature. A file anybody with write access
/// to the state directory could author therefore names nobody: the
/// signature is over the claims, and only the passphrase makes it.
///
/// The newest such session wins when a terminal holds more than one,
/// which is what the last `joy auth --user <address>` typed here meant.
fn member_key_from_session(project: &Project) -> Option<String> {
    let project_id = crate::auth::session::project_id_of(project);
    let tty = crate::auth::session::current_tty();
    let now = chrono::Utc::now();
    let sessions = crate::auth::session::list_project_sessions(&project_id).ok()?;
    // A session of this terminal that ran out is said ONCE and then
    // removed, so the person learns why the next command asks again (or
    // why they act unproven) instead of meeting a silent change.
    for ended in sessions.iter().filter(|sess| {
        !is_ai_member(&sess.claims.member) && sess.claims.expires <= now && sess.claims.tty == tty
    }) {
        hint_once(SESSION_ENDED);
        let _ = crate::auth::session::remove_session(&project_id, &ended.claims.member);
    }
    sessions
        .into_iter()
        .filter(|sess| !is_ai_member(&sess.claims.member))
        .filter(|sess| sess.claims.expires > now && sess.claims.tty == tty)
        .find(|sess| {
            let Some(member) = project.member_by_key(&sess.claims.member) else {
                return false;
            };
            let Some(pk_hex) = member.verify_key.as_ref() else {
                return false;
            };
            let Ok(pk) = crate::auth::PublicKey::from_hex(pk_hex) else {
                return false;
            };
            crate::auth::session::validate_session(sess, &pk, &project_id).is_ok()
        })
        .map(|sess| sess.claims.member)
}

/// Steps 3 and 4 of [`resolve_identity`]'s order: the repository's own
/// `user.email`, then the person's global one. `git2`'s own config
/// layering already tries local before global before system when asked
/// through the repository's `Config` ([`crate::vcs::forge::user_identity`]
/// reads it exactly that way), so ONE read answers both steps: a value
/// only the global file carries is found the moment the local file has
/// none, and there is no separate "global only" reader to keep in step
/// with git's own precedence.
fn member_key_from_git_config(root: &Path, project: &Project) -> Option<String> {
    let (_, email) = crate::vcs::forge::user_identity(root);
    crate::privacy::member_key_for_email(project, &email?)
}

/// Step 5 of [`resolve_identity`]'s order, and the last one: the account
/// the forge tool for the current remote's host reports, matched by an
/// address its own API vouches for. Asked only when no git config named
/// a member, because a forge login is a credential to ONE host, never a
/// project wide identity (operator decision 2026-09-19, JOY-02AE-1A).
///
/// `None` on every one of: no remote configured, no installed connector
/// claims the remote's host, the connector answers `known: false`
/// (nobody signed in there), or none of the addresses it vouches for
/// (`ForgeIdentity::emails`) is a member, nor a member key the account
/// owns (a project founded under an alias). Both directions are the
/// login path's, [`crate::privacy::member_key_for_email_or_forge`] and
/// [`crate::privacy::member_key_owned_by`], never a copy here: joy-core
/// holds no forge knowledge of its own, so "which plugin answers for
/// this remote" and "who is signed in there" stay the plugin's
/// judgement.
fn member_key_from_forge_account(root: &Path, project: &Project) -> Option<String> {
    // With an address in git config that named no member, the login
    // path's function does the whole job: direction one (the account's
    // own addresses) and direction two (a project keyed by an alias the
    // account owns), with its cache. The resolver used to know direction
    // one only, so a member `joy auth` had just found was unknown to the
    // very next command (JOY-02AE-1A).
    let (_, email) = crate::vcs::forge::user_identity(root);
    if let Some(email) = email.filter(|e| !e.trim().is_empty()) {
        return crate::privacy::member_key_for_email_or_forge(project, root, &email, None);
    }
    // No git config at all: the same two directions, without an address
    // to cache under.
    let remotes = crate::vcs::default_vcs()
        .all_remotes(root)
        .unwrap_or_default();
    let ctx = crate::forge_plugins::CallContext::in_project(root);
    let spec = crate::forge_plugins::responsible_plugin(project.forge.as_deref(), &ctx, &remotes)?;
    let acting = crate::forge_plugins::identity(spec, None, &ctx)?;
    crate::privacy::member_key_for_any(project, &acting.emails)
        .or_else(|| crate::privacy::member_key_owned_by(project, spec, &ctx, &acting))
}

/// What an AI reads when the session in `JOY_SESSION` no longer stands
/// and nothing more specific was said: it expired, its file is gone, or
/// the value is not a session at all.
const DEAD_AI_SESSION: &str =
    "JOY_SESSION is no longer valid: redeem a fresh token (joy auth --token <TOKEN>)";

/// What a person reads once when the session they made at this terminal
/// has run out.
const SESSION_ENDED: &str = "Your joy session here has ended: sign in again with `joy auth`";

/// Print one identity hint to stderr, at most once per process.
///
/// [`resolve_identity`] is asked more than once by some commands (the
/// guard asks, and then the command asks for the acting member), and a
/// warning that a person has already read is noise the second time: the
/// same `JOY_SESSION` fact would be printed twice by `joy add`. The set
/// is per process and tiny, because the number of distinct hints is.
fn hint_once(hint: &str) {
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    static PRINTED: Mutex<Option<BTreeSet<String>>> = Mutex::new(None);
    let Ok(mut printed) = PRINTED.lock() else {
        eprintln!("{hint}");
        return;
    };
    if printed
        .get_or_insert_with(BTreeSet::new)
        .insert(hint.to_string())
    {
        eprintln!("{hint}");
    }
}

/// The at-rest member key the current command acts as (operator decision
/// 2026-09-19, JOY-02AE-1A). Every
/// joy-cli command that needs to know who is acting asks this and
/// nothing else, so the order lives in one place: [`resolve_identity`]
/// reads the delegation session first, then the session of this
/// terminal, then git config, then the forge account, and nothing after
/// that.
///
/// The answer is a key of the member map, never an address: an anonymous
/// project (ADR-042) answers with its opaque `m-<hex>` id, so a caller
/// that puts the answer into `project.yaml`, a session file or a commit
/// trailer writes no cleartext address there by accident.
///
/// [`JoyError::UnknownActingMember`] when nothing answers at all: no
/// name on the call, no session of either kind, no git config naming a
/// member, and no forge account naming one either, which is a fresh
/// clone or a second machine where nobody signed in yet. The auth
/// commands ask [`acting_member`] instead, which never reads a session.
///
/// This answers with the AI member under a delegation session, because
/// that is who is acting. A command that needs the human BEHIND the
/// action, which is every command that unwraps a passphrase identity,
/// asks [`acting_human_key`].
pub fn acting_member_key(root: &Path) -> Result<String, JoyError> {
    let member = resolve_identity(root)?.member.id().to_string();
    if member.trim().is_empty() {
        return Err(JoyError::UnknownActingMember);
    }
    Ok(member)
}

/// The at-rest member key of the HUMAN the current command acts for
///: the same answer as [`acting_member_key`] for a
/// person at a terminal, and the delegating operator under a delegation
/// session, never the AI.
///
/// This is the question every command asks that needs a passphrase, a
/// seed or a wrap: an AI member has no `kdf_nonce` and no
/// `seed_wrap_passphrase` and can never have one, so resolving the AI
/// there would refuse work the operator is entitled to do. The operator
/// is not guessed: it is the `delegated_by` claim the session was signed
/// with at redemption, so it names the person who issued the token and
/// nobody else.
///
/// [`JoyError::UnknownActingMember`] when nothing answers, and the same
/// error when a session names an AI without an operator, because that
/// session cannot say whose passphrase to ask for.
pub fn acting_human_key(root: &Path) -> Result<String, JoyError> {
    let identity = resolve_identity(root)?;
    let member = identity.member.id().to_string();
    if !is_ai_member(&member) {
        if member.trim().is_empty() {
            return Err(JoyError::UnknownActingMember);
        }
        return Ok(member);
    }
    let operator = identity
        .delegated_by
        .map(|human| human.id().to_string())
        .filter(|human| !human.trim().is_empty())
        .ok_or(JoyError::UnknownActingMember)?;
    // The claim carries whichever identifier the issuer held: the at-rest
    // key in anonymous mode and in every app-issued token, an address
    // when a person typed one (`joy auth token add --user`). Answer with
    // the key in both cases, so the caller's lookups are all by key.
    let key = load_project_optional(root).and_then(|project| {
        project
            .has_member_key(&operator)
            .then(|| operator.clone())
            .or_else(|| crate::privacy::member_key_for_email(&project, &operator))
    });
    Ok(key.unwrap_or(operator))
}

/// The name given to this very call: the global `--user <address>`,
/// which the command line carries as `JOY_USER` (operator, 2026-09-27).
/// Empty and whitespace count as absent.
pub fn named_user() -> Option<String> {
    std::env::var("JOY_USER")
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// The address joy OFFERS a person when it asks them who they are: git
/// config `user.email`, empty treated as absent.
///
/// This reads the CURRENT DIRECTORY's config, not `root`'s, because it
/// serves interactive prompts that already run at the project root; it
/// is a prefill, and the person at the terminal can type something else
/// instead. [`resolve_identity`] does not call this: since the operator's
/// 2026-09-19 correction (JOY-02AE-1A) git config IS a source there
/// again, read directly and root-scoped through
/// [`crate::vcs::forge::user_identity`], not through this offer.
pub fn git_config_prefill() -> Option<String> {
    crate::vcs::default_vcs()
        .user_email()
        .ok()
        .map(|email| email.trim().to_string())
        .filter(|email| !email.is_empty())
}

/// The member a bare `joy auth`, `joy auth init` or `joy auth --otp`
/// acts as before anybody typed a name: an explicit name first (`named`,
/// else the call's own `--user`, see [`named_user`]), then this
/// repository's own git config (local before global), then the account
/// the forge tool for the remote's host reports, and the typed error
/// when none of the three answers (operator decision 2026-09-19,
/// JOY-02AE-1A, completed in a later addition to the
/// same item).
///
/// Unlike [`resolve_identity`] this never reads the session: `joy auth`
/// is the command that MAKES the session, so a bare `joy auth` signs in
/// the person git config names and replaces whoever was signed in
/// before (operator, 2026-09-27), instead of renewing them.
///
/// This still differs from [`resolve_identity`] in what it hands back:
/// that function checks each candidate against `project.yaml` before
/// answering, because it decides who IS acting; this one hands back the
/// raw candidate, unchecked, because its callers ask the question
/// earlier, before a member necessarily exists to check against (`joy
/// auth init --user <address>` the very first time), or do their own
/// check right after (every caller reports "X is not a registered
/// project member" by name), or do a fuller check on top of this one
/// (`joy auth`'s passphrase path also tries the forge-alias fallback of
/// [`crate::privacy::member_key_for_email_or_forge`], which is how a
/// forge alias sitting in git config still finds its member).
///
/// The project is never guessed from (for instance "it has only one
/// human"): the project file travels with every clone, so a guess would
/// let anyone who clones an unenrolled project claim the member it
/// guesses.
pub fn acting_member(
    root: &Path,
    project: &Project,
    named: Option<&str>,
) -> Result<String, JoyError> {
    if let Some(named) = named
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(named_user)
    {
        return Ok(named);
    }
    let (_, config_email) = crate::vcs::forge::user_identity(root);
    if let Some(email) = config_email.filter(|s| !s.trim().is_empty()) {
        return Ok(email);
    }
    forge_account_candidate(root, project).ok_or(JoyError::UnknownActingMember)
}

/// The last of [`acting_member`]'s three sources: an address the account
/// the forge tool for the remote's host reports vouches for, not yet
/// checked against `project.yaml` (the caller does that). This asks the
/// same plugin the same question [`member_key_from_forge_account`] does
/// for [`resolve_identity`], stopped one step short of that function's
/// own validation, because `acting_member`'s callers are not all sure a
/// member exists yet either.
fn forge_account_candidate(root: &Path, project: &Project) -> Option<String> {
    let remotes = crate::vcs::default_vcs()
        .all_remotes(root)
        .unwrap_or_default();
    let ctx = crate::forge_plugins::CallContext::in_project(root);
    let spec = crate::forge_plugins::responsible_plugin(project.forge.as_deref(), &ctx, &remotes)?;
    let acting = crate::forge_plugins::identity(spec, None, &ctx)?;
    acting.emails.first().cloned()
}

/// The signature a commit of `member` carries in THIS checkout.
/// git config is consulted for the display name only, and only when that
/// name maps to this very member; everything else comes from the member
/// id.
///
/// `member` may be an address rather than a member key: a host that names
/// the member itself holds one (`--user`, the desktop's mask, enrolment).
/// The project decides what is signed, so an address in an anonymous
/// project is signed as its opaque id and never as itself (ADR-042).
pub fn commit_signature(root: &Path, member: &str) -> Result<(String, String), JoyError> {
    let project = load_project_optional(root);
    // The at-rest key of the acting member, so the name check below
    // compares like with like whatever the caller was holding.
    let key = project.as_ref().and_then(|p| {
        p.member_by_key(member)
            .is_some()
            .then(|| member.to_string())
            .or_else(|| crate::privacy::member_key_for_email(p, member))
    });
    let (config_name, config_email) = crate::vcs::forge::user_identity(root);
    let config_name = config_name.filter(|_| {
        config_name_belongs_to(
            project.as_ref(),
            key.as_deref(),
            member,
            config_email.as_deref(),
        )
    });
    crate::vcs::forge::member_signature(project.as_ref(), member, config_name.as_deref())
}

/// Whether this checkout's `user.name` may stand in as the display name
/// of `member` ("name is git config `user.name` when it maps to the
/// acting member, else the member id").
///
/// The one way a name maps to a member: a `user.email` in the SAME git
/// config resolves to it. A checkout configured for somebody else does
/// not lend its display name to my commit, and an address the project
/// cannot place lends nothing either. A config carrying `user.name` with
/// NO `user.email` names nobody at all: the device pin that used to
/// answer this case is gone (JOY-02AE-1A) and nothing replaces it, so
/// the name in the same git config the member came from may be
/// borrowed, and that is the whole rule now.
fn config_name_belongs_to(
    project: Option<&Project>,
    key: Option<&str>,
    member: &str,
    config_email: Option<&str>,
) -> bool {
    match config_email {
        Some(email) => match (project, key) {
            (Some(p), Some(key)) => {
                crate::privacy::member_key_for_email(p, email).as_deref() == Some(key)
            }
            _ => email == member,
        },
        None => false,
    }
}

/// The two signature fields the member acting in `root` commits with
///, for a caller that holds no identity of its own.
///
/// This is what the git binary used to take from `user.name` and
/// `user.email` when joy shelled `git commit`. libgit2 asks for the
/// signature instead, and joy knows who acts, so the answer is joy's
/// own identity resolution: [`resolve_identity`]'s order (the
/// delegation session, then git config, then the forge account, since
/// the operator's 2026-09-19 correction, JOY-02AE-1A). A project founded
/// without a git config is still committable through `--user`, a
/// delegation session or a forge login, which it was not while a git
/// process wrote the commit and demanded `user.email` itself.
pub fn acting_signature(root: &Path) -> Result<(String, String), JoyError> {
    let member = resolve_identity(root)?.member.id().to_string();
    commit_signature(root, &member)
}

/// Check whether the project has any AI members.
pub fn has_ai_members(root: &Path) -> bool {
    let project = load_project_optional(root);
    match project {
        Some(p) => p.member_keys().any(|k| is_ai_member(k)),
        None => false,
    }
}

/// Reject a token-redeemed AI session that no longer traces to a live
/// delegation (F3, JI-0175-B0). Returns `Some(reason)` to reject, `None`
/// to accept.
///
/// A token-redeemed session records the delegation_verifier it was bound
/// to in `claims.delegation_key`. Three checks:
///
/// 1. the claim must be present at all: every AI session is redeemed
///    from a delegation token and carries the binding, so a session
///    without one was written by something that must not mint sessions.
/// 2. some member's `ai_delegations[<ai>]` must still carry that verifier.
///    Rotating the delegation (`joy auth delegation rotate`) or removing
///    the delegating member changes or drops it, so a revoked session
///    dies at the next command, not only at its TTL.
/// 3. when the session carries the delegation private key in its
///    `JOY_SESSION` env (crypt scope), that key must derive the verifier.
///    This proves possession of the delegation key: a session file alone
///    — which anyone able to write the state dir could author for any
///    registered AI member — is no longer enough.
pub fn token_session_rejection(
    project: &Project,
    sess: &crate::auth::session::SessionToken,
    delegation_private: Option<&[u8; 32]>,
) -> Option<String> {
    let Some(verifier) = sess.claims.delegation_key.as_ref() else {
        return Some(format!(
            "the session for {} carries no delegation binding; redeem a fresh token              (joy auth --token <TOKEN>)",
            sess.claims.member
        ));
    };
    // F2: redemption records WHO delegates in the signed claims. A
    // session without it cannot name the person behind the AI, so it is
    // not honored either.
    if sess.claims.delegated_by.is_none() {
        return Some(format!(
            "the session for {} names no delegating operator; redeem a fresh token",
            sess.claims.member
        ));
    }
    let registered = project.members().any(|(_, m)| {
        m.ai_delegations
            .get(&sess.claims.member)
            .is_some_and(|d| &d.delegation_verifier == verifier)
    });
    if !registered {
        return Some(format!(
            "the delegation for {} was rotated or removed; this session is no longer valid \
             (ask the operator for a fresh token)",
            sess.claims.member
        ));
    }
    if let Some(seed) = delegation_private {
        let derived = crate::auth::IdentityKeypair::from_seed(seed)
            .public_key()
            .to_hex();
        if &derived != verifier {
            return Some(format!(
                "the session's delegation key does not match the registered delegation for {}",
                sess.claims.member
            ));
        }
    }
    None
}

/// Build the cross-project JOY_SESSION warning text.
///
/// Extracted as a pure helper so it can be asserted directly in unit
/// tests without touching stderr capture or environment mutation.
fn cross_project_session_warning(
    session_project: &str,
    session_member: &str,
    current_project: &str,
) -> String {
    format!(
        "Warning: JOY_SESSION belongs to project {session_project} \
         (member {session_member}), but the current project is {current_project}. \
         Ask the human for a delegation in this project: \
         joy auth token add {session_member}"
    )
}

/// Verify that the private key bytes from JOY_SESSION derive to the public
/// key recorded in the session claims. This is the core proof-of-possession
/// check for AI sessions under ADR-033.
fn ephemeral_public_matches(
    sess: &crate::auth::session::SessionToken,
    ephemeral_private: &[u8; 32],
) -> bool {
    let Some(ref stored_pk_hex) = sess.claims.session_public_key else {
        return false;
    };
    let kp = crate::auth::IdentityKeypair::from_seed(ephemeral_private);
    kp.public_key().to_hex() == *stored_pk_hex
}

fn load_project_optional(root: &Path) -> Option<Project> {
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    store::read_project(&project_path).ok()
}

#[allow(dead_code)]
fn validate_member(member: &str, project: &Option<Project>) -> Result<(), JoyError> {
    let Some(project) = project else {
        return Ok(());
    };
    if !project.has_members() {
        return Ok(());
    }
    if !project.has_member_key(member) {
        return Err(JoyError::Other(format!(
            "'{}' is not a registered project member. \
             Use `joy member add {}` to register.",
            member, member
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_log_user_simple() {
        let id = Identity {
            member: "alice@example.com".into(),
            delegated_by: None,
            authenticated: false,
        };
        assert_eq!(id.log_user(), "alice@example.com");
    }

    #[test]
    fn identity_log_user_delegated() {
        let id = Identity {
            member: "ai:claude@joy".into(),
            delegated_by: Some("horst@joydev.com".into()),
            authenticated: false,
        };
        assert_eq!(id.log_user(), "ai:claude@joy delegated-by:horst@joydev.com");
    }

    #[test]
    fn cross_project_warning_names_session_and_current_projects() {
        let msg = cross_project_session_warning("JOY", "ai:claude@joy", "JI");
        assert!(msg.contains("belongs to project JOY"));
        assert!(msg.contains("member ai:claude@joy"));
        assert!(msg.contains("current project is JI"));
        assert!(msg.contains("joy auth token add ai:claude@joy"));
    }
}
