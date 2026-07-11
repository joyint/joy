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
    let git_email = crate::vcs::default_vcs().user_email()?;
    let project = load_project_optional(root);
    let project_id = crate::auth::session::project_id(root).ok();

    // In anonymous mode (ADR-042) the member map is keyed by an opaque id, not
    // the git e-mail; resolve the e-mail to that id so membership checks,
    // sessions and the audit actor all use the same key. In open mode (or when
    // the e-mail is not a member) this is just the e-mail.
    let member_key = project
        .as_ref()
        .and_then(|p| crate::privacy::member_key_for_email(p, &git_email))
        .unwrap_or_else(|| git_email.clone());

    // 1. JOY_SESSION: env var carries the ephemeral private key bound to
    //    the session (ADR-033). We derive the public key from it and match
    //    against `session_public_key` stored in the session file. Without
    //    possession of the env var a sibling terminal cannot reuse a
    //    session file it can read.
    if let Some(env_value) = std::env::var("JOY_SESSION").ok().filter(|s| !s.is_empty()) {
        if let Some((sid, ephemeral_private)) = crate::auth::session::parse_session_env(&env_value)
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
                                if sess.claims.job_id.is_some() {
                                    // Job-bound platform session (JOY-020B-D2):
                                    // additionally require the platform
                                    // signature and a live job binding. On
                                    // rejection, emit one stderr hint and fall
                                    // through unauthenticated (mirroring the
                                    // cross-project warning below): silently
                                    // degrading to the git-email identity would
                                    // make the subsequent guard denial name the
                                    // wrong actor.
                                    match validate_job_session(
                                        root,
                                        project,
                                        project_id.as_deref().unwrap_or_default(),
                                        &sess,
                                    ) {
                                        Ok(()) => {
                                            return Ok(Identity {
                                                member: sess.claims.member.clone().into(),
                                                // The approving human was
                                                // recorded at mint time as an
                                                // at-rest member key; inside the
                                                // job sandbox there is no
                                                // operator git e-mail to derive
                                                // it from.
                                                delegated_by: sess
                                                    .claims
                                                    .delegated_by
                                                    .clone()
                                                    .map(Into::into),
                                                authenticated: true,
                                            });
                                        }
                                        Err(reason) => {
                                            eprintln!(
                                                "{}",
                                                job_session_rejection_hint(&reason)
                                            );
                                        }
                                    }
                                } else {
                                    return Ok(Identity {
                                        member: sess.claims.member.clone().into(),
                                        // Record the delegating operator by their
                                        // at-rest member key (opaque id in anonymous
                                        // mode), never the cleartext git e-mail, so
                                        // the audit trail carries no PII (ADR-042).
                                        delegated_by: crate::privacy::delegated_by_at_rest(
                                            project, &git_email,
                                        )
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

/// Validate the job binding of a platform-issued session (JOY-020B-D2).
///
/// Called after the generic AI-session checks (expiry, member registered,
/// project match, ephemeral proof of possession) for sessions whose claims
/// carry a `job_id`. Accepts exactly when:
///
/// 1. the claims name the platform as issuer, the project has a registered
///    platform verify key, and the Ed25519 signature over the claims
///    verifies against it (ordinary AI sessions skip signature
///    verification; job sessions must not, because "signed by the
///    platform" is their whole authority), and
/// 2. the bound job item loads, is a `job`, is `in-progress`, and lists
///    the session member among its assignees.
///
/// Returns the rejection reason so callers can surface it: the auth wall
/// (guard.rs) makes every unauthenticated write fail with a message naming
/// the git-email identity, which is confusing when the real cause is a
/// stale job binding.
///
/// This is the single source of truth for the accept decision:
/// `resolve_identity` enforces it at runtime and `joy project member`
/// mirrors it for display (JOY-00F4-CF: display and runtime MUST agree).
pub fn validate_job_session(
    root: &Path,
    project: &Project,
    project_id: &str,
    sess: &crate::auth::session::SessionToken,
) -> Result<(), String> {
    let claims = &sess.claims;
    let Some(ref job_id) = claims.job_id else {
        return Err("session carries no job binding".into());
    };

    // (a) Platform issuer + signature over the claims.
    if claims.issuer.as_deref() != Some(crate::auth::session::PLATFORM_ISSUER) {
        return Err(format!(
            "issuer is {}, expected \"{}\"",
            claims.issuer.as_deref().unwrap_or("absent"),
            crate::auth::session::PLATFORM_ISSUER,
        ));
    }
    let Some(ref platform) = project.platform else {
        return Err(
            "no platform key registered in project.yaml (joy project platform-key <hex>)".into(),
        );
    };
    let platform_pk = crate::auth::PublicKey::from_hex(&platform.verify_key)
        .map_err(|e| format!("registered platform key is not a valid Ed25519 key: {e}"))?;
    crate::auth::session::validate_session(sess, &platform_pk, project_id)
        .map_err(|e| format!("platform signature check failed: {e}"))?;

    // (b) The job item gates the session lifecycle.
    if !crate::items::is_job_id(job_id) {
        return Err(format!("{job_id} is not a job id"));
    }
    let item = crate::items::load_item(root, job_id)
        .map_err(|_| format!("job {job_id} not found in this project"))?;
    if item.item_type != crate::model::item::ItemType::Job {
        return Err(format!("{job_id} is not a job item"));
    }
    if item.status != crate::model::item::Status::InProgress {
        return Err(format!(
            "job {job_id} is not in progress (status: {})",
            item.status
        ));
    }
    if !item
        .assignees
        .iter()
        .any(|a| a.member.id() == claims.member)
    {
        return Err(format!(
            "{} is not an assignee of job {job_id}",
            claims.member
        ));
    }

    Ok(())
}

/// Build the one-line stderr hint for a rejected job-bound session.
///
/// Extracted as a pure helper so it can be asserted directly in unit
/// tests, like [`cross_project_session_warning`].
fn job_session_rejection_hint(reason: &str) -> String {
    format!("job-bound session rejected: {reason}")
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

    #[test]
    fn job_session_rejection_hint_carries_reason() {
        let msg = job_session_rejection_hint("job TST-JOB-0001 is not in progress");
        assert_eq!(
            msg,
            "job-bound session rejected: job TST-JOB-0001 is not in progress"
        );
    }

    // -----------------------------------------------------------------
    // validate_job_session accept/reject matrix (JOY-020B-D2). Pure
    // file-system tests: no env vars, no session store, so they are
    // safe to run in parallel with the rest of the suite. The full
    // mint -> resolve_identity flow (which needs JOY_SESSION and
    // XDG_STATE_HOME) lives in tests/job_session_mint.rs.
    // -----------------------------------------------------------------

    use crate::auth::session::{create_session_for_job, SessionToken};
    use crate::auth::IdentityKeypair;
    use crate::model::item::{Assignee, Item, ItemType, JobSpec, Priority, Status};
    use crate::model::project::{Member, MemberCapabilities};
    use chrono::Duration;

    const AI: &str = "ai:claude@joy";
    const JOB: &str = "TST-JOB-0001";
    const PID: &str = "TST";

    fn platform_kp() -> IdentityKeypair {
        IdentityKeypair::from_seed(&[42u8; 32])
    }

    fn job_project(with_platform_key: bool) -> Project {
        let mut project = crate::model::Project::new("Test".into(), Some(PID.into()));
        project
            .register_member(AI, Member::new(MemberCapabilities::All))
            .unwrap();
        if with_platform_key {
            project
                .set_platform_key(&platform_kp().public_key().to_hex())
                .unwrap();
        }
        project
    }

    fn seed_job(root: &Path, status: Status, assignee: &str) {
        std::fs::create_dir_all(crate::store::joy_dir(root).join(crate::store::JOBS_DIR)).unwrap();
        let mut item = Item::new(
            JOB.into(),
            "sandbox job".into(),
            ItemType::Job,
            Priority::Medium,
            vec![],
        );
        item.status = status;
        item.assignees = vec![Assignee {
            member: assignee.into(),
            capabilities: vec![],
        }];
        item.job = Some(JobSpec {
            scope: vec!["TST-0001".into()],
            budget: None,
            window: None,
            feedback: None,
            attempts: vec![],
        });
        crate::items::save_item(root, &item).unwrap();
    }

    fn job_token(ttl: Duration) -> SessionToken {
        create_session_for_job(
            &platform_kp(),
            &IdentityKeypair::from_random(),
            AI,
            PID,
            JOB,
            Some("op@example.com".into()),
            ttl,
        )
    }

    #[test]
    fn job_session_accepted_when_platform_signed_and_job_live() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(true);
        seed_job(dir.path(), Status::InProgress, AI);
        let token = job_token(Duration::hours(1));
        assert_eq!(
            validate_job_session(dir.path(), &project, PID, &token),
            Ok(())
        );
    }

    #[test]
    fn job_session_rejected_without_platform_key() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(false);
        seed_job(dir.path(), Status::InProgress, AI);
        let token = job_token(Duration::hours(1));
        let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
        assert!(err.contains("no platform key registered"), "got: {err}");
    }

    #[test]
    fn job_session_rejected_when_signed_by_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(true);
        seed_job(dir.path(), Status::InProgress, AI);
        let rogue = IdentityKeypair::from_seed(&[7u8; 32]);
        let token = create_session_for_job(
            &rogue,
            &IdentityKeypair::from_random(),
            AI,
            PID,
            JOB,
            None,
            Duration::hours(1),
        );
        let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
        assert!(err.contains("signature check failed"), "got: {err}");
    }

    #[test]
    fn job_session_rejected_when_issuer_is_not_platform() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(true);
        seed_job(dir.path(), Status::InProgress, AI);
        let mut token = job_token(Duration::hours(1));
        token.claims.issuer = None;
        let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
        assert!(err.contains("issuer is absent"), "got: {err}");

        token.claims.issuer = Some("somebody-else".into());
        let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
        assert!(err.contains("issuer is somebody-else"), "got: {err}");
    }

    #[test]
    fn job_session_rejected_when_claims_tampered() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(true);
        seed_job(dir.path(), Status::InProgress, AI);
        let mut token = job_token(Duration::hours(1));
        token.claims.member = "ai:attacker@joy".into();
        let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
        assert!(err.contains("signature check failed"), "got: {err}");
    }

    #[test]
    fn job_session_rejected_when_expired() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(true);
        seed_job(dir.path(), Status::InProgress, AI);
        let token = job_token(Duration::seconds(-1));
        let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
        assert!(err.contains("expired"), "got: {err}");
    }

    #[test]
    fn job_session_rejected_when_job_not_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(true);
        for status in [Status::New, Status::Open, Status::Review, Status::Closed] {
            seed_job(dir.path(), status.clone(), AI);
            let token = job_token(Duration::hours(1));
            let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
            assert!(err.contains("is not in progress"), "got: {err}");
        }
    }

    #[test]
    fn job_session_rejected_when_member_not_assignee() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(true);
        seed_job(dir.path(), Status::InProgress, "ai:other@joy");
        let token = job_token(Duration::hours(1));
        let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
        assert!(err.contains("is not an assignee"), "got: {err}");
    }

    #[test]
    fn job_session_rejected_when_job_missing() {
        let dir = tempfile::tempdir().unwrap();
        let project = job_project(true);
        // No job seeded at all.
        let token = job_token(Duration::hours(1));
        let err = validate_job_session(dir.path(), &project, PID, &token).unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }
}
