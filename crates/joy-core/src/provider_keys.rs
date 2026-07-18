// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Repo-stored provider API keys for AI members (the operator's model,
//! 2026-07-18): the key lives ON the AI member in `project.yaml`,
//! pairwise-encrypted per person (joy-crypt `provider_key`). A personal
//! key carries wraps for its owner and the platform; a team key
//! (`for_all`) carries one wrap per member with an identity key. The
//! plaintext exists only at set time (the owner's client) and at use
//! time (the unwrapping principal).

use std::collections::BTreeMap;

use crate::error::JoyError;
use crate::model::project::{Member, Project, ProviderKeyEntry, PLATFORM_RECIPIENT};

pub use joy_crypt::provider_key::{provider_key_info, unwrap_provider_key, wrap_provider_key};

/// A resolved wrap for one principal: who owns the key, and the
/// ciphertext addressed to the principal.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedWrap {
    pub owner: String,
    pub wrap_hex: String,
    pub for_all: bool,
    pub budget_cents_month: Option<u64>,
}

/// The wrap a principal may use for an AI member's key. Preference: a
/// team key (any owner), else the principal's own personal key. The
/// platform principal is [`PLATFORM_RECIPIENT`]; a personal key also
/// carries a platform wrap, and the platform acts FOR the delegating
/// person, so `acting_for` selects among personal keys.
pub fn resolve_wrap(
    ai_member: &Member,
    principal: &str,
    acting_for: Option<&str>,
) -> Option<ResolvedWrap> {
    let pick = |owner: &str, entry: &ProviderKeyEntry| -> Option<ResolvedWrap> {
        entry.wraps.get(principal).map(|wrap| ResolvedWrap {
            owner: owner.to_string(),
            wrap_hex: wrap.clone(),
            for_all: entry.for_all,
            budget_cents_month: entry.budget_cents_month,
        })
    };
    // team keys first, deterministic owner order
    for (owner, entry) in &ai_member.provider_keys {
        if entry.for_all {
            if let Some(found) = pick(owner, entry) {
                return Some(found);
            }
        }
    }
    // personal: the acting person's own key (or the principal's own)
    let personal_owner = acting_for.unwrap_or(principal);
    ai_member
        .provider_keys
        .get(personal_owner)
        .filter(|entry| !entry.for_all)
        .and_then(|entry| pick(personal_owner, entry))
}

/// The owner's x25519 public key bytes, from their verify_key on record
/// (the reserved platform recipient resolves via `project.platform`).
pub fn owner_public(project: &Project, owner: &str) -> Result<[u8; 32], JoyError> {
    let verify_hex = if owner == PLATFORM_RECIPIENT {
        project
            .platform
            .as_ref()
            .map(|p| p.verify_key.clone())
            .ok_or_else(|| JoyError::AuthFailed("no platform key registered".into()))?
    } else {
        project
            .member_by_key(owner)
            .and_then(|m| m.verify_key.clone())
            .ok_or_else(|| JoyError::AuthFailed(format!("{owner} has no identity key")))?
    };
    Ok(joy_crypt::identity::PublicKey::from_hex(&verify_hex)
        .map_err(|e| JoyError::AuthFailed(format!("verify_key of {owner}: {e}")))?
        .to_x25519_public_bytes())
}

/// (recipient id, x25519 public) pairs a key entry is wrapped for, plus
/// the members skipped for lack of an identity key.
pub type WrapRecipients = (Vec<(String, [u8; 32])>, Vec<String>);

/// The recipients a NEW key entry must be wrapped for: the owner, the
/// platform (when registered), and — for a team key — every member with
/// an identity key. Returns (recipient id, x25519 public) pairs;
/// members without identity keys are skipped and reported.
pub fn wrap_recipients(
    project: &Project,
    owner: &str,
    for_all: bool,
) -> Result<WrapRecipients, JoyError> {
    let mut recipients = Vec::new();
    let mut skipped = Vec::new();
    let mut add = |id: &str| -> Result<(), JoyError> {
        match owner_public(project, id) {
            Ok(public) => {
                recipients.push((id.to_string(), public));
                Ok(())
            }
            Err(_) => {
                skipped.push(id.to_string());
                Ok(())
            }
        }
    };
    add(owner)?;
    if project.platform.is_some() {
        add(PLATFORM_RECIPIENT)?;
    }
    if for_all {
        let others: Vec<String> = project
            .members()
            .map(|(key, _)| key.clone())
            .filter(|key| key != owner && !key.starts_with("ai:"))
            .collect();
        for key in others {
            add(&key)?;
        }
    }
    recipients.dedup_by(|a, b| a.0 == b.0);
    Ok((recipients, skipped))
}

