// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What an AI member may do, for whom (JI-019D-46).
//!
//! Two people stand behind an AI member's actions, and each of them
//! signed what they allow:
//!
//! * A person with the manage capability signed the **project maximum**:
//!   the capabilities and the interaction level in the AI member's own
//!   entry ([`super::vouch`]).
//! * The person the AI acts for may have signed a **grant** of their
//!   own, within that maximum: fewer capabilities, a lower level. It
//!   sits in their delegation entry, and the token they issue names it,
//!   so it cannot be dropped behind their back.
//!
//! What the AI may do for that person is what both allow: the
//! capabilities in both lists, and the level that leaves the person
//! more say. [`effective`] is the one place that works it out; the
//! guard, the turn and the job all ask it.
//!
//! Neither signature can be made by the AI: it holds no person's key,
//! and a signature made with an AI member's key counts for nothing.

use chrono::Utc;

use super::{session, vouch, IdentityKeypair, PublicKey};
use crate::error::JoyError;
use crate::model::item::Capability;
use crate::model::project::{
    ai_member_name, grant_text, DelegationGrant, Member, MemberLayout, Project,
};
use joy_model::InteractionLevel;

/// What a token says when the person who issued it had no grant of
/// their own: the project maximum applies as it stands.
pub const NO_GRANT: &str = "none";

/// What an AI member may do for the person it acts for.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    pub capabilities: Vec<Capability>,
    pub level: InteractionLevel,
}

impl Effective {
    pub fn allows(&self, capability: &Capability) -> bool {
        self.capabilities.contains(capability)
    }
}

/// The level that leaves the person more say.
fn more_oversight(a: InteractionLevel, b: InteractionLevel) -> InteractionLevel {
    a.max(b)
}

/// The text a person's own grant is signed over. Whose grant it is, is
/// said by their public key: that stays the same when a project turns
/// anonymous and their id changes.
fn personal_text(
    project: &Project,
    ai: &str,
    capabilities: &[Capability],
    level: InteractionLevel,
    delegator_key_hex: &str,
) -> String {
    grant_text(
        ai_member_name(ai),
        capabilities,
        level,
        delegator_key_hex,
        &session::project_id_of(project),
    )
}

/// What a token carries for this grant, in words anybody can read: the
/// capabilities and the level (`review,create|proposing`), or
/// [`NO_GRANT`]. It rides inside the token's signed claims. The guard
/// holds the delegator's entry against it, so a grant that was dropped
/// or changed after the token was issued shows; that the entry itself
/// is what the person signed is checked on its own.
pub fn grant_hash(grant: Option<&DelegationGrant>) -> String {
    match grant {
        None => NO_GRANT.to_string(),
        Some(grant) => grant_claim(&grant.capabilities, grant.level),
    }
}

/// [`grant_hash`] of a grant with these capabilities and this level,
/// for a host that has to say it before the grant is signed (the token
/// minted in the same gesture names it).
pub fn grant_claim(capabilities: &[Capability], level: InteractionLevel) -> String {
    let mut caps: Vec<Capability> = capabilities.to_vec();
    caps.sort();
    caps.dedup();
    let caps: Vec<String> = caps.iter().map(|c| c.to_string()).collect();
    format!("{}|{level}", caps.join(","))
}

/// What the token a person issues for `ai` right now has to carry.
pub fn token_grant(delegator: &Member, ai: &str) -> String {
    grant_hash(delegator.delegation_to(ai).and_then(|d| d.grant.as_ref()))
}

/// The project maximum of an AI member, once its signature is checked.
fn maximum(project: &Project, ai_key: &str) -> Result<Effective, String> {
    let name = ai_member_name(ai_key);
    let member = project
        .member_by_key(ai_key)
        .ok_or_else(|| format!("{name} is not a member of this project"))?;
    vouch::verify_maximum(project, ai_key, member)?;
    Ok(Effective {
        capabilities: vouch::capability_list(&member.capabilities),
        level: vouch::maximum_level(member),
    })
}

/// The capabilities and the level a person's own grant for `ai` is to
/// have, checked against the project maximum: what is not said stays
/// what their grant says now, or what the project allows.
fn personal_target(
    project: &Project,
    ai_key: &str,
    delegator: &Member,
    capabilities: Option<&[Capability]>,
    level: Option<InteractionLevel>,
) -> Result<(Vec<Capability>, InteractionLevel), JoyError> {
    let name = ai_member_name(ai_key);
    let max = maximum(project, ai_key).map_err(JoyError::Other)?;
    let current = delegator
        .delegation_to(ai_key)
        .and_then(|d| d.grant.as_ref());
    let mut caps: Vec<Capability> = match (capabilities, current) {
        (Some(caps), _) => caps.to_vec(),
        (None, Some(grant)) => grant.capabilities.clone(),
        (None, None) => max.capabilities.clone(),
    };
    caps.sort();
    caps.dedup();
    let level = level.or(current.map(|g| g.level)).unwrap_or(max.level);
    let beyond: Vec<String> = caps
        .iter()
        .filter(|cap| !max.allows(cap))
        .map(|cap| cap.to_string())
        .collect();
    if !beyond.is_empty() {
        return Err(JoyError::Other(format!(
            "the project does not allow {name}: {}. It allows: {}",
            beyond.join(", "),
            list(&max.capabilities)
        )));
    }
    if more_oversight(level, max.level) != level {
        return Err(JoyError::Other(format!(
            "the project allows {name} at most {}, not {level}",
            max.level
        )));
    }
    Ok((caps, level))
}

fn delegator_key_hex(delegator: &Member) -> Result<&str, JoyError> {
    delegator
        .verify_key
        .as_deref()
        .ok_or_else(|| JoyError::Other("you have no key yet: run `joy auth init`".into()))
}

fn not_delegated(name: &str) -> JoyError {
    JoyError::Other(format!(
        "you have not delegated to {name} yet: joy auth token add {name}"
    ))
}

