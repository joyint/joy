// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Bringing a project from before the member files over (JI-019D-46).
//!
//! Such a project keeps its whole member map in project.yaml, calls an AI
//! member `ai:<name>@joy`, and has a manager's attestation over every
//! entry. Brought over, it has one file per member, AI members known by
//! their name, an origin for every person and a signed maximum for every
//! AI member.
//!
//! The one thing that cannot be done without a person is the signature
//! under what an AI member may do. So this does not run with the other
//! reconciles when a project is opened: it runs the first time a person
//! is there with their key (a login, an unlock, a command of somebody
//! signed in), in one step and without a word. Until then the project is
//! read and written as it was, by the same code as before, and nothing
//! about it is weaker or stronger. A server holds nobody's key and never
//! runs it.
//!
//! Whoever is first signs the maxima with their own key, whatever they
//! may do in the project: what they sign is what a manager had attested,
//! taken over as it stands, and their name is on it.

use std::collections::BTreeMap;
use std::path::Path;

use crate::auth::vouch::{self, Occasion};
use crate::auth::{session, IdentityKeypair};
use crate::error::JoyError;
use crate::member_id;
use crate::model::project::{
    ai_member_name, is_ai_member, Member, MemberCapabilities, MemberLayout, Origin, Project,
};
use crate::store;
use joy_model::InteractionLevel;

/// Whether the project at `root` still keeps its members in project.yaml.
pub fn pending(root: &Path) -> bool {
    store::load_project(root).is_ok_and(|project| is_pending(&project))
}

fn is_pending(project: &Project) -> bool {
    project.member_layout() == MemberLayout::InProject && project.has_members()
}

/// The level an AI member is brought over with: the most careful floor a
/// manager had set on any of its capabilities, and where none was set,
/// the level a new AI member starts with.
fn level_before(member: &Member) -> InteractionLevel {
    let floors = match &member.capabilities {
        MemberCapabilities::Specific(map) => map
            .values()
            .filter_map(|config| config.max_interaction_level)
            .max(),
        MemberCapabilities::All => None,
    };
    floors.unwrap_or(vouch::DEFAULT_AI_LEVEL)
}

/// The member as its file will have it, and the key it is known by.
/// Unsigned: [`migrate`] signs the AI members afterwards.
fn convert(
    project_id: &str,
    anonymous: bool,
    head: Option<&str>,
    old_key: &str,
    mut member: Member,
) -> (String, Member) {
    let ai = is_ai_member(old_key);
    let key = if ai {
        ai_member_name(old_key).to_string()
    } else {
        old_key.to_string()
    };
    // A person of an anonymous project is already known everywhere by an
    // id; it stays, and becomes the file's name. Everybody else gets one
    // that two people bringing the same project over arrive at alike.
    member.file_id = Some(if anonymous && !ai {
        old_key.to_string()
    } else {
        member_id::migrated_member_file_id(project_id, old_key)
    });
    let attestation = member.attestation.take();
    if ai {
        member.interaction_level = Some(level_before(&member));
    } else {
        member.interaction_level = None;
        // Who invited them is what the attestation of that time says.
        // Its signature covers what it covered then and stays readable
        // in the commit before this one.
        member.origin = attestation.map(|att| Origin {
            attester: att.attester,
            signed_at: att.signed_at,
            signature: None,
            invitation: None,
            commit: head.map(str::to_string),
        });
    }
    // No setting hangs on a single capability any more.
    if let MemberCapabilities::Specific(map) = &mut member.capabilities {
        for config in map.values_mut() {
            *config = Default::default();
        }
    }
    // The AI members a person delegated to are known by their names now.
    member.ai_delegations = std::mem::take(&mut member.ai_delegations)
        .into_iter()
        .map(|(ai, entry)| (ai_member_name(&ai).to_string(), entry))
        .collect();
    (key, member)
}

/// Bring the project at `root` over, signed by the person `signer_key`
/// with `keypair`. `Ok(false)`: there was nothing to do, or the one who
/// asked is not a person of this project with that key (an AI session
/// never brings a project over).
pub fn migrate(root: &Path, signer_key: &str, keypair: &IdentityKeypair) -> Result<bool, JoyError> {
    let mut project = store::load_project(root)?;
    if !is_pending(&project) || is_ai_member(signer_key) {
        return Ok(false);
    }
    let holds_the_key = project
        .member_by_key(signer_key)
        .and_then(|m| m.verify_key.as_deref())
        == Some(keypair.public_key().to_hex().as_str());
    if !holds_the_key {
        return Ok(false);
    }

    let project_id = session::project_id_of(&project);
    let anonymous = project.privacy_mode() == crate::model::project::PrivacyMode::Anonymous;
    let head = crate::vcs::forge::head_oid(root);

    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut members: BTreeMap<String, Member> = BTreeMap::new();
    for (old_key, member) in project.take_members() {
        let (key, member) = convert(&project_id, anonymous, head.as_deref(), &old_key, member);
        if key != old_key {
            renamed.push((old_key, key.clone()));
        }
        members.insert(key, member);
    }
    project.replace_members(members);
    project.set_member_layout(MemberLayout::Files);

    // The signature under what each AI member may do: the first person
    // who is there with a key.
    let ai_members: Vec<String> = project
        .member_keys()
        .filter(|key| is_ai_member(key))
        .cloned()
        .collect();
    for key in ai_members {
        let mut member = project.member_by_key(&key).cloned().expect("just inserted");
        vouch::sign(
            &project,
            signer_key,
            keypair,
            &key,
            &mut member,
            Occasion::New,
        );
        *project.member_by_key_mut(&key).expect("just inserted") = member;
    }

    store::save_project(root, &project)?;

    // Everything else that names an AI member the old way: the crypt
    // zones in project.yaml, items, jobs, milestones, releases, logs.
    let joy = store::joy_dir(root);
    crate::privacy::rewrite_project_keeping_the_member_list(
        &joy.join(store::PROJECT_FILE),
        &renamed,
    )?;
    let mut staged = vec![format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE)];
    for (dir, ext) in [
        (store::ITEMS_DIR, "yaml"),
        (store::JOBS_DIR, "yaml"),
        (store::MILESTONES_DIR, "yaml"),
        (store::RELEASES_DIR, "yaml"),
        (store::LOG_DIR, "log"),
    ] {
        crate::privacy::rewrite_dir(&joy.join(dir), ext, &renamed)?;
        if joy.join(dir).is_dir() {
            staged.push(format!("{}/{dir}", store::JOY_DIR));
        }
    }
    let staged: Vec<&str> = staged.iter().map(String::as_str).collect();
    crate::git_ops::auto_git_add(root, &staged);
    Ok(true)
}

/// [`migrate`] for a caller that must not fail because of it: a login
/// that worked stays a login that worked. What went wrong is said on
/// stderr, and the project stays as it was.
pub fn migrate_quietly(root: &Path, signer_key: &str, keypair: &IdentityKeypair) {
    if let Err(e) = migrate(root, signer_key, keypair) {
        eprintln!("Warning: this project's members could not be moved to their own files: {e}");
    }
}
