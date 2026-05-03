// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy crypt` - manage Crypt zones, items, paths, and grants.
//!
//! Implements the default-zone CLI surface from Crypt.md / ADR-038.
//! `--zone <name>` (JOY-0149-8D), `--all` (JOY-014A-F9), and the Git
//! filter integration (JOY-014B-09) extend the same vocabulary.
//!
//! Cross-member `grant` requires a pairwise wrap (X25519 ECDH over the
//! Ed25519 verify_key), which is tracked separately. Until that lands,
//! this CLI delivers single-member zone management plus `revoke`,
//! `list`, and `status` so the data-model and Git-filter work can land
//! in parallel.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};

use joy_core::crypt as core_crypt;
use joy_core::model::project::{CryptZone, Project};
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::commands::auth::read_passphrase;

#[derive(Args)]
pub struct CryptArgs {
    #[command(subcommand)]
    command: CryptCommand,

    /// Passphrase of the acting member (non-interactive). Required by
    /// every subcommand that touches zone keys (add, rm, revoke).
    #[arg(long, global = true)]
    passphrase: Option<String>,

    /// Operate on a named zone (default: the implicit "default" zone).
    /// Auto-creates the zone on first reference.
    #[arg(long, global = true)]
    zone: Option<String>,
}

#[derive(Subcommand)]
enum CryptCommand {
    /// Encrypt an item (by ID) or path glob within a zone.
    Add(TargetArgs),
    /// Remove an item or path from a zone (returns it to plaintext).
    Rm(TargetArgs),
    /// Grant a member access to the zone (requires pairwise wrap; see
    /// note on the module for status).
    Grant(MemberRefArgs),
    /// Revoke a member's access to the zone.
    Revoke(MemberRefArgs),
    /// Show what is encrypted and who has access.
    List,
    /// High-level summary of Crypt configuration.
    Status,
}

#[derive(Args)]
struct TargetArgs {
    /// Item ID (e.g. JOY-0123) or path glob (e.g. data/customer-x/).
    target: String,
}

#[derive(Args)]
struct MemberRefArgs {
    /// Member ID (email or ai:tool@joy).
    member: String,
}

pub fn run(args: CryptArgs) -> Result<()> {
    let zone = args.zone.unwrap_or_else(|| core_crypt::DEFAULT_ZONE.to_string());
    match args.command {
        CryptCommand::Add(t) => run_add(&zone, &t.target, args.passphrase.as_deref()),
        CryptCommand::Rm(t) => run_rm(&zone, &t.target, args.passphrase.as_deref()),
        CryptCommand::Grant(m) => run_grant(&zone, &m.member),
        CryptCommand::Revoke(m) => run_revoke(&zone, &m.member),
        CryptCommand::List => run_list(&zone),
        CryptCommand::Status => run_status(),
    }
}

fn load_context() -> Result<(std::path::PathBuf, Project, String)> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let project = store::read_project(&project_path)?;
    let email = joy_core::vcs::default_vcs().user_email()?;
    Ok((root, project, email))
}

