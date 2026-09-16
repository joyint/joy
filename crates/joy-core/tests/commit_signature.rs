// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What a commit joy writes is signed with (D4.5 of the forge connection
//! NG design, JOY-0297-1A): the acting member decides both fields, git
//! config is a prefill for the display name and nothing else, and an
//! anonymous project never lets an address into a commit.

use std::path::Path;

use joy_core::auth::{generate_salt, seed as seed_mod, IdentityKeypair};
use joy_core::init::{self, InitOptions};
use joy_core::model::project::Project;

/// A project with one enrolled founder, as `joy init` plus `joy auth init`
/// leave it. Returns the checkout and the founder's seed.
fn founded(root: &Path, founder: &str) -> [u8; 32] {
    init::init(InitOptions {
        name: Some("Signed".into()),
        acronym: Some("SG".into()),
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
            seed_wrap_passphrase: seed_mod::wrap_seed_with_passphrase(&seed, "a b c d", &salt)
                .unwrap(),
            seed_wrap_recovery: seed_mod::wrap_seed_with_recovery(&seed, &recovery, &salt).unwrap(),
        },
    )
    .unwrap();
    write(root, &project);
    *seed.as_bytes()
}

fn write(root: &Path, project: &Project) {
    let path = joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE);
    joy_core::store::write_yaml(&path, project).unwrap();
}

/// The two signature fields of the commit at HEAD.
fn head_signature(root: &Path) -> (String, String, String, String) {
    let repo = git2::Repository::open(root).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    let author = commit.author();
    let committer = commit.committer();
    (
        author.name().unwrap().to_string(),
        author.email().unwrap().to_string(),
        committer.name().unwrap().to_string(),
        committer.email().unwrap().to_string(),
    )
}

/// The acceptance of J9: a commit in an anonymous mode project carries the
/// opaque `m-<id>` in BOTH signature fields, whatever git config says.
#[test]
fn a_commit_in_an_anonymous_project_carries_the_opaque_id_in_both_fields() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let seed = founded(root, "scotty@example.com");

    // The checkout's git config names the person by name and address.
    // Neither may reach a commit in this project.
    joy_core::vcs::forge::local_config_set(root, "user.name", "Scotty").unwrap();
    joy_core::vcs::forge::local_config_set(root, "user.email", "scotty@example.com").unwrap();

    let mut project = joy_core::store::load_project(root).unwrap();
    let renamed = joy_core::privacy::switch_to_anonymous(root, &mut project, &seed).unwrap();
    let opaque = renamed
        .into_iter()
        .find(|(email, _)| email == "scotty@example.com")
        .map(|(_, id)| id)
        .expect("the founder was rekeyed");
    assert!(joy_core::member_id::is_opaque_member_id(&opaque));

    let (name, email) = joy_core::identity::commit_signature(root, &opaque).unwrap();
    assert_eq!(name, opaque);
    assert_eq!(email, opaque);

    joy_core::vcs::forge::stage_paths(root, &[".joy"]).unwrap();
    joy_core::vcs::forge::commit_index(root, "joy: anonymous", &name, &email).unwrap();

    let (author_name, author_email, committer_name, committer_email) = head_signature(root);
    assert_eq!(author_name, opaque);
    assert_eq!(author_email, opaque);
    assert_eq!(committer_name, opaque);
    assert_eq!(committer_email, opaque);
    for field in [author_name, author_email, committer_name, committer_email] {
        assert!(!field.contains('@'), "{field} must carry no address");
        assert!(!field.contains("Scotty"), "{field} must carry no name");
    }
}

/// Open mode: the e-mail is the member, and the git config name rides
/// along only while it maps to that very member.
#[test]
fn an_open_project_signs_with_the_member_and_a_name_it_can_attribute() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    founded(root, "scotty@example.com");

    joy_core::vcs::forge::local_config_set(root, "user.name", "Scotty").unwrap();
    joy_core::vcs::forge::local_config_set(root, "user.email", "scotty@example.com").unwrap();
    assert_eq!(
        joy_core::identity::commit_signature(root, "scotty@example.com").unwrap(),
        ("Scotty".to_string(), "scotty@example.com".to_string())
    );

    // The same checkout, configured for somebody else: the name no longer
    // maps to the acting member, so the member id stands in for it and
    // the commit stays attributable.
    joy_core::vcs::forge::local_config_set(root, "user.name", "Somebody Else").unwrap();
    joy_core::vcs::forge::local_config_set(root, "user.email", "else@example.com").unwrap();
    assert_eq!(
        joy_core::identity::commit_signature(root, "scotty@example.com").unwrap(),
        (
            "scotty@example.com".to_string(),
            "scotty@example.com".to_string()
        )
    );
}
