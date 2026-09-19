// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `resolve_identity`'s order after the operator's 2026-09-19 correction
//! (JOY-02AE-1A, correcting D3.9 of the forge connection NG design): a
//! delegation session first, then git config (repository before
//! global), then the forge account, and nothing else. The device pin of
//! D3.9 is retired from this order for good; no case here depends on it.
//!
//! ONE test in its own binary, on purpose (the same reason as
//! acting_member.rs and git_config_nosystem.rs): HOME, libgit2's config
//! search paths, the plugin search directories and JOY_SESSION are all
//! process state, and the case only means something when it moves them
//! step by step.
//!
//! Unix only: step 4's stub connector is a shell script, exactly as
//! forge_plugin_runner.rs's are; a Windows stub is a different file
//! format and is out of scope here.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use joy_core::auth::redeem::redeem_ai_session;
use joy_core::auth::token::{create_token, encode_token, TokenIssueParams, TokenSigningKeys};
use joy_core::auth::{session, IdentityKeypair};
use joy_core::forge_plugins;
use joy_core::identity::{acting_member_key, resolve_identity};
use joy_core::init::{init, InitOptions};
use joy_core::model::project::{AiDelegationEntry, Member, MemberCapabilities};

/// A machine with no git identity anywhere: no global config, no XDG
/// config, no system config, and a working directory outside every
/// repository. Copied from `acting_member.rs`'s helper of the same name
/// and purpose.
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
fn git_config_says_globally(home: &Path, email: &str) {
    std::fs::write(
        home.join(".gitconfig"),
        format!("[user]\n\temail = {email}\n\tname = Somebody\n"),
    )
    .unwrap();
}

fn forget_the_global_git_config(home: &Path) {
    let path = home.join(".gitconfig");
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

/// Write the repository's OWN `user.email` (`git config --local`).
fn git_config_says_locally(root: &Path, email: &str) {
    joy_core::vcs::forge::local_config_set(root, "user.email", email).unwrap();
}

fn forget_the_local_git_config(root: &Path) {
    let repo = git2::Repository::open(root).unwrap();
    let mut local = repo
        .config()
        .unwrap()
        .open_level(git2::ConfigLevel::Local)
        .unwrap();
    // Absent to begin with on a fresh repository; either way the
    // key must not survive this call.
    let _ = local.remove("user.email");
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
        name: Some("Order".into()),
        acronym: Some("OR".into()),
        user: Some(founder.to_string()),
        ..InitOptions::new(root.to_path_buf())
    })
    .unwrap();
}

/// Write one executable stub and hand back its path (copied from
/// `forge_plugin_runner.rs`'s helper of the same name and purpose).
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
/// are the raw answers to those two verbs, so one template serves both
/// the step 4 match and the "nobody signed in" negative case.
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

/// Register a live, redeemable AI delegation from `human` to `ai` and
/// return the `JOY_SESSION` value for it: the real path (`joy auth token
/// add` then `joy auth --token`) collapsed into one call, the same way
/// `crate::auth::redeem`'s own unit tests build one, so step 1 is
/// exercised through the real redemption code rather than a hand rolled
/// session file.
fn a_delegation_session(root: &Path, human: &str, ai: &str) -> String {
    let delegator = IdentityKeypair::from_seed(&[11u8; 32]);
    let delegation_seed = [12u8; 32];
    let delegation = IdentityKeypair::from_seed(&delegation_seed);

    let path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project = joy_core::store::load_project(root).unwrap();
    project
        .register_member(ai, Member::new(MemberCapabilities::All))
        .unwrap();
    {
        let founder = project
            .member_by_key_mut(human)
            .expect("the human is already a member");
        founder.verify_key = Some(delegator.public_key().to_hex());
        founder.ai_delegations.insert(
            ai.to_string(),
            AiDelegationEntry {
                delegation_verifier: delegation.public_key().to_hex(),
                delegation_salt: Some("00".repeat(32)),
                created: chrono::Utc::now(),
                rotated: None,
            },
        );
    }
    joy_core::store::write_yaml(&path, &project).unwrap();

    let project_id = session::project_id(root).unwrap();
    let token = encode_token(&create_token(
        TokenSigningKeys {
            delegator: &delegator,
            delegation: &delegation,
            delegation_seed: &delegation_seed,
        },
        TokenIssueParams {
            ai_member: ai,
            human,
            project_id: &project_id,
            ttl: None,
        },
    ));
    let redeemed = redeem_ai_session(&project, &project_id, &token).expect("the token redeems");
    session::save_session(&project_id, &redeemed.token).unwrap();
    redeemed.session_env
}

