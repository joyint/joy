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
use joy_core::model::project::Project;
use joy_core::store;
use joy_core::vcs::Vcs;

use crate::color;
use crate::commands::auth::read_passphrase;

#[derive(Args)]
pub struct CryptArgs {
    #[command(subcommand)]
    command: CryptCommand,

    /// Passphrase of the acting member (non-interactive).
    #[arg(long, global = true)]
    passphrase: Option<String>,

    /// Read the passphrase from a single line on stdin. Mutually
    /// exclusive with `--passphrase`.
    #[arg(long = "passphrase-stdin", global = true)]
    passphrase_stdin: bool,

    /// Named zone (default: "default", auto-created).
    #[arg(long, global = true)]
    zone: Option<String>,
}

#[derive(Subcommand)]
enum CryptCommand {
    /// Encrypt an item (by ID) or path glob within a zone.
    Add(AddArgs),
    /// Remove an item or path from a zone (returns it to plaintext).
    Rm(RmArgs),
    /// Grant a member access to the zone.
    Grant(MemberRefArgs),
    /// Revoke a member's access to the zone.
    Revoke(MemberRefArgs),
    /// Show what is encrypted and who has access.
    Ls,
    /// High-level summary of Crypt configuration.
    Status,
    /// Manage named zones (ls, rm).
    Zone(ZoneArgs),
    /// Decrypt a Crypt-marked file to stdout.
    Read(FileArgs),
    /// Encrypt stdin into the given Crypt-marked file.
    Write(FileArgs),
    /// Open $EDITOR on a temporary plaintext copy; re-encrypt on save.
    Edit(FileArgs),
    /// Decrypt a Crypt-marked file in place; pair with `joy crypt lock`.
    Unlock(FileArgs),
    /// Re-encrypt a previously unlocked file.
    Lock(FileArgs),
}

#[derive(Args)]
struct AddArgs {
    /// Item ID (e.g. JOY-0123) or path glob (e.g. data/customer-x/).
    /// Required unless `--all` is given.
    #[arg(value_hint = clap::ValueHint::AnyPath)]
    target: Option<String>,
    /// Encrypt every item in the project under the addressed zone and
    /// flip the project default so newly created items inherit the
    /// zone. Mutually exclusive with the positional target.
    #[arg(long, conflicts_with = "target")]
    all: bool,
}