/// Unwrap the acting member's wrap for the zone, or generate a fresh
/// zone key if `autocreate` is allowed and no wrap exists.
fn unlock_zone(zone: &str, passphrase_flag: Option<&str>, autocreate: bool) -> Result<UnlockedZone> {
    let (root, project, email) = load_context()?;
    let acting = project
        .members
        .get(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member", email))?;
    if acting.verify_key.is_none() {
        bail!(
            "Authentication not initialized for {}. Run `joy auth init` first.",
            email
        );
    }
    let passphrase = read_passphrase(passphrase_flag, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(acting, &passphrase)?;

    let zone_key = match acting.crypt_wraps.get(zone) {
        Some(wrap_hex) => core_crypt::unwrap_for_member(wrap_hex, zone, &unlocked.seed)?,
        None => {
            if !autocreate {
                bail!(
                    "{} has no access to zone '{}'. Ask a member with access to grant first.",
                    email,
                    zone
                );
            }
            core_crypt::ZoneKey::generate()
        }
    };

    Ok(UnlockedZone {
        root,
        project,
        acting_email: email,
        acting_seed: unlocked.seed,
        zone: zone.to_string(),
        zone_key,
    })
}

struct UnlockedZone {
    root: std::path::PathBuf,
    project: Project,
    acting_email: String,
    acting_seed: [u8; 32],
    zone: String,
    zone_key: core_crypt::ZoneKey,
}

impl UnlockedZone {
    fn save(mut self, summary: &str) -> Result<()> {
        self.project
            .crypt
            .zones
            .entry(self.zone.clone())
            .or_insert_with(CryptZone::default);
        let wrap_hex =
            core_crypt::wrap_for_member(&self.zone_key, &self.zone, &self.acting_seed);
        let m = self.project.members.get_mut(&self.acting_email).unwrap();
        m.crypt_wraps.insert(self.zone.clone(), wrap_hex);

        let project_path = store::joy_dir(&self.root).join(store::PROJECT_FILE);
        store::write_yaml_preserve(&project_path, &self.project)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&self.root, &[&rel]);
        joy_core::git_ops::auto_git_post_command(&self.root, summary, &self.acting_email);
        Ok(())
    }
}

fn run_add(zone: &str, target: &str, passphrase: Option<&str>) -> Result<()> {
    let mut unlocked = unlock_zone(zone, passphrase, true)?;

    let summary = if looks_like_item_id(target) {
        let item_path = joy_core::items::find_item_file(&unlocked.root, target)?;
        let mut item: joy_core::model::item::Item = store::read_yaml(&item_path)?;
        if item.crypt_zone.as_deref() == Some(zone) {
            println!("{} is already in zone '{}'.", target, zone);
            return Ok(());
        }
        item.crypt_zone = Some(zone.to_string());
        store::write_yaml(&item_path, &item)?;
        if let Ok(rel) = item_path.strip_prefix(&unlocked.root) {
            joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
        }
        println!("Added {} to zone '{}'.", target, zone);
        format!("crypt add {target} (zone {zone})")
    } else {
        let zone_entry = unlocked
            .project
            .crypt
            .zones
            .entry(zone.to_string())
            .or_insert_with(CryptZone::default);
        if zone_entry.paths.iter().any(|p| p == target) {
            println!("Path '{}' is already in zone '{}'.", target, zone);
            return Ok(());
        }
        zone_entry.paths.push(target.to_string());
        println!("Added path '{}' to zone '{}'.", target, zone);
        format!("crypt add {target} (zone {zone})")
    };

    unlocked.save(&summary)
}

fn run_rm(zone: &str, target: &str, passphrase: Option<&str>) -> Result<()> {
    let mut unlocked = unlock_zone(zone, passphrase, false)?;

    let summary = if looks_like_item_id(target) {
        let item_path = joy_core::items::find_item_file(&unlocked.root, target)?;
        let mut item: joy_core::model::item::Item = store::read_yaml(&item_path)?;
        match item.crypt_zone.as_deref() {
            Some(z) if z == zone => {
                item.crypt_zone = None;
                store::write_yaml(&item_path, &item)?;
                if let Ok(rel) = item_path.strip_prefix(&unlocked.root) {
                    joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
                }
                println!("Removed {} from zone '{}'.", target, zone);
            }
            Some(z) => bail!(
                "{} is in zone '{}', not '{}'. Use `--zone {}` to remove it.",
                target,
                z,
                zone,
                z
            ),
            None => {
                println!("{} is not in any Crypt zone.", target);
                return Ok(());
            }
        }
        format!("crypt rm {target} (zone {zone})")
    } else {
        let zone_entry = unlocked
            .project
            .crypt
            .zones
            .get_mut(zone)
            .ok_or_else(|| anyhow::anyhow!("zone '{}' does not exist", zone))?;
        let before = zone_entry.paths.len();
        zone_entry.paths.retain(|p| p != target);
        if zone_entry.paths.len() == before {
            println!("Path '{}' is not in zone '{}'.", target, zone);
            return Ok(());
        }
        println!("Removed path '{}' from zone '{}'.", target, zone);
        format!("crypt rm {target} (zone {zone})")
    };

    unlocked.save(&summary)
}

fn run_grant(_zone: &str, _target_member: &str) -> Result<()> {
    bail!(
        "joy crypt grant requires the X25519-based pairwise wrap primitive, tracked in \
         JOY-0157-86. The default-zone CLI ships without grant in this release; the \
         granter implicitly holds the zone key after `joy crypt add`."
    )
}

fn run_revoke(zone: &str, target_member: &str) -> Result<()> {
    let (root, mut project, email) = load_context()?;
    if !project.members.contains_key(target_member) {
        bail!("member '{}' not found", target_member);
    }
    let removed = project
        .members
        .get_mut(target_member)
        .map(|m| m.crypt_wraps.remove(zone).is_some())
        .unwrap_or(false);
    if !removed {
        println!(
            "{} had no access to zone '{}'; nothing to revoke.",
            target_member, zone
        );
        return Ok(());
    }
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);
    joy_core::git_ops::auto_git_post_command(
        &root,
        &format!("crypt revoke {target_member} (zone {zone})"),
        &email,
    );
    println!("Revoked {}'s access to zone '{}'.", target_member, zone);
    println!(
        "Note: rotating the zone key after a revoke is recommended; \
         previously-shared content remains decryptable to the revoked member \
         if they retained a copy of the working-directory ciphertext."
    );
    Ok(())
}

