// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Crypt zone-key domain glue (ADR-038, ADR-040, Crypt.md).
//!
//! The primitive zone/blob crypto (blob encrypt/decrypt, seed/pubkey
//! wrap/unwrap, `ZoneKey`) lives in the leaf crate `joy-crypt`
//! (`joy_crypt::zone`); callers use it directly from there. What stays
//! here is the joy-core-specific glue that the leaf crate must not know
//! about:
//!
//! - the ADR-040 thread-local active-zone-key session context, and
//! - [`platform_zone_keys`], which reads the [`crate::model::Project`]
//!   crypt config.

use std::cell::RefCell;
use std::collections::BTreeMap;

use joy_crypt::zone::{unwrap_for_member, ZoneKey};

// =====================================================================
// Active-session zone-key context (ADR-040)
//
// Joy CLI commands that authenticate up front (passphrase prompt or
// JOY_PASSPHRASE) populate a thread-local map of decrypted zone keys
// before reading items. joy-core's read paths consult this map when
// they encounter a JOYCRYPT blob; when the relevant zone is absent,
// the call returns JoyError::ZoneAccessDenied. Secrets are wiped on
// `clear_active_zone_keys` (typically a Drop guard at end of command).
// =====================================================================

thread_local! {
    static ACTIVE_ZONE_KEYS: RefCell<BTreeMap<String, [u8; 32]>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// Replace the thread-local active zone-keys with the given map.
/// Typically called once per joy command after passphrase verification.
pub fn set_active_zone_keys(keys: BTreeMap<String, [u8; 32]>) {
    ACTIVE_ZONE_KEYS.with(|c| *c.borrow_mut() = keys);
}

/// Wipe the thread-local active zone-keys. Call at the end of a
/// command to ensure no plaintext key material outlives the process
/// (Drop in main.rs covers normal exit).
pub fn clear_active_zone_keys() {
    ACTIVE_ZONE_KEYS.with(|c| c.borrow_mut().clear());
}

/// Look up an active zone key. Used by joy-core's read path when
/// decrypting a JOYCRYPT blob.
pub fn active_zone_key(zone: &str) -> Option<ZoneKey> {
    ACTIVE_ZONE_KEYS.with(|c| {
        c.borrow()
            .get(zone)
            .map(|bytes| ZoneKey::from_bytes(*bytes))
    })
}

/// Whether any zone key is currently active. Useful for joy-cli to
/// decide whether to prompt for passphrase before reading items.
pub fn has_active_zone_keys() -> bool {
    ACTIVE_ZONE_KEYS.with(|c| !c.borrow().is_empty())
}

/// Every zone the PLATFORM was granted (`crypt.zones.*.platform_wrap`,
/// written by `joy crypt grant <zone> platform`), unwrapped with the
/// platform's seed. A wrap that does not open (e.g. the platform key
/// rotated after the grant) is skipped — the zone simply stays closed
/// until re-granted.
pub fn platform_zone_keys(
    project: &crate::model::Project,
    platform_seed: &[u8; 32],
) -> BTreeMap<String, [u8; 32]> {
    let mut keys = BTreeMap::new();
    for (zone, entry) in &project.crypt.zones {
        if let Some(wrap_hex) = entry.platform_wrap.as_deref() {
            if let Ok(zone_key) = unwrap_for_member(wrap_hex, zone, platform_seed) {
                keys.insert(zone.clone(), *zone_key.as_bytes());
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_crypt::identity::Keypair;
    use joy_crypt::zone::{decrypt_blob, encrypt_blob, wrap_for_member};

    #[test]
    fn a_platform_grant_opens_the_zone_with_the_platform_seed() {
        let granter_seed = [1u8; 32];
        let platform_seed = [2u8; 32];
        let granter_kp = Keypair::from_seed(&granter_seed);
        let platform_kp = Keypair::from_seed(&platform_seed);
        let zk = ZoneKey::generate();
        let wrap_hex = wrap_for_member(
            &zk,
            "geheim",
            &granter_seed,
            &granter_kp.public_key(),
            &platform_kp.public_key(),
        );

        let mut project = crate::model::Project::new("t".into(), None);
        project
            .crypt
            .zones
            .entry("geheim".into())
            .or_default()
            .platform_wrap = Some(wrap_hex);

        let keys = platform_zone_keys(&project, &platform_seed);
        assert_eq!(keys.get("geheim"), Some(zk.as_bytes()));
        // the wrong seed opens nothing
        assert!(platform_zone_keys(&project, &[9u8; 32]).is_empty());

        // and the key actually decrypts zone content
        let blob = encrypt_blob("geheim", &zk, b"item: yaml");
        set_active_zone_keys(keys);
        let (zone, plain) = decrypt_blob(active_zone_key, &blob).unwrap();
        assert_eq!(zone, "geheim");
        assert_eq!(plain, b"item: yaml");
    }
}