/// Store a wrapped key entry on the AI member (replacing the owner's
/// previous entry for that member).
pub fn store_entry(
    ai_member: &mut Member,
    owner: &str,
    for_all: bool,
    wraps: BTreeMap<String, String>,
    budget_cents_month: Option<u64>,
) {
    ai_member.provider_keys.insert(
        owner.to_string(),
        ProviderKeyEntry {
            for_all,
            wraps,
            budget_cents_month,
        },
    );
}

/// One AI member's team-key coverage gap: identity-carrying recipients
/// (members + the platform) no team entry wraps yet. Personal keys are
/// exempt by design — they bind to their owner.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverageGap {
    pub ai_member: String,
    pub missing: Vec<String>,
}

/// The identity-carrying recipients every team key must cover: the
/// platform (when registered) plus every non-AI member with a
/// verify_key. Members still without an identity key cannot receive a
/// wrap yet; they surface via [`CoverageGap`] only once enrolled.
fn required_team_recipients(project: &Project) -> Vec<String> {
    let mut ids = Vec::new();
    if project.platform.is_some() {
        ids.push(PLATFORM_RECIPIENT.to_string());
    }
    for (key, member) in project.members() {
        if !key.starts_with("ai:") && member.verify_key.is_some() {
            ids.push(key.clone());
        }
    }
    ids
}

/// Team keys whose union of wraps misses an enrolled recipient — the
/// state a project enters when a member gains an identity key AFTER a
/// team key was set. Empty means nothing to re-wrap.
pub fn team_coverage_gaps(project: &Project) -> Vec<CoverageGap> {
    let required = required_team_recipients(project);
    let mut gaps = Vec::new();
    for (key, member) in project.members() {
        if !key.starts_with("ai:") {
            continue;
        }
        let team: Vec<&ProviderKeyEntry> = member
            .provider_keys
            .values()
            .filter(|e| e.for_all)
            .collect();
        if team.is_empty() {
            continue;
        }
        let missing: Vec<String> = required
            .iter()
            .filter(|id| !team.iter().any(|e| e.wraps.contains_key(*id)))
            .cloned()
            .collect();
        if !missing.is_empty() {
            gaps.push(CoverageGap {
                ai_member: key.clone(),
                missing,
            });
        }
    }
    gaps
}

/// What one re-wrap pass did for one AI member. `added` empty with
/// `missing` non-empty means the actor holds no wrap for any team entry
/// and cannot help — another recipient (or the platform) must act.
#[derive(Debug, Clone, PartialEq)]
pub struct RewrapReport {
    pub ai_member: String,
    pub added: Vec<String>,
    pub missing: Vec<String>,
}

