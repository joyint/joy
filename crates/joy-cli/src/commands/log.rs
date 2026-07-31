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
  joy log JOY-0001            Events of one item (short or full ID)
  joy log --limit 50          Show last 50 events
  joy log --since 7d          Show events from last 7 days")]
pub struct LogArgs {
    /// Item ID to filter by (short or full form), like joy show
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
    let mut entries =
        event_log::read_events(&root, since.as_deref(), args.item.as_deref(), fetch_limit)?;

    let has_more = !args.all && entries.len() > effective_limit;
    if has_more {
        entries.truncate(effective_limit);
    }

    // Resolve any member-id details (e.g. the assignee recorded on item.assigned)
    // for display, in both the terminal and --json. Only the opaque-id shape is
    // touched, so comment text, statuses and other details stay verbatim. The
    // event actor (entry.user) resolves on its own via MemberRef.
    for entry in &mut entries {
        if let Some(d) = &entry.details {
            if joy_core::member_id::is_opaque_member_id(d) {
                entry.details = Some(joy_core::member_ref::resolve_str(d));
            }
        }
    }

    if crate::output::is_json() {
        return crate::output::emit(LogPayload {
            total: entries.len(),
            has_more,
            events: entries,
        });
    }

    if entries.is_empty() {
        println!("No events found.");
        return Ok(());
    }

    let display_entries = &entries;

    for entry in display_entries {
        let local_time = format_local_time(&entry.timestamp);
        let details_str = entry
            .details
            .as_ref()
            .map(|d| format!(" - \"{d}\""))
            .unwrap_or_default();

        println!(
            "{} - {} - {}{} [{}]",
            color::label(&local_time),
            color::id(&entry.target),
            color::label(&entry.event_type),
            details_str,
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

#[derive(serde::Serialize)]
struct LogPayload {
    total: usize,
    has_more: bool,
    events: Vec<joy_core::event_log::LogEntry>,
}
