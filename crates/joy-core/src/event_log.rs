// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use chrono::Utc;

use crate::error::JoyError;
use crate::store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    ItemCreated,
    ItemUpdated,
    ItemStatusChanged,
    ItemDeleted,
    ItemAssigned,
    ItemUnassigned,
    DepAdded,
    DepRemoved,
    CommentAdded,
    MilestoneCreated,
    MilestoneDeleted,
    MilestoneLinked,
    MilestoneUnlinked,
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::ItemCreated => "item.created",
            Self::ItemUpdated => "item.updated",
            Self::ItemStatusChanged => "item.status_changed",
            Self::ItemDeleted => "item.deleted",
            Self::ItemAssigned => "item.assigned",
            Self::ItemUnassigned => "item.unassigned",
            Self::DepAdded => "dep.added",
            Self::DepRemoved => "dep.removed",
            Self::CommentAdded => "comment.added",
            Self::MilestoneCreated => "milestone.created",
            Self::MilestoneDeleted => "milestone.deleted",
            Self::MilestoneLinked => "milestone.linked",
            Self::MilestoneUnlinked => "milestone.unlinked",
        };
        write!(f, "{s}")
    }
}

impl EventType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "item.created" => Some(Self::ItemCreated),
            "item.updated" => Some(Self::ItemUpdated),
            "item.status_changed" => Some(Self::ItemStatusChanged),
            "item.deleted" => Some(Self::ItemDeleted),
            "item.assigned" => Some(Self::ItemAssigned),
            "item.unassigned" => Some(Self::ItemUnassigned),
            "dep.added" => Some(Self::DepAdded),
            "dep.removed" => Some(Self::DepRemoved),
            "comment.added" => Some(Self::CommentAdded),
            "milestone.created" => Some(Self::MilestoneCreated),
            "milestone.deleted" => Some(Self::MilestoneDeleted),
            "milestone.linked" => Some(Self::MilestoneLinked),
            "milestone.unlinked" => Some(Self::MilestoneUnlinked),
            _ => None,
        }
    }
}

pub struct Event {
    pub event_type: EventType,
    pub target: String,
    pub details: Option<String>,
    pub user: String,
}

/// Append an event to .joy/log/YYYY-MM-DD.log.
pub fn append_event(root: &Path, event: &Event) -> Result<(), JoyError> {
    let now = Utc::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

    let log_dir = store::joy_dir(root).join(store::LOG_DIR);
    fs::create_dir_all(&log_dir).map_err(|e| JoyError::CreateDir {
        path: log_dir.clone(),
        source: e,
    })?;

    let log_file = log_dir.join(format!("{date_str}.log"));

    let line = match &event.details {
        Some(details) => format!(
            "{timestamp} {target} {event_type} \"{details}\" [{user}]\n",
            event_type = event.event_type,
            target = event.target,
            user = event.user,
        ),
        None => format!(
            "{timestamp} {target} {event_type} [{user}]\n",
            event_type = event.event_type,
            target = event.target,
            user = event.user,
        ),
    };

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map_err(|e| JoyError::WriteFile {
            path: log_file.clone(),
            source: e,
        })?;

    file.write_all(line.as_bytes())
        .map_err(|e| JoyError::WriteFile {
            path: log_file,
            source: e,
        })
}

/// A parsed log entry for display.
#[derive(Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub event_type: String,
    pub target: String,
    pub details: Option<String>,
    pub user: String,
}

