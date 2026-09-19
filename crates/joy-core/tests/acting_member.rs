// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Two different questions that both live in `identity.rs`, and how the
//! operator's 2026-09-19 correction (JOY-02AE-1A, correcting D3.9 of the
//! forge connection NG design, JOY-0297-1A) moved only one of them:
//!
//! - [`resolve_identity`] answers "who is acting right now": the
//!   delegation session first, then git config (repository before
//!   global), then the forge account, and nothing else. The device pin
//!   is never read here any more, whatever it names.
//! - [`acting_member`] answers a narrower, earlier question: "who does a
//!   bare `joy auth` or a bare enrolment act as, before a name, a git
//!   config or a forge account settled it". This one is UNCHANGED: the
//!   name a person typed first, then the member this device pinned,
//!   then git config as a prefill they can still overrule.
//!
//! Because only the first of the two dropped the pin, this file's one
//! case has a scenario where they disagree: a device pinned to one
//! member whose git config now names another. `resolve_identity` (`joy
//! auth status`, every write) follows the config; `acting_member` (a
//! bare `joy auth` with no `--user`) still offers the pin. That is a
//! deliberate consequence of the correction, not an oversight: the two
//! functions serve different moments (deciding who already acted, and
//! prefilling who a person about to authenticate probably is).
//!
//! ONE test in its own binary, on purpose. The question is about process
//! state that has no per-thread version: HOME, libgit2's config search
//! paths and the working directory decide what `git config user.email`
//! answers, and the case only means something when the test moves them
//! step by step. A second test running beside it would see the git
//! identity this one writes halfway through.

use std::path::Path;

use joy_core::identity::{
    acting_human_key, acting_member, acting_member_key, pin_acting_member, pinned_member,
    resolve_identity,
};
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
fn resolve_identity_follows_git_config_acting_member_still_offers_the_pin() {
    let home = a_machine_without_a_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // 1. Founding names the founder and pins this device to them, and
    //    with no git config anywhere `acting_member` has only the pin to
    //    offer. `resolve_identity`, though, reads git config and the
    //    forge account, not the pin: with neither present yet (no
    //    config, no remote) it answers with nobody, exactly as it would
    //    on a machine with no identity anywhere.
    found(root, "a@b.c");
    let project = joy_core::store::load_project(root).unwrap();
    assert_eq!(pinned_member(root, &project).as_deref(), Some("a@b.c"));
    assert_eq!(acting_member(root, &project, None).unwrap(), "a@b.c");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "",
        "no git config and no forge account: the pin decides nothing here \
         (operator decision 2026-09-19, JOY-02AE-1A)"
    );

    // 2. A named address wins over everything: that is how a second person
    //    on one machine says who they are.
    assert_eq!(
        acting_member(root, &project, Some(" bea@example.com ")).unwrap(),
        "bea@example.com",
        "the named address wins, trimmed"
    );

    // 3. git config appears and names a DIFFERENT member of this
    //    project. `acting_member` still offers the pin first (a person
    //    can still overrule it, and often should), but `resolve_identity`
    //    now follows the config: this is the one point in the file where
    //    the two disagree, and it is the correction's whole point.
    add_member(root, "bea@example.com");
    let project = joy_core::store::load_project(root).unwrap();
    git_config_says(home.path(), "bea@example.com");
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "a@b.c",
        "acting_member is unaffected by this correction: the pin still \
         wins over the git config prefill"
    );
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com",
        "resolve_identity reads git config now, regardless of the pin"
    );

    // 4. The same project on a machine that pinned nobody, a fresh clone
    //    or a second machine. `acting_member`'s prefill is the git config
    //    address, unchanged; `resolve_identity` answers the very same
    //    member, because it was reading the config all along and never
    //    the pin.
    forget_the_pin(root);
    assert_eq!(pinned_member(root, &project), None);
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "bea@example.com",
        "the prefill a person is offered is still the git config address"
    );
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com",
        "forgetting the pin changes nothing here: resolve_identity was \
         never reading it"
    );

    // 5. A pin for somebody this project does not know is no answer at
    //    all: a member who was removed, or a project rekeyed to anonymous
    //    ids, must not keep deciding. This is `acting_member`'s own
    //    guard, unaffected by the correction; `resolve_identity` does
    //    not notice the pin change either way.
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
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com"
    );

    // 6. Bea authenticates here, on a machine whose git config already
    //    names her: the pin is written anyway (still true; a future
    //    cleanup may retire the write, see identity.rs's note on
    //    `pin_acting_member`). Removing `user.email` now DOES change
    //    what `resolve_identity` answers, which is the inverse of what
    //    this file asserted before the correction: the pin no longer
    //    stands in for a git config that went away.
    pin_acting_member(root, &project, "bea@example.com");
    assert_eq!(
        pinned_member(root, &project).as_deref(),
        Some("bea@example.com")
    );
    forget_the_git_config(home.path());
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "bea@example.com",
        "acting_member still has the pin to fall back on"
    );
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "",
        "removing user.email now DOES change resolve_identity's answer: \
         the pin behind acting_member is not a source resolve_identity \
         reads (JOY-02AE-1A)"
    );

    // ...and the next person who authenticates here replaces the pin,
    // which is still the one way `acting_member`'s answer changes absent
    // a config or a name; `resolve_identity` needs the config back too.
    git_config_says(home.path(), "bea@example.com");
    pin_acting_member(root, &project, "a@b.c");
    assert_eq!(pinned_member(root, &project).as_deref(), Some("a@b.c"));
    assert_eq!(acting_member(root, &project, None).unwrap(), "a@b.c");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com",
        "resolve_identity follows the config, not the pin `acting_member` just changed"
    );

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
    assert_eq!(resolve_identity(solo.path()).unwrap().member.id(), "");
    let err = acting_member_key(solo.path()).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
    let err = acting_human_key(solo.path()).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
}