/// Close every [`team_coverage_gaps`] hole the actor can: unwrap the
/// team key with the actor's own wrap, then wrap it for each missing
/// recipient. An actor who owns a team entry extends it in place; a
/// foreign actor (typically the platform) stores a complete entry of
/// its own — legitimate, because every team-key recipient knows the
/// plaintext. Idempotent: full coverage yields an empty report.
pub fn rewrap_team_keys(
    project: &mut Project,
    actor: &str,
    actor_x25519_secret: &[u8; 32],
) -> Result<Vec<RewrapReport>, JoyError> {
    struct Plan {
        ai_key: String,
        owner: String,
        extend: bool,
        wraps: BTreeMap<String, String>,
        budget: Option<u64>,
    }
    let gaps = team_coverage_gaps(project);
    let mut plans: Vec<Plan> = Vec::new();
    let mut reports = Vec::new();
    for gap in gaps {
        let ai = project
            .member_by_key(&gap.ai_member)
            .expect("gap came from this project");
        // the actor's own way in: any team entry that wraps them
        let source = ai
            .provider_keys
            .iter()
            .filter(|(_, e)| e.for_all)
            .find_map(|(owner, e)| e.wraps.get(actor).map(|w| (owner.clone(), w.clone())));
        let Some((source_owner, source_wrap)) = source else {
            reports.push(RewrapReport {
                ai_member: gap.ai_member,
                added: Vec::new(),
                missing: gap.missing,
            });
            continue;
        };
        let plaintext = unwrap_provider_key(
            actor_x25519_secret,
            &owner_public(project, &source_owner)?,
            &gap.ai_member,
            &source_wrap,
        )
        .map_err(|e| JoyError::AuthFailed(format!("unwrap as {actor}: {e}")))?;
        let own_entry = ai.provider_keys.get(actor).filter(|e| e.for_all);
        let budget = own_entry.and_then(|e| e.budget_cents_month).or_else(|| {
            ai.provider_keys
                .get(&source_owner)
                .and_then(|e| e.budget_cents_month)
        });
        let (extend, targets): (bool, Vec<String>) = if own_entry.is_some() {
            (true, gap.missing.clone())
        } else {
            // a fresh actor-owned entry stands alone: cover everyone
            let (recipients, _) = wrap_recipients(project, actor, true)?;
            (false, recipients.into_iter().map(|(id, _)| id).collect())
        };
        let mut wraps = BTreeMap::new();
        for id in &targets {
            let public = owner_public(project, id)?;
            wraps.insert(
                id.clone(),
                wrap_provider_key(actor_x25519_secret, &public, &gap.ai_member, &plaintext),
            );
        }
        reports.push(RewrapReport {
            ai_member: gap.ai_member.clone(),
            added: gap.missing,
            missing: Vec::new(),
        });
        plans.push(Plan {
            ai_key: gap.ai_member,
            owner: actor.to_string(),
            extend,
            wraps,
            budget,
        });
    }
    for plan in plans {
        let ai = project
            .member_by_key_mut(&plan.ai_key)
            .expect("planned from this project");
        if plan.extend {
            let entry = ai
                .provider_keys
                .get_mut(&plan.owner)
                .expect("extend plans require the actor's own entry");
            entry.wraps.extend(plan.wraps);
        } else {
            store_entry(ai, &plan.owner, true, plan.wraps, plan.budget);
        }
    }
    Ok(reports)
}

/// What [`purge_member_wraps`] removed for a departing member.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PurgeReport {
    /// Provider-key entries the member OWNED (their personal and team
    /// keys die with them — the ciphertext would be unresolvable once
    /// their verify_key leaves the record).
    pub owned_entries: usize,
    /// Wraps addressed TO the member in entries that stay.
    pub recipient_wraps: usize,
}