/// What a person signs to set their own grant for `ai`, for a host
/// whose key is somewhere else (a browser): the bytes, and the
/// capabilities and level they stand for. [`apply_personal`] takes the
/// signature.
pub fn personal_payload(
    project: &Project,
    ai_key: &str,
    delegator_key: &str,
    capabilities: Option<&[Capability]>,
    level: Option<InteractionLevel>,
) -> Result<(Vec<u8>, Vec<Capability>, InteractionLevel), JoyError> {
    let delegator = project
        .member_by_key(delegator_key)
        .ok_or_else(|| JoyError::Other(format!("{delegator_key} is not a member")))?;
    if delegator.delegation_to(ai_key).is_none() {
        return Err(not_delegated(ai_member_name(ai_key)));
    }
    let (caps, level) = personal_target(project, ai_key, delegator, capabilities, level)?;
    let text = personal_text(project, ai_key, &caps, level, delegator_key_hex(delegator)?);
    Ok((text.into_bytes(), caps, level))
}

/// Record a person's own grant for `ai` with the signature they made
/// over [`personal_payload`].
pub fn apply_personal(
    project: &mut Project,
    ai_key: &str,
    delegator_key: &str,
    capabilities: Vec<Capability>,
    level: InteractionLevel,
    signature: &[u8],
) -> Result<(), JoyError> {
    let delegator = project
        .member_by_key_mut(delegator_key)
        .ok_or_else(|| JoyError::Other(format!("{delegator_key} is not a member")))?;
    let entry = delegator
        .delegation_to_mut(ai_key)
        .ok_or_else(|| not_delegated(ai_member_name(ai_key)))?;
    entry.grant = Some(DelegationGrant {
        capabilities,
        level,
        signed_at: Utc::now(),
        signature: hex::encode(signature),
    });
    Ok(())
}

/// Set what `ai` may do for the person `delegator_key`, signed with
/// their key: at most the project maximum, and the refusal names it.
/// What is `None` stays as it is.
pub fn set_personal(
    project: &mut Project,
    ai_key: &str,
    delegator_key: &str,
    keypair: &IdentityKeypair,
    capabilities: Option<&[Capability]>,
    level: Option<InteractionLevel>,
) -> Result<(), JoyError> {
    let (bytes, caps, level) =
        personal_payload(project, ai_key, delegator_key, capabilities, level)?;
    let signature = keypair.sign(&bytes);
    apply_personal(project, ai_key, delegator_key, caps, level, &signature)
}

/// Drop a person's own grant for `ai`: the project maximum applies to
/// them again.
pub fn clear_personal(project: &mut Project, ai_key: &str, delegator_key: &str) {
    if let Some(entry) = project
        .member_by_key_mut(delegator_key)
        .and_then(|delegator| delegator.delegation_to_mut(ai_key))
    {
        entry.grant = None;
    }
}

/// Set the model `ai` runs on for the person `person_key`, or with
/// `None` leave it to the tool. It is theirs to pick only while the
/// project sets none for everybody; a model the project sets is said in
/// the refusal. Nothing is signed: a model is no permission.
pub fn set_personal_model(
    project: &mut Project,
    ai_key: &str,
    person_key: &str,
    model: Option<&str>,
) -> Result<(), JoyError> {
    let name = ai_member_name(ai_key).to_string();
    if !applies(project) {
        return Err(JoyError::Other(format!(
            "a model of your own for {name} needs the project's member files: \
             sign in once and the project is brought over"
        )));
    }
    let Some(ai) = project.member_by_key(ai_key) else {
        return Err(JoyError::Other(format!("member not found: {name}")));
    };
    if let Some(set) = ai.model.as_deref().filter(|m| !m.trim().is_empty()) {
        return Err(JoyError::Other(format!(
            "the project sets the model of {name} to {set} for everybody"
        )));
    }
    let person_key = match project.member_by_key(person_key) {
        Some(_) => person_key.to_string(),
        None => project
            .member_key_for_email(person_key)
            .ok_or_else(|| JoyError::Other(format!("{person_key} is not a project member")))?,
    };
    let entry = project
        .member_by_key_mut(&person_key)
        .and_then(|m| m.delegation_to_mut(ai_key))
        .ok_or_else(|| {
            JoyError::Other(format!(
                "delegate to {name} first: joy auth token add {name}"
            ))
        })?;
    entry.model = model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    Ok(())
}

/// Change what the project allows the AI member, before a person with
/// the manage capability signs it ([`vouch::sign`] with
/// [`vouch::Occasion::Changed`], or its payload and apply). What is
/// `None` stays as it is. An AI member never holds manage.
pub fn change_maximum(
    member: &mut Member,
    capabilities: Option<&[Capability]>,
    level: Option<InteractionLevel>,
) -> Result<(), JoyError> {
    if let Some(caps) = capabilities {
        if caps.contains(&Capability::Manage) {
            return Err(JoyError::Other(
                "an AI member never holds the manage capability".into(),
            ));
        }
        member.set_capabilities(crate::model::project::MemberCapabilities::Specific(
            caps.iter().map(|c| (*c, Default::default())).collect(),
        ));
    }
    if let Some(level) = level {
        member.interaction_level = Some(level);
    }
    Ok(())
}

/// What an AI member may do, as a person looking at it sees it: what
/// the project allows, what they allow themselves, and what comes out.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// The project maximum, or why there is none.
    pub project: Result<Effective, String>,
    /// The viewer's own grant, if they made one.
    pub mine: Option<Effective>,
    /// What the AI member may do for the viewer.
    pub effective: Result<Effective, String>,
}

/// [`View`] of `ai` for the person `viewer_key` (nobody: the project's
/// side alone, and the effective column is the maximum).
pub fn view(project: &Project, ai_key: &str, viewer_key: Option<&str>) -> View {
    let max = maximum(project, ai_key);
    let viewer = viewer_key.and_then(|key| project.member_by_key(key).map(|m| (key, m)));
    let mine = viewer
        .and_then(|(_, member)| member.delegation_to(ai_key))
        .and_then(|d| d.grant.as_ref())
        .map(|g| Effective {
            capabilities: g.capabilities.clone(),
            level: g.level,
        });
    let effective = match viewer {
        Some((key, _)) => effective_now(project, ai_key, key),
        None => max.clone(),
    };
    View {
        project: max,
        mine,
        effective,
    }
}

