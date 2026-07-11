// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! End-to-end test of platform-issued, job-bound sessions (JOY-020B-D2):
//! `auth::platform::mint_job_session` -> `identity::resolve_identity` in
//! a temp project with a seeded job item.
//!
//! Everything runs in ONE test function on purpose: the flow depends on
//! process-global env vars (`XDG_STATE_HOME` for the session store,
//! `JOY_SESSION` for the credential) which cargo's threaded test runner
//! would race across `#[test]` functions. The pure accept/reject matrix
//! of `validate_job_session` (no env access) lives in the identity.rs
//! unit tests instead.

use std::path::Path;

use chrono::Duration;
use joy_core::auth::platform::mint_job_session;
use joy_core::auth::IdentityKeypair;
use joy_core::identity::resolve_identity;
use joy_core::model::item::{Assignee, Item, ItemType, Priority, Status};
use joy_core::model::project::{Member, MemberCapabilities};
use joy_core::model::Project;
use joy_core::store;

const AI: &str = "ai:claude@joy";
const OPERATOR: &str = "op@example.com";
const JOB: &str = "TST-JOB-0001";
const PLATFORM_SEED: [u8; 32] = [42u8; 32];

fn setup_project(root: &Path) {
    let joy = store::joy_dir(root);
    std::fs::create_dir_all(joy.join(store::JOBS_DIR)).unwrap();

    let mut project = Project::new("Test".into(), Some("TST".into()));
    project
        .register_member(AI, Member::new(MemberCapabilities::All))
        .unwrap();
    project
        .register_member(OPERATOR, Member::new(MemberCapabilities::All))
        .unwrap();
    let platform_key = IdentityKeypair::from_seed(&PLATFORM_SEED)
        .public_key()
        .to_hex();
    project.set_platform_key(&platform_key).unwrap();
    store::write_yaml(&joy.join(store::PROJECT_FILE), &project).unwrap();

    seed_job(root, Status::InProgress);
}

fn seed_job(root: &Path, status: Status) {
    let mut item = Item::new(
        JOB.into(),
        "sandbox job".into(),
        ItemType::Job,
        Priority::Medium,
        vec![],
    );
    item.status = status;
    item.assignees = vec![Assignee {
        member: AI.into(),
        capabilities: vec![],
    }];
    joy_core::items::save_item(root, &item).unwrap();
}

/// Set an env var for the duration of the test. Wrapped so every
/// mutation site carries the same justification: this test binary owns
/// these variables (see module docs).
fn set_env(key: &str, value: &str) {
    // SAFETY: single-test binary; no concurrent env access.
    unsafe { std::env::set_var(key, value) };
}

fn remove_env(key: &str) {
    // SAFETY: single-test binary; no concurrent env access.
    unsafe { std::env::remove_var(key) };
}

#[test]
fn mint_then_resolve_identity_full_flow() {
    let project_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = project_dir.path();
    setup_project(root);
    set_env("XDG_STATE_HOME", state_dir.path().to_str().unwrap());
    remove_env("JOY_SESSION");

    // --- Baseline: without a session, identity is unauthenticated. ---
    let identity = resolve_identity(root).unwrap();
    assert!(!identity.authenticated, "no session yet");

    // --- Mint and resolve: the accept path. ---
    let env_value = mint_job_session(
        root,
        &PLATFORM_SEED,
        AI,
        JOB,
        OPERATOR,
        Duration::hours(2),
    )
    .unwrap();
    assert!(env_value.starts_with("joy_s_"), "got: {env_value}");
    set_env("JOY_SESSION", &env_value);

    let identity = resolve_identity(root).unwrap();
    assert!(identity.authenticated, "job session must authenticate");
    assert_eq!(identity.member.id(), AI);
    assert_eq!(
        identity.delegated_by.as_ref().map(|m| m.id().to_string()),
        Some(OPERATOR.to_string()),
        "delegated_by must come from the claims, not from git email"
    );
    assert_eq!(identity.log_user(), format!("{AI} delegated-by:{OPERATOR}"));

    // --- The job leaves in-progress: same session stops authenticating. ---
    seed_job(root, Status::Review);
    let identity = resolve_identity(root).unwrap();
    assert!(
        !identity.authenticated,
        "session must die with the job's in-progress phase"
    );
    assert_ne!(identity.member.id(), AI, "falls back to the git identity");

    // ...and returning to in-progress revives it (the binding is
    // enforced at command time, not by session-file lifecycle).
    seed_job(root, Status::InProgress);
    let identity = resolve_identity(root).unwrap();
    assert!(identity.authenticated);

    // --- Tampering with the stored claims breaks the signature. ---
    let project_id = "TST";
    let sid = joy_core::auth::session::session_id(project_id, AI);
    let session_path = state_dir
        .path()
        .join("joy")
        .join("sessions")
        .join(format!("{sid}.json"));
    let original = std::fs::read_to_string(&session_path).unwrap();
    let tampered = original.replace(OPERATOR, "attacker@evil.com");
    assert_ne!(original, tampered, "fixture must actually change");
    std::fs::write(&session_path, &tampered).unwrap();
    let identity = resolve_identity(root).unwrap();
    assert!(!identity.authenticated, "tampered claims must be rejected");
    std::fs::write(&session_path, &original).unwrap();

    // --- Wrong JOY_SESSION private key fails proof of possession. ---
    let rogue = IdentityKeypair::from_random();
    let forged_env =
        joy_core::auth::session::encode_session_env(&sid, &rogue.to_seed_bytes());
    set_env("JOY_SESSION", &forged_env);
    let identity = resolve_identity(root).unwrap();
    assert!(
        !identity.authenticated,
        "session file + wrong ephemeral key must not authenticate"
    );
    set_env("JOY_SESSION", &env_value);

    // --- An expired mint never authenticates. ---
    let expired_env = mint_job_session(
        root,
        &PLATFORM_SEED,
        AI,
        JOB,
        OPERATOR,
        Duration::seconds(-1),
    )
    .unwrap();
    set_env("JOY_SESSION", &expired_env);
    let identity = resolve_identity(root).unwrap();
    assert!(!identity.authenticated, "expired session must be rejected");

    remove_env("JOY_SESSION");
    remove_env("XDG_STATE_HOME");
}
