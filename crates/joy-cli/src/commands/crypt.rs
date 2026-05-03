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
    Add(AddArgs),
    /// Remove an item or path from a zone (returns it to plaintext).
    Rm(RmArgs),
    /// Grant a member access to the zone (requires pairwise wrap; see
    /// note on the module for status).
    Grant(MemberRefArgs),
    /// Revoke a member's access to the zone.
    Revoke(MemberRefArgs),
    /// Show what is encrypted and who has access.
    List,
    /// High-level summary of Crypt configuration.
    Status,
    /// Manage named zones (list, rm).
    Zone(ZoneArgs),
}

#[derive(Args)]
struct AddArgs {
    /// Item ID (e.g. JOY-0123) or path glob (e.g. data/customer-x/).
    /// Required unless `--all` is given.
    target: Option<String>,
    /// Encrypt every item in the project under the addressed zone and
    /// flip the project default so newly created items inherit the
    /// zone (ADR-038 whole-project mode). Mutually exclusive with the
    /// positional target.
    #[arg(long, conflicts_with = "target")]
    all: bool,
}

#[derive(Args)]
struct RmArgs {
    /// Item ID (e.g. JOY-0123) or path glob (e.g. data/customer-x/).
    /// Required unless `--all` is given.
    target: Option<String>,
    /// Remove every item from the addressed zone (and clear the
    /// project default). Inverse of `add --all`.
    #[arg(long, conflicts_with = "target")]
    all: bool,
}

#[derive(Args)]
struct MemberRefArgs {
    /// Member ID (email or ai:tool@joy).
    member: String,
}

#[derive(Args)]
struct ZoneArgs {
    #[command(subcommand)]
    command: ZoneCommand,
}

#[derive(Subcommand)]
enum ZoneCommand {
    /// List all configured zones with their member and item counts.
    List,
    /// Remove a named zone. Refuses to drop a zone that still has
    /// items or members assigned, so revocations have to happen first.
    Rm(ZoneRmArgs),
}

#[derive(Args)]
struct ZoneRmArgs {
    /// Name of the zone to remove. The implicit "default" zone may
    /// only be removed when it is empty.
    name: String,
}

pub fn run(args: CryptArgs) -> Result<()> {
    let zone = args.zone.unwrap_or_else(|| core_crypt::DEFAULT_ZONE.to_string());
    match args.command {
        CryptCommand::Add(t) => match (t.all, t.target.as_deref()) {
            (true, _) => run_add_all(&zone, args.passphrase.as_deref()),
            (false, Some(target)) => run_add(&zone, target, args.passphrase.as_deref()),
            (false, None) => bail!("specify a target item/path or use --all"),
        },
        CryptCommand::Rm(t) => match (t.all, t.target.as_deref()) {
            (true, _) => run_rm_all(&zone, args.passphrase.as_deref()),
            (false, Some(target)) => run_rm(&zone, target, args.passphrase.as_deref()),
            (false, None) => bail!("specify a target item/path or use --all"),
        },
        CryptCommand::Grant(m) => run_grant(&zone, &m.member, args.passphrase.as_deref()),
        CryptCommand::Revoke(m) => run_revoke(&zone, &m.member),
        CryptCommand::List => run_list(&zone),
        CryptCommand::Status => run_status(),
        CryptCommand::Zone(z) => match z.command {
            ZoneCommand::List => run_zone_list(),
            ZoneCommand::Rm(args) => run_zone_rm(&args.name),
        },
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
            core_crypt::wrap_for_self(&self.zone_key, &self.zone, &self.acting_seed);
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

fn run_add_all(zone: &str, passphrase: Option<&str>) -> Result<()> {
    let unlocked = unlock_zone(zone, passphrase, true)?;
    let items = joy_core::items::load_items(&unlocked.root).unwrap_or_default();
    let mut updated = 0usize;
    let mut already = 0usize;
    for item in &items {
        if item.crypt_zone.as_deref() == Some(zone) {
            already += 1;
            continue;
        }
        let item_path = joy_core::items::find_item_file(&unlocked.root, &item.id)?;
        let mut item: joy_core::model::item::Item = store::read_yaml(&item_path)?;
        item.crypt_zone = Some(zone.to_string());
        store::write_yaml(&item_path, &item)?;
        if let Ok(rel) = item_path.strip_prefix(&unlocked.root) {
            joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
        }
        updated += 1;
    }
    println!(
        "{} item(s) tagged with zone '{}'; {} already tagged.",
        updated, zone, already
    );
    println!(
        "Note: the project-default for new items is not yet auto-applied; \
         add new items with `joy add ... --crypt` (or `joy edit <ID> --crypt`) \
         to keep them in the zone."
    );
    unlocked.save(&format!("crypt add --all (zone {zone})"))
}

fn run_rm_all(zone: &str, passphrase: Option<&str>) -> Result<()> {
    let unlocked = unlock_zone(zone, passphrase, false)?;
    let items = joy_core::items::load_items(&unlocked.root).unwrap_or_default();
    let mut updated = 0usize;
    for item in &items {
        if item.crypt_zone.as_deref() != Some(zone) {
            continue;
        }
        let item_path = joy_core::items::find_item_file(&unlocked.root, &item.id)?;
        let mut item: joy_core::model::item::Item = store::read_yaml(&item_path)?;
        item.crypt_zone = None;
        store::write_yaml(&item_path, &item)?;
        if let Ok(rel) = item_path.strip_prefix(&unlocked.root) {
            joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
        }
        updated += 1;
    }
    let project_path = store::joy_dir(&unlocked.root).join(store::PROJECT_FILE);
    let project = unlocked.project; // owned
    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel]);
    joy_core::git_ops::auto_git_post_command(
        &unlocked.root,
        &format!("crypt rm --all (zone {zone})"),
        &unlocked.acting_email,
    );
    println!("Removed {} item(s) from zone '{}'.", updated, zone);
    Ok(())
}

