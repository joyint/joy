// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Session-scoped Crypt context for read commands.
//!
//! Joy commands that read items (`joy show`, `joy ls`, `joy edit`,
//! ...) call [`ensure_zone_keys`] before touching `joy-core` read
//! paths. The function is a no-op when the project has no Crypt
//! activity or the acting member has no wraps; otherwise it prompts
//! for the passphrase, derives the seed, unwraps every zone the user
//! has access to, and installs the resulting zone keys into
//! [`joy_core::crypt`]'s thread-local context. Per ADR-040 the keys
//! live only for the lifetime of the joy process.

use anyhow::Result;
use joy_core::vcs::Vcs;

use crate::commands::auth::read_passphrase;

/// Populate `joy_core::crypt`'s thread-local zone-key context for
/// the active member. Idempotent: returns immediately when keys are
/// already installed or when there are no wraps to unwrap.
pub fn ensure_zone_keys(passphrase_flag: Option<&str>) -> Result<()> {
    if joy_core::crypt::has_active_zone_keys() {
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let Some(root) = joy_core::store::find_project_root(&cwd) else {
        return Ok(());
    };
    let project_path = joy_core::store::joy_dir(&root).join(joy_core::store::PROJECT_FILE);
    let project = joy_core::store::read_project(&project_path)?;
    let email = match joy_core::vcs::default_vcs().user_email() {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    let Some(member) = project.members.get(&email) else {
        return Ok(());
    };
    if member.crypt_wraps.is_empty() {
        return Ok(());
    }

    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(member, &passphrase)?;
    let mut keys = std::collections::BTreeMap::new();
    for (zone, wrap_hex) in &member.crypt_wraps {
        if let Ok(zk) = joy_core::crypt::unwrap_for_member(wrap_hex, zone, &unlocked.seed) {
            keys.insert(zone.clone(), *zk.as_bytes());
        }
    }
    joy_core::crypt::set_active_zone_keys(keys);
    Ok(())
}
