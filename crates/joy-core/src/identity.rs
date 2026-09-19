// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Identity resolution for Joy CLI operations.
//!
//! Resolves the acting user's identity from, in this order (operator
//! decision 2026-09-19, item JOY-02AE-1A, correcting D3.9 of
//! docs/design/forge-connection-ng.md):
//! 1. The delegation session in `JOY_SESSION` (unchanged: an AI's
//!    identity has never come from anywhere else).
//! 2. `git config user.email` of the repository (`--local`).
//! 3. `git config user.email` global. One git2 read answers both: the
//!    config git2 opens through the repository already tries local
//!    before global before system (see
//!    [`crate::vcs::forge::user_identity`]), so a value only the global
//!    file carries is found the moment the local file has none.
//! 4. The account the forge tool for the remote's host reports (`gh`,
//!    `glab`, `tea`), matched by an address its own API vouches for.
//!
//! That is the whole list. Steps 2 and 3 are held against `project.yaml`
//! through [`crate::privacy::member_key_for_email`]; step 4 through
//! [`crate::privacy::member_key_for_any`], because a forge account can
//! vouch for more than one address. A match is the member.
//!
//! The device pin that used to stand at step 2 before this correction
//! ([`MEMBER_PIN_KEY`], [`pinned_member`]) is retired from here for
//! good: it is never read to decide an identity again. Its writers
//! ([`pin_acting_member`]) stay in place for the other things a pin is
//! still used for on this device, but nothing in this function reads one.
//!
//! A machine that answers none of the four, a fresh clone with no git
//! config and no forge login, or a checkout whose git config and forge
//! account both name nobody the project knows, is told so instead of
//! being guessed at: [`acting_member_key`] and [`acting_human_key`]
//! answer [`JoyError::UnknownActingMember`], whose text names the git
//! config the person can set. A read-only command keeps working with an
//! EMPTY member, exactly as it did before this correction.
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
/// Priority (operator decision 2026-09-19, item JOY-02AE-1A, correcting
/// D3.9 of the forge connection NG design): the delegation session
/// first, then git config, then the forge account, and nothing else.
/// 1. JOY_SESSION -- ephemeral-key-bound AI session handle (ADR-033)
/// 2. The member key: `git config user.email`, repository then global
///    (one read; see [`member_key_from_git_config`])
/// 3. Failing that, the forge account for the remote's host (see
///    [`member_key_from_forge_account`])
/// 4. A human session for that member
/// 5. Fallback: the same key, unauthenticated
///
/// (Steps 2 and 3 are the module doc's steps 2 to 4; they are numbered
/// together here because they fill the same variable below.)
///
/// A machine whose git config and forge account both name nobody the
/// project knows answers with an EMPTY member, and the callers that need
/// a name ([`acting_member_key`], [`acting_human_key`]) turn that into
/// [`JoyError::UnknownActingMember`]; a read-only command keeps working
/// with no member, exactly as it does on a machine with no git identity
/// at all. The prefill a person is OFFERED lives in
/// [`git_config_prefill`], which only `--user`-shaped paths call.
pub fn resolve_identity(root: &Path) -> Result<Identity, JoyError> {
    let project = load_project_optional(root);
    let project_id = crate::auth::session::project_id(root).ok();

    // 1. JOY_SESSION: env var carries the ephemeral private key bound to
    //    the session (ADR-033). We derive the public key from it and match
    //    against `session_public_key` stored in the session file. Without
    //    possession of the env var a sibling terminal cannot reuse a
    //    session file it can read.
    if let Some(env_value) = std::env::var("JOY_SESSION").ok().filter(|s| !s.is_empty()) {
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
                        // pinned identity would confuse the caller when
                        // the subsequent guard denial names the human
                        // instead of the AI they thought they were acting
                        // as. Emit a one-line stderr hint and continue
                        // with the fallback so read-only commands still
                        // work.
                        hint_once(&cross_project_session_warning(
                            &sess.claims.project_id,
                            &sess.claims.member,
                            current_pid,
                        ));
                    }
                }
            }
        }
    }

    // 2 and 3. git config, then the forge account, tried only now: a
    // delegated AI session already returned above, so this never reads a
    // config file or spawns a plugin for a command JOY_SESSION already
    // answered. `member_key_from_git_config` is one git2 read that
    // answers the repository and the global step together (see the
    // module doc); `member_key_from_forge_account` is asked only when
    // that read names nobody the project knows.
    let member_key = project
        .as_ref()
        .and_then(|p| {
            member_key_from_git_config(root, p).or_else(|| member_key_from_forge_account(root, p))
        })
        .unwrap_or_default();

    // 4. The human session of the member the key names: a passphrase
    //    session (`joy auth --user <address>`) turns the resolved address
    //    into an authenticated identity rather than a merely plausible
    //    one.
    if let Some(session_identity) = session_identity(root, &member_key, &project) {
        return Ok(session_identity);
    }

    // 5. Fallback: the resolved key, not authenticated. Empty when
    //    nothing answered at all: no git config names a member and no
    //    forge account does either, which is a fresh clone, a second
    //    machine, or an address the project does not (yet) know. The
    //    callers that need a name say so (see the note on this
    //    function); a read-only command keeps working with no member.
    Ok(Identity {
        member: member_key.into(),
        delegated_by: None,
        authenticated: false,
    })
}

