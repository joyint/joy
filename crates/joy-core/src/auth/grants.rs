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
use sha2::{Digest, Sha256};

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

/// What a token carries for this grant: a hash of the signed text and
/// the signature, or [`NO_GRANT`].
pub fn grant_hash(grant: Option<&DelegationGrant>) -> String {
    match grant {
        None => NO_GRANT.to_string(),
        Some(grant) => {
            let mut hasher = Sha256::new();
            for cap in &grant.capabilities {
                hasher.update(cap.to_string().as_bytes());
                hasher.update(b",");
            }
            hasher.update(grant.level.to_string().as_bytes());
            hasher.update(b"|");
            hasher.update(grant.signature.as_bytes());
            hex::encode(&hasher.finalize()[..16])
        }
    }
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

/// Sign a person's own grant for `ai`: at most the project maximum.
///
/// The capabilities and the level have to lie within what the project
/// allows the AI member; the refusal names the maximum.
pub fn sign_personal(
    project: &Project,
    ai_key: &str,
    delegator: &Member,
    keypair: &IdentityKeypair,
    capabilities: &[Capability],
    level: InteractionLevel,
) -> Result<DelegationGrant, JoyError> {
    let name = ai_member_name(ai_key);
    let max = maximum(project, ai_key).map_err(JoyError::Other)?;
    let beyond: Vec<String> = capabilities
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
    let delegator_key = delegator
        .verify_key
        .as_deref()
        .ok_or_else(|| JoyError::Other("you have no key yet: run `joy auth init`".into()))?;
    let mut caps = capabilities.to_vec();
    caps.sort();
    caps.dedup();
    let text = personal_text(project, ai_key, &caps, level, delegator_key);
    Ok(DelegationGrant {
        capabilities: caps,
        level,
        signed_at: Utc::now(),
        signature: hex::encode(keypair.sign(text.as_bytes())),
    })
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
    let delegator = project
        .member_by_key(delegator_key)
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
        member.ai_delegations.insert(
            "claude".into(),
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
        let signed = {
            let delegator = project.member_by_key(who).unwrap();
            sign_personal(project, "claude", delegator, kp, c, l).unwrap()
        };
        project
            .member_by_key_mut(who)
            .unwrap()
            .ai_delegations
            .get_mut("claude")
            .unwrap()
            .grant = Some(signed);
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
        let (project, _, dev_kp) = project();
        let dev = project.member_by_key(DEV).unwrap();
        let more = sign_personal(&project, "claude", dev, &dev_kp, &[Plan, Create], Confirmed)
            .unwrap_err()
            .to_string();
        assert!(more.contains("does not allow claude: create"), "{more}");
        let higher = sign_personal(&project, "claude", dev, &dev_kp, &[Plan], Autonomous)
            .unwrap_err()
            .to_string();
        assert!(higher.contains("at most confirmed"), "{higher}");
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
            .ai_delegations
            .get_mut("claude")
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
                .ai_delegations
                .get_mut("claude")
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
