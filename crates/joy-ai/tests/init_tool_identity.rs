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
//! ONE test in its own binary: it moves `XDG_STATE_HOME`, `HOME` and
//! `XDG_CONFIG_HOME`, which are process state, so a second test beside
//! it would read the device pin or the git identity this one writes.

use std::path::Path;

use joy_core::auth::{generate_salt, seed as seed_mod, IdentityKeypair};
use joy_core::init::{self, InitOptions};

const PASSPHRASE: &str = "correct horse battery staple";

/// A project founded by `founder` with `founder` enrolled, on a device
/// whose state directory is this test's own. Also leaves the
/// repository's own `user.email` naming `founder`: since the operator's
/// 2026-09-19 correction (JOY-02AE-1A, correcting D3.9) `resolve_identity`
/// reads git config again, a caller that wants activation attested by
/// `founder` has to set it explicitly, exactly as a working checkout
/// would have it.
fn founded(root: &Path, founder: &str) {
    init::init(InitOptions {
        name: Some("Activated".into()),
        acronym: Some("AV".into()),
        user: Some(founder.to_string()),
        ..InitOptions::new(root.to_path_buf())
    })
    .unwrap();
    act_as(root, founder);

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

/// Set this repository's own (local) `user.email`, the identity source
/// `resolve_identity` reads second, right after `JOY_SESSION`
/// (JOY-02AE-1A).
fn act_as(root: &Path, email: &str) {
    let repo = git2::Repository::open(root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.email", email).unwrap();
}

/// Model the desktop on a fresh clone: the project file travels, this
/// device's own state does not, and neither does a real clone's LOCAL
/// git config, which lives only in the checkout that wrote it and is
/// never carried by the git objects a clone copies. Removing the pin
/// alone used to be enough to model "nobody is known here"; since the
/// operator's 2026-09-19 correction (JOY-02AE-1A) git config is a source
/// again, so the repository's own `user.email` set by [`founded`] has to
/// go too, or the same checkout would still resolve to the founder.
fn forget_the_device_state(root: &Path) {
    let pin = joy_core::auth::session::app_state_project_file(root).unwrap();
    if pin.exists() {
        std::fs::remove_file(pin).unwrap();
    }
    let repo = git2::Repository::open(root).unwrap();
    let mut local = repo
        .config()
        .unwrap()
        .open_level(git2::ConfigLevel::Local)
        .unwrap();
    let _ = local.remove("user.email");
}

#[test]
fn activation_takes_its_attester_from_the_acting_member() {
    let state = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_STATE_HOME", state.path());
    // Isolate HOME/XDG_CONFIG_HOME too: since JOY-02AE-1A resolve_identity
    // reads git config, and a global config on whichever machine runs this
    // test must not be able to name a member the repository's own config
    // did not.
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    founded(root, "alice@example.com");

    // The desktop activates a tool. The attester is the member the
    // repository's own git config names (JOY-02AE-1A, correcting D3.9),
    // and the attestation is signed with her seed.
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