/// Steps 2 and 3 of [`resolve_identity`]'s order: the repository's own
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

/// Step 4 of [`resolve_identity`]'s order, and the last one: the account
/// the forge tool for the current remote's host reports, matched by an
/// address its own API vouches for. Asked only when no git config named
/// a member, because a forge login is a credential to ONE host, never a
/// project wide identity (operator decision 2026-09-19, JOY-02AE-1A).
///
/// `None` on every one of: no remote configured, no installed connector
/// claims the remote's host, the connector answers `known: false`
/// (nobody signed in there), or none of the addresses it vouches for
/// (`ForgeIdentity::emails`) is a member. This reuses
/// [`crate::forge_plugins::responsible_plugin`] and
/// [`crate::forge_plugins::identity`] exactly as
/// [`crate::privacy::member_key_for_email_or_forge`] does, rather than
/// duplicating them: joy-core holds no forge knowledge of its own, so
/// "which plugin answers for this remote" and "who is signed in there"
/// stay the plugin's judgement, never this function's.
fn member_key_from_forge_account(root: &Path, project: &Project) -> Option<String> {
    let remotes = crate::vcs::default_vcs()
        .all_remotes(root)
        .unwrap_or_default();
    let ctx = crate::forge_plugins::CallContext::in_project(root);
    let spec = crate::forge_plugins::responsible_plugin(project.forge.as_deref(), &ctx, &remotes)?;
    let acting = crate::forge_plugins::identity(spec, None, &ctx)?;
    crate::privacy::member_key_for_any(project, &acting.emails)
}

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

/// Try to build an Identity from an active session for a member.
fn session_identity(root: &Path, member: &str, project: &Option<Project>) -> Option<Identity> {
    if !check_session(root, member, project) {
        return None;
    }

    // A human member, always: [`check_session`] accepts no AI on a session
    // file alone (ADR-033), so this path never carries a delegation and
    // has nobody to name as the operator behind it. The AI branch of
    // `resolve_identity` above reads its operator out of the SIGNED
    // session claims instead.
    Some(Identity {
        member: member.into(),
        delegated_by: None,
        authenticated: true,
    })
}

/// The member this device acts as in this project, as
/// [`pin_acting_member`] wrote it. `None` when nothing is pinned or the
/// pinned member is no longer a member of the project (removed, or the
/// project was rekeyed to anonymous mode).
///
/// [`resolve_identity`] does not call this any more (operator decision
/// 2026-09-19, JOY-02AE-1A, correcting D3.9): the pin is never read to
/// decide who is acting. What is left of it is the enrolment-time
/// question [`acting_member`] still asks ("who does a bare `joy auth`
/// authenticate as, before a git config or an explicit `--user` names
/// somebody"), and the display line in `joy auth status` that names the
/// pin as the reason a member was already known before this correction.
pub fn pinned_member(root: &Path, project: &Project) -> Option<String> {
    let pin = read_member_pin(root)?;
    project.member_by_key(&pin).is_some().then_some(pin)
}

