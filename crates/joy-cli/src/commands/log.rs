// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use clap::Args;

use joy_core::store;

use crate::color;

#[derive(Args)]
pub struct LogArgs {
    /// Filter by item ID (e.g. IT-0001)
    #[arg(long)]
    item: Option<String>,

    /// Show changes since duration (e.g. 7d, 2w, 30d)
    #[arg(long)]
    since: Option<String>,

    /// Maximum number of entries to show
    #[arg(long, default_value = "20")]
    limit: usize,
}

/// A parsed commit from git log output.
struct LogEntry {
    hash: String,
    email: String,
    date: String,
    subject: String,
    item_ids: Vec<String>,
}

/// Parse a duration shorthand like "7d", "2w", "30d" into a git --since value.
fn parse_since(s: &str) -> Result<String> {
    let s = s.trim();
    if let Some(days) = s.strip_suffix('d') {
        let n: u64 = days
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid duration: {s}"))?;
        Ok(format!("{n} days ago"))
    } else if let Some(weeks) = s.strip_suffix('w') {
        let n: u64 = weeks
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid duration: {s}"))?;
        Ok(format!("{n} weeks ago"))
    } else {
        anyhow::bail!("invalid duration format: {s} (use e.g. 7d, 2w)")
    }
}

/// Extract item ID from a .joy/items/ filename like ".joy/items/IT-000A-some-title.yaml".
fn extract_item_id(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next()?;
    // Item filenames start with the ID pattern: XX-XXXX
    let parts: Vec<&str> = filename.splitn(3, '-').collect();
    if parts.len() >= 2 && parts[1].len() == 4 {
        Some(format!("{}-{}", parts[0], parts[1]))
    } else {
        None
    }
}

pub fn run(args: LogArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    // Build git log command
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(&root);
    cmd.args(["log", "--pretty=format:%H|%ae|%aI|%s", "--name-only"]);

    if let Some(ref since) = args.since {
        let since_val = parse_since(since)?;
        cmd.arg(format!("--since={since_val}"));
    }

    cmd.arg(format!("-{}", args.limit));

    // Path filter
    cmd.arg("--");
    if let Some(ref item_id) = args.item {
        // Find the specific item file via glob
        let items_dir = root.join(".joy").join("items");
        let mut found = false;
        if items_dir.is_dir() {
            for entry in std::fs::read_dir(&items_dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with(&format!("{}-", item_id)) && name_str.ends_with(".yaml") {
                    cmd.arg(format!(".joy/items/{name_str}"));
                    found = true;
                    break;
                }
            }
        }
        if !found {
            anyhow::bail!("no item file found for {item_id}");
        }
    } else {
        cmd.arg(".joy/items/");
    }

    let output = cmd
        .output()
        .map_err(|_| anyhow::anyhow!("failed to run git log"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git log failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries = parse_git_log(&stdout);

    if entries.is_empty() {
        println!("No changes found.");
        return Ok(());
    }

    for entry in &entries {
        // Date: take just YYYY-MM-DD from ISO 8601
        let date = if entry.date.len() >= 10 {
            &entry.date[..10]
        } else {
            &entry.date
        };

        let short_hash = if entry.hash.len() >= 7 {
            &entry.hash[..7]
        } else {
            &entry.hash
        };

        let ids_str = entry
            .item_ids
            .iter()
            .map(|id| color::id(id))
            .collect::<Vec<_>>()
            .join(", ");

        println!(
            "{}  {}  {}  {}",
            color::label(date),
            color::id(short_hash),
            color::label(&entry.email),
            entry.subject
        );
        if !entry.item_ids.is_empty() {
            println!("  {ids_str}");
        }
    }

    Ok(())
}

/// Parse git log output (format: %H|%ae|%aI|%s followed by name-only lines).
fn parse_git_log(output: &str) -> Vec<LogEntry> {
    let mut entries = Vec::new();
    let mut current: Option<LogEntry> = None;

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }

        // Try to parse as a commit line (contains | separators)
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() == 4
            && parts[0].len() == 40
            && parts[0].chars().all(|c| c.is_ascii_hexdigit())
        {
            // Save previous entry
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(LogEntry {
                hash: parts[0].to_string(),
                email: parts[1].to_string(),
                date: parts[2].to_string(),
                subject: parts[3].to_string(),
                item_ids: Vec::new(),
            });
        } else if let Some(ref mut entry) = current {
            // This is a filename line
            if let Some(id) = extract_item_id(line) {
                if !entry.item_ids.contains(&id) {
                    entry.item_ids.push(id);
                }
            }
        }
    }

    if let Some(entry) = current {
        entries.push(entry);
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_days() {
        assert_eq!(parse_since("7d").unwrap(), "7 days ago");
        assert_eq!(parse_since("30d").unwrap(), "30 days ago");
    }

    #[test]
    fn parse_since_weeks() {
        assert_eq!(parse_since("2w").unwrap(), "2 weeks ago");
    }

    #[test]
    fn parse_since_invalid() {
        assert!(parse_since("abc").is_err());
        assert!(parse_since("7x").is_err());
    }

    #[test]
    fn extract_id_from_filename() {
        assert_eq!(
            extract_item_id(".joy/items/IT-000A-some-title.yaml"),
            Some("IT-000A".to_string())
        );
        assert_eq!(
            extract_item_id(".joy/items/EP-0001-epic-name.yaml"),
            Some("EP-0001".to_string())
        );
    }

    #[test]
    fn parse_log_output() {
        let output = "abcdef1234567890abcdef1234567890abcdef00|horst@joydev.com|2026-03-10T12:00:00+01:00|feat: add item types\n.joy/items/IT-0001-do-stuff.yaml\n.joy/items/IT-0002-other.yaml\n";
        let entries = parse_git_log(output);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].item_ids, vec!["IT-0001", "IT-0002"]);
        assert_eq!(entries[0].email, "horst@joydev.com");
    }
}