#[test]
fn the_order_after_joy_02ae_1a_and_its_two_failure_shapes() {
    let home = a_machine_without_a_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Before a project even exists, resolve_identity answers with an
    // empty member: the state a fresh `joy init` starts from, and the
    // precursor of the "no members yet" failure shape the item
    // describes. The person may found themselves as the first member
    // (`joy init --user <address>`, `joy auth init`), which is
    // unaffected by this correction and is exercised in
    // founder_identity.rs, not repeated here.
    assert_eq!(resolve_identity(root).unwrap().member.id(), "");

    found(root, "a@b.c");
    add_member(root, "bea@example.com");
    add_member(root, "carol@example.com");

    // The device pin this project's founding wrote is not read by
    // `resolve_identity` any more (operator decision 2026-09-19,
    // JOY-02AE-1A): forget it up front, so nothing below can be passing
    // by accident because of it.
    let pin = session::app_state_project_file(root).unwrap();
    if pin.exists() {
        std::fs::remove_file(&pin).unwrap();
    }
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "",
        "no git config and no forge account yet: nobody answers, pin or not"
    );

    // Step 1: a delegation session outranks everything, even a git
    // config that already names a different, human member. Unchanged by
    // this correction, and checked here because the member key it used
    // to compare against (the pin) is gone.
    git_config_says_locally(root, "a@b.c");
    let session_env = a_delegation_session(root, "a@b.c", "ai:claude@joy");
    std::env::set_var("JOY_SESSION", &session_env);
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "ai:claude@joy",
        "a live delegation session outranks git config"
    );
    std::env::remove_var("JOY_SESSION");

    // Step 2: the repository's own git config, once no session answers.
    git_config_says_locally(root, "bea@example.com");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com"
    );

    // Step 3: no local config; the person's global one answers instead.
    forget_the_local_git_config(root);
    git_config_says_globally(home.path(), "carol@example.com");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "carol@example.com",
        "a value only the global file carries is found the moment the \
         local file has none"
    );

    // Local shadows global when both are set and disagree: git2's own
    // config precedence, which is the whole reason steps 2 and 3 are one
    // function rather than two. Guards against a future change
    // accidentally reading the global file first.
    git_config_says_locally(root, "bea@example.com");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com",
        "the repository's own config wins over the global one"
    );
    forget_the_local_git_config(root);
    forget_the_global_git_config(home.path());

    // Step 4: neither local nor global names a member; the forge account
    // for the remote's host does. The stub plays the connector and
    // shadows whatever real gh/glab/tea backed plugin this machine may
    // have installed: `set_plugin_dirs` is searched before PATH (D2.2),
    // so the stub answers even where a real one exists.
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
    add_member(root, "dana@example.com");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "dana@example.com",
        "the forge account is asked last, and answers when git config does not"
    );

    // Failure shape: git config names nobody (already forgotten above)
    // and the forge account names nobody the project knows either. A
    // read command keeps working with an empty member; a write asks for
    // one, and the sentence names the git config the person can set.
    stub(
        stub_path.parent().unwrap(),
        forge_plugins::COMBINED_BINARY,
        &plugin_stub(r#"{"claims":true}"#, r#"{"known":false}"#),
    );
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "",
        "a forge account that names nobody the project knows is no answer"
    );
    let err = acting_member_key(root).unwrap_err();
    assert!(
        matches!(err, joy_core::error::JoyError::UnknownActingMember),
        "{err}"
    );
    assert!(
        err.to_string().contains("git config user.email"),
        "the refusal names the remedy: {err}"
    );
}
