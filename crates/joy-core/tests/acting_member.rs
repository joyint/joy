// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Two different questions that both live in `identity.rs`, and how the
//! operator's 2026-09-19 correction (JOY-02AE-1A, correcting D3.9 of the
//! forge connection NG design, JOY-0297-1A) settled them the same way in
//! the end:
//!
//! - [`resolve_identity`] answers "who is acting right now": the
//!   delegation session first, then git config (repository before
//!   global), then the forge account, and nothing else.
//! - [`acting_member`] answers a narrower, earlier question: "who does a
//!   bare `joy auth`, `joy auth init` or `joy auth --otp` act as, before
//!   a name settled it". A first pass at the correction left this one
//!   reading the device pin the other had already dropped, which meant
//!   the two disagreed on a machine whose pin and git config named
//!   different members; a later addition to the same item retired the
//!   pin from here too, so this function now reads the SAME sources in
//!   the SAME order the other one does.
//!
//! They still differ in what they hand back, and that is the whole
//! content of this file now: [`resolve_identity`] answers with a member
//! KEY it has already checked against `project.yaml`; [`acting_member`]
//! answers with the raw candidate address, unchecked, because its
//! callers ask the question earlier (before a member necessarily
//! exists) or do a fuller check of their own afterward (`joy auth`'s
//! passphrase path also tries the forge-alias fallback of
//! `member_key_for_email_or_forge`, which is how a forge alias sitting
//! in git config still finds its member).
//!
//! Unix only: the forge-account step's stub connector is a shell
//! script, exactly as `resolve_identity_order.rs`'s is.
//!
//! ONE test in its own binary, on purpose. The question is about process
//! state that has no per-thread version: HOME, libgit2's config search
//! paths, the plugin search directories and the working directory decide
//! what `git config user.email` and the forge account answer, and the
//! case only means something when the test moves them step by step.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use joy_core::forge_plugins;
use joy_core::identity::{acting_human_key, acting_member, acting_member_key, resolve_identity};
use joy_core::init::{init, InitOptions};
use joy_core::model::project::{Member, MemberCapabilities};

/// A machine with no git identity anywhere: no global config, no XDG
/// config, no system config, and a working directory outside every
/// repository. Copied from `resolve_identity_order.rs`'s helper of the
/// same name and purpose.
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

/// Write the repository's OWN `user.email` (`git config --local`).
fn git_config_says_locally(root: &Path, email: &str) {
    joy_core::vcs::forge::local_config_set(root, "user.email", email).unwrap();
}

/// Write a global `user.email`, the way `git config --global` would.
fn git_config_says_globally(home: &Path, email: &str) {
    std::fs::write(
        home.join(".gitconfig"),
        format!("[user]\n\temail = {email}\n\tname = Somebody\n"),
    )
    .unwrap();
}

fn forget_the_local_git_config(root: &Path) {
    let repo = git2::Repository::open(root).unwrap();
    let mut local = repo
        .config()
        .unwrap()
        .open_level(git2::ConfigLevel::Local)
        .unwrap();
    // Absent to begin with in places; either way the key must not
    // survive this call.
    let _ = local.remove("user.email");
}

fn forget_the_global_git_config(home: &Path) {
    let path = home.join(".gitconfig");
    if path.exists() {
        std::fs::remove_file(path).unwrap();
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

/// Write one executable stub and hand back its path (copied from
/// `resolve_identity_order.rs`'s helper of the same name and purpose).
fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    {
        let mut file = std::fs::File::create(&path).expect("write the stub");
        file.write_all(body.as_bytes()).expect("write the stub");
        file.flush().expect("flush the stub");
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make the stub executable");
    path
}

/// A minimal protocol 2 connector body: `claims_json` and `identity_json`
/// are the raw answers to those two verbs.
fn plugin_stub(claims_json: &str, identity_json: &str) -> String {
    let template = r#"#!/bin/sh
if [ "$1" = version ]; then
  echo '{"protocol":2,"plugin":"test stub","forges":["github","gitlab","gitea"]}'
  exit 0
fi
shift
verb="$1"
shift
case "$verb" in
  claims) echo '__CLAIMS__' ;;
  identity) echo '__IDENTITY__' ;;
  *) echo '{}' ;;
