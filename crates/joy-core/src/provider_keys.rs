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

/// The recipients a NEW key entry must be wrapped for: the owner, the
/// platform (when registered), and — for a team key — every member with
/// an identity key. Returns (recipient id, x25519 public) pairs;
/// members without identity keys are skipped and reported.
pub fn wrap_recipients(
    project: &Project,
    owner: &str,
    for_all: bool,
) -> Result<(Vec<(String, [u8; 32])>, Vec<String>), JoyError> {
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
}
