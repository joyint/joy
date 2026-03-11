// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use clap::Args;

use joy_core::event_log;
use joy_core::store;

use crate::color;

#[derive(Args)]
#[command(after_help = "\
Shows the event log from .joy/log/ (one file per day, append-only).
Events are recorded automatically by all joy commands.
Timestamps are displayed in your local timezone.

Examples:
  joy log                     Show last 20 events
  joy log --limit 50          Show last 50 events
  joy log --item JOY-0001     Filter by item ID
  joy log --since 7d          Show events from last 7 days")]
pub struct LogArgs {
    /// Filter by item ID (e.g. JOY-0001)
    #[arg(long)]
    item: Option<String>,

    /// Show changes since duration (e.g. 7d, 2w, 30d)
    #[arg(long)]
    since: Option<String>,

    /// Maximum number of entries to show
    #[arg(long, default_value = "20")]
    limit: usize,

    /// Show all entries (no limit)
    #[arg(short, long)]
    all: bool,
}

/// Parse a duration shorthand like "7d", "2w" into a YYYY-MM-DD date string.
fn parse_since(s: &str) -> Result<String> {
    let s = s.trim();
    let days = if let Some(d) = s.strip_suffix('d') {
        d.parse::<i64>()
            .map_err(|_| anyhow::anyhow!("invalid duration: {s}"))?
    } else if let Some(w) = s.strip_suffix('w') {
        w.parse::<i64>()
            .map_err(|_| anyhow::anyhow!("invalid duration: {s}"))?
            * 7
    } else {
        anyhow::bail!("invalid duration format: {s} (use e.g. 7d, 2w)")
    };

    let since_date = Utc::now() - chrono::Duration::days(days);
    Ok(since_date.format("%Y-%m-%d").to_string())
}

/// Convert a UTC ISO 8601 timestamp to local timezone display format.
fn format_local_time(utc_str: &str) -> String {
    if let Ok(utc_dt) = utc_str.parse::<DateTime<Utc>>() {
        let local_dt: DateTime<Local> = utc_dt.into();
        local_dt.format("%Y-%m-%d %H:%M:%S%.3f (%Z)").to_string()
    } else {
        utc_str.to_string()
    }
}

pub fn run(args: LogArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd).ok_or(joy_core::error::JoyError::NotInitialized)?;

    let since = args.since.as_deref().map(parse_since).transpose()?;

    let effective_limit = if args.all { usize::MAX } else { args.limit };
    // Request one extra to detect if there are more entries
    let fetch_limit = effective_limit.saturating_add(1);
    let entries =
        event_log::read_events(&root, since.as_deref(), args.item.as_deref(), fetch_limit)?;

    if entries.is_empty() {
        println!("No events found.");
        return Ok(());
    }

    let has_more = !args.all && entries.len() > effective_limit;
    let display_entries = if has_more {
        &entries[..effective_limit]
    } else {
        &entries
    };

    for entry in display_entries {
        let local_time = format_local_time(&entry.timestamp);
        let details_str = entry
            .details
            .as_ref()
            .map(|d| format!(" \"{d}\""))
            .unwrap_or_default();

        println!(
            "{} - {} - {} - {} [{}]",
            color::label(&local_time),
            color::id(&entry.target),
            color::label(&entry.event_type),
            details_str.trim_start(),
            color::user(&entry.user),
        );
    }

    if has_more {
        println!(
            "{}",
            color::label("(more entries available, use --all or --limit)")
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_days() {
        let result = parse_since("7d").unwrap();
        assert_eq!(result.len(), 10); // YYYY-MM-DD
    }

    #[test]
    fn parse_since_weeks() {
        let result = parse_since("2w").unwrap();
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn parse_since_invalid() {
        assert!(parse_since("abc").is_err());
        assert!(parse_since("7x").is_err());
    }

    #[test]
    fn format_local_time_valid() {
        let result = format_local_time("2026-03-11T16:14:32.320Z");
        assert!(result.contains("2026-03-11"));
        assert!(result.contains("32.320"));
    }

    #[test]
    fn format_local_time_invalid() {
        let result = format_local_time("not-a-date");
        assert_eq!(result, "not-a-date");
    }
}
