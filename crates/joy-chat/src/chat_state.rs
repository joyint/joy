// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Personal, per-member chat state (JOY-01F5): pins, sidebar order,
//! last-read markers, and local hides. Lives in the gitignored
//! `.joy/chat-state.yaml` (like `.joy/config.yaml`) so reading a chat,
//! pinning it, or dragging the sidebar never creates a commit.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use joy_core::error::JoyError;
use joy_core::store;

pub const CHAT_STATE_FILE: &str = "chat-state.yaml";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatState {
    /// Pinned chat ids (personal).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<String>,
    /// Sidebar order for team chats; unlisted chats follow, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Newest message `at` seen per chat (unread = anything newer).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub last_read: BTreeMap<String, DateTime<Utc>>,
    /// Locally hidden chats (a silent personal hide).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
}

fn path(root: &Path) -> std::path::PathBuf {
    store::joy_dir(root).join(CHAT_STATE_FILE)
}

pub fn load(root: &Path) -> ChatState {
    store::read_yaml(&path(root)).unwrap_or_default()
}

pub fn save(root: &Path, state: &ChatState) -> Result<(), JoyError> {
    store::write_yaml(&path(root), state)
}

/// Mark a chat read up to `at`.
pub fn mark_read(root: &Path, chat_id: &str, at: DateTime<Utc>) -> Result<(), JoyError> {
    let mut state = load(root);
    state.last_read.insert(chat_id.to_string(), at);
    save(root, &state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn state_roundtrip_and_defaults() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        assert_eq!(load(dir.path()), ChatState::default());
        let mut s = ChatState::default();
        s.pinned.push("general".into());
        s.order = vec!["general".into(), "abc".into()];
        save(dir.path(), &s).unwrap();
        mark_read(dir.path(), "abc", "2026-07-04T10:00:00Z".parse().unwrap()).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.pinned, vec!["general".to_string()]);
        assert!(loaded.last_read.contains_key("abc"));
    }
}
