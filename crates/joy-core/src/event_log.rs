// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use chrono::Utc;

use crate::error::JoyError;
use crate::store;
use crate::vcs::Vcs;

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
    CommentEdited,
    CommentRemoved,
    MilestoneCreated,
    MilestoneUpdated,
    MilestoneDeleted,
    MilestoneLinked,
    MilestoneUnlinked,
    ReleaseCreated,
    GuardDenied,
    GuardWarned,
    AuthSessionCreated,
    JobDelegated,
    JobStatusChanged,
    JobReviewed,
    /// Project-level configuration change (e.g. platform key
    /// registration). Details carry a structural description, never
    /// user-authored content.
    ProjectUpdated,
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::JobDelegated => "job.delegated",
            Self::JobStatusChanged => "job.status_changed",
            Self::JobReviewed => "job.reviewed",
            Self::ItemCreated => "item.created",
            Self::ItemUpdated => "item.updated",
            Self::ItemStatusChanged => "item.status_changed",
            Self::ItemDeleted => "item.deleted",
            Self::ItemAssigned => "item.assigned",
            Self::ItemUnassigned => "item.unassigned",
            Self::DepAdded => "dep.added",
            Self::DepRemoved => "dep.removed",
            Self::CommentAdded => "comment.added",
            Self::CommentEdited => "comment.edited",
            Self::CommentRemoved => "comment.removed",
            Self::MilestoneCreated => "milestone.created",
            Self::MilestoneUpdated => "milestone.updated",
            Self::MilestoneDeleted => "milestone.deleted",
            Self::MilestoneLinked => "milestone.linked",
            Self::MilestoneUnlinked => "milestone.unlinked",
            Self::ReleaseCreated => "release.created",
            Self::GuardDenied => "guard.denied",
            Self::GuardWarned => "guard.warned",
            Self::AuthSessionCreated => "auth.session_created",
            Self::ProjectUpdated => "project.updated",
        };
        write!(f, "{s}")
    }
}

impl EventType {
    /// True if this event's `details` field would carry user-authored
    /// item content (titles, descriptions, comment text). Such payloads
    /// are stripped at log-write time so the log holds only structural
    /// metadata. Identifiers (member emails, item / milestone IDs,
    /// state names) and system-generated strings (guard reasons, auth
    /// session notices) are not user content. See JOY-0175-9B.
    pub fn carries_user_content(&self) -> bool {
        matches!(
            self,
            Self::ItemCreated
                | Self::ItemUpdated
                | Self::ItemDeleted
                | Self::CommentAdded
                | Self::MilestoneCreated
                | Self::MilestoneUpdated
                | Self::MilestoneDeleted
                | Self::ReleaseCreated
        )
    }

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
            "comment.edited" => Some(Self::CommentEdited),
            "comment.removed" => Some(Self::CommentRemoved),
            "milestone.created" => Some(Self::MilestoneCreated),
            "milestone.updated" => Some(Self::MilestoneUpdated),
            "milestone.deleted" => Some(Self::MilestoneDeleted),
            "milestone.linked" => Some(Self::MilestoneLinked),
            "milestone.unlinked" => Some(Self::MilestoneUnlinked),
            "release.created" => Some(Self::ReleaseCreated),
            "guard.denied" => Some(Self::GuardDenied),
            "guard.warned" => Some(Self::GuardWarned),
            "auth.session_created" => Some(Self::AuthSessionCreated),
            "project.updated" => Some(Self::ProjectUpdated),
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

    let effective_details = if event.event_type.carries_user_content() {
        None
    } else {
        event.details.as_deref()
    };

