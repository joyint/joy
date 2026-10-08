// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Who stands behind a member entry, and how that is checked.
//!
//! A member entry is something a person with the manage capability put
//! there, and a signature says so. What exactly is signed depends on how
//! the project keeps its members (JI-019D-46):
//!
//! * In a project from before the member files, an **attestation** over
//!   the member's address, capabilities and invitation, re-signed on
//!   every change ([`super::attestation`]).
//! * With member files, a person carries an **origin**: who invited them,
//!   signed once and never again, whatever they may do later. An AI
//!   member carries the signature under its capabilities and level, the
//!   **project maximum**, re-signed whenever those change. An AI member
//!   has no origin: a person's delegation is what brings it in.
//!
//! Every host signs through [`sign`] (or [`payload`] and [`apply`] when
//! the key is somewhere else, as in a browser) and never builds what is
//! signed itself.

use chrono::Utc;

use super::{attestation, session, IdentityKeypair, PublicKey};
use crate::error::JoyError;
use crate::model::item::Capability;
use crate::model::project::{
    grant_text, is_ai_member, origin_text, Granted, Member, MemberCapabilities, MemberLayout,
    Origin, Project,
};
use joy_model::InteractionLevel;

/// The level a new AI member's project maximum starts at, and the one a
/// member from before the member files is brought over with.
pub const DEFAULT_AI_LEVEL: InteractionLevel = InteractionLevel::Autonomous;

/// The scope word of a project maximum in the signed text; a person's
/// own grant carries their member id there.
pub const PROJECT_SCOPE: &str = "project";

/// Why a member entry is being signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occasion {
    /// The member is being added.
    New,
    /// What the member may do has changed.
    Changed,
    /// Whoever had signed for this member leaves the project, and the
    /// one removing them signs in their place.
    SignerLeft,
}

enum Form {
    Attestation,
    Origin,
    Maximum,
}

fn form(project: &Project, member_id: &str) -> Form {
    match project.member_layout() {
        MemberLayout::InProject => Form::Attestation,
        MemberLayout::Files if is_ai_member(member_id) => Form::Maximum,
        MemberLayout::Files => Form::Origin,
    }
}

/// The capabilities of a member as a list, in their fixed order.
pub fn capability_list(capabilities: &MemberCapabilities) -> Vec<Capability> {
    match capabilities {
        // Both arms in the capability's own order, never in the order
        // lists are SHOWN in (`Capability::ALL`): this list is part of
        // signed texts, and a display order must be free to change.
        MemberCapabilities::All => {
            let mut all = Capability::ALL.to_vec();
            all.sort();
            all
        }
        MemberCapabilities::Specific(map) => map.keys().copied().collect(),
    }
}

/// The level of an AI member's project maximum.
pub fn maximum_level(member: &Member) -> InteractionLevel {
    member.interaction_level.unwrap_or(DEFAULT_AI_LEVEL)
}

fn maximum_text(project: &Project, name: &str, member: &Member) -> String {
    grant_text(
        &crate::migrations::ai_member_name::read(name),
        &capability_list(&member.capabilities),
        maximum_level(member),
        PROJECT_SCOPE,
        &session::project_id_of(project),
    )
}

fn origin_bytes(project: &Project, address: &str, invitation: Option<&str>) -> Vec<u8> {
    origin_text(
        &session::project_id_of(project),
        address,
        invitation.unwrap_or_default(),
    )
    .into_bytes()
}

/// What the signer signs for `member` on this occasion, or `None` when
/// there is nothing to sign: a person's capabilities changing leaves
/// their origin as it is.
///
/// `member_id` is what the signature names the member by: an AI member's
/// name, a person's address.
pub fn payload(
    project: &Project,
    member_id: &str,
    member: &Member,
    occasion: Occasion,
) -> Option<Vec<u8>> {
    match form(project, member_id) {
        Form::Attestation => Some(
            attestation::signed_fields_for(
                member_id,
                &member.capabilities,
                member.enrollment_verifier.as_deref(),
            )
            .canonical_bytes(),
        ),
        Form::Maximum => Some(maximum_text(project, member_id, member).into_bytes()),
        Form::Origin => match occasion {
            Occasion::Changed => None,
            Occasion::New => Some(origin_bytes(
                project,
                member_id,
                member.enrollment_verifier.as_deref(),
            )),
            Occasion::SignerLeft => {
                let invitation = member.origin.as_ref().and_then(|o| o.invitation.as_deref());
                Some(origin_bytes(project, member_id, invitation))
            }
        },
    }
}