/// [`View`] in words, as a card or a table shows it: capability names
/// and a level for each side, empty where a side has none, and the one
/// sentence that says why the AI member may do nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shown {
    /// What the project allows at most.
    pub capabilities: Vec<String>,
    pub level: String,
    /// What the viewer allows for themselves; empty when they made no
    /// grant of their own and the project's side applies to them.
    pub my_capabilities: Vec<String>,
    pub my_level: String,
    /// What the AI member may do for the viewer.
    pub effective_capabilities: Vec<String>,
    pub effective_level: String,
    /// Why it may do nothing, when that is so.
    pub problem: String,
    /// The model the project sets for everybody; empty leaves the choice
    /// to each person.
    pub model: String,
    /// The model the viewer picked for themselves; empty is what the
    /// tool takes by itself. It counts only while the project sets none.
    pub my_model: String,
    /// The model the AI member runs on for the viewer; empty is what the
    /// tool takes by itself.
    pub effective_model: String,
}

/// [`Shown`] of the AI member `ai_key` for the person `viewer_key`, on
/// the desktop, on the platform and in the CLI alike.
///
/// A project from before the member files has no signed sides: there
/// the member's entry is shown as it stands, until a person brings the
/// project over.
pub fn shown(project: &Project, ai_key: &str, viewer_key: Option<&str>) -> Shown {
    let names = |side: &Effective| -> Vec<String> {
        side.capabilities.iter().map(|c| c.to_string()).collect()
    };
    let mut shown = Shown {
        model: project
            .member_by_key(ai_key)
            .and_then(|m| m.model.clone())
            .unwrap_or_default(),
        my_model: viewer_key
            .and_then(|key| person_in(project, key))
            .and_then(|m| m.delegation_to(ai_key))
            .and_then(|d| d.model.clone())
            .unwrap_or_default(),
        ..Shown::default()
    };
    shown.effective_model = if shown.model.is_empty() {
        shown.my_model.clone()
    } else {
        shown.model.clone()
    };
    if !applies(project) {
        if let Some(member) = project.member_by_key(ai_key) {
            shown.capabilities = vouch::capability_list(&member.capabilities)
                .iter()
                .map(|c| c.to_string())
                .collect();
            shown.level = member
                .interaction_level
                .map(|l| l.to_string())
                .unwrap_or_default();
            shown.effective_capabilities = shown.capabilities.clone();
            shown.effective_level = shown.level.clone();
        }
        return shown;
    }
    let view = view(project, ai_key, viewer_key);
    match &view.project {
        Ok(side) => {
            shown.capabilities = names(side);
            shown.level = side.level.to_string();
        }
        Err(why) => shown.problem = why.clone(),
    }
    if let Some(mine) = &view.mine {
        shown.my_capabilities = names(mine);
        shown.my_level = mine.level.to_string();
    }
    match &view.effective {
        Ok(side) => {
            shown.effective_capabilities = names(side);
            shown.effective_level = side.level.to_string();
        }
        Err(why) if shown.problem.is_empty() => shown.problem = why.clone(),
        Err(_) => {}
    }
    shown
}

/// What `ai` may do for `delegator_key` as the project stands right
/// now, for a host that acts for a signed-in person and holds no token
/// of theirs: their grant as it is in their entry, checked against its
/// signature.
pub fn effective_now(
    project: &Project,
    ai_key: &str,
    delegator_key: &str,
) -> Result<Effective, String> {
    // by their key, or by an address the project knows them under
    let issued = project
        .member_by_key(delegator_key)
        .or_else(|| {
            let key = project.member_key_for_email(delegator_key)?;
            project.member_by_key(&key)
        })
        .map(|member| token_grant(member, ai_key));
    effective(project, ai_key, delegator_key, issued.as_deref())
}

/// The level a turn of `ai` for `delegator_key` runs at: the one the
/// person chose for it, never beyond what [`effective_now`] allows.
pub fn turn_level(
    project: &Project,
    ai_key: &str,
    delegator_key: &str,
    chosen: Option<InteractionLevel>,
) -> Result<InteractionLevel, String> {
    let may = effective_now(project, ai_key, delegator_key)?;
    Ok(more_oversight(may.level, chosen.unwrap_or(may.level)))
}

fn list(capabilities: &[Capability]) -> String {
    capabilities
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What `ai` may do when it acts for `delegator`, or in words why it
/// may do nothing.
///
/// `token_grant` is what the AI's session says its token was issued
/// with ([`token_grant`] at that time); a session from a token issued
/// before grants existed says nothing, which reads as [`NO_GRANT`].
///
/// A project that keeps its members in project.yaml has no signed
/// maximum; there the member's capabilities are what they are and this
/// function is not asked ([`applies`]).
pub fn effective(
    project: &Project,
    ai_key: &str,
    delegator_key: &str,
    token_grant: Option<&str>,
) -> Result<Effective, String> {
    let name = ai_member_name(ai_key);
    let max = maximum(project, ai_key)?;
    // The person is named by their key, or by an address the project
    // knows them under (an anonymous project keys them by an id).
    let delegator = project
        .member_by_key(delegator_key)
        .or_else(|| {
            let key = project.member_key_for_email(delegator_key)?;
            project.member_by_key(&key)
        })
        .ok_or_else(|| format!("{name} acts for someone who is not a member of this project"))?;
    let grant = delegator
        .delegation_to(ai_key)
        .and_then(|d| d.grant.as_ref());
    let issued_with = token_grant.unwrap_or(NO_GRANT);
    if grant_hash(grant) != issued_with {
        return Err(format!(
            "what {name} may do for its delegator has changed since its token was issued. \
             Issue a new one: joy auth token add {name}"
        ));
    }
    let Some(grant) = grant else {
        return Ok(max);
    };
    let key = delegator
        .verify_key
        .as_deref()
        .and_then(|hex| PublicKey::from_hex(hex).ok().map(|pk| (hex, pk)));
    let signed = key.is_some_and(|(hex, pk)| {
        let text = personal_text(project, ai_key, &grant.capabilities, grant.level, hex);
        hex::decode(&grant.signature)
            .ok()
            .is_some_and(|sig| pk.verify(text.as_bytes(), &sig).is_ok())
    });
    if !signed {
        return Err(format!(
            "what {name} may do for its delegator was changed without a signature for it"
        ));
    }
    Ok(Effective {
        capabilities: max
            .capabilities
            .iter()
            .copied()
            .filter(|cap| grant.capabilities.contains(cap))
            .collect(),
        level: more_oversight(max.level, grant.level),
    })
}

/// Whether signed grants govern the AI members of this project: they do
/// once it keeps its members in files.
pub fn applies(project: &Project) -> bool {
    project.member_layout() == MemberLayout::Files
}

/// A person as the project knows them: by their key, or by an address
/// it has for them (an anonymous project keys people by an id).
fn person_in<'a>(project: &'a Project, key: &str) -> Option<&'a crate::model::project::Member> {
    project.member_by_key(key).or_else(|| {
        let key = project.member_key_for_email(key)?;
        project.member_by_key(&key)
    })
}