/// The raw pin, whether or not it still names a member.
fn read_member_pin(root: &Path) -> Option<String> {
    let path = crate::auth::session::app_state_project_file(root).ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let state: serde_json::Value = serde_json::from_str(&text).ok()?;
    state
        .get(MEMBER_PIN_KEY)?
        .as_str()
        .map(str::to_string)
        .filter(|pin| !pin.is_empty())
}

/// The key of the pin inside the per-project app state file (ADR
/// JAPP-02BD-56). The file is the person's own device state, never the
/// repository: a pin is one person's choice on one machine and must not
/// travel to the team through a committed file.
const MEMBER_PIN_KEY: &str = "member";

/// Remember `member` as the one this device acts as in this project.
/// Called at the three moments where a person says who they are on this
/// machine: founding the project, enrolling in it, and authenticating in
/// it.
///
/// As of the operator's 2026-09-19 correction (JOY-02AE-1A) this write is
/// no longer what makes an identity answer: [`resolve_identity`] reads
/// git config and the forge account instead, and never this pin. The
/// write stays because [`acting_member`] still reads it for the
/// enrolment-time question ("who does a bare `joy auth` authenticate
/// as"), and because `joy auth status` names it when it happens to be
/// the reason a member was already known. A future cleanup may remove it
/// once nothing reads it either; until then it costs nothing to keep.
///
/// `project` is the project the member belongs to; a pin is only worth
/// keeping for a member it actually knows.
///
/// Best effort: a state directory that cannot be written costs a pin, not
/// the enrolment that just succeeded.
pub fn pin_acting_member(root: &Path, project: &Project, member: &str) {
    if project.member_by_key(member).is_none() {
        return;
    }
    if let Err(e) = set_member_pin(root, member) {
        eprintln!("Warning: could not remember the acting member on this device: {e}");
    }
}

/// Write the pin into the per-project app state file, keeping every other
/// key in it (the forge login of D4.1c lives in the same object).
fn set_member_pin(root: &Path, member: &str) -> Result<(), JoyError> {
    let path = crate::auth::session::app_state_project_file(root)?;
    let mut state: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !state.is_object() {
        state = serde_json::json!({});
    }
    state[MEMBER_PIN_KEY] = serde_json::Value::String(member.to_string());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| JoyError::CreateDir {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let text = serde_json::to_string_pretty(&state)
        .map_err(|e| JoyError::AuthFailed(format!("cannot write the member pin: {e}")))?;
    std::fs::write(&path, text).map_err(|e| JoyError::WriteFile {
        path: path.clone(),
        source: e,
    })
}

/// The at-rest member key the current command acts as (operator decision
/// 2026-09-19, JOY-02AE-1A, correcting D3.9 of package J11). Every
/// joy-cli command that needs to know who is acting asks this and
/// nothing else, so the order lives in one place: [`resolve_identity`]
/// reads the delegation session first, then git config, then the forge
/// account, and nothing after that.
///
/// The answer is a key of the member map, never an address: an anonymous
/// project (ADR-042) answers with its opaque `m-<hex>` id, so a caller
/// that puts the answer into `project.yaml`, a session file or a commit
/// trailer writes no cleartext address there by accident.
///
/// [`JoyError::UnknownActingMember`] when nothing answers at all: no
/// session, no git config naming a member, and no forge account naming
/// one either, which is a fresh clone or a second machine with neither
/// configured. A command that lets a person name somebody (`--user`)
/// asks [`acting_member`] instead, which takes that name first and
/// offers the git config as a prefill.
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
/// (D3.9, package J11): the same answer as [`acting_member_key`] for a
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

/// The member a local enrolment or authentication acts as, resolved
/// WITHOUT demanding a git config (D3.9): the address the host named
/// (`--user`, the app's mask), then the member pinned on this device,
/// then git config as a prefill. The typed error when nothing answers.
///
/// This is a different question from [`resolve_identity`]'s and the
/// operator's 2026-09-19 correction (JOY-02AE-1A) leaves it as it was:
/// this answers "who does a bare `joy auth` or enrolment act as before
/// anybody typed a name", where offering the pin first and the git
/// config only as a prefill is still right, because the person is being
/// ASKED and can correct either.
///
/// These three are the whole list. Guessing a member from the project
/// (for instance "it has only one human") is deliberately not in it: the
/// project file travels with every clone, so a guess would let anyone who
/// clones a project claim the member it guesses, while the pin is this
/// device's own state and `--user` is a person speaking.
///
/// The git config address is returned raw, not resolved to a member key,
/// so every existing "X is not a registered project member" text keeps
/// naming what the person configured.
pub fn acting_member(
    root: &Path,
    project: &Project,
    named: Option<&str>,
) -> Result<String, JoyError> {
    if let Some(named) = named.map(str::trim).filter(|n| !n.is_empty()) {
        return Ok(named.to_string());
    }
    if let Some(pin) = pinned_member(root, project) {
        return Ok(pin);
    }
    git_config_prefill().ok_or(JoyError::UnknownActingMember)
}

/// The signature a commit of `member` carries in THIS checkout (D4.5).
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
            root,
            project.as_ref(),
            key.as_deref(),
            member,
            config_email.as_deref(),
        )
    });
    crate::vcs::forge::member_signature(project.as_ref(), member, config_name.as_deref())
}

