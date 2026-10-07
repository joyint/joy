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
    let name = ai_member_name(ai_key).to_string();
    let delegator = project
        .member_by_key_mut(delegator_key)
        .ok_or_else(|| JoyError::Other(format!("{delegator_key} is not a member")))?;
    let entry = delegator
        .ai_delegations
        .iter_mut()
        .find(|(key, _)| ai_member_name(key) == name)
        .map(|(_, entry)| entry)
        .ok_or_else(|| not_delegated(&name))?;
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
    let name = ai_member_name(ai_key).to_string();
    if let Some(delegator) = project.member_by_key_mut(delegator_key) {
        for (key, entry) in delegator.ai_delegations.iter_mut() {
            if ai_member_name(key) == name {
                entry.grant = None;
            }
        }
    }
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

/// What `ai` may do for `delegator_key` as the project stands right
/// now, for a host that acts for a signed-in person and holds no token
/// of theirs: their grant as it is in their entry, checked against its
/// signature.
pub fn effective_now(
    project: &Project,
    ai_key: &str,
    delegator_key: &str,
) -> Result<Effective, String> {
    let issued = project
        .member_by_key(delegator_key)
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

/// The level a job is released with, said when a person approves it.
///
/// A job with an AI assignee runs at the level it asks for, and at most
/// at the assignee's project maximum: a job that asks for more is not
/// approved, a job that says nothing gets the maximum written in. From
/// here on the level is the job's own, and a maximum lowered later does
/// not reach into a job that was already approved (JI-0166-D8). A job
/// with no AI assignee, or in a project from before the member files,
/// keeps what it has.
pub fn job_level_at_approval(
    project: &Project,
    job: &crate::model::item::Item,
) -> Result<Option<InteractionLevel>, String> {
    let Some(assignee) = job.assignees.first().map(|a| a.member.id()) else {
        return Ok(job.interaction_level);
    };
    if !applies(project) || !crate::model::project::is_ai_member(assignee) {
        return Ok(job.interaction_level);
    }
    let name = ai_member_name(assignee);
    let max = maximum(project, assignee)?;
    match job.interaction_level {
        None => Ok(Some(max.level)),
        Some(wanted) if more_oversight(wanted, max.level) == wanted => Ok(Some(wanted)),
        Some(wanted) => Err(format!(
            "this job asks for {wanted}, and the project allows {name} at most {}",
            max.level
        )),
    }
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
        // the older spelling of the AI member finds the same member
        assert_eq!(
            effective(&project, "ai:claude@joy", DEV, None).unwrap(),
            eff
        );
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
}
