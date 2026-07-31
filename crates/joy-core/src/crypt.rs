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

use joy_crypt::zone::ZoneKey;

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