/// Read events from .joy/log/ files, newest first.
pub fn read_events(
    root: &Path,
    since: Option<&str>,
    item_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<LogEntry>, JoyError> {
    let log_dir = store::joy_dir(root).join(store::LOG_DIR);
    if !log_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut log_files: Vec<_> = fs::read_dir(&log_dir)
        .map_err(|e| JoyError::ReadFile {
            path: log_dir.clone(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
        .collect();

    // Sort descending by filename (newest day first)
    log_files.sort_by_key(|e| std::cmp::Reverse(e.file_name()));

    // Filter by date if --since is provided
    let since_date = since.map(|s| s.to_string());

    let mut entries = Vec::new();

    for file_entry in &log_files {
        let filename = file_entry.file_name();
        let filename = filename.to_string_lossy();
        let file_date = filename.trim_end_matches(".log");

        if let Some(ref since) = since_date {
            if file_date < since.as_str() {
                break;
            }
        }

        let content = fs::read_to_string(file_entry.path()).map_err(|e| JoyError::ReadFile {
            path: file_entry.path(),
            source: e,
        })?;

        // Parse lines in reverse (newest first within a day)
        let mut day_entries: Vec<LogEntry> = Vec::new();
        for line in content.lines() {
            if let Some(entry) = parse_log_line(line) {
                if let Some(filter) = item_filter {
                    if !entry.target.contains(filter) {
                        continue;
                    }
                }
                day_entries.push(entry);
            }
        }

        day_entries.reverse();
        entries.extend(day_entries);

        if entries.len() >= limit {
            entries.truncate(limit);
            break;
        }
    }

    entries.truncate(limit);
    Ok(entries)
}

/// Parse a single log line into a LogEntry.
fn parse_log_line(line: &str) -> Option<LogEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Format: TIMESTAMP ACRONYM EVENT_TYPE TARGET ["DETAILS"] [USER]
    // Extract user from trailing [user]
    let user_start = line.rfind('[')?;
    let user_end = line.rfind(']')?;
    if user_end <= user_start {
        return None;
    }
    let user = line[user_start + 1..user_end].to_string();
    let rest = line[..user_start].trim();

    // Extract optional details from "..."
    let (rest, details) = if let Some(dq_start) = rest.rfind('"') {
        let before_last = &rest[..dq_start];
        if let Some(dq_open) = before_last.rfind('"') {
            let details = rest[dq_open + 1..dq_start].to_string();
            let rest = rest[..dq_open].trim();
            (rest, Some(details))
        } else {
            (rest, None)
        }
    } else {
        (rest, None)
    };

    // Split remaining: TIMESTAMP TARGET EVENT_TYPE
    let parts: Vec<&str> = rest.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return None;
    }

    Some(LogEntry {
        timestamp: parts[0].to_string(),
        target: parts[1].to_string(),
        event_type: parts[2].to_string(),
        details,
        user,
    })
}

/// Get git user.email for the current user.
pub fn get_git_email() -> Result<String, JoyError> {
    let output = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()
        .map_err(|_| JoyError::Git("failed to run git config".to_string()))?;

    if !output.status.success() {
        return Err(JoyError::Git("git user.email not configured".to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Convenience: append an event, loading git email automatically.
/// Errors are silently ignored to avoid breaking the main command flow.
pub fn log_event(root: &Path, event_type: EventType, target: &str, details: Option<&str>) {
    let Ok(user) = get_git_email() else {
        return;
    };
    let event = Event {
        event_type,
        target: target.to_string(),
        details: details.map(|s| s.to_string()),
        user,
    };
    let _ = append_event(root, &event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let log_dir = dir.join(".joy").join("log");
        fs::create_dir_all(log_dir).unwrap();
    }

    #[test]
    fn append_and_read() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let event = Event {
            event_type: EventType::ItemCreated,
            target: "JOY-0001".to_string(),
            details: Some("User login".to_string()),
            user: "test@example.com".to_string(),
        };
        append_event(dir.path(), &event).unwrap();

        let entries = read_events(dir.path(), None, None, 100).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "item.created");
        assert_eq!(entries[0].target, "JOY-0001");
        assert_eq!(entries[0].details.as_deref(), Some("User login"));
        assert_eq!(entries[0].user, "test@example.com");
    }

    #[test]
    fn filter_by_item() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        for (target, details) in [
            ("JOY-0001", "First"),
            ("JOY-0002", "Second"),
            ("JOY-0001", "Update"),
        ] {
            let event = Event {
                event_type: EventType::ItemCreated,
                target: target.to_string(),
                details: Some(details.to_string()),
                user: "test@example.com".to_string(),
            };
            append_event(dir.path(), &event).unwrap();
        }

        let entries = read_events(dir.path(), None, Some("JOY-0001"), 100).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn parse_line_with_details() {
        let line =
            r#"2026-03-11T16:14:32.320Z JOY-0048 item.created "OAuth flow" [horst@joydev.com]"#;
        let entry = parse_log_line(line).unwrap();
        assert_eq!(entry.timestamp, "2026-03-11T16:14:32.320Z");
        assert_eq!(entry.event_type, "item.created");
        assert_eq!(entry.target, "JOY-0048");
        assert_eq!(entry.details.as_deref(), Some("OAuth flow"));
        assert_eq!(entry.user, "horst@joydev.com");
    }

    #[test]
    fn parse_line_without_details() {
        let line = "2026-03-11T16:14:32.320Z JOY-0048 item.status_changed [horst@joydev.com]";
        let entry = parse_log_line(line).unwrap();
        assert_eq!(entry.target, "JOY-0048");
        assert!(entry.details.is_none());
    }

    #[test]
    fn event_type_roundtrip() {
        let et = EventType::ItemStatusChanged;
        assert_eq!(et.to_string(), "item.status_changed");
        assert_eq!(EventType::parse("item.status_changed"), Some(et));
    }

    #[test]
    fn empty_log_dir() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let entries = read_events(dir.path(), None, None, 100).unwrap();
        assert!(entries.is_empty());
    }
}
