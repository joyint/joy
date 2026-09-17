// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The one identity call site package J11 moves outside joy-cli:
//! `ai_setup::init_tool`, the desktop app's activation entry (D3.9 and
//! package J11 of the forge connection NG design, JOY-02A0-6E).
//!
//! It used to attest the AI member it registers with `git config
//! user.email`; it now asks `joy_core::identity::acting_human_key`. The
//! two cases that matter to a desktop are that activation works on a
//! checkout with no git identity at all once the person is known, and
//! that a checkout where nobody is known says so in D4.5's words instead
//! of registering a member attested by a guess.
//!
//! ONE test in its own binary: it moves `XDG_STATE_HOME`, which is
//! process state, so a second test beside it would read the device pin
//! this one writes.

use std::path::Path;

use joy_core::auth::{generate_salt, seed as seed_mod, IdentityKeypair};
use joy_core::init::{self, InitOptions};

const PASSPHRASE: &str = "correct horse battery staple";

/// A project founded by `founder` with `founder` enrolled, on a device
/// whose state directory is this test's own.
fn founded(root: &Path, founder: &str) {
    init::init(InitOptions {
        name: Some("Activated".into()),
        acronym: Some("AV".into()),
        user: Some(founder.to_string()),
        ..InitOptions::new(root.to_path_buf())
    })
    .unwrap();

    let salt = generate_salt();
    let seed = seed_mod::Seed::generate();
    let recovery = seed_mod::RecoveryKey::generate();
    let keypair = IdentityKeypair::from_seed(seed.as_bytes());
    let mut project = joy_core::store::load_project(root).unwrap();
    joy_core::auth::enroll::apply_enrollment(
        &mut project,
        founder,
        joy_core::auth::enroll::Proof::FirstContact,
        joy_core::auth::enroll::EnrollmentMaterial {
            verify_key: keypair.public_key().to_hex(),
            kdf_nonce: salt.to_hex(),
            seed_wrap_passphrase: seed_mod::wrap_seed_with_passphrase(&seed, PASSPHRASE, &salt)
                .unwrap(),
            seed_wrap_recovery: seed_mod::wrap_seed_with_recovery(&seed, &recovery, &salt).unwrap(),
        },
    )
    .unwrap();
    let path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    joy_core::store::write_yaml(&path, &project).unwrap();
}

/// Model the desktop on a fresh clone: the project file travels, this
/// device's own state does not.
fn forget_the_device_state(root: &Path) {
    let pin = joy_core::auth::session::app_state_project_file(root).unwrap();
    if pin.exists() {
        std::fs::remove_file(pin).unwrap();
    }
}

#[test]
fn activation_takes_its_attester_from_the_acting_member() {
    let state = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_STATE_HOME", state.path());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    founded(root, "alice@example.com");

    // The desktop activates a tool. Nothing here reads git config: the
    // attester is the member this device pinned when the project was
    // founded, and the attestation is signed with her seed.
    joy_ai::ai_setup::init_tool(root, "claude", PASSPHRASE, &mut |_| {}).unwrap();
    let project = joy_core::store::load_project(root).unwrap();
    let member = project
        .member_by_key("ai:claude@joy")
        .expect("the tool's member is registered");
    let attestation = member
        .attestation
        .as_ref()
        .expect("the member carries an attestation");
    assert_eq!(
        attestation.attester, "alice@example.com",
        "the acting member attested it"
    );

    // The same checkout on a device that knows nobody: activation
    // refuses, with the sentence D4.5 writes for a host that has a member
    // picker, and it registers nothing.
    forget_the_device_state(root);
    let err = joy_ai::ai_setup::init_tool(root, "qwen", PASSPHRASE, &mut |_| {}).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
    assert!(
        err.to_string()
            .starts_with("this project does not know who you are, pick your member"),
        "the first line is the app's own remedy: {err}"
    );
    assert!(
        err.to_string()
            .contains("In the app that is the member picker"),
        "and the command line's remedies are named as the command line's: {err}"
    );
    let project = joy_core::store::load_project(root).unwrap();
    assert!(
        project.member_by_key("ai:qwen@joy").is_none(),
        "no member is registered on an attester joy had to guess"
    );
}
