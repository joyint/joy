// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Platform-issued, job-bound joy sessions for sandboxed agents
//! (JOY-020B-D2).
//!
//! The execution platform runs AI agents inside job sandboxes where no
//! human operator is present to redeem a delegation token. Instead, the
//! platform holds its own Ed25519 keypair: the public half is registered
//! in project.yaml (`joy project platform-key <hex>`, Manage-guarded) and
//! the private half never leaves the platform. [`mint_job_session`] signs
//! session claims bound to a specific job item with that key; every joy
//! command inside the sandbox then verifies the signature against the
//! registered key and accepts the session only while the job is
//! in-progress with the AI among its assignees (see
//! [`crate::identity::validate_job_session`]).
//!
//! Print-free by design, like [`super::login`]: the platform links
//! joy-core directly and must not depend on CLI output.

use std::path::Path;

use chrono::Duration;

use super::session;
use super::IdentityKeypair;
use crate::error::JoyError;
use crate::model::item::ItemType;
use crate::model::project::is_ai_member;
use crate::store;

/// Mint a job-bound session for a sandboxed AI agent, signed by the
/// platform key, and return the `joy_s_...` value for the sandbox's
/// `JOY_SESSION` env var.
///
/// * `platform_signing_key` -- the 32-byte Ed25519 seed of the platform's
///   session-signing keypair. Its public half must already be registered
///   in project.yaml (`project.platform.verify_key`); minting fails
///   otherwise, because the resulting session would be rejected by every
///   command anyway.
/// * `member` -- the AI member the session acts as (`ai:...`), must be
///   registered in the project.
/// * `job_id` -- the job item the session is bound to. It must exist and
///   be a `job`; its status and assignee list are deliberately NOT
///   checked here but at command time, so a session minted moments before
///   the job flips to in-progress works, and one that outlives the job's
///   in-progress phase stops working.
/// * `delegated_by` -- the human who approved the job (released it at
///   triage, authorizing the spend). Accepted as an at-rest member key or
///   an e-mail; recorded at rest via the privacy helpers so anonymous
///   projects never get cleartext PII into their audit trail (ADR-042).
/// * `ttl` -- session lifetime; the platform should size it to the job's
///   execution window.
///
/// The session file is written to this machine's joy state dir
/// (`~/.local/state/joy/sessions`, honoring `XDG_STATE_HOME`): the
/// platform must call this inside the sandbox's state-dir context (or
/// with `XDG_STATE_HOME` pointing into the sandbox) so the file lands
/// where the sandboxed joy commands look for it.
///
/// LIMITATION (accepted): session files are keyed by (project, member),
/// so there is ONE live job session per AI member per project (and per
/// state dir). Minting for a second concurrent job of the same AI member
/// in the same state dir displaces the first session. Sandboxes get
/// isolated state dirs, which is also what keeps this a non-issue in
/// practice.
pub fn mint_job_session(
    root: &Path,
    platform_signing_key: &[u8; 32],
    member: &str,
    job_id: &str,
    delegated_by: &str,
    ttl: Duration,
) -> Result<String, JoyError> {
    let project = store::load_project(root)?;
    let project_id = session::project_id_of(&project);

    // The signing key must be the registered platform key. Failing here
    // beats minting a session that every command will reject.
    let platform_keypair = IdentityKeypair::from_seed(platform_signing_key);
    let Some(ref platform) = project.platform else {
        return Err(JoyError::AuthFailed(
            "no platform key registered in project.yaml; \
             run `joy project platform-key <hex>` first"
                .into(),
        ));
    };
    if platform_keypair.public_key().to_hex() != platform.verify_key {
        return Err(JoyError::AuthFailed(
            "platform signing key does not match the verify key registered in project.yaml".into(),
        ));
    }

    if !is_ai_member(member) {
        return Err(JoyError::AuthFailed(format!(
            "job-bound sessions are for AI members only; {member} is not an `ai:` id"
        )));
    }
    if !project.has_member_key(member) {
        return Err(JoyError::AuthFailed(format!(
            "{member} is not a registered project member"
        )));
    }

    // Early wiring check: the job must exist and be a job item. Status
    // and assignees are enforced at command time (see module docs).
    let item = crate::items::load_item(root, job_id)?;
    if item.item_type != ItemType::Job {
        return Err(JoyError::AuthFailed(format!("{job_id} is not a job item")));
    }

    // Record the approving human at rest: an at-rest member key passes
    // through unchanged; an e-mail resolves per privacy mode (opaque id
    // in anonymous mode, dropped entirely when unresolvable there).
    let delegated_by_at_rest = if project.has_member_key(delegated_by) {
        Some(delegated_by.to_string())
    } else {
        crate::privacy::delegated_by_at_rest(&project, delegated_by)
    };

    // Fresh ephemeral keypair for proof of possession (ADR-033): the
    // private half lives only in the returned env value, the public half
    // in the signed claims.
    let ephemeral_keypair = IdentityKeypair::from_random();
    let token = session::create_session_for_job(
        &platform_keypair,
        &ephemeral_keypair,
        member,
        &project_id,
        &item.id,
        delegated_by_at_rest.clone(),
        ttl,
    );
    session::save_session(&project_id, &token)?;

    let actor = match delegated_by_at_rest {
        Some(ref operator) => format!("{member} delegated-by:{operator}"),
        None => member.to_string(),
    };
    crate::event_log::log_event_as(
        root,
        crate::event_log::EventType::AuthSessionCreated,
        "auth",
        Some(&format!(
            "job-bound session created for {member} (job {})",
            item.id
        )),
        &actor,
    );

    let sid = session::session_storage_id(&project_id, &token.claims);
    Ok(session::encode_session_env(
        &sid,
        &ephemeral_keypair.to_seed_bytes(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::item::{Assignee, Item, Priority, Status};
    use crate::model::project::{Member, MemberCapabilities};
    use crate::model::Project;

    const AI: &str = "ai:claude@joy";
    const JOB: &str = "TST-JOB-0001";

    fn platform_seed() -> [u8; 32] {
        [42u8; 32]
    }

    fn setup(root: &Path, with_platform_key: bool) {
        let joy = store::joy_dir(root);
        std::fs::create_dir_all(joy.join(store::JOBS_DIR)).unwrap();
        let mut project = Project::new("Test".into(), Some("TST".into()));
        project
            .register_member(AI, Member::new(MemberCapabilities::All))
            .unwrap();
        if with_platform_key {
            let key = IdentityKeypair::from_seed(&platform_seed())
                .public_key()
                .to_hex();
            project.set_platform_key(&key).unwrap();
        }
        store::write_yaml(&joy.join(store::PROJECT_FILE), &project).unwrap();

        let mut item = Item::new(
            JOB.into(),
            "sandbox job".into(),
            ItemType::Job,
            Priority::Medium,
            vec![],
        );
        item.status = Status::InProgress;
        item.assignees = vec![Assignee {
            member: AI.into(),
            capabilities: vec![],
        }];
        crate::items::save_item(root, &item).unwrap();
    }

    // The successful-mint path needs the session store (XDG_STATE_HOME)
    // and is covered by tests/job_session_mint.rs, which owns the env
    // mutations. The failure paths below all error before touching the
    // session store, so they are parallel-safe here.

    #[test]
    fn mint_refuses_unregistered_platform_key() {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path(), false);
        let err = mint_job_session(
            dir.path(),
            &platform_seed(),
            AI,
            JOB,
            "op@example.com",
            Duration::hours(1),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("no platform key registered"),
            "got: {err}"
        );
    }

    #[test]
    fn mint_refuses_mismatched_signing_key() {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path(), true);
        let err = mint_job_session(
            dir.path(),
            &[7u8; 32],
            AI,
            JOB,
            "op@example.com",
            Duration::hours(1),
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not match"), "got: {err}");
    }

    #[test]
    fn mint_refuses_non_ai_and_unregistered_members() {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path(), true);
        let err = mint_job_session(
            dir.path(),
            &platform_seed(),
            "human@example.com",
            JOB,
            "op@example.com",
            Duration::hours(1),
        )
        .unwrap_err();
        assert!(err.to_string().contains("AI members only"), "got: {err}");

        let err = mint_job_session(
            dir.path(),
            &platform_seed(),
            "ai:unknown@joy",
            JOB,
            "op@example.com",
            Duration::hours(1),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not a registered project member"),
            "got: {err}"
        );
    }

    #[test]
    fn mint_refuses_missing_job() {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path(), true);
        let err = mint_job_session(
            dir.path(),
            &platform_seed(),
            AI,
            "TST-JOB-0099",
            "op@example.com",
            Duration::hours(1),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"), "got: {err}");
    }
}