/// The model `ai` runs on for `person_key`: the one the project sets for
/// everybody, else the one the person picked for themselves, else None,
/// which is what the tool takes by itself. One rule for a chat turn, a
/// job round and the card, on every host.
pub fn model_for(project: &Project, ai_key: &str, person_key: &str) -> Option<String> {
    let filled = |m: &Option<String>| m.clone().filter(|m| !m.trim().is_empty());
    project
        .member_by_key(ai_key)
        .and_then(|m| filled(&m.model))
        .or_else(|| {
            person_in(project, person_key)
                .and_then(|m| m.delegation_to(ai_key))
                .and_then(|d| filled(&d.model))
        })
}

/// [`model_for`] for a host that holds the project's root.
pub fn model_for_at(root: &std::path::Path, ai_key: &str, person_key: &str) -> Option<String> {
    let project = crate::store::load_project(root).ok()?;
    model_for(&project, ai_key, person_key)
}

/// Whether `member` can be given a job by `person_key`, and the level it
/// may run at for them at most.
///
/// A job is taken by its assignee, and taking it is a job status change:
/// that needs the jobs capability (`guard`). A person holds it or not.
/// An AI member holds it for `person_key` when the project allows it and
/// the person did not take it away for themselves; it then runs at most
/// at the level it may have for them. In a project from before the
/// member files an AI member is not judged here, as it was not before.
pub fn job_assignee(
    project: &Project,
    member: &str,
    person_key: &str,
) -> Result<Option<InteractionLevel>, String> {
    if !crate::model::project::is_ai_member(member) {
        let holds = project
            .member_by_key(member)
            .is_some_and(|m| vouch::capability_list(&m.capabilities).contains(&Capability::Jobs));
        return if holds {
            Ok(None)
        } else {
            Err(format!("{member} does not hold the jobs capability"))
        };
    }
    if !applies(project) {
        return Ok(None);
    }
    let may = effective_now(project, member, person_key)?;
    if !may.allows(&Capability::Jobs) {
        return Err(format!(
            "{} does not hold the jobs capability for you",
            ai_member_name(member)
        ));
    }
    Ok(Some(may.level))
}

/// The members `person_key` can give a job to, each with the level it
/// may run at for them at most (None for a person, who has no level):
/// what a job form offers. The same rule decides again when the job is
/// opened ([`job_terms`]), because a member can be changed in between.
pub fn job_assignees(
    project: &Project,
    person_key: &str,
) -> Vec<(String, Option<InteractionLevel>)> {
    project
        .members()
        .filter_map(|(key, _)| {
            job_assignee(project, key, person_key)
                .ok()
                .map(|level| (key.clone(), level))
        })
        .collect()
}

/// The levels a job can run at: proposing or autonomous, nothing in
/// between (operator 2026-10-08). The middle level asks a person before
/// a command runs, and a job has nobody there to ask.
pub const JOB_LEVELS: [InteractionLevel; 2] =
    [InteractionLevel::Proposing, InteractionLevel::Autonomous];

/// The level every job has until somebody says otherwise.
pub const JOB_DEFAULT_LEVEL: InteractionLevel = InteractionLevel::Proposing;

/// A level as a job may name it, or the sentence that says which it may.
pub fn job_level(level: InteractionLevel) -> Result<InteractionLevel, String> {
    if JOB_LEVELS.contains(&level) {
        Ok(level)
    } else {
        Err(format!(
            "a job runs at proposing or at autonomous, not at {level}"
        ))
    }
}

/// The level a job runs at, checked for `person_key`: when they write
/// the job, when they change it, and when they approve it (new -> open),
/// after which its assignee takes it.
///
/// A job runs at proposing unless it says autonomous, and autonomous
/// only where its AI assignee may run at autonomous for the person. An
/// assignee without the jobs capability is refused. From the approval on
/// the level is the job's own, and a member changed later does not reach
/// into a job that is already open (JI-0166-D8). A job with no AI
/// assignee, or in a project from before the member files, is not
/// judged on its assignee; it still names one of the two levels.
pub fn job_terms(
    project: &Project,
    job: &crate::model::item::Item,
    person_key: &str,
) -> Result<Option<InteractionLevel>, String> {
    let wanted = job_level(job.interaction_level.unwrap_or(JOB_DEFAULT_LEVEL))?;
    let Some(assignee) = job.assignees.first().map(|a| a.member.id()) else {
        return Ok(Some(wanted));
    };
    let Some(most) = job_assignee(project, assignee, person_key)? else {
        return Ok(Some(wanted));
    };
    if wanted == InteractionLevel::Autonomous && most != InteractionLevel::Autonomous {
        return Err(format!(
            "this job asks for autonomous, and {} may run at most at {most} for you",
            ai_member_name(assignee)
        ));
    }
    Ok(Some(wanted))
}

/// A job's level, changed by `person_key`: allowed as long as the job is
/// not finished, also after its approval and between two rounds. That is
/// how a person lets a job that proposed go on to do the work: they
/// switch it to autonomous once the proposal is settled (operator
/// 2026-10-08). The same rule as when the job was written decides
/// ([`job_terms`]); what does not reach into an open job is a change of
/// the MEMBER, not a change the person makes to the job.
pub fn job_level_change(
    project: &Project,
    job: &crate::model::item::Item,
    level: InteractionLevel,
    person_key: &str,
) -> Result<InteractionLevel, String> {
    use crate::model::item::Status;
    if matches!(job.status, Status::Closed | Status::Deferred) {
        return Err(format!(
            "job {} is {}; its level is no longer changed",
            job.id, job.status
        ));
    }
    let mut wanted = job.clone();
    wanted.interaction_level = Some(level);
    job_terms(project, &wanted, person_key).map(|granted| granted.unwrap_or(level))
}