fn run_zone_list() -> Result<()> {
    let (root, project, _email) = load_context()?;
    if project.crypt.is_empty() {
        println!("No zones configured.");
        return Ok(());
    }
    let items = joy_core::items::load_items(&root).unwrap_or_default();
    println!("{:<20} {:>8} {:>8} {:>8}", "ZONE", "PATHS", "ITEMS", "MEMBERS");
    for (name, zone) in &project.crypt.zones {
        let item_count = items
            .iter()
            .filter(|i| i.crypt_zone.as_deref() == Some(name.as_str()))
            .count();
        let member_count = project
            .members
            .values()
            .filter(|m| m.crypt_wraps.contains_key(name))
            .count();
        println!(
            "{:<20} {:>8} {:>8} {:>8}",
            name,
            zone.paths.len(),
            item_count,
            member_count
        );
    }
    Ok(())
}

fn run_zone_rm(name: &str) -> Result<()> {
    let (root, mut project, email) = load_context()?;
    if !project.crypt.zones.contains_key(name) {
        bail!("zone '{}' is not registered", name);
    }
    let items = joy_core::items::load_items(&root).unwrap_or_default();
    let item_count = items
        .iter()
        .filter(|i| i.crypt_zone.as_deref() == Some(name))
        .count();
    let member_count = project
        .members
        .values()
        .filter(|m| m.crypt_wraps.contains_key(name))
        .count();
    if item_count > 0 || member_count > 0 {
        bail!(
            "zone '{}' is not empty: {} item(s), {} member(s) still assigned. \
             Run `joy crypt rm --all --zone {}` and `joy crypt revoke <member> --zone {}` first.",
            name,
            item_count,
            member_count,
            name,
            name
        );
    }
    project.crypt.zones.remove(name);
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&root, &[&rel]);
    joy_core::git_ops::auto_git_post_command(&root, &format!("crypt zone rm {name}"), &email);
    println!("Removed zone '{}'.", name);
    Ok(())
}

fn run_grant(zone: &str, target_member: &str, passphrase: Option<&str>) -> Result<()> {
    let unlocked = unlock_zone(zone, passphrase, false)?;
    let target = unlocked
        .project
        .members
        .get(target_member)
        .ok_or_else(|| anyhow::anyhow!("member '{}' not found", target_member))?;
    let target_verify_hex = target.verify_key.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no verify_key yet. They must run `joy auth` (or `joy auth --otp`) before \
             they can receive a Crypt grant.",
            target_member
        )
    })?;
    let target_verify_key = joy_core::auth::PublicKey::from_hex(target_verify_hex)?;
    let granter_verify_hex = unlocked
        .project
        .members
        .get(&unlocked.acting_email)
        .and_then(|m| m.verify_key.clone())
        .ok_or_else(|| anyhow::anyhow!("granter has no verify_key registered"))?;
    let granter_verify_key = joy_core::auth::PublicKey::from_hex(&granter_verify_hex)?;

    // Wrap the zone key for the target. The granter's verify_key
    // travels in the wrap header so the target can locate the right
    // X25519 public for ECDH.
    let wrap_hex = joy_core::crypt::wrap_for_member(
        &unlocked.zone_key,
        &unlocked.zone,
        &unlocked.acting_seed,
        &granter_verify_key,
        &target_verify_key,
    );

    // Persist the new wrap on the target's member entry. Re-load the
    // project to keep the granter's own wrap untouched (unlocked.save
    // would overwrite the granter's crypt_wraps entry too).
    let project_path = store::joy_dir(&unlocked.root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;
    project
        .crypt
        .zones
        .entry(unlocked.zone.clone())
        .or_insert_with(CryptZone::default);
    let m = project
        .members
        .get_mut(target_member)
        .expect("target member existed at unwrap time");
    m.crypt_wraps.insert(unlocked.zone.clone(), wrap_hex);

    // Ensure the granter's own wrap is also present (auto-create path
    // when this is the first add+grant in the same session).
    let granter_wrap = joy_core::crypt::wrap_for_self(
        &unlocked.zone_key,
        &unlocked.zone,
        &unlocked.acting_seed,
    );
    let g = project.members.get_mut(&unlocked.acting_email).unwrap();
    g.crypt_wraps
        .entry(unlocked.zone.clone())
        .or_insert(granter_wrap);

    store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel]);
    joy_core::git_ops::auto_git_post_command(
        &unlocked.root,
        &format!("crypt grant {target_member} (zone {})", unlocked.zone),
        &unlocked.acting_email,
    );
    println!(
        "Granted {} access to zone '{}'.",
        target_member, unlocked.zone
    );
    Ok(())
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
