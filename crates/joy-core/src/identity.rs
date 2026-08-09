// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Identity resolution for Joy CLI operations.
//!
//! Resolves the acting user's identity from:
//! 1. Active session (if one exists for any member)
//! 2. `git config user.email` (fallback for projects without auth)
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
/// Priority:
/// 1. JOY_SESSION -- ephemeral-key-bound AI session handle (ADR-033)
/// 2. Human session by git email
/// 3. Fallback: git email, unauthenticated
pub fn resolve_identity(root: &Path) -> Result<Identity, JoyError> {
    // An AI identity comes entirely from JOY_SESSION and does not need a
    // git e-mail; a missing `user.email` must not abort before that branch
    // runs (JI-0175-B0). A bare checkout or a container without a git user
    // then still authenticates a valid session. The e-mail is only needed
    // for the human/fallback path below, where its absence degrades to an
    // unauthenticated identity (the guard denies writes with a clear
    // message) rather than hard-failing every command.
    let git_email = crate::vcs::default_vcs().user_email().unwrap_or_default();
    let project = load_project_optional(root);
    let project_id = crate::auth::session::project_id(root).ok();

    // In anonymous mode (ADR-042) the member map is keyed by an opaque id, not
    // the git e-mail; resolve the e-mail to that id so membership checks,
    // sessions and the audit actor all use the same key. A miss consults
    // the project's forge plugin (JOY-0253-8A): a forge alias address in
    // the git config then still resolves to its member. In open mode (or
    // when nothing resolves) this is just the e-mail.
    let member_key = project
        .as_ref()
        .and_then(|p| crate::privacy::member_key_for_email_or_forge(p, root, &git_email, None))
        .unwrap_or_else(|| git_email.clone());

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
                                    eprintln!("{reason}");
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
                        // git-email identity would confuse the caller when
                        // the subsequent guard denial names the human
                        // instead of the AI they thought they were acting
                        // as. Emit a one-line stderr hint and continue
                        // with the fallback so read-only commands still
                        // work.
                        eprintln!(
                            "{}",
                            cross_project_session_warning(
                                &sess.claims.project_id,
                                &sess.claims.member,
                                current_pid,
                            )
                        );
                    }
                }
            }
        }
    }

    // 2. Human session by git email
    if let Some(ref pid) = project_id {
        if let Some(session_identity) = session_identity(root, &member_key, pid, &project) {
            return Ok(session_identity);
        }
    }

    // 3. Fallback: resolved member key (git email in open mode), not authenticated
    Ok(Identity {
        member: member_key.into(),
        delegated_by: None,
        authenticated: false,
    })
}

/// Try to build an Identity from an active session for a member.
fn session_identity(
    root: &Path,
    member: &str,
    project_id: &str,
    project: &Option<Project>,
) -> Option<Identity> {
    if !check_session(root, member, project) {
        return None;
    }

    // Read the session to get delegated_by info
    let delegated_by = crate::auth::session::load_session(project_id, member)
        .ok()
        .flatten()
        .and_then(|_sess| {
            // AI sessions are delegated by a human operator. Record that operator
            // as the at-rest member key (the opaque id in anonymous mode), never
            // their cleartext e-mail, so the audit trail and commit trailer carry
            // no PII in anonymous mode (ADR-042). MemberRef resolves it back for
            // authorized display.
            if is_ai_member(member) {
                let email = crate::vcs::default_vcs().user_email().ok()?;
                match project.as_ref() {
                    Some(p) => crate::privacy::delegated_by_at_rest(p, &email).map(MemberRef::from),
                    None => Some(MemberRef::from(email)),
                }
            } else {
                None
            }
        });

    Some(Identity {
        member: member.into(),
        delegated_by,
        authenticated: true,
    })
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