#[derive(Args)]
struct RmArgs {
    /// Item ID (e.g. JOY-0123) or path glob (e.g. data/customer-x/).
    /// Required unless `--all` is given.
    #[arg(value_hint = clap::ValueHint::AnyPath)]
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
struct FileArgs {
    /// Path to the file (relative to project root or absolute).
    #[arg(value_hint = clap::ValueHint::FilePath)]
    file: String,
}

#[derive(Args)]
struct ZoneArgs {
    #[command(subcommand)]
    command: ZoneCommand,
}

#[derive(Subcommand)]
enum ZoneCommand {
    /// List all configured zones with their member and item counts.
    Ls,
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
    let zone = args
        .zone
        .unwrap_or_else(|| core_crypt::DEFAULT_ZONE.to_string());
    let stdin = args.passphrase_stdin;
    let pp = args.passphrase.as_deref();
    match args.command {
        CryptCommand::Add(t) => match (t.all, t.target.as_deref()) {
            (true, _) => run_add_all(&zone, pp, stdin),
            (false, Some(target)) => run_add(&zone, target, pp, stdin),
            (false, None) => bail!("specify a target item/path or use --all"),
        },
        CryptCommand::Rm(t) => match (t.all, t.target.as_deref()) {
            (true, _) => run_rm_all(&zone, pp, stdin),
            (false, Some(target)) => run_rm(&zone, target, pp, stdin),
            (false, None) => bail!("specify a target item/path or use --all"),
        },
        CryptCommand::Grant(m) => run_grant(&zone, &m.member, pp, stdin),
        CryptCommand::Revoke(m) => run_revoke(&zone, &m.member),
        CryptCommand::Ls => run_list(&zone),
        CryptCommand::Status => run_status(),
        CryptCommand::Zone(z) => match z.command {
            ZoneCommand::Ls => run_zone_list(),
            ZoneCommand::Rm(args) => run_zone_rm(&args.name),
        },
        CryptCommand::Read(f) => run_read(&f.file, pp, stdin),
        CryptCommand::Write(f) => run_write(&f.file, pp, stdin),
        CryptCommand::Edit(f) => run_edit(&f.file, pp, stdin),
        CryptCommand::Unlock(f) => run_unlock(&f.file, pp, stdin),
        CryptCommand::Lock(f) => run_lock(&f.file, pp, stdin),
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
fn unlock_zone(
    zone: &str,
    passphrase_flag: Option<&str>,
    passphrase_stdin: bool,
    autocreate: bool,
) -> Result<UnlockedZone> {
    let (root, project, email) = load_context()?;
    let acting = project
        .member_by_email(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member", email))?;
    if acting.verify_key.is_none() {
        bail!(
            "Authentication not initialized for {}. Run `joy auth init` first.",
            email
        );
    }
    let passphrase = read_passphrase(passphrase_flag, passphrase_stdin, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(acting, &passphrase)?;

    let zone_key = match acting.crypt_wraps.get(zone) {
        Some(wrap_hex) => core_crypt::unwrap_for_member(wrap_hex, zone, &unlocked.seed)?,
        None => {
            if !autocreate {
                bail!(
                    "{} has no access to zone '{}'. Ask a member with access to grant first.",
                    joy_core::member_ref::resolve_str(&email),
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
    /// Persist the zone wrap into project.yaml. Splits out from the
    /// later git_add/post_command so callers can encrypt items in
    /// between - if the process dies after this call, the wrap is on
    /// disk and the still-plaintext item is recoverable on retry.
    fn persist_wrap(&mut self) -> Result<()> {
        self.project
            .crypt
            .zones
            .entry(self.zone.clone())
            .or_default();
        let wrap_hex = core_crypt::wrap_for_self(&self.zone_key, &self.zone, &self.acting_seed);
        let m = self
            .project
            .member_by_email_mut(&self.acting_email)
            .unwrap();
        m.crypt_wraps.insert(self.zone.clone(), wrap_hex);

        let project_path = store::joy_dir(&self.root).join(store::PROJECT_FILE);
        store::write_yaml_preserve(&project_path, &self.project)?;
        Ok(())
    }

    /// Make the zone key available to joy-core's encrypt/decrypt
    /// thread-local context. Called before encrypting an item file.
    fn install_zone_key(&self) {
        let mut keys = std::collections::BTreeMap::new();
        keys.insert(self.zone.clone(), *self.zone_key.as_bytes());
        joy_core::crypt::set_active_zone_keys(keys);
    }

    /// Wrap up: stage project.yaml, run the post-command hook.
    fn finalize(self, summary: &str) -> Result<()> {
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&self.root, &[&rel]);
        joy_core::git_ops::auto_git_post_command(&self.root, summary, &self.acting_email);
        Ok(())
    }
}

fn run_add(zone: &str, target: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let mut unlocked = unlock_zone(zone, passphrase, stdin, true)?;

    if looks_like_item_id(target) {
        let item_path = joy_core::items::find_item_file(&unlocked.root, target)?;
        let mut item: joy_core::model::item::Item = store::read_yaml(&item_path)?;
        if item.crypt_zone.as_deref() == Some(zone) {
            println!("{} is already in zone '{}'.", target, zone);
            return Ok(());
        }
        // Persist the wrap to project.yaml first; if encryption fails
        // afterwards the item file is still plaintext on disk and a
        // retry can complete the operation.
        unlocked.persist_wrap()?;
        item.crypt_zone = Some(zone.to_string());
        unlocked.install_zone_key();
        joy_core::items::update_item(&unlocked.root, &item)?;
        joy_core::crypt::clear_active_zone_keys();
        if let Ok(rel) = item_path.strip_prefix(&unlocked.root) {
            joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
        }
        println!("Added {} to zone '{}'.", target, zone);
        unlocked.finalize(&format!("crypt add {target} (zone {zone})"))
    } else {
        let resolved = resolve_target_paths(&unlocked.root, target)?;
        let mut count = 0usize;
        // Persist wrap first for the same crash-safety reason.
        unlocked.persist_wrap()?;
        let zone_entry = unlocked
            .project
            .crypt
            .zones
            .entry(zone.to_string())
            .or_default();
        if !zone_entry.paths.iter().any(|p| p == target) {
            zone_entry.paths.push(target.to_string());
        }
        // Re-write project.yaml after we've added the path to the
        // zones registry. Wrap was already persisted above; this
        // adds the registry entry.
        let project_path = store::joy_dir(&unlocked.root).join(store::PROJECT_FILE);
        store::write_yaml_preserve(&project_path, &unlocked.project)?;
        unlocked.install_zone_key();
        for file in &resolved {
            encrypt_file_in_place(file, zone, &unlocked.zone_key)?;
            if let Ok(rel) = file.strip_prefix(&unlocked.root) {
                joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
            }
            count += 1;
        }
        joy_core::crypt::clear_active_zone_keys();
        println!(
            "Added '{}' to zone '{}' ({} file(s) encrypted).",
            target, zone, count
        );
        unlocked.finalize(&format!("crypt add {target} (zone {zone})"))
    }
}

/// Resolve a user-provided target into a list of concrete file
/// paths. A single file resolves to itself; a directory resolves to
/// every regular file under it (recursive). The path is interpreted
/// relative to the project root.
fn resolve_target_paths(root: &std::path::Path, target: &str) -> Result<Vec<std::path::PathBuf>> {
    let absolute = if std::path::Path::new(target).is_absolute() {
        std::path::PathBuf::from(target)
    } else {
        root.join(target)
    };
    if absolute.is_file() {
        return Ok(vec![absolute]);
    }
    if absolute.is_dir() {
        let mut out = Vec::new();
        walk_files(&absolute, &mut out)?;
        return Ok(out);
    }
    bail!(
        "'{}' is not a file or directory under {}",
        target,
        root.display()
    );
}

fn walk_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Read a file, encrypt it under the given zone key, write the blob
/// back atomically. No-op when the file is already a JOYCRYPT blob
/// (idempotent re-encryption protection).
fn encrypt_file_in_place(
    path: &std::path::Path,
    zone: &str,
    zone_key: &joy_core::crypt::ZoneKey,
) -> Result<()> {
    let bytes = std::fs::read(path)?;
    if joy_core::crypt::looks_like_blob(&bytes) {
        return Ok(());
    }
    let blob = joy_core::crypt::encrypt_blob(zone, zone_key, &bytes);
    write_atomic(path, &blob)
}

/// Decrypt a file in place. No-op when the file is already plaintext.
fn decrypt_file_in_place(path: &std::path::Path) -> Result<()> {
    let bytes = std::fs::read(path)?;
    if !joy_core::crypt::looks_like_blob(&bytes) {
        return Ok(());
    }
    let (_zone, plaintext) =
        joy_core::crypt::decrypt_blob(joy_core::crypt::active_zone_key, &bytes)?;
    write_atomic(path, &plaintext)
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("crypt"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Resolve a user-supplied file string to an absolute path under the
/// project root. Errors if the path escapes the project.
fn resolve_file_path(root: &std::path::Path, file: &str) -> Result<std::path::PathBuf> {
    let p = std::path::Path::new(file);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    Ok(abs)
}

/// Look up the zone a file belongs to via `crypt.zones[].paths`.
/// Matches: exact path equality OR the registered path is a prefix
/// (directory marker, ends with `/`).
fn zone_for_path(
    project: &Project,
    root: &std::path::Path,
    abs_path: &std::path::Path,
) -> Option<String> {
    let rel = abs_path
        .strip_prefix(root)
        .ok()?
        .to_string_lossy()
        .to_string();
    for (name, zone) in &project.crypt.zones {
        for p in &zone.paths {
            let trimmed = p.trim_end_matches('/');
            if rel == trimmed || rel.starts_with(&format!("{}/", trimmed)) {
                return Some(name.clone());
            }
        }
    }
    None
}

/// Common: load context, prompt, unlock the zone the file belongs to.
/// The zone is read from the blob header if the file is encrypted,
/// otherwise looked up via crypt.zones[].paths.
fn unlock_for_file(
    abs_path: &std::path::Path,
    passphrase_flag: Option<&str>,
    passphrase_stdin: bool,
) -> Result<(std::path::PathBuf, String, core_crypt::ZoneKey)> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let project_path = store::joy_dir(&root).join(store::PROJECT_FILE);
    let project = store::read_project(&project_path)?;
    let email = joy_core::vcs::default_vcs().user_email()?;
    let acting = project
        .member_by_email(&email)
        .ok_or_else(|| anyhow::anyhow!("{} is not a registered project member", email))?;

    // Determine the zone: either from the blob magic on disk or from
    // the project's zones[].paths registry.
    let zone_name = match std::fs::read(abs_path) {
        Ok(bytes) if joy_core::crypt::looks_like_blob(&bytes) => {
            // Peek the zone-name from the header.
            let zone_len = bytes.get(9).copied().unwrap_or(0) as usize;
            let end = 10 + zone_len;
            std::str::from_utf8(bytes.get(10..end).unwrap_or(&[]))
                .map_err(|_| anyhow::anyhow!("invalid zone name in blob header"))?
                .to_string()
        }
        _ => zone_for_path(&project, &root, abs_path).ok_or_else(|| {
            anyhow::anyhow!(
                "{} is not in any Crypt zone (and not encrypted). \
                 Run `joy crypt add <path>` first.",
                abs_path.display()
            )
        })?,
    };

    // Chat zones keep their key in the chat itself on refs/joy/chats, not
    // in project.yaml: follow the blob's own `chat:<cid>#<epoch>` header to
    // the key, the same way a zone file follows its header to project.yaml.
    if let Some((cid, epoch)) = zone_name
        .strip_prefix("chat:")
        .and_then(|r| r.rsplit_once('#'))
    {
        let passphrase = read_passphrase(passphrase_flag, passphrase_stdin, "Passphrase: ")?;
        let unlocked = joy_core::auth::unlock_identity(acting, &passphrase)?;
        let ck = joy_core::chat_store::epoch_content_key(&root, cid, epoch, &unlocked.seed)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no key for chat zone '{zone_name}': not a participant, or chat/epoch absent"
                )
            })?;
        return Ok((root, zone_name, core_crypt::ZoneKey::from_bytes(ck)));
    }

    let wrap_hex = acting.crypt_wraps.get(&zone_name).ok_or_else(|| {
        joy_core::error::JoyError::ZoneAccessDenied {
            zone: zone_name.clone(),
        }
    })?;

    let passphrase = read_passphrase(passphrase_flag, passphrase_stdin, "Passphrase: ")?;
    let unlocked = joy_core::auth::unlock_identity(acting, &passphrase)?;
    let zone_key = core_crypt::unwrap_for_member(wrap_hex, &zone_name, &unlocked.seed)?;
    Ok((root, zone_name, zone_key))
}

fn run_read(file: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let abs = resolve_file_path(&root, file)?;
    let bytes = std::fs::read(&abs)?;
    if !joy_core::crypt::looks_like_blob(&bytes) {
        // Already plaintext - just stream it.
        use std::io::Write;
        std::io::stdout().write_all(&bytes)?;
        return Ok(());
    }
    let (_root, _zone, zone_key) = unlock_for_file(&abs, passphrase, stdin)?;
    let mut keys = std::collections::BTreeMap::new();
    let zone_len = bytes[9] as usize;
    let zone_name = std::str::from_utf8(&bytes[10..10 + zone_len])?.to_string();
    keys.insert(zone_name, *zone_key.as_bytes());
    joy_core::crypt::set_active_zone_keys(keys);
    let (_zone, plaintext) =
        joy_core::crypt::decrypt_blob(joy_core::crypt::active_zone_key, &bytes)?;
    joy_core::crypt::clear_active_zone_keys();
    use std::io::Write;
    std::io::stdout().write_all(&plaintext)?;
    Ok(())
}

fn run_write(file: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let abs = resolve_file_path(&root, file)?;
    let (_root, zone, zone_key) = unlock_for_file(&abs, passphrase, stdin)?;

    use std::io::Read;
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let blob = joy_core::crypt::encrypt_blob(&zone, &zone_key, &input);
    write_atomic(&abs, &blob)?;
    if let Ok(rel) = abs.strip_prefix(&root) {
        joy_core::git_ops::auto_git_add(&root, &[&rel.to_string_lossy()]);
    }
    eprintln!(
        "Encrypted {} ({} bytes) into zone '{}'.",
        abs.display(),
        input.len(),
        zone
    );
    Ok(())
}

fn run_unlock(file: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let abs = resolve_file_path(&root, file)?;
    let bytes = std::fs::read(&abs)?;
    if !joy_core::crypt::looks_like_blob(&bytes) {
        bail!(
            "{} is already plaintext (or not encrypted by Crypt).",
            abs.display()
        );
    }
    let (_root, _zone, zone_key) = unlock_for_file(&abs, passphrase, stdin)?;
    let mut keys = std::collections::BTreeMap::new();
    let zone_len = bytes[9] as usize;
    let zone_name = std::str::from_utf8(&bytes[10..10 + zone_len])?.to_string();
    keys.insert(zone_name.clone(), *zone_key.as_bytes());
    joy_core::crypt::set_active_zone_keys(keys);
    let (_zone, plaintext) =
        joy_core::crypt::decrypt_blob(joy_core::crypt::active_zone_key, &bytes)?;
    joy_core::crypt::clear_active_zone_keys();
    write_atomic(&abs, &plaintext)?;
    println!(
        "Unlocked {}. Other processes on this machine can now read it. \
         Run `joy crypt lock {}` when done.",
        abs.display(),
        file
    );
    Ok(())
}

fn run_lock(file: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let abs = resolve_file_path(&root, file)?;
    let bytes = std::fs::read(&abs)?;
    if joy_core::crypt::looks_like_blob(&bytes) {
        println!("{} is already encrypted; nothing to do.", abs.display());
        return Ok(());
    }
    let (_root, zone, zone_key) = unlock_for_file(&abs, passphrase, stdin)?;
    let blob = joy_core::crypt::encrypt_blob(&zone, &zone_key, &bytes);
    write_atomic(&abs, &blob)?;
    if let Ok(rel) = abs.strip_prefix(&root) {
        joy_core::git_ops::auto_git_add(&root, &[&rel.to_string_lossy()]);
    }
    println!("Locked {} into zone '{}'.", abs.display(), zone);
    Ok(())
}

fn run_edit(file: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;
    let abs = resolve_file_path(&root, file)?;
    let (_root, zone, zone_key) = unlock_for_file(&abs, passphrase, stdin)?;

    // Decrypt to a temp file in $TMPDIR, open editor, re-encrypt.
    let plaintext = match std::fs::read(&abs) {
        Ok(b) if joy_core::crypt::looks_like_blob(&b) => {
            let mut keys = std::collections::BTreeMap::new();
            keys.insert(zone.clone(), *zone_key.as_bytes());
            joy_core::crypt::set_active_zone_keys(keys);
            let (_z, pt) = joy_core::crypt::decrypt_blob(joy_core::crypt::active_zone_key, &b)?;
            joy_core::crypt::clear_active_zone_keys();
            pt
        }
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };

    let tmpdir = std::env::temp_dir();
    let tmp = tmpdir.join(format!(
        "joy-crypt-edit-{}-{}",
        std::process::id(),
        abs.file_name().and_then(|s| s.to_str()).unwrap_or("file")
    ));
    std::fs::write(&tmp, &plaintext)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    // Run via `sh -c` so EDITOR can be a shell command with flags or
    // pipes ("code -w", "vim -O", ...). The temp path arrives as $1.
    let cmd_line = format!("{} \"$@\"", editor);
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd_line)
        .arg("sh")
        .arg(&tmp)
        .status()?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("editor exited with non-zero status");
    }

    let edited = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);

    let blob = joy_core::crypt::encrypt_blob(&zone, &zone_key, &edited);
    write_atomic(&abs, &blob)?;
    if let Ok(rel) = abs.strip_prefix(&root) {
        joy_core::git_ops::auto_git_add(&root, &[&rel.to_string_lossy()]);
    }
    println!(
        "Saved {} ({} bytes plaintext, encrypted under '{}').",
        abs.display(),
        edited.len(),
        zone
    );
    Ok(())
}

fn run_rm(zone: &str, target: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let mut unlocked = unlock_zone(zone, passphrase, stdin, false)?;

    if looks_like_item_id(target) {
        // Install the zone key so read_yaml decrypts the existing
        // ciphertext blob before parsing.
        unlocked.install_zone_key();
        let item_path = joy_core::items::find_item_file(&unlocked.root, target)?;
        let mut item: joy_core::model::item::Item = store::read_yaml(&item_path)?;
        match item.crypt_zone.as_deref() {
            Some(z) if z == zone => {
                item.crypt_zone = None;
                // Item is now plain (crypt_zone=None) so save_item
                // writes plaintext YAML.
                joy_core::items::update_item(&unlocked.root, &item)?;
                joy_core::crypt::clear_active_zone_keys();
                if let Ok(rel) = item_path.strip_prefix(&unlocked.root) {
                    joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
                }
                println!("Removed {} from zone '{}'.", target, zone);
            }
            Some(z) => {
                joy_core::crypt::clear_active_zone_keys();
                bail!(
                    "{} is in zone '{}', not '{}'. Use `--zone {}` to remove it.",
                    target,
                    z,
                    zone,
                    z
                );
            }
            None => {
                joy_core::crypt::clear_active_zone_keys();
                println!("{} is not in any Crypt zone.", target);
                return Ok(());
            }
        }
        unlocked.finalize(&format!("crypt rm {target} (zone {zone})"))
    } else {
        let resolved = resolve_target_paths(&unlocked.root, target)?;
        let removed_from_registry;
        {
            let zone_entry = unlocked
                .project
                .crypt
                .zones
                .get_mut(zone)
                .ok_or_else(|| anyhow::anyhow!("zone '{}' does not exist", zone))?;
            let before = zone_entry.paths.len();
            zone_entry.paths.retain(|p| p != target);
            removed_from_registry = zone_entry.paths.len() != before;
        }
        unlocked.install_zone_key();
        for file in &resolved {
            decrypt_file_in_place(file)?;
            if let Ok(rel) = file.strip_prefix(&unlocked.root) {
                joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
            }
        }
        joy_core::crypt::clear_active_zone_keys();
        // Persist the registry change.
        let project_path = store::joy_dir(&unlocked.root).join(store::PROJECT_FILE);
        store::write_yaml_preserve(&project_path, &unlocked.project)?;
        if removed_from_registry {
            println!("Removed path '{}' from zone '{}'.", target, zone);
        }
        println!("Decrypted {} file(s).", resolved.len());
        unlocked.finalize(&format!("crypt rm {target} (zone {zone})"))
    }
}

fn run_add_all(zone: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let mut unlocked = unlock_zone(zone, passphrase, stdin, true)?;
    // Persist wrap before encrypting any items - if encryption fails
    // mid-way, items still on plaintext are recoverable; the wrap
    // ensures the zone key survives.
    unlocked.persist_wrap()?;
    unlocked.install_zone_key();
    let items = joy_core::items::load_items(&unlocked.root).unwrap_or_default();
    let mut updated = 0usize;
    let mut already = 0usize;
    for item in &items {
        if item.crypt_zone.as_deref() == Some(zone) {
            already += 1;
            continue;
        }
        let mut item = item.clone();
        item.crypt_zone = Some(zone.to_string());
        joy_core::items::update_item(&unlocked.root, &item)?;
        let item_path = joy_core::items::find_item_file(&unlocked.root, &item.id)?;
        if let Ok(rel) = item_path.strip_prefix(&unlocked.root) {
            joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
        }
        updated += 1;
    }
    joy_core::crypt::clear_active_zone_keys();
    println!(
        "{} item(s) encrypted under zone '{}'; {} already in zone.",
        updated, zone, already
    );
    unlocked.finalize(&format!("crypt add --all (zone {zone})"))
}

fn run_rm_all(zone: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    let unlocked = unlock_zone(zone, passphrase, stdin, false)?;
    unlocked.install_zone_key();
    let items = joy_core::items::load_items(&unlocked.root).unwrap_or_default();
    let mut updated = 0usize;
    for item in &items {
        if item.crypt_zone.as_deref() != Some(zone) {
            continue;
        }
        let mut item = item.clone();
        item.crypt_zone = None;
        // crypt_zone is None now, so save_item writes plaintext YAML.
        joy_core::items::update_item(&unlocked.root, &item)?;
        let item_path = joy_core::items::find_item_file(&unlocked.root, &item.id)?;
        if let Ok(rel) = item_path.strip_prefix(&unlocked.root) {
            joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel.to_string_lossy()]);
        }
        updated += 1;
    }
    joy_core::crypt::clear_active_zone_keys();
    let project_path = store::joy_dir(&unlocked.root).join(store::PROJECT_FILE);
    store::write_yaml_preserve(&project_path, &unlocked.project)?;
    println!("Decrypted {} item(s) in zone '{}'.", updated, zone);
    unlocked.finalize(&format!("crypt rm --all (zone {zone})"))
}

fn run_zone_list() -> Result<()> {
    let (root, project, _email) = load_context()?;
    println!("{}", color::header("Crypt zones"));
    println!();
    if project.crypt.is_empty() {
        println!("{}", color::footer("No zones configured."));
        return Ok(());
    }
    let metas = joy_core::items::list_item_metadata(&root).unwrap_or_default();
    println!(
        "   {:<18} {:>8} {:>8} {:>8}",
        color::label("ZONE"),
        color::label("PATHS"),
        color::label("ITEMS"),
        color::label("MEMBERS"),
    );
    for (name, zone) in &project.crypt.zones {
        let item_count = metas
            .iter()
            .filter(|m| m.zone() == Some(name.as_str()))
            .count();
        let member_count = project
            .member_values()
            .filter(|m| m.crypt_wraps.contains_key(name))
            .count();
        println!(
            "   {:<18} {:>8} {:>8} {:>8}",
            name,
            zone.paths.len(),
            item_count,
            member_count
        );
    }
    println!();
    println!(
        "{}",
        color::footer(&format!(
            "{} zone(s) registered.",
            project.crypt.zones.len()
        ))
    );
    Ok(())
}

fn run_zone_rm(name: &str) -> Result<()> {
    let (root, mut project, email) = load_context()?;
    if !project.crypt.zones.contains_key(name) {
        bail!("zone '{}' is not registered", name);
    }
    let metas = joy_core::items::list_item_metadata(&root).unwrap_or_default();
    let item_count = metas.iter().filter(|m| m.zone() == Some(name)).count();
    let member_count = project
        .member_values()
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

fn run_grant(zone: &str, target_member: &str, passphrase: Option<&str>, stdin: bool) -> Result<()> {
    use joy_core::model::project::is_ai_member;
    let unlocked = unlock_zone(zone, passphrase, stdin, false)?;
    // `target_member` is a user-supplied identifier that is polymorphic by
    // kind: an `ai:` synthetic id for AI tools (always the at-rest map key),
    // or a git e-mail for humans (privacy-dependent map key). Resolve each via
    // the matching accessor so anonymous-mode lookups stay correct (ADR-042).
    let target = if is_ai_member(target_member) {
        unlocked.project.member_by_key(target_member)
    } else {
        unlocked.project.member_by_email(target_member)
    }
    .ok_or_else(|| anyhow::anyhow!("member '{}' not found", target_member))?;

    let granter_verify_hex = unlocked
        .project
        .member_by_email(&unlocked.acting_email)
        .and_then(|m| m.verify_key.clone())
        .ok_or_else(|| anyhow::anyhow!("granter has no verify_key registered"))?;
    let granter_verify_key = joy_core::auth::PublicKey::from_hex(&granter_verify_hex)?;

    // The PLATFORM grant (container concept: "Zonen-Grant an die
    // Plattform", user-decided per zone): the zone key wrapped against
    // the registered platform verify_key, stored zone-major. With it the
    // platform serves the zone to joy-unlocked app sessions.
    if target_member.eq_ignore_ascii_case("platform") {
        let platform_verify_hex = unlocked
            .project
            .platform
            .as_ref()
            .map(|p| p.verify_key.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no platform is registered in this project (joy project platform-key);                      nothing to grant to"
                )
            })?;
        let platform_verify_key = joy_core::auth::PublicKey::from_hex(&platform_verify_hex)?;
        let granter_verify_hex = unlocked
            .project
            .member_by_email(&unlocked.acting_email)
            .and_then(|m| m.verify_key.clone())
            .ok_or_else(|| anyhow::anyhow!("granter has no verify_key registered"))?;
        let granter_verify_key = joy_core::auth::PublicKey::from_hex(&granter_verify_hex)?;
        let wrap_hex = joy_core::crypt::wrap_for_member(
            &unlocked.zone_key,
            &unlocked.zone,
            &unlocked.acting_seed,
            &granter_verify_key,
            &platform_verify_key,
        );
        let project_path = store::joy_dir(&unlocked.root).join(store::PROJECT_FILE);
        let mut project = store::read_project(&project_path)?;
        project
            .crypt
            .zones
            .entry(unlocked.zone.clone())
            .or_default()
            .platform_wrap = Some(wrap_hex);
        // the granter keeps their own wrap (first add+grant session)
        let granter_wrap = joy_core::crypt::wrap_for_self(
            &unlocked.zone_key,
            &unlocked.zone,
            &unlocked.acting_seed,
        );
        let g = project.member_by_email_mut(&unlocked.acting_email).unwrap();
        g.crypt_wraps
            .entry(unlocked.zone.clone())
            .or_insert(granter_wrap);
        store::write_yaml_preserve(&project_path, &project)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel]);
        joy_core::git_ops::auto_git_post_command(
            &unlocked.root,
            &format!("crypt grant platform (zone {})", unlocked.zone),
            &unlocked.acting_email,
        );
        println!(
            "Granted the platform access to zone '{}'. It serves the zone's content to              authenticated app sessions; revoke by removing crypt.zones.{}.platform_wrap.",
            unlocked.zone, unlocked.zone
        );
        return Ok(());
    }

    // ADR-041 §4: AI Tool grants target one wrap per (operator, AI),
    // stored zone-major under crypt.zones.<zone>.delegations.<ai>.<op>.
    // Human grants stay member-major under members.<who>.crypt_wraps as
    // before; the wrap layouts coexist in the same `project.yaml`.
    let target_is_ai = is_ai_member(target_member);

    let project_path = store::joy_dir(&unlocked.root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;
    project
        .crypt
        .zones
        .entry(unlocked.zone.clone())
        .or_default();

    if target_is_ai {
        // Walk every operator who has a delegation entry for this AI and
        // write a wrap targeting their delegation_verifier. Per ADR-041
        // §1, the delegation pubkey is stable per (operator, AI); the
        // wrap therefore covers every token that operator has issued and
        // every token they will issue until they rotate.
        let ai_id = target_member;
        let _ = target; // target lookup was for existence check; not used further on AI path
        let mut wraps: Vec<(String, String)> = Vec::new();
        for (operator_email, member) in project.members() {
            let Some(entry) = member.ai_delegations.get(ai_id) else {
                continue;
            };
            let delegation_pk = joy_core::auth::PublicKey::from_hex(&entry.delegation_verifier)?;
            let wrap_hex = joy_core::crypt::wrap_for_member(
                &unlocked.zone_key,
                &unlocked.zone,
                &unlocked.acting_seed,
                &granter_verify_key,
                &delegation_pk,
            );
            wraps.push((operator_email.clone(), wrap_hex));
        }
        if wraps.is_empty() {
            anyhow::bail!(
                "No operator has a delegation registered for {ai_id}. Each operator who wants to \
                 use {ai_id} must first run `joy auth token add {ai_id}` once to register their \
                 per-(operator, AI) delegation."
            );
        }
        let zone_entry = project
            .crypt
            .zones
            .get_mut(&unlocked.zone)
            .expect("zone entry just inserted");
        let delegations_for_ai = zone_entry.delegations.entry(ai_id.to_string()).or_default();
        let count = wraps.len();
        for (op, wrap) in wraps {
            delegations_for_ai.insert(op, wrap);
        }

        // Ensure the granter's own (human) wrap is also present so they
        // do not lose access by being the first in the zone.
        let granter_wrap = joy_core::crypt::wrap_for_self(
            &unlocked.zone_key,
            &unlocked.zone,
            &unlocked.acting_seed,
        );
        let g = project.member_by_email_mut(&unlocked.acting_email).unwrap();
        g.crypt_wraps
            .entry(unlocked.zone.clone())
            .or_insert(granter_wrap);

        store::write_yaml_preserve(&project_path, &project)?;
        let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
        joy_core::git_ops::auto_git_add(&unlocked.root, &[&rel]);
        joy_core::git_ops::auto_git_post_command(
            &unlocked.root,
            &format!(
                "crypt grant {ai_id} (zone {}, {count} delegations)",
                unlocked.zone
            ),
            &unlocked.acting_email,
        );
        println!(
            "Granted {ai_id} access to zone '{}' for {count} operator delegation(s).",
            unlocked.zone
        );
        return Ok(());
    }

    // Human grant: target gets a wrap on members.<email>.crypt_wraps.<zone>.
    let target_verify_hex = target.verify_key.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no verify_key yet. They must run `joy auth` (or `joy auth --otp`) before \
             they can receive a Crypt grant.",
            target_member
        )
    })?;
    let target_verify_key = joy_core::auth::PublicKey::from_hex(target_verify_hex)?;

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

    let m = project
        .member_by_email_mut(target_member)
        .expect("target member existed at unwrap time");
    m.crypt_wraps.insert(unlocked.zone.clone(), wrap_hex);

    // Ensure the granter's own wrap is also present (auto-create path
    // when this is the first add+grant in the same session).
    let granter_wrap =
        joy_core::crypt::wrap_for_self(&unlocked.zone_key, &unlocked.zone, &unlocked.acting_seed);
    let g = project.member_by_email_mut(&unlocked.acting_email).unwrap();
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
        color::user(target_member),
        unlocked.zone
    );
    Ok(())
}