esac
"#;
    template
        .replace("__CLAIMS__", claims_json)
        .replace("__IDENTITY__", identity_json)
}

#[test]
fn acting_member_follows_the_same_order_resolve_identity_does() {
    let home = a_machine_without_a_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    found(root, "a@b.c");
    let project = joy_core::store::load_project(root).unwrap();

    // With the device pin gone from here too, founding leaves nothing
    // for `acting_member` to answer with either: no name, no git config,
    // no forge account. The two functions agree on this now, which they
    // did not in the first pass at this correction.
    let err = acting_member(root, &project, None).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
    assert_eq!(resolve_identity(root).unwrap().member.id(), "");

    // 1. A named address wins over everything, whether or not it is a
    //    member yet: that is how a person about to enrol, or a second
    //    person on one machine, says who they are.
    assert_eq!(
        acting_member(root, &project, Some(" bea@example.com ")).unwrap(),
        "bea@example.com",
        "the named address wins, trimmed, and unvalidated"
    );

    // 2. The repository's own git config, once nothing was named.
    add_member(root, "bea@example.com");
    let project = joy_core::store::load_project(root).unwrap();
    git_config_says_locally(root, "bea@example.com");
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "bea@example.com"
    );
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com",
        "the two now agree: both read the same git config"
    );

    // A name still beats a git config that is already set.
    assert_eq!(
        acting_member(root, &project, Some("carol@example.com")).unwrap(),
        "carol@example.com"
    );

    // 3. No local config; the person's global one answers instead,
    //    exactly as resolve_identity's own git2 read does (one config,
    //    local before global before system).
    forget_the_local_git_config(root);
    add_member(root, "carol@example.com");
    let project = joy_core::store::load_project(root).unwrap();
    git_config_says_globally(home.path(), "carol@example.com");
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "carol@example.com"
    );
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "carol@example.com"
    );

    // Local shadows global when both are set and disagree: the same
    // precedence resolve_identity relies on, checked here too now that
    // acting_member shares the same git config read.
    git_config_says_locally(root, "bea@example.com");
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "bea@example.com",
        "the repository's own config wins over the global one"
    );
    forget_the_local_git_config(root);
    forget_the_global_git_config(home.path());

    // 4. Neither local nor global names anybody; the forge account for
    //    the remote's host answers instead. This is new for
    //    acting_member since the pin's removal: it used to give up right
    //    here, offering nothing beyond the pin and the git config
    //    prefill.
    let repo = git2::Repository::open(root).unwrap();
    repo.remote("origin", "git@github.example.com:o/r.git")
        .unwrap();
    let plugins = tempfile::tempdir().unwrap();
    let stub_path = stub(
        plugins.path(),
        forge_plugins::COMBINED_BINARY,
        &plugin_stub(
            r#"{"claims":true}"#,
            r#"{"known":true,"login":"dana","user_id":"1","emails":["dana@example.com"]}"#,
        ),
    );
    std::env::remove_var(forge_plugins::PLUGIN_DIR_ENV);
    forge_plugins::set_plugin_dirs(vec![plugins.path().to_path_buf()]);
    assert_eq!(
        acting_member(root, &project, None).unwrap(),
        "dana@example.com",
        "the forge account is asked last, and answers when git config does not"
    );
    // dana is not a member of this project yet: acting_member hands back
    // the raw candidate regardless, but resolve_identity checks first
    // and still answers with nobody.
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "",
        "acting_member's candidate is unchecked; resolve_identity still validates"
    );
    add_member(root, "dana@example.com");
    let project = joy_core::store::load_project(root).unwrap();
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "dana@example.com",
        "now that dana is a member, the two agree again"
    );

    // 5. Neither git config nor the forge account names anybody: joy
    //    says it does not know, and names the way out. The project file
    //    is never guessed from, because it travels with every clone.
    stub(
        stub_path.parent().unwrap(),
        forge_plugins::COMBINED_BINARY,
        &plugin_stub(r#"{"claims":true}"#, r#"{"known":false}"#),
    );
    let err = acting_member(root, &project, None).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
    assert!(err.to_string().contains("--user <address>"), "{err}");
    assert_eq!(resolve_identity(root).unwrap().member.id(), "");
    let err = acting_member_key(root).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
    let err = acting_human_key(root).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
}
