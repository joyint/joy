// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `resolve_identity`'s order after the operator's 2026-09-19 correction
//! (JOY-02AE-1A) and
//! the operator's addition of 2026-09-27: a delegation session first,
//! then the person who signed in at this terminal, then git config
//! (repository before global), then the forge account, and nothing
//! else. The device pin is retired from this order for good; no
//! case here depends on it.
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

// Nothing of the developer's shell and session reaches this binary
// (JOY-02BB-C7).
joy_test_env::isolate!();

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

/// Sign `member` in at this terminal: give them a verify key and save the
/// session `joy auth --user <member>` would leave behind, signed with
/// the matching identity key. The seed is the member's own, so two
/// members never share a key.
fn a_human_session(root: &Path, member: &str) {
    let mut seed = [21u8; 32];
    for (at, byte) in member.bytes().enumerate().take(32) {
        seed[at] ^= byte;
    }
    let keypair = IdentityKeypair::from_seed(&seed);
    let path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let mut project = joy_core::store::load_project(root).unwrap();
    project
        .member_by_key_mut(member)
        .expect("the member is registered")
        .verify_key = Some(keypair.public_key().to_hex());
    joy_core::store::write_yaml(&path, &project).unwrap();
    let project_id = session::project_id(root).unwrap();
    let token = session::create_session(&keypair, member, &project_id, None);
    session::save_session(&project_id, &token).unwrap();
}

fn sign_out(root: &Path, member: &str) {
    let project_id = session::project_id(root).unwrap();
    session::remove_session(&project_id, member).unwrap();
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
                grant: None,
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
            grant: None,
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

    // Step 2 on its own (operator, 2026-09-27): with no git identity
    // anywhere, the person who signed in at this terminal is the
    // member, authenticated. This is what "name yourself once" means on
    // a machine with no git config: `joy auth --user <address>` leaves
    // exactly this session behind.
    a_human_session(root, "bea@example.com");
    let signed_in = resolve_identity(root).unwrap();
    assert_eq!(
        signed_in.member.id(),
        "bea@example.com",
        "a session names the member with no git config at all"
    );
    assert!(signed_in.authenticated);
    assert_eq!(signed_in.delegated_by, None);

    // Step 1: a delegation session outranks everything, even a git
    // config that already names a different, human member, and even the
    // human session that just answered. Unchanged by either correction,
    // and checked here because the member key it used to compare
    // against (the pin) is gone.
    git_config_says_locally(root, "a@b.c");
    let session_env = a_delegation_session(root, "a@b.c", "ai:claude@joy");
    std::env::set_var("JOY_SESSION", &session_env);
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "ai:claude@joy",
        "a live delegation session outranks git config and a human session"
    );

    // A delegation that no longer stands never turns the agent into the
    // person who signed in here. Bea's session is live at this very
    // terminal, and the process still carries JOY_SESSION: it goes on as
    // unproven as git config, and never as bea.
    let project_file = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    let with_delegation = std::fs::read_to_string(&project_file).unwrap();
    let mut project = joy_core::store::load_project(root).unwrap();
    project
        .member_by_key_mut("a@b.c")
        .unwrap()
        .ai_delegations
        .clear();
    joy_core::store::write_yaml(&project_file, &project).unwrap();
    let cut_off = resolve_identity(root).unwrap();
    assert_eq!(
        cut_off.member.id(),
        "a@b.c",
        "a removed delegation leaves the agent with what git config says"
    );
    assert!(
        !cut_off.authenticated,
        "and never with the session a person made here"
    );
    std::fs::write(&project_file, with_delegation).unwrap();
    // the same for a value that names no session at all
    std::env::set_var("JOY_SESSION", "joy_s_not-a-session");
    let nobody = resolve_identity(root).unwrap();
    assert_ne!(nobody.member.id(), "bea@example.com");
    assert!(!nobody.authenticated);
    std::env::remove_var("JOY_SESSION");

    // Step 2 before step 3: git config names a@b.c, bea's session names
    // bea, and bea it is. This is how a second person acts at a checkout
    // whose config names the first, without touching that config.
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com",
        "the session of this terminal outranks git config"
    );

    // A second sign-in replaces the first: `save_session` keeps one
    // person per project and device (operator, 2026-09-27), so carol's
    // session is the only one left and the one that answers.
    a_human_session(root, "carol@example.com");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "carol@example.com",
        "the latest sign-in is the one that stands"
    );
    let project_id = session::project_id(root).unwrap();
    assert!(
        session::load_session(&project_id, "bea@example.com")
            .unwrap()
            .is_none(),
        "bea's session went when carol signed in"
    );

    // Step 0: a name on the call itself (`--user`, carried as JOY_USER)
    // outranks every session and is never remembered: unauthenticated,
    // like git config, and gone with the variable.
    std::env::set_var("JOY_USER", "a@b.c");
    let named = resolve_identity(root).unwrap();
    assert_eq!(named.member.id(), "a@b.c", "the name on the call wins");
    assert!(!named.authenticated, "a name proves nothing by itself");
    std::env::remove_var("JOY_USER");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "carol@example.com",
        "nothing of the name remains"
    );

    // A session that ran out is not honoured, and it is cleared away the
    // first time it is met, so its end is said once and not on every
    // command after it.
    {
        let keypair = IdentityKeypair::from_seed(&[33u8; 32]);
        let project_id = session::project_id(root).unwrap();
        let ended = session::create_session(
            &keypair,
            "bea@example.com",
            &project_id,
            Some(chrono::Duration::hours(-1)),
        );
        // saving it replaces carol's, as every sign-in does
        session::save_session(&project_id, &ended).unwrap();
        assert!(session::load_session(&project_id, "bea@example.com")
            .unwrap()
            .is_some());
        let after = resolve_identity(root).unwrap();
        assert_eq!(after.member.id(), "a@b.c", "an ended session names nobody");
        assert!(!after.authenticated);
        assert!(
            session::load_session(&project_id, "bea@example.com")
                .unwrap()
                .is_none(),
            "and it is removed once it was met"
        );
    }

    // Signing out gives git config its turn again.
    sign_out(root, "carol@example.com");
    sign_out(root, "bea@example.com");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "a@b.c",
        "with the sessions gone, git config answers"
    );
    assert!(!resolve_identity(root).unwrap().authenticated);

    // Step 3: the repository's own git config, once no session answers.
    git_config_says_locally(root, "bea@example.com");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "bea@example.com"
    );

    // Step 4: no local config; the person's global one answers instead.
    forget_the_local_git_config(root);
    git_config_says_globally(home.path(), "carol@example.com");
    assert_eq!(
        resolve_identity(root).unwrap().member.id(),
        "carol@example.com",
        "a value only the global file carries is found the moment the \
         local file has none"
    );

    // Local shadows global when both are set and disagree: git2's own
    // config precedence, which is the whole reason steps 3 and 4 are one
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

    // Step 5: neither local nor global names a member; the forge account
    // for the remote's host does. The stub plays the connector and
    // shadows whatever real gh/glab/tea backed plugin this machine may
    // have installed: `set_plugin_dirs` is searched before PATH,
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
        err.to_string().contains("--user <address>"),
        "the refusal names the remedy: {err}"
    );
}