/// Put `signature`, made by `signer` over [`payload`] for the same
/// arguments, on the member.
pub fn apply(
    project: &Project,
    signer: &str,
    member_id: &str,
    member: &mut Member,
    occasion: Occasion,
    signature: &[u8],
) {
    let signature = hex::encode(signature);
    match form(project, member_id) {
        Form::Attestation => {
            member.attestation = Some(crate::model::project::Attestation {
                attester: signer.into(),
                signed_fields: attestation::signed_fields_for(
                    member_id,
                    &member.capabilities,
                    member.enrollment_verifier.as_deref(),
                ),
                signed_at: Utc::now(),
                signature,
            });
        }
        Form::Maximum => {
            member.interaction_level = Some(maximum_level(member));
            member.granted = Some(Granted {
                by: signer.into(),
                at: Utc::now(),
                signature,
            });
        }
        Form::Origin => {
            if occasion == Occasion::Changed {
                return;
            }
            let invitation = match occasion {
                Occasion::New => member.enrollment_verifier.clone(),
                _ => member.origin.as_ref().and_then(|o| o.invitation.clone()),
            };
            member.origin = Some(Origin {
                attester: signer.into(),
                signed_at: Utc::now(),
                signature: Some(signature),
                invitation,
                commit: None,
            });
        }
    }
}

/// Sign for `member` with the signer's own key: [`payload`] and
/// [`apply`] in one, for a host that holds the key.
pub fn sign(
    project: &Project,
    signer: &str,
    keypair: &IdentityKeypair,
    member_id: &str,
    member: &mut Member,
    occasion: Occasion,
) {
    if let Some(bytes) = payload(project, member_id, member, occasion) {
        let signature = keypair.sign(&bytes);
        apply(project, signer, member_id, member, occasion, &signature);
    }
}

/// Who signed for this member, in whichever form.
pub fn signer_of(member: &Member) -> Option<&str> {
    if let Some(att) = &member.attestation {
        return Some(att.attester.id());
    }
    if let Some(granted) = &member.granted {
        return Some(granted.by.id());
    }
    member.origin.as_ref().map(|o| o.attester.id())
}

/// The members `leaving` signed for: whoever removes `leaving` signs for
/// them in turn ([`Occasion::SignerLeft`]), so nobody is left with a
/// signature that names someone who is gone.
pub fn signed_by(project: &Project, leaving: &str) -> Vec<String> {
    project
        .members()
        .filter(|(key, member)| key.as_str() != leaving && signer_of(member) == Some(leaving))
        .map(|(key, _)| key.clone())
        .collect()
}

fn signer_key(project: &Project, who: &str, signer: &str) -> Result<PublicKey, JoyError> {
    let entry = project.member_by_key(signer).ok_or_else(|| {
        JoyError::AuthFailed(format!(
            "the entry of {who} is signed by {signer}, who is not a member of this project. \
             Ask a member with the manage capability to remove and re-add {who}."
        ))
    })?;
    let key = entry.verify_key.as_ref().ok_or_else(|| {
        JoyError::AuthFailed(format!(
            "the entry of {who} is signed by {signer}, who has no public key. \
             Ask a member with the manage capability to remove and re-add {who}."
        ))
    })?;
    Ok(PublicKey::from_hex(key)?)
}

fn check(key: &PublicKey, bytes: &[u8], signature_hex: &str) -> bool {
    hex::decode(signature_hex)
        .ok()
        .is_some_and(|sig| key.verify(bytes, &sig).is_ok())
}

/// Whether a person may sign in, as far as their origin goes (member
/// files only; a project from before checks the attestation instead).
///
/// A signed origin has to verify against its signer's key. An origin
/// without a signature belongs to someone who was in the project before
/// the member files and is taken as given. No origin at all is the
/// founder, and only one person can be that.
pub fn verify_origin(
    project: &Project,
    address: &str,
    member_key: &str,
    member: &Member,
) -> Result<(), JoyError> {
    let Some(origin) = &member.origin else {
        let without_origin = project
            .members()
            .filter(|(key, m)| !is_ai_member(key) && m.origin.is_none())
            .count();
        if without_origin > 1 {
            return Err(JoyError::AuthFailed(format!(
                "{address} was not invited by anyone and is not the only one of whom that is \
                 true, so this entry was not made by joy. Ask a member with the manage \
                 capability to remove and re-add {address}."
            )));
        }
        return Ok(());
    };
    let Some(signature) = &origin.signature else {
        return Ok(());
    };
    let _ = member_key;
    let key = signer_key(project, address, origin.attester.id())?;
    let bytes = origin_bytes(project, address, origin.invitation.as_deref());
    if !check(&key, &bytes, signature) {
        return Err(JoyError::AuthFailed(format!(
            "the invitation of {address} does not verify against {}, so this entry was not \
             made by joy. Ask a member with the manage capability to remove and re-add \
             {address}.",
            origin.attester
        )));
    }
    Ok(())
}

