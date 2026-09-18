// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The founding step on a machine that has no git identity at all
//! (D3.9 of the forge connection NG design, JOY-0297-1A).
//!
//! Its own test binary, because it isolates the git configuration of the
//! whole process: HOME, the XDG config home, git's own `GIT_CONFIG_NOSYSTEM`
//! switch and libgit2's search paths are process state, and a case about a
//! MISSING identity is only honest when nothing on the machine can supply
//! one.

use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

use joy_core::host::HostKind;
use joy_core::init::{self, InitOptions, TerminalAsk};
use joy_core::model::project::{Member, MemberCapabilities};

/// A machine with no git identity anywhere: no repository config, no
/// global config, no XDG config, no system config. Applied once for this
/// binary; every case then just uses it.
fn a_machine_without_a_git_identity() {
    static ISOLATED: OnceLock<tempfile::TempDir> = OnceLock::new();
    ISOLATED.get_or_init(|| {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
        std::env::set_var("XDG_STATE_HOME", home.path().join(".state"));
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        // Outside any repository: joy looks at the working directory
        // first, and this crate's own checkout is a repository.
        std::env::set_current_dir(home.path()).unwrap();
        // The first look settles joy's view of the variable and empties
        // the system level; the other levels are pointed at the empty
        // home, where Windows also looks (USERPROFILE).
        joy_core::vcs::forge::user_email();
        unsafe {
            for level in [
                git2::ConfigLevel::Global,
                git2::ConfigLevel::XDG,
                git2::ConfigLevel::ProgramData,
            ] {
                git2::opts::set_search_path(level, home.path().to_str().unwrap()).unwrap();
            }
        }
        assert_eq!(
            joy_core::vcs::forge::user_email(),
            None,
            "the test machine must have no git identity"
        );
        home
    });
}

/// D3.9: on a host with a person at it, `joy init` asks for the address
/// instead of failing, and the project is founded on what they typed.
/// The fake stdin is the person.
#[test]
fn an_interactive_host_asks_for_the_address_and_completes() {
    a_machine_without_a_git_identity();
    let dir = tempfile::tempdir().unwrap();

    let typed = Cursor::new(b"founder@example.com\n".to_vec());
    let result = init::init(InitOptions {
        name: Some("Asked".into()),
        acronym: Some("AS".into()),
        host: HostKind::Interactive,
        ask: Some(Box::new(TerminalAsk::new(typed, Vec::new()))),
        ..InitOptions::new(dir.path().to_path_buf())
    })
    .expect("init completes on what the person typed");

    assert_eq!(result.founder, "founder@example.com");
    let project = joy_core::store::load_project(dir.path()).unwrap();
    assert!(project.member_by_key("founder@example.com").is_some());
}

/// D3.9: a host with nobody at it refuses with the named sentence, and
/// leaves nothing half-initialized behind.
#[test]
fn a_background_or_delegated_host_refuses_by_name() {
    a_machine_without_a_git_identity();
    for host in [HostKind::Background, HostKind::Delegated] {
        let dir = tempfile::tempdir().unwrap();
        let err = init::init(InitOptions {
            name: Some("Refused".into()),
            host,
            // an ask is at hand and is still not used
            ask: Some(Box::new(TerminalAsk::new(
                Cursor::new(b"founder@example.com\n".to_vec()),
                Vec::new(),
            ))),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "this project does not know who you are; run joy init --user <address>",
            "{host:?}"
        );
        assert!(!dir.path().join(".joy").exists(), "{host:?}");
    }
}

/// The acceptance of J9: `joy init --user a@b.c` in a repository with no
/// git config succeeds, and the enrolment that follows enrols that member
/// without reading git config. The enrolment here is the OTP redemption,
/// the one joy-core owns; the CLI's `joy auth init` path is driven in
/// joy-cli's own test.
#[test]
fn an_explicit_user_founds_and_enrols_without_a_git_config() {
    a_machine_without_a_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let result = init::init(InitOptions {
        name: Some("No Config".into()),
        acronym: Some("NC".into()),
        user: Some("a@b.c".into()),
        ..InitOptions::new(root.to_path_buf())
    })
    .expect("--user needs no git config");
    assert_eq!(result.founder, "a@b.c");

    // A second member with an open invitation, added the way a manager
    // adds one. Nothing here knows a git address either.
    let otp = joy_core::auth::otp::generate_otp();
    invite(root, "mate@example.com", &otp);

    // The host names nobody: the OTP is the identity proof and finds its
    // own member, which is what "no git config is read" means here.
    let outcome = joy_core::auth::enroll::redeem_with_passphrase(
        root,
        &otp,
        "correct horse battery staple",
        None,
    )
    .expect("the enrolment resolves its member without git config");
    assert_eq!(outcome.member_key, "mate@example.com");

    // ...and this device now knows who acts here, still without a git
    // config: the pin of D3.9, read back through resolve_identity.
    let identity = joy_core::identity::resolve_identity(root).unwrap();
    assert_eq!(identity.member.id(), "mate@example.com");
    assert!(identity.authenticated, "the redemption opened a session");
}

/// The named member wins over everything else, which is what the desktop
/// mask and `--user` hand in (D4.4).
#[test]
fn the_enrolment_takes_the_member_the_host_names() {
    a_machine_without_a_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init::init(InitOptions {
        name: Some("Named".into()),
        acronym: Some("NM".into()),
        user: Some("founder@example.com".into()),
        ..InitOptions::new(root.to_path_buf())
    })
    .unwrap();
    let otp = joy_core::auth::otp::generate_otp();
    invite(root, "mate@example.com", &otp);

    let outcome = joy_core::auth::enroll::redeem_with_passphrase(
        root,
        &otp,
        "correct horse battery staple",
        Some("mate@example.com"),
    )
    .unwrap();
    assert_eq!(outcome.member_key, "mate@example.com");
}

/// Add a member with a pending invitation and persist the project.
fn invite(root: &Path, address: &str, otp: &str) {
    let project_path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project = joy_core::store::load_project(root).unwrap();
    let mut member = Member::new(MemberCapabilities::All);
    member.enrollment_verifier = Some(joy_core::auth::otp::hash_otp(otp).unwrap());
    project.register_member(address, member).unwrap();
    joy_core::store::write_yaml(&project_path, &project).unwrap();
}
