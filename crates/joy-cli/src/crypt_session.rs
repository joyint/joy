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
use joy_core::context::Context;
use joy_core::vcs::Vcs;

use crate::commands::auth::read_passphrase;

/// Load the project [`Context`] and install zone keys before returning,
/// so any subsequent read or write of items in a Crypt zone succeeds
/// without each command having to remember to call
/// [`ensure_zone_keys`]. Plaintext-only projects skip the prompt via
/// the [`ensure_zone_keys`] pre-check (JOY-0173-B3).
pub fn load_context(passphrase: Option<&str>) -> Result<Context> {
    load_context_with_stdin(passphrase, false)
}

/// Same as [`load_context`] but also forwards a `--passphrase-stdin`
/// signal to the prompt helper (JOY-018E-21). The plain `load_context`
/// wrapper covers the many callers that have no passphrase flag at
/// all and therefore can never use stdin either.
pub fn load_context_with_stdin(passphrase: Option<&str>, from_stdin: bool) -> Result<Context> {
    let ctx = Context::load()?;
    ensure_zone_keys_with_stdin(passphrase, from_stdin)?;
    Ok(ctx)
}

/// Populate `joy_core::crypt`'s thread-local zone-key context for
/// the active member. Idempotent: returns immediately when keys are
/// already installed or when there are no wraps to unwrap.
///
/// Two unwrap paths coexist (ADR-041):
/// - Human members: unwrap `members.<email>.crypt_wraps[zone]` using the
///   passphrase-derived seed.
/// - AI Tool sessions with `--crypt` scope: unwrap `crypt.zones[zone]
///   .delegations[ai-member][operator]` using the delegation private key
///   embedded in `JOY_SESSION` (no passphrase prompt for the AI; the
///   operator already typed it at token issuance).
pub fn ensure_zone_keys(passphrase_flag: Option<&str>) -> Result<()> {
    ensure_zone_keys_with_stdin(passphrase_flag, false)
}

/// Variant of [`ensure_zone_keys`] that accepts a `--passphrase-stdin`
/// signal forwarded by callers that surface that flag on their clap
/// args (JOY-018E-21).
pub fn ensure_zone_keys_with_stdin(passphrase_flag: Option<&str>, from_stdin: bool) -> Result<()> {
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
    let Some(member) = project.member_by_email(&email) else {
        return Ok(());
    };

    // AI Tool path (ADR-041 §5): the delegation private key embedded in
    // JOY_SESSION unwraps zone keys via per-(operator, AI) wraps under
    // crypt.zones.<zone>.delegations.<ai>.<operator>. No passphrase
    // prompt - the operator already authenticated at token issuance.
    if joy_core::model::project::is_ai_member(&email) {
        let env_value = match std::env::var("JOY_SESSION") {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        let Some((_, _, Some(delegation_priv))) =
            joy_core::auth::session::parse_session_env_full(&env_value)
        else {
            // No --crypt scope on this session; AI cannot unwrap zones.
            return Ok(());
        };
        let mut keys = std::collections::BTreeMap::new();
        for (zone_name, zone) in &project.crypt.zones {
            let Some(per_ai) = zone.delegations.get(&email) else {
                continue;
            };
            // Try each operator wrap: the AI session derives from one
            // specific operator's delegation, so only that operator's
            // wrap will unwrap. Walking is cheap (one or a few entries).
            for wrap_hex in per_ai.values() {
                if let Ok(zk) =
                    joy_crypt::zone::unwrap_for_member(wrap_hex, zone_name, &delegation_priv)
                {
                    keys.insert(zone_name.clone(), *zk.as_bytes());
                    break;
                }
            }
        }
        joy_core::crypt::set_active_zone_keys(keys);
        return Ok(());
    }

    if member.crypt_wraps.is_empty() {
        return Ok(());
    }

    // Pre-check: if no item in the project is actually encrypted (and
    // no plaintext item carries a crypt_zone marker pointing at one of
    // our wrapped zones), unwrapping serves no purpose. Skip the
    // prompt entirely. See JOY-0173-B3.
    let metas = joy_core::items::list_item_metadata(&root).unwrap_or_default();
    let any_relevant_encrypted = metas.iter().any(|m| {
        m.zone()
            .map(|z| member.crypt_wraps.contains_key(z))
            .unwrap_or(false)
    });
    if !any_relevant_encrypted {
        return Ok(());
    }

    // Allow non-interactive callers (CI, scripts, tests) to supply
    // the passphrase via JOY_PASSPHRASE when no --passphrase flag was
    // passed on the command line. Visible to the same process tree as
    // a CLI flag, but does not require every command to expose its
    // own --passphrase argument.
    let env_passphrase = std::env::var("JOY_PASSPHRASE").ok();
    let effective_flag = passphrase_flag.or(env_passphrase.as_deref());
    let passphrase = read_passphrase(effective_flag, from_stdin, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(member, &passphrase)?;
    let mut keys = std::collections::BTreeMap::new();
    for (zone, wrap_hex) in &member.crypt_wraps {
        if let Ok(zk) = joy_crypt::zone::unwrap_for_member(wrap_hex, zone, &unlocked.seed) {
            keys.insert(zone.clone(), *zk.as_bytes());
        }
    }
    joy_core::crypt::set_active_zone_keys(keys);
    Ok(())
}
