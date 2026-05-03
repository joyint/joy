// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy crypt filter` - Git clean/smudge/textconv binary (JOY-014B-09).
//! Hidden subcommand under `joy crypt`; only Git invokes it via the
//! filter / diff drivers configured in `.git/config`.
//!
//! Working dir holds Crypt blobs (binary, magic-headed) for items
//! marked via `joy crypt add`. Git history holds plaintext. The filter
//! is configured in `.git/config` (filter.joy-crypt.{clean,smudge},
//! diff.joy-crypt.textconv) and runs once per file invocation.
//!
//! - clean (working -> git): reads ciphertext from stdin, decrypts via
//!   the active session's zone-key sidecar, writes plaintext to
//!   stdout. If the input is not a Crypt blob, passes it through
//!   unchanged so that newly added files do not lose data.
//! - smudge (git -> working): reads plaintext from stdin, looks at
//!   the YAML's `crypt_zone:` field (item files are the only
//!   supported case in this release; path-based files pass through),
//!   encrypts with that zone's key, writes the blob to stdout.
//! - textconv: reads the file at the given path, decrypts if it is a
//!   Crypt blob, writes plaintext to stdout. Used by `git diff` /
//!   `git blame` / forge web views.
//!
//! Failures are written to stderr and the binary exits non-zero so
//! Git aborts the operation rather than silently corrupting data.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};

use joy_core::auth::session;
use joy_core::crypt::{self as core_crypt, ZoneKey};
use joy_core::store;
use joy_core::vcs::Vcs;

use std::io::{Read, Write};

#[derive(Args)]
pub struct FilterArgs {
    #[command(subcommand)]
    command: FilterCommand,
}

#[derive(Subcommand)]
enum FilterCommand {
    /// Decrypt working-directory ciphertext on `git add` so Git
    /// stores plaintext history.
    Clean {
        /// Path of the file being filtered (provided by Git via %f).
        /// Optional: clean reads everything from stdin so the path is
        /// only used for diagnostics.
        #[arg(value_name = "PATH")]
        path: Option<String>,
    },
    /// Encrypt plaintext from Git on checkout so the working
    /// directory holds ciphertext.
    Smudge {
        /// Path of the file being smudged (Git's %f). Used to
        /// resolve the zone for non-YAML files in a follow-up; the
        /// item-file path uses the YAML's own `crypt_zone:` field.
        #[arg(value_name = "PATH")]
        path: Option<String>,
    },
    /// Decrypt a file for `git diff` / `git blame`. Reads the file
    /// from disk rather than stdin per Git's textconv contract.
    Textconv {
        /// Path of the file to decrypt.
        #[arg(value_name = "PATH")]
        path: String,
    },
}

pub fn run(args: FilterArgs) -> Result<()> {
    match args.command {
        FilterCommand::Clean { path } => run_clean(path.as_deref()),
        FilterCommand::Smudge { path } => run_smudge(path.as_deref()),
        FilterCommand::Textconv { path } => run_textconv(&path),
    }
}

fn run_clean(_path: Option<&str>) -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;

    if !core_crypt::looks_like_blob(&input) {
        // Already plaintext (e.g. file added before encryption was
        // configured). Pass through; subsequent edits will encrypt.
        std::io::stdout().write_all(&input)?;
        return Ok(());
    }

    let zone_keys = active_zone_keys()?;
    let (_zone, plaintext) = core_crypt::decrypt_blob(
        |name| zone_keys.get(name).map(|k| ZoneKey::from_bytes(*k)),
        &input,
    )?;
    std::io::stdout().write_all(&plaintext)?;
    Ok(())
}

fn run_smudge(_path: Option<&str>) -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;

    // YAML item files announce their zone via `crypt_zone: <name>`.
    // Non-YAML paths pass through plaintext in this release; the
    // path-glob smudge lands in a follow-up.
    let Some(zone) = read_crypt_zone_from_yaml(&input) else {
        std::io::stdout().write_all(&input)?;
        return Ok(());
    };

    let zone_keys = active_zone_keys()?;
    let raw = zone_keys
        .get(&zone)
        .ok_or_else(|| anyhow::anyhow!(
            "no active zone key for '{}'; run `joy auth` to populate the session",
            zone
        ))?;
    let zk = ZoneKey::from_bytes(*raw);
    let blob = core_crypt::encrypt_blob(&zone, &zk, &input);
    std::io::stdout().write_all(&blob)?;
    Ok(())
}

fn run_textconv(path: &str) -> Result<()> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("failed to read {path}: {e}"))?;
    if !core_crypt::looks_like_blob(&bytes) {
        std::io::stdout().write_all(&bytes)?;
        return Ok(());
    }
    let zone_keys = active_zone_keys()?;
    let (_zone, plaintext) = core_crypt::decrypt_blob(
        |name| zone_keys.get(name).map(|k| ZoneKey::from_bytes(*k)),
        &bytes,
    )?;
    std::io::stdout().write_all(&plaintext)?;
    Ok(())
}

/// Load the active session's zone-key sidecar. Map values are 32-byte
/// AES-256-GCM keys (hex on disk, raw bytes here).
fn active_zone_keys() -> Result<std::collections::BTreeMap<String, [u8; 32]>> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or_else(|| {
        anyhow::anyhow!("not inside a Joy project (run `joy init` first)")
    })?;
    let project_id = session::project_id(&root)?;
    let email = joy_core::vcs::default_vcs().user_email().unwrap_or_default();
    let sidecar = session::load_zone_keys(&project_id, &email)?;
    let mut out = std::collections::BTreeMap::new();
    for (zone, hex_str) in sidecar {
        let bytes = hex::decode(&hex_str)
            .map_err(|e| anyhow::anyhow!("zone-key sidecar corrupt for '{zone}': {e}"))?;
        if bytes.len() != 32 {
            bail!("zone-key sidecar for '{zone}' has wrong length: {}", bytes.len());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        out.insert(zone, arr);
    }
    Ok(out)
}

/// Cheap regex-free YAML probe for `crypt_zone: <name>`. Avoids
/// dragging serde_yaml into the filter binary's hot path. Returns the
/// zone name (without quotes) or None.
fn read_crypt_zone_from_yaml(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("crypt_zone:") {
            let value = rest.trim();
            // Strip surrounding quotes if present, ignore null/empty.
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            if value.is_empty() || value == "null" || value == "~" {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_crypt_zone_finds_default() {
        let yaml = b"id: JOY-0123\ncrypt_zone: default\ntitle: x\n";
        assert_eq!(read_crypt_zone_from_yaml(yaml), Some("default".into()));
    }

    #[test]
    fn read_crypt_zone_handles_quoted_name() {
        let yaml = b"id: JOY-0123\ncrypt_zone: \"customer-x\"\n";
        assert_eq!(read_crypt_zone_from_yaml(yaml), Some("customer-x".into()));
    }

    #[test]
    fn read_crypt_zone_returns_none_when_absent() {
        assert_eq!(read_crypt_zone_from_yaml(b"id: x\n"), None);
        assert_eq!(read_crypt_zone_from_yaml(b"crypt_zone: null\n"), None);
        assert_eq!(read_crypt_zone_from_yaml(b"crypt_zone: ~\n"), None);
    }
}