/// A job that is already open, when it is started: its level is its own
/// since the approval and stays, also one from before jobs had two
/// levels. What is asked again is whether the assignee can still take
/// it for the person: the jobs capability, and a level it may run at.
fn job_start_terms(
    project: &Project,
    job: &crate::model::item::Item,
    person_key: &str,
) -> Result<Option<InteractionLevel>, String> {
    let level = job.interaction_level.unwrap_or(JOB_DEFAULT_LEVEL);
    let Some(assignee) = job.assignees.first().map(|a| a.member.id()) else {
        return Ok(Some(level));
    };
    let Some(most) = job_assignee(project, assignee, person_key)? else {
        return Ok(Some(level));
    };
    if more_oversight(level, most) != level {
        return Err(format!(
            "this job runs at {level}, and {} may run at most at {most} for you",
            ai_member_name(assignee)
        ));
    }
    Ok(Some(level))
}

/// The level a job has after a status step by `person_key`, or why the
/// step is refused: THE rule every host asks when a job changes status.
///
/// Two steps put a job to work, and both ask [`job_terms`]: approving it
/// (`new -> open`), after which its AI assignee may take it, and
/// starting it (`open -> in-progress`), by the assignee or by a person
/// who orders exactly this job. Every other step leaves the level alone.
pub fn job_step(
    project: &Project,
    job: &crate::model::item::Item,
    from: &crate::model::item::Status,
    to: &crate::model::item::Status,
    person_key: &str,
) -> Result<Option<InteractionLevel>, String> {
    use crate::model::item::Status;
    match (from, to) {
        (Status::New, Status::Open) => job_terms(project, job, person_key),
        (Status::Open, Status::InProgress) => job_start_terms(project, job, person_key),
        _ => Ok(job.interaction_level),
    }
}

/// Where a job stands against the times it was given: before its start,
/// inside them, or past its end. An AI member takes an open job only
/// inside them; a job without times is always inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobWindowState {
    NotYet,
    Open,
    Over,
}

pub fn job_window(job: &crate::model::item::Item, now: chrono::DateTime<Utc>) -> JobWindowState {
    let Some(window) = job.job.as_ref().and_then(|spec| spec.window.as_ref()) else {
        return JobWindowState::Open;
    };
    if window.deadline.is_some_and(|end| now > end) {
        JobWindowState::Over
    } else if window.not_before.is_some_and(|start| now < start) {
        JobWindowState::NotYet
    } else {
        JobWindowState::Open
    }
}

/// The open jobs an AI member may take now, in the order it takes them:
/// by priority, then the oldest first. Only jobs inside their times
/// ([`job_window`]) and never run before; `jobs` is every job of the
/// project. One at a time is the caller's to keep.
pub fn jobs_to_take<'a>(
    jobs: &'a [crate::model::item::Item],
    assignee: &str,
    now: chrono::DateTime<Utc>,
) -> Vec<&'a crate::model::item::Item> {
    use crate::model::item::{Priority, Status};
    let rank = |p: &Priority| match p {
        Priority::Extreme => 0,
        Priority::Critical => 1,
        Priority::High => 2,
        Priority::Medium => 3,
        Priority::Low => 4,
    };
    let name = ai_member_name(assignee);
    let mut mine: Vec<&crate::model::item::Item> = jobs
        .iter()
        .filter(|job| job.status == Status::Open)
        .filter(|job| job.assignees.first().is_some_and(|a| a.member.id() == name))
        .filter(|job| job.job.as_ref().is_none_or(|spec| spec.activity.is_empty()))
        .filter(|job| job_window(job, now) == JobWindowState::Open)
        .collect();
    mine.sort_by(|a, b| {
        rank(&a.priority)
            .cmp(&rank(&b.priority))
            .then(a.created.cmp(&b.created))
            .then(a.id.cmp(&b.id))
    });
    mine
}