    let line = match effective_details {
        Some(details) => {
            let escaped = escape_details(details);
            format!(
                "{timestamp} {target} {event_type} \"{escaped}\" [{user}]\n",
                event_type = event.event_type,
                target = event.target,
                user = event.user,
            )
        }
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
            path: log_file.clone(),
            source: e,
        })?;
    let rel = format!("{}/{}/{}.log", store::JOY_DIR, store::LOG_DIR, date_str);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// A parsed log entry for display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub event_type: String,
    pub target: String,
    pub details: Option<String>,
    /// The acting member. Resolves to name/e-mail on display and in `--json`
    /// (ADR-042); the raw at-rest value is the e-mail (open) or opaque id (anon).
    pub user: crate::member_ref::MemberRef,
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

/// Escape newlines and backslashes in details for single-line log format.
fn escape_details(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

/// Unescape details read from log files.
fn unescape_details(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Validate that a string looks like an ISO 8601 timestamp (starts with YYYY-).
fn is_valid_timestamp(s: &str) -> bool {
    s.len() >= 20 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && s.as_bytes()[10] == b'T'
}

/// Parse a single log line into a LogEntry.
fn parse_log_line(line: &str) -> Option<LogEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Fast reject: valid lines always start with a digit (timestamp year)
    if !line.as_bytes().first().is_some_and(|b| b.is_ascii_digit()) {
        return None;
    }

    // Format: TIMESTAMP TARGET EVENT_TYPE ["DETAILS"] [USER]
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
            let details = unescape_details(&rest[dq_open + 1..dq_start]);
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

    // Validate timestamp format
    if !is_valid_timestamp(parts[0]) {
        return None;
    }

    Some(LogEntry {
        timestamp: parts[0].to_string(),
        target: parts[1].to_string(),
        event_type: parts[2].to_string(),
        details,
        user: user.into(),
    })
}

/// Read all events (oldest first, no limit). Used for release computation.
/// The event types that count as an ATTRIBUTE change of an item: what
/// `joy show`'s footer and the apps' history tab render as "Updated"
/// lines. Creation has its own line, deletion ends the item, and
/// comment events keep their audit on the comment itself.
pub const ITEM_ATTRIBUTE_EVENTS: &[&str] = &[
    "item.updated",
    "item.status_changed",
    "item.assigned",
    "item.unassigned",
    "dep.added",
    "dep.removed",
    "milestone.linked",
    "milestone.unlinked",
];

/// The attribute-change history of one item, oldest first, DERIVED from
/// the event log. The log is the one audit record — (timestamp, id,
/// action, actor), append-only, value-free (decision JOY-0175-9B) — and
/// every display joins it at lookup time; nothing is stored on the item.
pub fn item_attribute_history(root: &Path, item_id: &str) -> Result<Vec<LogEntry>, JoyError> {
    let mut entries: Vec<LogEntry> = read_events(root, None, Some(item_id), usize::MAX)?
        .into_iter()
        .filter(|e| ITEM_ATTRIBUTE_EVENTS.contains(&e.event_type.as_str()))
        .collect();
    entries.reverse(); // read_events answers newest first
    Ok(entries)
}

/// [`item_attribute_history`] for every item at once (one log pass), for
/// the servers' item listings: item id -> oldest-first entries.
pub fn attribute_history_by_item(
    root: &Path,
) -> Result<std::collections::BTreeMap<String, Vec<LogEntry>>, JoyError> {
    let mut map: std::collections::BTreeMap<String, Vec<LogEntry>> =
        std::collections::BTreeMap::new();
    for entry in read_all_events(root)? {
        if ITEM_ATTRIBUTE_EVENTS.contains(&entry.event_type.as_str()) {
            map.entry(entry.target.clone()).or_default().push(entry);
        }
    }
    for entries in map.values_mut() {
        entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    }
    Ok(map)
}

pub fn read_all_events(root: &Path) -> Result<Vec<LogEntry>, JoyError> {
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

    // Sort ascending by filename (oldest day first)
    log_files.sort_by_key(|e| e.file_name());

    let mut entries = Vec::new();
    for file_entry in &log_files {
        let content = fs::read_to_string(file_entry.path()).map_err(|e| JoyError::ReadFile {
            path: file_entry.path(),
            source: e,
        })?;
        for line in content.lines() {
            if let Some(entry) = parse_log_line(line) {
                entries.push(entry);
            }
        }
    }

    Ok(entries)
}

/// Find the timestamp of the last release.created event, if any.
pub fn last_release_timestamp(root: &Path) -> Result<Option<String>, JoyError> {
    let events = read_all_events(root)?;
    let last = events
        .iter()
        .rev()
        .find(|e| e.event_type == "release.created");
    Ok(last.map(|e| e.timestamp.clone()))
}

/// Collect unique item IDs that were closed after a given timestamp.
/// If cutoff is None, returns all items ever closed.
/// Returns deduplicated item IDs (an item closed multiple times appears once).
pub fn closed_item_ids_since(root: &Path, cutoff: Option<&str>) -> Result<Vec<String>, JoyError> {
    let events = read_all_events(root)?;
    let mut seen = std::collections::HashSet::new();
    let mut results: Vec<String> = Vec::new();

    for entry in &events {
        if entry.event_type != "item.status_changed" {
            continue;
        }
        let is_close = entry
            .details
            .as_deref()
            .is_some_and(|d| d.contains("-> closed"));
        if !is_close {
            continue;
        }
        if let Some(cutoff) = cutoff {
            if entry.timestamp.as_str() <= cutoff {
                continue;
            }
        }
        if seen.insert(entry.target.clone()) {
            results.push(entry.target.clone());
        }
    }

    Ok(results)
}

/// Actor statistics: event count and unique item count.
pub struct ActorStats {
    pub id: String,
    pub events: usize,
    pub items: usize,
}

/// Collect actor stats for a specific set of item IDs.
/// Only counts events whose target matches one of the given item IDs.
pub fn actors_for_items(root: &Path, item_ids: &[String]) -> Result<Vec<ActorStats>, JoyError> {
    let id_set: std::collections::HashSet<&str> = item_ids.iter().map(|s| s.as_str()).collect();
    let events = read_all_events(root)?;
    let mut event_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut item_sets: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();

    for entry in &events {
        if !id_set.contains(entry.target.as_str()) {
            continue;
        }
        if entry.event_type.starts_with("item.") || entry.event_type.starts_with("comment.") {
            *event_counts.entry(entry.user.id().to_string()).or_default() += 1;
            item_sets
                .entry(entry.user.id().to_string())
                .or_default()
                .insert(entry.target.clone());
        }
    }

    let mut result: Vec<ActorStats> = event_counts
        .into_iter()
        .map(|(id, events)| {
            let items = item_sets.get(&id).map(|s| s.len()).unwrap_or(0);
            ActorStats { id, events, items }
        })
        .collect();
    result.sort_by_key(|a| std::cmp::Reverse(a.events));
    Ok(result)
}

/// Get git user.email for the current user.
pub fn get_git_email() -> Result<String, JoyError> {
    crate::vcs::default_vcs().user_email()
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

/// Like `log_event`, but uses a pre-resolved identity string.
/// This allows the caller to pass the `Identity::log_user()` value
/// which may include `delegated-by:` for AI members.
pub fn log_event_as(
    root: &Path,
    event_type: EventType,
    target: &str,
    details: Option<&str>,
    user: &str,
) {
    let event = Event {
        event_type,
        target: target.to_string(),
        details: details.map(|s| s.to_string()),
        user: user.to_string(),
    };
    let _ = append_event(root, &event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let log_dir = dir.join(".joy").join("logs");
        fs::create_dir_all(log_dir).unwrap();
    }

    #[test]
    fn append_and_read() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let event = Event {
            event_type: EventType::ItemStatusChanged,
            target: "JOY-0001".to_string(),
            details: Some("new -> in-progress".to_string()),
            user: "test@example.com".to_string(),
        };
        append_event(dir.path(), &event).unwrap();

        let entries = read_events(dir.path(), None, None, 100).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "item.status_changed");
        assert_eq!(entries[0].target, "JOY-0001");
        assert_eq!(entries[0].details.as_deref(), Some("new -> in-progress"));
        assert_eq!(entries[0].user, "test@example.com");
    }

    #[test]
    fn filter_by_item() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        for target in ["JOY-0001", "JOY-0002", "JOY-0001"] {
            let event = Event {
                event_type: EventType::ItemCreated,
                target: target.to_string(),
                details: None,
                user: "test@example.com".to_string(),
            };
            append_event(dir.path(), &event).unwrap();
        }

        let entries = read_events(dir.path(), None, Some("JOY-0001"), 100).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn user_content_is_stripped_at_write_time() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        for kind in [
            EventType::ItemCreated,
            EventType::ItemUpdated,
            EventType::ItemDeleted,
            EventType::CommentAdded,
            EventType::MilestoneCreated,
            EventType::MilestoneUpdated,
            EventType::MilestoneDeleted,
            EventType::ReleaseCreated,
        ] {
            assert!(
                kind.carries_user_content(),
                "{kind} must be content-bearing"
            );
        }
        for kind in [
            EventType::ItemStatusChanged,
            EventType::ItemAssigned,
            EventType::ItemUnassigned,
            EventType::DepAdded,
            EventType::DepRemoved,
            EventType::CommentEdited,
            EventType::CommentRemoved,
            EventType::MilestoneLinked,
            EventType::MilestoneUnlinked,
            EventType::GuardDenied,
            EventType::GuardWarned,
            EventType::AuthSessionCreated,
        ] {
            assert!(!kind.carries_user_content(), "{kind} must be structural");
        }

        let event = Event {
            event_type: EventType::CommentAdded,
            target: "JOY-0001".to_string(),
            details: Some("a secret comment that should never land in the log".to_string()),
            user: "test@example.com".to_string(),
        };
        append_event(dir.path(), &event).unwrap();

        let entries = read_events(dir.path(), None, None, 100).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "comment.added");
        assert_eq!(entries[0].details, None);
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

    #[test]
    fn escape_roundtrip() {
        assert_eq!(escape_details("simple"), "simple");
        assert_eq!(escape_details("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_details("back\\slash"), "back\\\\slash");
        assert_eq!(escape_details("both\nand\\"), "both\\nand\\\\");

        assert_eq!(unescape_details("simple"), "simple");
        assert_eq!(unescape_details("line1\\nline2"), "line1\nline2");
        assert_eq!(unescape_details("back\\\\slash"), "back\\slash");
        assert_eq!(unescape_details("both\\nand\\\\"), "both\nand\\");
    }

    #[test]
    fn multiline_details_roundtrip() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        // Use a structural event type so the writer does not strip
        // details; the round-trip targets the escape/unescape path,
        // not the strip rule (covered separately).
        let multiline = "First line\nSecond line\nThird with \\backslash";
        let event = Event {
            event_type: EventType::GuardDenied,
            target: "JOY-0001".to_string(),
            details: Some(multiline.to_string()),
            user: "test@example.com".to_string(),
        };
        append_event(dir.path(), &event).unwrap();

        let entries = read_events(dir.path(), None, None, 100).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].details.as_deref(), Some(multiline));
    }

    #[test]
    fn reject_non_timestamp_lines() {
        assert!(parse_log_line(">").is_none());
        assert!(parse_log_line("> some text [user@x.com]").is_none());
        assert!(parse_log_line("Apple Reminders <-- CalDAV --> joyint.com").is_none());
        assert!(parse_log_line("").is_none());
        assert!(parse_log_line("   ").is_none());
    }

    #[test]
    fn timestamp_validation() {
        assert!(is_valid_timestamp("2026-03-11T16:14:32.320Z"));
        assert!(!is_valid_timestamp(">"));
        assert!(!is_valid_timestamp("not-a-timestamp"));
        assert!(!is_valid_timestamp("2026"));
    }
}
