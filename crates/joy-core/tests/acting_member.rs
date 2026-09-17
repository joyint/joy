// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Who joy thinks is acting, and in which order it asks (D3.9 of the
//! forge connection NG design, JOY-0297-1A): the session first, then the
//! member this device pinned, then git config as a prefill.
//!
//! ONE test in its own binary, on purpose. The question is about process
//! state that has no per-thread version: HOME, libgit2's config search
//! paths and the working directory decide what `git config user.email`
//! answers, and the case only means something when the test moves them
//! step by step. A second test running beside it would see the git
//! identity this one writes halfway through.

use std::path::Path;

use joy_core::identity::{acting_member, pin_acting_member, pinned_member, resolve_identity};
use joy_core::init::{init, InitOptions};
use joy_core::model::project::{Member, MemberCapabilities};

/// A machine with no git identity anywhere: no global config, no XDG
/// config, no system config, and a working directory outside every
/// repository.
fn a_machine_without_a_git_identity() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
    std::env::set_var("XDG_STATE_HOME", home.path().join(".state"));
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    std::env::set_current_dir(home.path()).unwrap();
    // The first look settles joy's view of the variable and empties the
    // system level; the other levels are pointed at the empty home.
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
}

/// Write a global `user.email`, the way `git config --global` would.
fn git_config_says(home: &Path, email: &str) {
    std::fs::write(
        home.join(".gitconfig"),
        format!("[user]\n\temail = {email}\n\tname = Somebody\n"),
    )
    .unwrap();
    assert_eq!(joy_core::vcs::forge::user_email().as_deref(), Some(email));
}

fn forget_the_git_config(home: &Path) {
    std::fs::remove_file(home.join(".gitconfig")).unwrap();
    assert_eq!(joy_core::vcs::forge::user_email(), None);
}

/// Model another machine, or a fresh clone: the project file travels, the
/// device state does not.
fn forget_the_pin(root: &Path) {
    let pin = joy_core::auth::session::app_state_project_file(root).unwrap();
    if pin.exists() {
        std::fs::remove_file(pin).unwrap();
    }
}

fn add_member(root: &Path, address: &str) {
    let path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project = joy_core::store::load_project(root).unwrap();
    project
        .register_member(address, Member::new(MemberCapabilities::All))
        .unwrap();
    joy_core::store::write_yaml(&path, &project).unwrap();
}

fn found(root: &Path, founder: &str) {
    init(InitOptions {
        name: Some("Acting".into()),
        acronym: Some("AC".into()),
        user: Some(founder.to_string()),
        ..InitOptions::new(root.to_path_buf())
    })
    .unwrap();
}

#[test]
fn the_pin_answers_before_git_config_and_the_two_resolvers_agree() {
    let home = a_machine_without_a_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // 1. Founding names the founder, and this device remembers that it is
    //    the founder's device. Without that, nothing on a machine with no
    //    git config could say who acts here.
    found(root, "a@b.c");
    let project = joy_core::store::load_project(root).unwrap();
    assert_eq!(pinned_member(root, &project).as_deref(), Some("a@b.c"));
    assert_eq!(acting_member(root, &project, None).unwrap(), "a@b.c");
    assert_eq!(resolve_identity(root).unwrap().member.id(), "a@b.c");

    // 2. A named address wins over everything: that is how a second person
    //    on one machine says who they are.
    assert_eq!(
        acting_member(root, &project, Some(" bea@example.com ")).unwrap(),
        "bea@example.com",
        "the named address wins, trimmed"
    );

    // 3. git config appears and names a DIFFERENT member of this project.
    //    D3.9 puts the pin before it, and the point of the order is that
    //    both resolvers give the same answer: `joy auth status` and
    //    `joy auth init` cannot contradict each other.
    add_member(root, "bea@example.com");
    let project = joy_core::store::load_project(root).unwrap();
    git_config_says(home.path(), "bea@example.com");
    assert_eq!(acting_member(root, &project, None).unwrap(), "a@b.c");
    assert_eq!(resolve_identity(root).unwrap().member.id(), "a@b.c");

    // 4. The same project on a machine that pinned nobody: git config is
    //    the prefill it was always meant to be.
    forget_the_pin(root);
    assert_eq!(pinned_member(root, &project), None);
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "bea@example.com"
    );
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com"
    );

    // 5. A pin for somebody this project does not know is no answer at
    //    all: a member who was removed, or a project rekeyed to anonymous
    //    ids, must not keep deciding.
    pin_acting_member(root, &project, "ghost@example.com");
    assert_eq!(
        pinned_member(root, &project),
        None,
        "a pin is only kept for a member the project knows"
    );
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "bea@example.com"
    );

    // 6. Bea authenticates here, on a machine whose git config already
    //    names her: the pin is written anyway. Dropping it because the
    //    config agrees today would stand this project back on a git
    //    setting tomorrow, when the setting goes.
    pin_acting_member(root, &project, "bea@example.com");
    assert_eq!(
        pinned_member(root, &project).as_deref(),
        Some("bea@example.com")
    );
    forget_the_git_config(home.path());
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "bea@example.com",
        "removing user.email changes nothing once the member is known here"
    );
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com"
    );

    // ...and the next person who authenticates here replaces the pin,
    // which is the one way the answer on this machine changes.
    git_config_says(home.path(), "bea@example.com");
    pin_acting_member(root, &project, "a@b.c");
    assert_eq!(pinned_member(root, &project).as_deref(), Some("a@b.c"));
    assert_eq!(acting_member(root, &project, None).unwrap(), "a@b.c");
    assert_eq!(resolve_identity(root).unwrap().member.id(), "a@b.c");

    // 7. Neither pin nor git config, and a project with exactly one human
    //    member: joy says it does not know, and names the way out. It does
    //    NOT read the answer off the project file, because that file
    //    travels with every clone, and guessing from it would hand the
    //    founder's identity to anyone who clones an unenrolled project.
    let solo = tempfile::tempdir().unwrap();
    found(solo.path(), "only@example.com");
    forget_the_pin(solo.path());
    forget_the_git_config(home.path());
    let solo_project = joy_core::store::load_project(solo.path()).unwrap();
    let err = acting_member(solo.path(), &solo_project, None).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
    assert!(err.to_string().contains("--user <address>"), "{err}");
}