/// [`turn_level`] for a host that holds the project's root: THE level a
/// chat turn of `ai` for `delegator_key` runs at, on the desktop, on
/// the platform and in the CLI alike.
///
/// A project from before the member files has no signed maximum; there
/// the turn runs at what the person chose, else at the AI member's own
/// level, else at the project default, as it did before.
pub fn turn_level_at(
    root: &std::path::Path,
    ai_key: &str,
    delegator_key: &str,
    chosen: Option<InteractionLevel>,
) -> Result<InteractionLevel, String> {
    let project = crate::store::load_project(root).ok();
    if let Some(project) = project.as_ref().filter(|p| applies(p)) {
        return turn_level(project, ai_key, delegator_key, chosen);
    }
    Ok(chosen
        .or_else(|| {
            project
                .as_ref()
                .and_then(|p| p.member_by_key(ai_key))
                .and_then(|m| m.interaction_level)
        })
        .unwrap_or_else(|| crate::store::load_interaction_level_defaults(root).default))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::vouch::Occasion;
    use crate::model::project::{AiDelegationEntry, MemberCapabilities};
    use Capability::{Create, Implement, Plan, Review};
    use InteractionLevel::{Autonomous, Confirmed, Proposing};

    const FOUNDER: &str = "founder@example.com";
    const DEV: &str = "dev@example.com";

    fn keypair(byte: u8) -> IdentityKeypair {
        IdentityKeypair::from_seed(&[byte; 32])
    }

    fn caps(list: &[Capability]) -> MemberCapabilities {
        MemberCapabilities::Specific(list.iter().map(|c| (*c, Default::default())).collect())
    }

    fn person(kp: &IdentityKeypair, capabilities: MemberCapabilities) -> Member {
        let mut member = Member::new(capabilities);
        member.verify_key = Some(kp.public_key().to_hex());
        member.put_delegation(
            "claude",
            AiDelegationEntry {
                delegation_verifier: "00".repeat(32),
                delegation_salt: Some("11".repeat(32)),
                created: Utc::now(),
                rotated: None,
                grant: None,
                model: None,
            },
        );
        member
    }

    /// Founder with manage, a developer, and claude allowed plan,
    /// implement and review at confirmed.
    fn project() -> (Project, IdentityKeypair, IdentityKeypair) {
        let mut project = Project::new("Shop".into(), Some("SH".into()));
        project.set_member_layout(MemberLayout::Files);
        let (founder_kp, dev_kp) = (keypair(1), keypair(2));
        project
            .register_member(FOUNDER, person(&founder_kp, MemberCapabilities::All))
            .unwrap();
        project
            .register_member(DEV, person(&dev_kp, caps(&[Implement, Create])))
            .unwrap();
        let mut claude = Member::new(caps(&[Plan, Implement, Review]));
        claude.interaction_level = Some(Confirmed);
        vouch::sign(
            &project,
            FOUNDER,
            &founder_kp,
            "claude",
            &mut claude,
            Occasion::New,
        );
        project.register_member("claude", claude).unwrap();
        (project, founder_kp, dev_kp)
    }

    fn grant(
        project: &mut Project,
        kp: &IdentityKeypair,
        who: &str,
        c: &[Capability],
        l: InteractionLevel,
    ) {
        set_personal(project, "claude", who, kp, Some(c), Some(l)).unwrap();
    }

    #[test]
    fn without_a_grant_of_their_own_the_project_maximum_applies() {
        let (project, _, _) = project();
        let eff = effective(&project, "claude", DEV, Some(NO_GRANT)).unwrap();
        assert_eq!(eff.capabilities, [Plan, Implement, Review]);
        assert_eq!(eff.level, Confirmed);
        // a token from before grants existed says nothing: the same
        assert_eq!(effective(&project, "claude", DEV, None).unwrap(), eff);
    }

    #[test]
    fn a_persons_grant_narrows_what_the_ai_may_do_for_them_and_nobody_else() {
        let (mut project, _, dev_kp) = project();
        grant(&mut project, &dev_kp, DEV, &[Review], Proposing);
        let dev = project.member_by_key(DEV).unwrap();
        let issued = token_grant(dev, "claude");
        assert_ne!(issued, NO_GRANT);

        let for_dev = effective(&project, "claude", DEV, Some(&issued)).unwrap();
        assert_eq!(for_dev.capabilities, [Review]);
        assert_eq!(for_dev.level, Proposing);

        let for_founder = effective(&project, "claude", FOUNDER, Some(NO_GRANT)).unwrap();
        assert_eq!(for_founder.capabilities, [Plan, Implement, Review]);
        assert_eq!(for_founder.level, Confirmed);
    }

    #[test]
    fn a_grant_cannot_go_beyond_the_project_maximum() {
        let (mut project, _, dev_kp) = project();
        let more = set_personal(
            &mut project,
            "claude",
            DEV,
            &dev_kp,
            Some(&[Plan, Create]),
            Some(Confirmed),
        )
        .unwrap_err()
        .to_string();
        assert!(more.contains("does not allow claude: create"), "{more}");
        let higher = set_personal(
            &mut project,
            "claude",
            DEV,
            &dev_kp,
            Some(&[Plan]),
            Some(Autonomous),
        )
        .unwrap_err()
        .to_string();
        assert!(higher.contains("at most confirmed"), "{higher}");
    }

    #[test]
    fn what_is_not_said_stays_and_the_view_shows_all_three_sides() {
        let (mut project, _, dev_kp) = project();
        // only a level: the capabilities are the project's
        set_personal(&mut project, "claude", DEV, &dev_kp, None, Some(Proposing)).unwrap();
        let seen = view(&project, "claude", Some(DEV));
        assert_eq!(seen.project.as_ref().unwrap().level, Confirmed);
        assert_eq!(
            seen.mine.as_ref().unwrap().capabilities,
            [Plan, Implement, Review]
        );
        assert_eq!(seen.mine.as_ref().unwrap().level, Proposing);
        assert_eq!(seen.effective.as_ref().unwrap().level, Proposing);
        // only capabilities: the level stays proposing
        set_personal(&mut project, "claude", DEV, &dev_kp, Some(&[Review]), None).unwrap();
        let seen = view(&project, "claude", Some(DEV));
        assert_eq!(seen.mine.unwrap().level, Proposing);
        assert_eq!(seen.effective.unwrap().capabilities, [Review]);
        // a turn never runs beyond it, and may run below
        assert_eq!(
            turn_level(&project, "claude", DEV, Some(Autonomous)),
            Ok(Proposing)
        );
        assert_eq!(
            turn_level(&project, "claude", FOUNDER, Some(Autonomous)),
            Ok(Confirmed)
        );
        assert_eq!(
            turn_level(&project, "claude", FOUNDER, Some(Proposing)),
            Ok(Proposing)
        );
        assert_eq!(turn_level(&project, "claude", FOUNDER, None), Ok(Confirmed));
        // dropped again: the project maximum
        clear_personal(&mut project, "claude", DEV);
        assert_eq!(view(&project, "claude", Some(DEV)).mine, None);
    }

    #[test]
    fn a_person_who_has_not_delegated_is_told_how() {
        let (mut project, _, dev_kp) = project();
        project
            .member_by_key_mut(DEV)
            .unwrap()
            .ai_delegations
            .clear();
        let why = set_personal(&mut project, "claude", DEV, &dev_kp, None, Some(Proposing))
            .unwrap_err()
            .to_string();
        assert!(why.contains("joy auth token add claude"), "{why}");
    }

    #[test]
    fn a_lowered_maximum_clamps_a_grant_without_rewriting_it() {
        let (mut project, founder_kp, dev_kp) = project();
        grant(&mut project, &dev_kp, DEV, &[Plan, Implement], Confirmed);
        let issued = token_grant(project.member_by_key(DEV).unwrap(), "claude");

        // the manager takes implement away and lowers the level
        let mut claude = project.member_by_key("claude").unwrap().clone();
        claude.set_capabilities(caps(&[Plan, Review]));
        claude.interaction_level = Some(Proposing);
        vouch::sign(
            &project,
            FOUNDER,
            &founder_kp,
            "claude",
            &mut claude,
            Occasion::Changed,
        );
        *project.member_by_key_mut("claude").unwrap() = claude;

        let eff = effective(&project, "claude", DEV, Some(&issued)).unwrap();
        assert_eq!(eff.capabilities, [Plan]);
        assert_eq!(eff.level, Proposing);
    }

    #[test]
    fn dropping_or_editing_a_grant_stops_the_ai_instead_of_freeing_it() {
        let (mut project, _, dev_kp) = project();
        grant(&mut project, &dev_kp, DEV, &[Review], Proposing);
        let issued = token_grant(project.member_by_key(DEV).unwrap(), "claude");

        // the grant is removed from the file: the token still names it
        let mut dropped = project.clone();
        dropped
            .member_by_key_mut(DEV)
            .unwrap()
            .delegation_to_mut("claude")
            .unwrap()
            .grant = None;
        let why = effective(&dropped, "claude", DEV, Some(&issued)).unwrap_err();
        assert!(
            why.contains("has changed since its token was issued"),
            "{why}"
        );

        // the grant is widened by hand, signature kept
        let mut widened = project.clone();
        {
            let g = widened
                .member_by_key_mut(DEV)
                .unwrap()
                .delegation_to_mut("claude")
                .unwrap()
                .grant
                .as_mut()
                .unwrap();
            g.capabilities = vec![Plan, Implement, Review];
            g.level = Confirmed;
        }
        assert!(effective(&widened, "claude", DEV, Some(&issued)).is_err());
        // even with a token that names the widened grant, the signature
        // does not cover it
        let forged = token_grant(widened.member_by_key(DEV).unwrap(), "claude");
        let why = effective(&widened, "claude", DEV, Some(&forged)).unwrap_err();
        assert!(why.contains("without a signature"), "{why}");
    }

    #[test]
    fn a_maximum_changed_by_hand_leaves_the_ai_with_nothing() {
        let (mut project, _, _) = project();
        project
            .member_by_key_mut("claude")
            .unwrap()
            .set_capabilities(caps(&[Plan, Implement, Review, Create]));
        let why = effective(&project, "claude", DEV, None).unwrap_err();
        assert!(why.contains("changed without a signature"), "{why}");
    }
    /// The project of [`project`], with claude allowed `list` at `level`.
    fn project_where_claude_may(
        list: &[Capability],
        level: InteractionLevel,
    ) -> (Project, IdentityKeypair, IdentityKeypair) {
        let (mut project, founder_kp, dev_kp) = project();
        let mut claude = project.member_by_key("claude").cloned().unwrap();
        change_maximum(&mut claude, Some(list), Some(level)).unwrap();
        vouch::sign(
            &project,
            FOUNDER,
            &founder_kp,
            "claude",
            &mut claude,
            Occasion::Changed,
        );
        *project.member_by_key_mut("claude").unwrap() = claude;
        (project, founder_kp, dev_kp)
    }

    fn job_for(assignee: &str, level: Option<InteractionLevel>) -> crate::model::item::Item {
        let mut job = crate::templates::render_item(
            &crate::model::item::ItemType::Job,
            "SH-JOB-0001-AA",
            "Do it",
        )
        .unwrap();
        job.assignees.push(crate::model::item::Assignee {
            member: assignee.into(),
            capabilities: Vec::new(),
        });
        job.interaction_level = level;
        job
    }

    /// A job is offered to, and taken by, a member that holds the jobs
    /// capability for the person: the list a form shows and the rule
    /// that decides when the job is opened are the same function.
    #[test]
    fn a_job_goes_only_to_a_member_that_holds_jobs_for_the_person() {
        use Capability::Jobs;
        // claude as the project stands: no jobs capability
        let (project, _, _) = project();
        let offered: Vec<String> = job_assignees(&project, DEV)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(offered, [FOUNDER], "the developer holds no jobs either");
        let why = job_terms(&project, &job_for("claude", None), DEV).unwrap_err();
        assert!(
            why.contains("does not hold the jobs capability for you"),
            "{why}"
        );
        let why = job_terms(&project, &job_for(DEV, None), FOUNDER).unwrap_err();
        assert!(why.contains("does not hold the jobs capability"), "{why}");

        // the project allows it jobs at confirmed: it is offered, and a
        // job runs at proposing, which is what a job says when it says
        // nothing
        let (project, _, _) = project_where_claude_may(&[Implement, Jobs], Confirmed);
        assert!(job_assignees(&project, DEV).contains(&("claude".to_string(), Some(Confirmed))));
        assert_eq!(
            job_terms(&project, &job_for("claude", None), DEV).unwrap(),
            Some(Proposing)
        );
        assert_eq!(
            job_terms(&project, &job_for("claude", Some(Proposing)), DEV).unwrap(),
            Some(Proposing)
        );
        // autonomous only where the member may run at autonomous
        let why = job_terms(&project, &job_for("claude", Some(Autonomous)), DEV).unwrap_err();
        assert!(
            why.contains("may run at most at confirmed for you"),
            "{why}"
        );
        // a job has two levels, nothing in between
        let why = job_terms(&project, &job_for("claude", Some(Confirmed)), DEV).unwrap_err();
        assert!(why.contains("proposing or at autonomous"), "{why}");
        assert!(job_level(Confirmed).is_err());

        let (mut project, _, dev_kp) = project_where_claude_may(&[Implement, Jobs], Autonomous);
        assert_eq!(
            job_terms(&project, &job_for("claude", Some(Autonomous)), DEV).unwrap(),
            Some(Autonomous)
        );
        // changed after the job was written: the person takes jobs away
        // for themselves, and approving the job is refused for them only
        grant(&mut project, &dev_kp, DEV, &[Implement], Autonomous);
        assert!(job_terms(&project, &job_for("claude", None), DEV).is_err());
        assert!(job_terms(&project, &job_for("claude", None), FOUNDER).is_ok());
        assert!(!job_assignees(&project, DEV)
            .iter()
            .any(|(key, _)| key == "claude"));
        // a person's own level holds the job too: no autonomous for them
        grant(&mut project, &dev_kp, DEV, &[Implement, Jobs], Proposing);
        assert!(job_terms(&project, &job_for("claude", Some(Autonomous)), DEV).is_err());
        assert!(job_terms(&project, &job_for("claude", Some(Autonomous)), FOUNDER).is_ok());
    }

    /// The model: the project's for everybody, else each person's own,
    /// else what the tool takes by itself.
    #[test]
    fn the_model_is_the_projects_or_else_each_persons_own() {
        let (mut project, _, _) = project();
        assert_eq!(model_for(&project, "claude", DEV), None);

        // the project leaves the choice: the developer picks, for them only
        set_personal_model(&mut project, "claude", DEV, Some("sonnet")).unwrap();
        assert_eq!(
            model_for(&project, "claude", DEV).as_deref(),
            Some("sonnet")
        );
        assert_eq!(
            model_for(&project, "claude", DEV).as_deref(),
            Some("sonnet")
        );
        assert_eq!(model_for(&project, "claude", FOUNDER), None);
        let card = shown(&project, "claude", Some(DEV));
        assert_eq!(
            (
                card.model.as_str(),
                card.my_model.as_str(),
                card.effective_model.as_str()
            ),
            ("", "sonnet", "sonnet")
        );

        // a new delegation keeps the pick
        let fresh = AiDelegationEntry {
            delegation_verifier: "22".repeat(32),
            delegation_salt: Some("33".repeat(32)),
            created: Utc::now(),
            rotated: None,
            grant: None,
            model: None,
        };
        project
            .member_by_key_mut(DEV)
            .unwrap()
            .put_delegation("claude", fresh);
        assert_eq!(
            model_for(&project, "claude", DEV).as_deref(),
            Some("sonnet")
        );

        // the project sets one: it holds for everybody, and a person's
        // own pick is refused with the model named
        project.member_by_key_mut("claude").unwrap().model = Some("opus".into());
        assert_eq!(model_for(&project, "claude", DEV).as_deref(), Some("opus"));
        assert_eq!(
            model_for(&project, "claude", FOUNDER).as_deref(),
            Some("opus")
        );
        let why = set_personal_model(&mut project, "claude", DEV, Some("haiku"))
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("sets the model of claude to opus for everybody"),
            "{why}"
        );
        assert_eq!(shown(&project, "claude", Some(DEV)).effective_model, "opus");

        // handed back to the tool
        project.member_by_key_mut("claude").unwrap().model = None;
        set_personal_model(&mut project, "claude", DEV, None).unwrap();
        assert_eq!(model_for(&project, "claude", DEV), None);
    }

    /// The open jobs an AI member takes, in its order: the most urgent
    /// first, then the oldest, only inside the times a job was given,
    /// and never one that already ran.
    #[test]
    fn an_ai_member_takes_open_jobs_by_priority_and_inside_their_times() {
        use crate::model::item::{JobWindow, Priority, Status};
        let now = Utc::now();
        let job = |id: &str, priority: Priority, age_minutes: i64| {
            let mut job = job_for("claude", None);
            job.id = id.into();
            job.status = Status::Open;
            job.priority = priority;
            job.created = now - chrono::Duration::minutes(age_minutes);
            job
        };
        let window = |job: &mut crate::model::item::Item, start: i64, end: i64| {
            job.job = Some(crate::model::item::JobSpec {
                scope: vec!["SH-0001-AA".into()],
                budget: None,
                window: Some(JobWindow {
                    not_before: Some(now + chrono::Duration::minutes(start)),
                    deadline: Some(now + chrono::Duration::minutes(end)),
                }),
                feedback: None,
                activity: Vec::new(),
                base_branch: None,
                result_branch: None,
            });
        };
        let mut later = job("later", Priority::Extreme, 50);
        window(&mut later, 30, 90);
        let mut over = job("over", Priority::Extreme, 50);
        window(&mut over, -90, -30);
        let mut inside = job("inside", Priority::Low, 5);
        window(&mut inside, -10, 10);
        let mut not_open = job("new", Priority::Extreme, 50);
        not_open.status = Status::New;
        let mut others = job("vibes", Priority::Extreme, 50);
        others.assignees[0].member = "vibe".into();
        let jobs = vec![
            job("medium-old", Priority::Medium, 40),
            later,
            job("high", Priority::High, 1),
            over,
            job("medium-new", Priority::Medium, 10),
            inside,
            not_open,
            others,
        ];
        assert_eq!(job_window(&jobs[1], now), JobWindowState::NotYet);
        assert_eq!(job_window(&jobs[3], now), JobWindowState::Over);
        assert_eq!(job_window(&jobs[0], now), JobWindowState::Open);
        let order: Vec<&str> = jobs_to_take(&jobs, "claude", now)
            .into_iter()
            .map(|job| job.id.as_str())
            .collect();
        assert_eq!(order, ["high", "medium-old", "medium-new", "inside"]);
    }

    /// Approving a job gives it one of the two job levels; starting an
    /// open job leaves its level alone and asks only whether the assignee
    /// can still take it; other steps judge nothing.
    #[test]
    fn approving_a_job_sets_its_level_and_starting_it_keeps_it() {
        use crate::model::item::Status;
        use Capability::Jobs;
        let (project, _, _) = project_where_claude_may(&[Implement, Jobs], Confirmed);
        let (approve, start) = (
            (Status::New, Status::Open),
            (Status::Open, Status::InProgress),
        );
        let step = |job: &crate::model::item::Item, (from, to): &(Status, Status)| {
            job_step(&project, job, from, to, DEV)
        };
        assert_eq!(
            step(&job_for("claude", None), &approve).unwrap(),
            Some(Proposing)
        );
        assert!(step(&job_for("claude", Some(Autonomous)), &approve).is_err());
        assert!(step(&job_for("claude", Some(Confirmed)), &approve).is_err());
        // an open job from before jobs had two levels keeps its level
        assert_eq!(
            step(&job_for("claude", Some(Confirmed)), &start).unwrap(),
            Some(Confirmed)
        );
        // ...unless the member may no longer run at it
        assert!(step(&job_for("claude", Some(Autonomous)), &start).is_err());
        // stopping judges nothing
        assert_eq!(
            job_step(
                &project,
                &job_for("claude", Some(Autonomous)),
                &Status::InProgress,
                &Status::Open,
                DEV
            )
            .unwrap(),
            Some(Autonomous)
        );
    }
}