/// Whether the project maximum of the AI member `name` is what a person
/// signed. The reason comes back in words a refusal can say.
///
/// What is checked is that a person stands behind it and no AI member:
/// the signer is a member with a key, and not an AI. That the person may
/// manage the project is asked of them when they write the maximum
/// (every host does, before it signs); here it is not, because the
/// person who brings a project from before the member files over signs
/// the maxima as they were, whoever they are (JI-019D-46).
pub fn verify_maximum(project: &Project, name: &str, member: &Member) -> Result<(), String> {
    let Some(granted) = &member.granted else {
        return Err(format!(
            "nobody has signed what {name} may do in this project"
        ));
    };
    let signer = granted.by.id();
    if is_ai_member(signer) {
        return Err(format!(
            "what {name} may do is signed by {signer}, and an AI member signs for nobody"
        ));
    }
    let entry = project
        .member_by_key(signer)
        .ok_or_else(|| format!("what {name} may do is signed by someone who is not a member"))?;
    let key = entry
        .verify_key
        .as_deref()
        .and_then(|hex| PublicKey::from_hex(hex).ok())
        .ok_or_else(|| format!("what {name} may do is signed by a member without a key"))?;
    if !check(
        &key,
        maximum_text(project, name, member).as_bytes(),
        &granted.signature,
    ) {
        return Err(format!(
            "what {name} may do was changed without a signature for it"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keypair(byte: u8) -> IdentityKeypair {
        IdentityKeypair::from_seed(&[byte; 32])
    }

    fn files_project() -> (Project, IdentityKeypair) {
        let mut project = Project::new("Shop".into(), Some("SH".into()));
        project.set_member_layout(MemberLayout::Files);
        let founder_kp = keypair(1);
        let mut founder = Member::new(MemberCapabilities::All);
        founder.verify_key = Some(founder_kp.public_key().to_hex());
        project
            .register_member("founder@example.com", founder)
            .unwrap();
        (project, founder_kp)
    }

    fn caps(list: &[Capability]) -> MemberCapabilities {
        MemberCapabilities::Specific(list.iter().map(|c| (*c, Default::default())).collect())
    }

    #[test]
    fn a_person_gets_an_origin_once_and_a_change_leaves_it_alone() {
        let (project, founder_kp) = files_project();
        let mut second = Member::new(caps(&[Capability::Implement]));
        second.enrollment_verifier = Some("otp-hash".into());
        sign(
            &project,
            "founder@example.com",
            &founder_kp,
            "second@example.com",
            &mut second,
            Occasion::New,
        );
        let origin = second.origin.clone().expect("an origin");
        assert_eq!(origin.attester.id(), "founder@example.com");
        assert_eq!(origin.invitation.as_deref(), Some("otp-hash"));
        assert!(second.attestation.is_none() && second.granted.is_none());

        // the invitation is redeemed and the capabilities change: the
        // origin still verifies, untouched
        second.enrollment_verifier = None;
        second.set_capabilities(MemberCapabilities::All);
        sign(
            &project,
            "founder@example.com",
            &founder_kp,
            "second@example.com",
            &mut second,
            Occasion::Changed,
        );
        assert_eq!(second.origin, Some(origin));
        let mut with_second = project.clone();
        with_second
            .register_member("second@example.com", second.clone())
            .unwrap();
        verify_origin(
            &with_second,
            "second@example.com",
            "second@example.com",
            &second,
        )
        .unwrap();
    }

    #[test]
    fn an_origin_made_for_someone_else_does_not_verify() {
        let (mut project, founder_kp) = files_project();
        let mut second = Member::new(MemberCapabilities::All);
        sign(
            &project,
            "founder@example.com",
            &founder_kp,
            "second@example.com",
            &mut second,
            Occasion::New,
        );
        project
            .register_member("third@example.com", second.clone())
            .unwrap();
        let err = verify_origin(&project, "third@example.com", "third@example.com", &second)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not verify"), "{err}");
    }

    #[test]
    fn only_one_person_is_the_founder() {
        let (mut project, _) = files_project();
        let stranger = Member::new(MemberCapabilities::All);
        project
            .register_member("stranger@example.com", stranger.clone())
            .unwrap();
        let err = verify_origin(
            &project,
            "stranger@example.com",
            "stranger@example.com",
            &stranger,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("was not invited by anyone"), "{err}");
    }

    #[test]
    fn an_ai_members_maximum_is_signed_and_a_change_by_hand_shows() {
        let (mut project, founder_kp) = files_project();
        let mut claude = Member::new(caps(&[Capability::Plan, Capability::Implement]));
        claude.adapter = Some("claude".into());
        sign(
            &project,
            "founder@example.com",
            &founder_kp,
            "claude",
            &mut claude,
            Occasion::New,
        );
        assert_eq!(claude.interaction_level, Some(DEFAULT_AI_LEVEL));
        assert!(claude.origin.is_none() && claude.attestation.is_none());
        project.register_member("claude", claude.clone()).unwrap();
        verify_maximum(&project, "claude", &claude).unwrap();

        // one more capability, written without a signature
        let mut more = claude.clone();
        more.set_capabilities(caps(&[
            Capability::Plan,
            Capability::Implement,
            Capability::Delete,
        ]));
        let why = verify_maximum(&project, "claude", &more).unwrap_err();
        assert!(why.contains("changed without a signature"), "{why}");

        // a lower level, the same
        let mut lower = claude.clone();
        lower.interaction_level = Some(InteractionLevel::Proposing);
        assert!(verify_maximum(&project, "claude", &lower).is_err());

        // signed again after the change: good
        sign(
            &project,
            "founder@example.com",
            &founder_kp,
            "claude",
            &mut more,
            Occasion::Changed,
        );
        verify_maximum(&project, "claude", &more).unwrap();
    }

    #[test]
    fn a_maximum_no_person_signed_counts_for_nothing() {
        let (mut project, _) = files_project();
        let dev_kp = keypair(2);
        let mut dev = Member::new(caps(&[Capability::Implement]));
        dev.verify_key = Some(dev_kp.public_key().to_hex());
        project.register_member("dev@example.com", dev).unwrap();

        let mut claude = Member::new(caps(&[Capability::Plan]));
        assert!(verify_maximum(&project, "claude", &claude)
            .unwrap_err()
            .contains("nobody has signed"));

        // any person's signature stands: whether they may manage was
        // asked when they wrote it
        sign(
            &project,
            "dev@example.com",
            &dev_kp,
            "claude",
            &mut claude,
            Occasion::New,
        );
        verify_maximum(&project, "claude", &claude).unwrap();

        // signed with the AI's own name as the signer
        let mut selfmade = Member::new(caps(&[Capability::Plan]));
        sign(
            &project,
            "claude",
            &dev_kp,
            "claude",
            &mut selfmade,
            Occasion::New,
        );
        assert!(verify_maximum(&project, "claude", &selfmade)
            .unwrap_err()
            .contains("signs for nobody"));

        // signed by somebody who is not in the project
        let mut foreign = Member::new(caps(&[Capability::Plan]));
        sign(
            &project,
            "stranger@example.com",
            &keypair(3),
            "claude",
            &mut foreign,
            Occasion::New,
        );
        assert!(verify_maximum(&project, "claude", &foreign)
            .unwrap_err()
            .contains("not a member"));
    }

    #[test]
    fn a_project_from_before_still_gets_an_attestation() {
        let mut project = Project::new("Old".into(), Some("OL".into()));
        let founder_kp = keypair(1);
        let mut founder = Member::new(MemberCapabilities::All);
        founder.verify_key = Some(founder_kp.public_key().to_hex());
        project
            .register_member("founder@example.com", founder)
            .unwrap();
        let mut ai = Member::new(caps(&[Capability::Plan]));
        sign(
            &project,
            "founder@example.com",
            &founder_kp,
            "claude",
            &mut ai,
            Occasion::New,
        );
        let att = ai.attestation.as_ref().expect("an attestation");
        assert!(ai.granted.is_none() && ai.origin.is_none());
        attestation::verify_attestation(att, &founder_kp.public_key(), "claude", &ai).unwrap();
        assert_eq!(signer_of(&ai), Some("founder@example.com"));
    }
}