fn run_revoke(zone: &str, target_member: &str) -> Result<()> {
    use joy_core::model::project::is_ai_member;
    let (root, mut project, email) = load_context()?;
    // `target_member` is polymorphic (see run_grant): resolve AI ids by at-rest
    // key, human identifiers by privacy-aware e-mail lookup (ADR-042).
    let target_exists = if is_ai_member(target_member) {
        project.has_member_key(target_member)
    } else {
        project.member_by_email(target_member).is_some()
    };
    if !target_exists {
        bail!("member '{}' not found", target_member);
    }

    // ADR-041 §5: AI Tool revoke removes the entire delegations.<ai> map
    // under the zone (all operator-keyed wraps for that AI). Human
    // revoke continues to remove from members.<who>.crypt_wraps.
    let removed = if is_ai_member(target_member) {
        let zone_entry = project.crypt.zones.get_mut(zone);
        match zone_entry {
            Some(z) => z.delegations.remove(target_member).is_some(),
            None => false,
        }
    } else {
        project
            .member_by_email_mut(target_member)
            .map(|m| m.crypt_wraps.remove(zone).is_some())
            .unwrap_or(false)
    };
    if !removed {
        println!(
            "{} had no access to zone '{}'; nothing to revoke.",
            color::user(target_member),
            zone
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
    println!(
        "Revoked {}'s access to zone '{}'.",
        color::user(target_member),
        zone
    );
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
    println!("{}", color::header(&format!("Crypt zone: {zone}")));
    println!();
    if cfg.is_empty() && project.member_values().all(|m| m.crypt_wraps.is_empty()) {
        println!("{}", color::footer("No Crypt zones configured."));
        return Ok(());
    }

    println!("{}", color::section("Paths"));
    if let Some(z) = cfg.zones.get(zone) {
        if z.paths.is_empty() {
            println!("  {}", color::inactive("(none)"));
        } else {
            for p in &z.paths {
                println!("  {p}");
            }
        }
    } else {
        println!("  {}", color::inactive("(zone not registered)"));
    }

    println!();
    println!("{}", color::section("Items"));
    let metas = joy_core::items::list_item_metadata(&root).unwrap_or_default();
    let mut found_any = false;
    for meta in &metas {
        if meta.zone() == Some(zone) {
            if meta.encrypted_zone.is_some() {
                println!("  {} {}", meta.id, color::inactive("(encrypted)"));
            } else {
                println!("  {}", meta.id);
            }
            found_any = true;
        }
    }
    if !found_any {
        println!("  {}", color::inactive("(none)"));
    }

    println!();
    println!("{}", color::section("Members with access"));
    let mut access_count = 0;
    for (email, member) in project.members() {
        if member.crypt_wraps.contains_key(zone) {
            println!("  {}", color::user(email));
            access_count += 1;
        }
    }
    if access_count == 0 {
        println!("  {}", color::inactive("(none)"));
    }

    println!();
    println!(
        "{}",
        color::footer(&format!(
            "Zone '{zone}': {access_count} member(s) with access."
        ))
    );
    Ok(())
}

fn run_status() -> Result<()> {
    let (root, project, email) = load_context()?;
    let cfg = &project.crypt;
    let zone_count = cfg.zones.len();
    // Metadata walk: count items in any zone without prompting.
    let metas = joy_core::items::list_item_metadata(&root).unwrap_or_default();
    let item_count_total = metas.iter().filter(|m| m.zone().is_some()).count();
    let me_access = project
        .member_by_email(&email)
        .map(|m| m.crypt_wraps.len())
        .unwrap_or(0);

    println!("{}", color::header("Crypt status"));
    println!();
    println!("  zones registered:  {}", zone_count);
    println!("  items in any zone: {}", item_count_total);
    println!("  your access:       {} zone(s)", me_access);
    println!();
    let footer = if zone_count == 0 && item_count_total == 0 {
        "No encryption configured. Use `joy crypt add <id|path>` to start.".to_string()
    } else if me_access == 0 {
        format!(
            "{} zone(s) registered, you have no access. Ask a current key-holder for `joy crypt grant`.",
            zone_count
        )
    } else {
        format!(
            "{} zone(s) registered, {} accessible to you.",
            zone_count, me_access
        )
    };
    println!("{}", color::footer(&footer));
    Ok(())
}

fn looks_like_item_id(s: &str) -> bool {
    !s.contains('/') && !s.contains('.') && s.contains('-')
}
