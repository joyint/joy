// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Bringing a project from before the member files over (JI-019D-46).
//!
//! Such a project keeps its whole member map in project.yaml, writes an
//! AI member by its legacy form ([`joy_model::migrations::ai_member_name`]), and has a
//! manager's attestation over every entry. Brought over, it has one file per member, AI members known by
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
    is_ai_member, Member, MemberCapabilities, MemberLayout, Origin, Project,
};
use crate::store;
use joy_model::{migrations::ai_member_name, InteractionLevel};

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

/// The member as its file will have it. Unsigned: [`migrate`] signs the
/// AI members afterwards.
fn convert(
    project_id: &str,
    anonymous: bool,
    head: Option<&str>,
    key: &str,
    mut member: Member,
) -> Member {
    let ai = is_ai_member(key);
    // A person of an anonymous project is already known everywhere by an
    // id; it stays, and becomes the file's name. Everybody else gets one
    // that two people bringing the same project over arrive at alike:
    // for an AI member over its legacy form, which is what such a project
    // has on disk.
    member.file_id = Some(if anonymous && !ai {
        key.to_string()
    } else if ai {
        member_id::migrated_member_file_id(project_id, &ai_member_name::legacy_form(key))
    } else {
        member_id::migrated_member_file_id(project_id, key)
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
    member
}

/// A project converted and waiting for the signatures under what its AI
/// members may do.
pub struct Prepared {
    project: Project,
    renamed: Vec<(String, String)>,
    /// One per AI member, in the order of [`Prepared::payloads`].
    ai_members: Vec<String>,
}

impl Prepared {
    /// What the person signs: one text per AI member, in the order the
    /// signatures are handed to [`finish`].
    pub fn payloads(&self) -> Vec<(String, Vec<u8>)> {
        self.ai_members
            .iter()
            .filter_map(|key| {
                let member = self.project.member_by_key(key)?;
                let bytes = vouch::payload(&self.project, key, member, Occasion::New)?;
                Some((key.clone(), bytes))
            })
            .collect()
    }
}

/// The first half of bringing a project over, for a host whose person
/// holds their key somewhere else (a browser): the converted project
/// and what is to be signed. `None`: there is nothing to do, or
/// `signer_key` is not a person of this project with a key.
pub fn prepare(root: &Path, signer_key: &str) -> Result<Option<Prepared>, JoyError> {
    let mut project = store::load_project(root)?;
    if !is_pending(&project) || is_ai_member(signer_key) {
        return Ok(None);
    }
    let has_a_key = project
        .member_by_key(signer_key)
        .is_some_and(|m| m.verify_key.is_some());
    if !has_a_key {
        return Ok(None);
    }

    let project_id = session::project_id_of(&project);
    let anonymous = project.privacy_mode() == crate::model::project::PrivacyMode::Anonymous;
    let head = crate::vcs::forge::head_oid(root);

    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut members: BTreeMap<String, Member> = BTreeMap::new();
    for (key, member) in project.take_members() {
        let member = convert(&project_id, anonymous, head.as_deref(), &key, member);
        // What is read is the name already; the files that are not
        // read through the model (items, logs, releases) still carry
        // the legacy form and are rewritten below.
        if is_ai_member(&key) {
            renamed.push((ai_member_name::legacy_form(&key), key.clone()));
        }
        members.insert(key, member);
    }
    project.replace_members(members);
    project.set_member_layout(MemberLayout::Files);
    let ai_members = project
        .member_keys()
        .filter(|key| is_ai_member(key))
        .cloned()
        .collect();
    Ok(Some(Prepared {
        project,
        renamed,
        ai_members,
    }))
}

/// The second half: the signatures `signer_key` made over
/// [`Prepared::payloads`], in that order. Each is checked against the
/// signer's key before anything is written.
pub fn finish(
    root: &Path,
    prepared: Prepared,
    signer_key: &str,
    signatures: &[Vec<u8>],
) -> Result<(), JoyError> {
    let Prepared {
        mut project,
        renamed,
        ai_members,
    } = prepared;
    if signatures.len() != ai_members.len() {
        return Err(JoyError::Other(format!(
            "{} AI members to sign for, {} signatures",
            ai_members.len(),
            signatures.len()
        )));
    }
    let signer_public = project
        .member_by_key(signer_key)
        .and_then(|m| m.verify_key.as_deref())
        .ok_or_else(|| JoyError::Other(format!("{signer_key} has no key")))
        .and_then(|hex| Ok(crate::auth::PublicKey::from_hex(hex)?))?;
    for (key, signature) in ai_members.iter().zip(signatures) {
        let mut member = project.member_by_key(key).cloned().expect("converted");
        let bytes = vouch::payload(&project, key, &member, Occasion::New).expect("an AI member");
        signer_public.verify(&bytes, signature).map_err(|_| {
            JoyError::AuthFailed(format!(
                "the signature for {key} was not made with the key of {signer_key}"
            ))
        })?;
        vouch::apply(
            &project,
            signer_key,
            key,
            &mut member,
            Occasion::New,
            signature,
        );
        *project.member_by_key_mut(key).expect("converted") = member;
    }

    store::save_project(root, &project)?;

    // Everything else that names an AI member by its legacy form: the
    // crypt zones in project.yaml, items, jobs, milestones, releases,
    // logs.
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
    Ok(())
}

/// Bring the project at `root` over, signed by the person `signer_key`
/// with `keypair`. `Ok(false)`: there was nothing to do, or the one who
/// asked is not a person of this project with that key (an AI session
/// never brings a project over).
pub fn migrate(root: &Path, signer_key: &str, keypair: &IdentityKeypair) -> Result<bool, JoyError> {
    let Some(prepared) = prepare(root, signer_key)? else {
        return Ok(false);
    };
    let holds_the_key = prepared
        .project
        .member_by_key(signer_key)
        .and_then(|m| m.verify_key.as_deref())
        == Some(keypair.public_key().to_hex().as_str());
    if !holds_the_key {
        return Ok(false);
    }
    let signatures: Vec<Vec<u8>> = prepared
        .payloads()
        .iter()
        .map(|(_, bytes)| keypair.sign(bytes))
        .collect();
    finish(root, prepared, signer_key, &signatures)?;
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