fn run_list(zone: &str) -> Result<()> {
    let (root, project, _email) = load_context()?;
    let cfg = &project.crypt;
    if cfg.is_empty() && project.members.values().all(|m| m.crypt_wraps.is_empty()) {
        println!("No Crypt zones configured.");
        return Ok(());
    }

    println!("Zone: {}", zone);
    println!();
    println!("Paths:");
    if let Some(z) = cfg.zones.get(zone) {
        if z.paths.is_empty() {
            println!("  (none)");
        } else {
            for p in &z.paths {
                println!("  {}", p);
            }
        }
    } else {
        println!("  (zone not registered)");
    }

    println!();
    println!("Items:");
    let items = joy_core::items::load_items(&root).unwrap_or_default();
    let mut found_any = false;
    for item in &items {
        if item.crypt_zone.as_deref() == Some(zone) {
            println!("  {} {}", item.id, item.title);
            found_any = true;
        }
    }
    if !found_any {
        println!("  (none)");
    }

    println!();
    println!("Members with access:");
    let mut found_any = false;
    for (email, member) in &project.members {
        if member.crypt_wraps.contains_key(zone) {
            println!("  {}", email);
            found_any = true;
        }
    }
    if !found_any {
        println!("  (none)");
    }

    Ok(())
}

fn run_status() -> Result<()> {
    let (root, project, email) = load_context()?;
    let cfg = &project.crypt;
    let zone_count = cfg.zones.len();
    let item_count_total = joy_core::items::load_items(&root)
        .unwrap_or_default()
        .iter()
        .filter(|i| i.crypt_zone.is_some())
        .count();
    let me_access = project
        .members
        .get(&email)
        .map(|m| m.crypt_wraps.len())
        .unwrap_or(0);

    println!("Crypt status:");
    println!("  zones registered:  {}", zone_count);
    println!("  items in any zone: {}", item_count_total);
    println!("  your access:       {} zone(s)", me_access);
    if zone_count == 0 && item_count_total == 0 {
        println!();
        println!("No encryption configured. Use `joy crypt add <id|path>` to start.");
    }
    Ok(())
}

fn looks_like_item_id(s: &str) -> bool {
    !s.contains('/') && !s.contains('.') && s.contains('-')
}