/// Whether this checkout's `user.name` may stand in as the display name
/// of `member` (D4.5: "name is git config `user.name` when it maps to the
/// acting member, else the member id").
///
/// Two ways a name maps to a member, and the config decides which
/// question is asked:
///
/// - With a `user.email`, that address answers it. A checkout configured
///   for somebody else does not lend its display name to my commit, and
///   an address the project cannot place lends nothing either.
/// - With NO `user.email`, the config names nobody, and the member this
///   device pinned answers instead (D3.9). This is the half of J11's
///   acceptance that the email branch alone would break: removing ONLY
///   `user.email` and keeping `user.name` used to drop the author's name
///   from every commit joy writes, because the name was gated on the
///   address. Now it does not, and the author line is byte identical
///   before and after the removal.
///
///   This pin read is untouched by the operator's 2026-09-19 correction
///   (JOY-02AE-1A): that correction is about [`resolve_identity`]
///   deciding WHICH member acts, and this function answers a narrower,
///   later question about a member already decided, whether ITS commit
///   may also borrow this checkout's display name.
fn config_name_belongs_to(
    root: &Path,
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
        None => match (project, key) {
            (Some(project), Some(key)) => pinned_member(root, project).as_deref() == Some(key),
            _ => false,
        },
    }
}

/// The two signature fields the member acting in `root` commits with
/// (D4.5), for a caller that holds no identity of its own.
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

/// Check if the member has an active, valid session.
fn check_session(root: &Path, member: &str, project: &Option<Project>) -> bool {
    let Some(project) = project else {
        return false;
    };
    if !project.has_member_key(member) {
        return false;
    };
    let Ok(project_id) = crate::auth::session::project_id(root) else {
        return false;
    };
    let Ok(Some(sess)) = crate::auth::session::load_session(&project_id, member) else {
        return false;
    };

    // Check expiry and member match
    if sess.claims.expires <= chrono::Utc::now() || sess.claims.member != member {
        return false;
    }

    // For human members: validate session signature against public key + TTY binding
    if !is_ai_member(member) {
        let m = project.member_by_key(member).unwrap();
        let Some(ref pk_hex) = m.verify_key else {
            return false;
        };
        let Ok(pk) = crate::auth::PublicKey::from_hex(pk_hex) else {
            return false;
        };
        if crate::auth::session::validate_session(&sess, &pk, &project_id).is_err() {
            return false;
        }
        // TTY binding: session must come from the same terminal context.
        // Both session TTY and current TTY must match (including None == None
        // for non-interactive contexts like CI, test harnesses, or AI tools).
        let current_tty = crate::auth::session::current_tty();
        if sess.claims.tty != current_tty {
            return false;
        }
        return true;
    }

    // For AI members: under ADR-033 the only valid authentication path is
    // the JOY_SESSION env var matched to the ephemeral public key. A
    // session file on its own no longer authenticates anyone.
    false
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