/// Strip a departing member out of every AI member's provider keys.
/// Call before `remove_member`. NOTE this is bookkeeping, not
/// revocation: a team-key recipient knew the plaintext, so real
/// revocation means rotating the key at the provider and setting the
/// new one.
pub fn purge_member_wraps(project: &mut Project, member_key: &str) -> PurgeReport {
    let mut report = PurgeReport::default();
    let ai_keys: Vec<String> = project
        .member_keys()
        .filter(|k| k.starts_with("ai:"))
        .cloned()
        .collect();
    for ai_key in ai_keys {
        let ai = project.member_by_key_mut(&ai_key).expect("just listed");
        if ai.provider_keys.remove(member_key).is_some() {
            report.owned_entries += 1;
        }
        for entry in ai.provider_keys.values_mut() {
            if entry.wraps.remove(member_key).is_some() {
                report.recipient_wraps += 1;
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::project::MemberCapabilities;

    fn member() -> Member {
        Member::new(MemberCapabilities::All)
    }

    #[test]
    fn team_keys_win_and_personal_keys_bind_to_their_owner() {
        let mut ai = member();
        store_entry(
            &mut ai,
            "alice@example.com",
            false,
            BTreeMap::from([
                ("alice@example.com".into(), "wrap-alice".into()),
                (PLATFORM_RECIPIENT.into(), "wrap-platform-alice".into()),
            ]),
            None,
        );
        store_entry(
            &mut ai,
            "bob@example.com",
            true,
            BTreeMap::from([
                ("bob@example.com".into(), "wrap-bob".into()),
                ("alice@example.com".into(), "wrap-alice-from-bob".into()),
                (PLATFORM_RECIPIENT.into(), "wrap-platform-bob".into()),
            ]),
            Some(500),
        );

        // the platform acting for alice: bob's TEAM key wins
        let hit = resolve_wrap(&ai, PLATFORM_RECIPIENT, Some("alice@example.com")).unwrap();
        assert_eq!(hit.owner, "bob@example.com");
        assert_eq!(hit.wrap_hex, "wrap-platform-bob");
        assert!(hit.for_all);
        assert_eq!(hit.budget_cents_month, Some(500));

        // alice locally: the team key covers her directly
        let hit = resolve_wrap(&ai, "alice@example.com", None).unwrap();
        assert_eq!(hit.wrap_hex, "wrap-alice-from-bob");

        // without the team key, the platform uses the ACTING person's
        // personal entry, never someone else's
        ai.provider_keys.remove("bob@example.com");
        let hit = resolve_wrap(&ai, PLATFORM_RECIPIENT, Some("alice@example.com")).unwrap();
        assert_eq!(hit.owner, "alice@example.com");
        assert_eq!(hit.wrap_hex, "wrap-platform-alice");
        assert!(resolve_wrap(&ai, PLATFORM_RECIPIENT, Some("carol@example.com")).is_none());
    }

    use joy_crypt::identity::Keypair;

    fn identity_member(seed: u8) -> (Member, Keypair) {
        let kp = Keypair::from_seed(&[seed; 32]);
        let mut m = member();
        m.verify_key = Some(kp.public_key().to_hex());
        (m, kp)
    }

    /// alice owns a team key covering alice+platform; bob enrolled
    /// later, carol has no identity yet.
    fn project_with_gap() -> (Project, Keypair, Keypair, Keypair) {
        let mut project = Project::new("t".into(), None);
        let (alice, alice_kp) = identity_member(1);
        let (bob, bob_kp) = identity_member(2);
        let carol = member();
        let ai = member();
        project.register_member("alice@example.com", alice).unwrap();
        project.register_member("bob@example.com", bob).unwrap();
        project.register_member("carol@example.com", carol).unwrap();
        project.register_member("ai:mistral@joy", ai).unwrap();
        let platform_kp = Keypair::from_seed(&[9; 32]);
        project
            .set_platform_key(&platform_kp.public_key().to_hex())
            .unwrap();
        let wraps = BTreeMap::from([
            (
                "alice@example.com".into(),
                wrap_provider_key(
                    &alice_kp.to_x25519_secret_bytes(),
                    &alice_kp.public_key().to_x25519_public_bytes(),
                    "ai:mistral@joy",
                    "sk-team",
                ),
            ),
            (
                PLATFORM_RECIPIENT.into(),
                wrap_provider_key(
                    &alice_kp.to_x25519_secret_bytes(),
                    &platform_kp.public_key().to_x25519_public_bytes(),
                    "ai:mistral@joy",
                    "sk-team",
                ),
            ),
        ]);
        store_entry(
            project.member_by_key_mut("ai:mistral@joy").unwrap(),
            "alice@example.com",
            true,
            wraps,
            Some(700),
        );
        (project, alice_kp, bob_kp, platform_kp)
    }

    #[test]
    fn gaps_name_the_enrolled_but_uncovered_member_only() {
        let (project, ..) = project_with_gap();
        let gaps = team_coverage_gaps(&project);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].ai_member, "ai:mistral@joy");
        // bob is enrolled and missing; carol has no identity yet
        assert_eq!(gaps[0].missing, vec!["bob@example.com".to_string()]);
    }

    #[test]
    fn the_platform_rewraps_for_the_late_enrollee_and_stays_idempotent() {
        let (mut project, _alice_kp, bob_kp, platform_kp) = project_with_gap();
        let reports = rewrap_team_keys(
            &mut project,
            PLATFORM_RECIPIENT,
            &platform_kp.to_x25519_secret_bytes(),
        )
        .unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].added, vec!["bob@example.com".to_string()]);
        assert!(reports[0].missing.is_empty());

        // bob resolves a wrap and unwraps against the platform's public
        let ai = project.member_by_key("ai:mistral@joy").unwrap();
        let hit = resolve_wrap(ai, "bob@example.com", None).unwrap();
        assert_eq!(hit.owner, PLATFORM_RECIPIENT);
        assert!(hit.for_all);
        assert_eq!(hit.budget_cents_month, Some(700));
        let owner_pub = owner_public(&project, &hit.owner).unwrap();
        assert_eq!(
            unwrap_provider_key(
                &bob_kp.to_x25519_secret_bytes(),
                &owner_pub,
                "ai:mistral@joy",
                &hit.wrap_hex,
            )
            .unwrap(),
            "sk-team"
        );

        // second pass: full coverage, nothing to do
        assert!(team_coverage_gaps(&project).is_empty());
        let again = rewrap_team_keys(
            &mut project,
            PLATFORM_RECIPIENT,
            &platform_kp.to_x25519_secret_bytes(),
        )
        .unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn the_owner_extends_their_own_entry_in_place() {
        let (mut project, alice_kp, bob_kp, _platform_kp) = project_with_gap();
        rewrap_team_keys(
            &mut project,
            "alice@example.com",
            &alice_kp.to_x25519_secret_bytes(),
        )
        .unwrap();
        let ai = project.member_by_key("ai:mistral@joy").unwrap();
        // still ONE entry, alice's, now covering bob too
        assert_eq!(ai.provider_keys.len(), 1);
        let entry = ai.provider_keys.get("alice@example.com").unwrap();
        assert_eq!(entry.budget_cents_month, Some(700));
        let owner_pub = owner_public(&project, "alice@example.com").unwrap();
        assert_eq!(
            unwrap_provider_key(
                &bob_kp.to_x25519_secret_bytes(),
                &owner_pub,
                "ai:mistral@joy",
                entry.wraps.get("bob@example.com").unwrap(),
            )
            .unwrap(),
            "sk-team"
        );
    }

    #[test]
    fn an_actor_without_a_wrap_reports_the_gap_instead_of_touching_it() {
        let (mut project, ..) = project_with_gap();
        // bob has no wrap yet, so bob cannot re-wrap for himself
        let bob_kp = Keypair::from_seed(&[2; 32]);
        let reports = rewrap_team_keys(
            &mut project,
            "bob@example.com",
            &bob_kp.to_x25519_secret_bytes(),
        )
        .unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].added.is_empty());
        assert_eq!(reports[0].missing, vec!["bob@example.com".to_string()]);
        let ai = project.member_by_key("ai:mistral@joy").unwrap();
        assert_eq!(ai.provider_keys.len(), 1);
    }

    #[test]
    fn purging_a_member_drops_their_entries_and_their_wraps() {
        let (mut project, ..) = project_with_gap();
        // alice also holds a personal key on the same AI member
        store_entry(
            project.member_by_key_mut("ai:mistral@joy").unwrap(),
            "bob@example.com",
            false,
            BTreeMap::from([
                ("bob@example.com".into(), "w-bob".into()),
                (PLATFORM_RECIPIENT.into(), "w-p".into()),
            ]),
            None,
        );
        let report = purge_member_wraps(&mut project, "alice@example.com");
        assert_eq!(report.owned_entries, 1);
        assert_eq!(report.recipient_wraps, 0);
        let report = purge_member_wraps(&mut project, PLATFORM_RECIPIENT);
        assert_eq!(report.owned_entries, 0);
        assert_eq!(report.recipient_wraps, 1);
        let ai = project.member_by_key("ai:mistral@joy").unwrap();
        assert_eq!(ai.provider_keys.len(), 1);
        assert!(ai.provider_keys.contains_key("bob@example.com"));
    }
}
