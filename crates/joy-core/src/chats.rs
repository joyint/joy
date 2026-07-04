// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Read/write for git-native chats under `.joy/chats/` (JOY-01F1).
//! Mirrors the item store: one YAML per chat, staged into git on save so
//! the forge stays the source of truth.

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::error::JoyError;
use crate::member_ref::MemberRef;
use crate::model::chat::{Chat, ChatMessage};
use crate::store;

/// Subdir of `.joy/` holding chats.
pub const CHATS_DIR: &str = "chats";

fn chats_dir(root: &Path) -> std::path::PathBuf {
    store::joy_dir(root).join(CHATS_DIR)
}

/// A fresh, short, file-safe chat id.
pub fn new_chat_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

/// Save a chat to `.joy/chats/<id>.yaml` and stage it.
pub fn save_chat(root: &Path, chat: &Chat) -> Result<(), JoyError> {
    let dir = chats_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| JoyError::CreateDir {
        path: dir.clone(),
        source: e,
    })?;
    let filename = format!("{}.yaml", chat.id);
    store::write_yaml(&dir.join(&filename), chat)?;
    let rel = format!("{}/{}/{}", store::JOY_DIR, CHATS_DIR, filename);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// Load one chat by id, if present.
pub fn load_chat(root: &Path, id: &str) -> Result<Option<Chat>, JoyError> {
    let path = chats_dir(root).join(format!("{id}.yaml"));
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(store::read_yaml(&path)?))
}

/// Load every chat, newest-updated first.
pub fn load_chats(root: &Path) -> Result<Vec<Chat>, JoyError> {
    let dir = chats_dir(root);
    let mut chats = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(chats);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        chats.push(store::read_yaml::<Chat>(&path)?);
    }
    chats.sort_by_key(|c| std::cmp::Reverse(c.updated));
    Ok(chats)
}

/// Open (or create) a chat and persist it.
pub fn open_chat(
    root: &Path,
    participants: Vec<MemberRef>,
    title: Option<String>,
    now: DateTime<Utc>,
) -> Result<Chat, JoyError> {
    let mut chat = Chat::new(new_chat_id(), participants, now);
    chat.title = title;
    save_chat(root, &chat)?;
    Ok(chat)
}

/// Append a message and persist. Returns the appended message.
pub fn append_message(
    root: &Path,
    chat: &mut Chat,
    author: MemberRef,
    text: impl Into<String>,
    now: DateTime<Utc>,
) -> Result<ChatMessage, JoyError> {
    let message = ChatMessage {
        at: now,
        author,
        text: text.into(),
    };
    chat.messages.push(message.clone());
    chat.updated = now;
    save_chat(root, chat)?;
    Ok(message)
}

/// Record the ACP session id of an AI participant and persist.
pub fn set_ai_session(
    root: &Path,
    chat: &mut Chat,
    member: &MemberRef,
    session_id: impl Into<String>,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    chat.ai_sessions
        .insert(member.id().to_string(), session_id.into());
    chat.updated = now;
    save_chat(root, chat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ts(sec: u32) -> DateTime<Utc> {
        format!("2026-07-04T00:00:{sec:02}Z").parse().unwrap()
    }

    #[test]
    fn chat_roundtrip_append_and_ai_session() {
        let dir = tempdir().unwrap();
        let horst = MemberRef::new("horst@example.com");
        let geordi = MemberRef::new("geordi@example.org");
        let claude = MemberRef::new("ai:claude@joy");

        let mut chat = open_chat(
            dir.path(),
            vec![horst.clone(), geordi.clone(), claude.clone()],
            Some("Standup".into()),
            ts(0),
        )
        .unwrap();

        append_message(dir.path(), &mut chat, horst.clone(), "moin", ts(1)).unwrap();
        append_message(dir.path(), &mut chat, geordi.clone(), "hi Horst", ts(2)).unwrap();
        set_ai_session(dir.path(), &mut chat, &claude, "acp-session-42", ts(3)).unwrap();

        let loaded = load_chat(dir.path(), &chat.id).unwrap().unwrap();
        assert_eq!(loaded, chat);
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].text, "moin");
        assert_eq!(loaded.messages[1].author, geordi);
        assert_eq!(
            loaded.ai_sessions.get("ai:claude@joy").unwrap(),
            "acp-session-42"
        );

        let all = load_chats(dir.path()).unwrap();
        assert_eq!(all.len(), 1);
    }
}
