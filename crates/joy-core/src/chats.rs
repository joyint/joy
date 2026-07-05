// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Read/write for git-native chats under `.joy/chats/` (JOY-01F1).
//! Mirrors the item store: one YAML per chat, staged into git on save so
//! the forge stays the source of truth.

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::error::JoyError;
use crate::member_ref::MemberRef;
use crate::model::chat::{Chat, ChatKind, ChatMessage, MessageKind};
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
    let mut chat: Chat = store::read_yaml(&path)?;
    normalize(&mut chat);
    Ok(Some(chat))
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
        let mut chat = store::read_yaml::<Chat>(&path)?;
        normalize(&mut chat);
        chats.push(chat);
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

/// Append a member message and persist. Refuses on a frozen
/// (delete-for-all) chat. Returns the appended message.
pub fn append_message(
    root: &Path,
    chat: &mut Chat,
    author: MemberRef,
    text: impl Into<String>,
    now: DateTime<Utc>,
) -> Result<ChatMessage, JoyError> {
    if chat.read_only {
        return Err(JoyError::GuardDenied(format!(
            "chat {} was deleted for everyone and is read-only",
            chat.id
        )));
    }
    append_kind(root, chat, author, text, MessageKind::Text, now)
}

/// Append a system notice ("@xy left"); allowed even on read-only chats
/// (the delete-for-all notice itself must land).
pub fn append_notice(
    root: &Path,
    chat: &mut Chat,
    author: MemberRef,
    text: impl Into<String>,
    now: DateTime<Utc>,
) -> Result<ChatMessage, JoyError> {
    append_kind(root, chat, author, text, MessageKind::Notice, now)
}

fn append_kind(
    root: &Path,
    chat: &mut Chat,
    author: MemberRef,
    text: impl Into<String>,
    kind: MessageKind,
    now: DateTime<Utc>,
) -> Result<ChatMessage, JoyError> {
    append_kind_with_id(root, chat, author, text, kind, now, None)
}

/// The channel append (ADR JAPP-00C9): a caller that minted the message
/// id client-side passes it through, making retries idempotent — an id
/// already in the chat is a successful no-op returning the stored copy.
pub fn append_kind_with_id(
    root: &Path,
    chat: &mut Chat,
    author: MemberRef,
    text: impl Into<String>,
    kind: MessageKind,
    now: DateTime<Utc>,
    id: Option<String>,
) -> Result<ChatMessage, JoyError> {
    let id = id.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    if let Some(existing) = chat.messages.iter().find(|m| m.id == id) {
        return Ok(existing.clone());
    }
    let message = ChatMessage {
        id,
        at: now,
        author,
        text: text.into(),
        kind,
        delegated_by: None,
        turn_ms: None,
        tool_steps: None,
    };
    chat.messages.push(message.clone());
    chat.updated = now;
    save_chat(root, chat)?;
    Ok(message)
}

/// Persist an AI turn's reply WITH its attribution and cost metadata
/// (delegated-by, wall time, tool steps): the info icon must survive a
/// reload like everything else in the chat (ADR JAPP-00C9).
#[allow(clippy::too_many_arguments)]
pub fn append_ai_reply(
    root: &Path,
    chat: &mut Chat,
    author: MemberRef,
    text: impl Into<String>,
    now: DateTime<Utc>,
    id: Option<String>,
    delegated_by: Option<String>,
    turn_ms: Option<u32>,
    tool_steps: Option<u32>,
) -> Result<ChatMessage, JoyError> {
    let message = append_kind_with_id(root, chat, author, text, MessageKind::Text, now, id)?;
    let stored = chat
        .messages
        .iter_mut()
        .find(|m| m.id == message.id)
        .expect("just appended");
    stored.delegated_by = delegated_by;
    stored.turn_ms = turn_ms;
    stored.tool_steps = tool_steps;
    let enriched = stored.clone();
    save_chat(root, chat)?;
    Ok(enriched)
}

/// Every load path funnels through here: pre-channel messages get their
/// deterministic synthetic id and the timeline is ordered by time (a
/// merge unions divergent appends; the order must be identical on every
/// client because seq = position).
fn normalize(chat: &mut Chat) {
    for m in &mut chat.messages {
        if m.id.is_empty() {
            m.id = m.synthetic_id();
        }
    }
    chat.messages
        .sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
}

// ---- lifecycle (JOY-01F6) ------------------------------------------------

/// The fixed id of the team-wide General chat.
pub const GENERAL_CHAT_ID: &str = "general";

/// Ensure the General chat exists (fixed id, participants = all project
/// members, expressed as an empty list). Returns it.
pub fn ensure_general(root: &Path, now: DateTime<Utc>) -> Result<Chat, JoyError> {
    if let Some(chat) = load_chat(root, GENERAL_CHAT_ID)? {
        return Ok(chat);
    }
    let mut chat = Chat::new(GENERAL_CHAT_ID, Vec::new(), now);
    chat.kind = ChatKind::General;
    chat.title = Some("General".into());
    chat.subtitle = Some("for all team members".into());
    save_chat(root, &chat)?;
    Ok(chat)
}

/// The chat's effective participants: General carries an empty list that
/// means "every project member" — resolve it for display and turn logic.
pub fn effective_participants(
    root: &Path,
    chat: &Chat,
) -> Result<Vec<crate::member_ref::MemberRef>, JoyError> {
    if chat.kind == ChatKind::General && chat.participants.is_empty() {
        let project = crate::store::load_project(root)?;
        return Ok(project
            .members()
            .map(|(key, _)| crate::member_ref::MemberRef::new(key.clone()))
            .collect());
    }
    Ok(chat.participants.clone())
}

fn guard_not_general(chat: &Chat, action: &str) -> Result<(), JoyError> {
    if chat.kind == ChatKind::General {
        return Err(JoyError::GuardDenied(format!(
            "the General chat cannot be {action}"
        )));
    }
    Ok(())
}

/// Leave a chat: drop the member from participants and post the notice.
pub fn leave(
    root: &Path,
    chat: &mut Chat,
    member: &MemberRef,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    guard_not_general(chat, "left")?;
    chat.participants.retain(|p| p.id() != member.id());
    append_notice(
        root,
        chat,
        member.clone(),
        format!("@{} left", member.id()),
        now,
    )?;
    Ok(())
}

/// Add (or re-add) a member and post the notice.
pub fn add_participant(
    root: &Path,
    chat: &mut Chat,
    member: MemberRef,
    added_by: &MemberRef,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    guard_not_general(chat, "changed: everyone is in it")?;
    if chat.read_only {
        return Err(JoyError::GuardDenied(format!(
            "chat {} was deleted for everyone and is read-only",
            chat.id
        )));
    }
    if !chat.participants.iter().any(|p| p.id() == member.id()) {
        chat.participants.push(member.clone());
        append_notice(
            root,
            chat,
            added_by.clone(),
            format!("@{} was added", member.id()),
            now,
        )?;
    }
    Ok(())
}

/// Rename a chat (General keeps its identity).
pub fn rename(
    root: &Path,
    chat: &mut Chat,
    title: impl Into<String>,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    guard_not_general(chat, "renamed")?;
    chat.title = Some(title.into());
    chat.updated = now;
    save_chat(root, chat)
}

/// Set the subtitle (General keeps its identity).
pub fn set_subtitle(
    root: &Path,
    chat: &mut Chat,
    subtitle: impl Into<String>,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    guard_not_general(chat, "changed")?;
    chat.subtitle = Some(subtitle.into());
    chat.updated = now;
    save_chat(root, chat)
}

/// Delete for everyone: freeze the chat and post the notice. The file
/// stays until every participant also deleted it locally.
pub fn delete_for_all(
    root: &Path,
    chat: &mut Chat,
    by: &MemberRef,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    guard_not_general(chat, "deleted")?;
    chat.read_only = true;
    append_notice(
        root,
        chat,
        by.clone(),
        format!("@{} deleted this chat", by.id()),
        now,
    )?;
    Ok(())
}

/// A member's local delete of a frozen chat. Once every participant did,
/// the file itself is removed (garbage collection).
pub fn delete_for_me(
    root: &Path,
    chat: &mut Chat,
    member: &MemberRef,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    guard_not_general(chat, "deleted")?;
    if !chat.deleted_for.iter().any(|m| m.id() == member.id()) {
        chat.deleted_for.push(member.clone());
        chat.updated = now;
        save_chat(root, chat)?;
    }
    // AI members never "delete" a chat; garbage collection waits for the
    // humans only.
    let everyone_done = chat
        .participants
        .iter()
        .filter(|p| !p.id().starts_with("ai:"))
        .all(|p| chat.deleted_for.iter().any(|m| m.id() == p.id()));
    if everyone_done {
        let path = chats_dir(root).join(format!("{}.yaml", chat.id));
        let _ = std::fs::remove_file(&path);
        let rel = format!("{}/{}/{}.yaml", store::JOY_DIR, CHATS_DIR, chat.id);
        crate::git_ops::auto_git_add(root, &[&rel]);
    }
    Ok(())
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

    #[test]
    fn general_is_protected_and_lazily_created() {
        let dir = tempdir().unwrap();
        let g = ensure_general(dir.path(), ts(0)).unwrap();
        assert_eq!(g.id, GENERAL_CHAT_ID);
        assert_eq!(g.subtitle.as_deref(), Some("for all team members"));
        // idempotent
        let again = ensure_general(dir.path(), ts(1)).unwrap();
        assert_eq!(again.created, g.created);

        let mut g = again;
        let horst = MemberRef::new("horst@example.com");
        assert!(leave(dir.path(), &mut g, &horst, ts(2)).is_err());
        assert!(rename(dir.path(), &mut g, "X", ts(2)).is_err());
        assert!(delete_for_all(dir.path(), &mut g, &horst, ts(2)).is_err());
    }

    #[test]
    fn leave_readd_and_delete_semantics() {
        let dir = tempdir().unwrap();
        let horst = MemberRef::new("horst@example.com");
        let geordi = MemberRef::new("geordi@example.org");
        let mut chat = open_chat(
            dir.path(),
            vec![horst.clone(), geordi.clone()],
            Some("Release".into()),
            ts(0),
        )
        .unwrap();
        chat.created_by = Some(horst.clone());
        save_chat(dir.path(), &chat).unwrap();

        // leave posts the notice and drops the member
        leave(dir.path(), &mut chat, &geordi, ts(1)).unwrap();
        assert_eq!(chat.participants.len(), 1);
        assert!(matches!(
            chat.messages.last().unwrap().kind,
            MessageKind::Notice
        ));
        assert!(chat.messages.last().unwrap().text.contains("left"));

        // re-add restores membership with a notice
        add_participant(dir.path(), &mut chat, geordi.clone(), &horst, ts(2)).unwrap();
        assert_eq!(chat.participants.len(), 2);
        assert!(chat.messages.last().unwrap().text.contains("was added"));

        // delete-for-all freezes it: member messages refuse, notices work
        delete_for_all(dir.path(), &mut chat, &horst, ts(3)).unwrap();
        assert!(chat.read_only);
        assert!(append_message(dir.path(), &mut chat, horst.clone(), "hi", ts(4)).is_err());
        assert!(add_participant(
            dir.path(),
            &mut chat,
            MemberRef::new("x@y.z"),
            &horst,
            ts(4)
        )
        .is_err());

        // per-member local delete; file is GCed when everyone did
        delete_for_me(dir.path(), &mut chat, &horst, ts(5)).unwrap();
        assert!(load_chat(dir.path(), &chat.id).unwrap().is_some());
        delete_for_me(dir.path(), &mut chat, &geordi, ts(6)).unwrap();
        assert!(load_chat(dir.path(), &chat.id).unwrap().is_none());
    }

    #[test]
    fn rename_and_subtitle() {
        let dir = tempdir().unwrap();
        let horst = MemberRef::new("horst@example.com");
        let mut chat = open_chat(dir.path(), vec![horst], None, ts(0)).unwrap();
        rename(dir.path(), &mut chat, "Sprint 13", ts(1)).unwrap();
        set_subtitle(dir.path(), &mut chat, "countdown", ts(2)).unwrap();
        let loaded = load_chat(dir.path(), &chat.id).unwrap().unwrap();
        assert_eq!(loaded.title.as_deref(), Some("Sprint 13"));
        assert_eq!(loaded.subtitle.as_deref(), Some("countdown"));
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 5, 3, 0, 0).unwrap()
    }

    #[test]
    fn append_mints_a_uuid_and_is_idempotent_per_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        let mut chat = open_chat(dir.path(), vec![MemberRef::new("a@x")], None, now()).unwrap();
        let m1 = append_message(dir.path(), &mut chat, MemberRef::new("a@x"), "hi", now()).unwrap();
        assert!(!m1.id.is_empty());
        // a retry with the SAME id (outbox resend) must not duplicate
        let again = append_kind_with_id(
            dir.path(),
            &mut chat,
            MemberRef::new("a@x"),
            "hi",
            MessageKind::Text,
            now(),
            Some(m1.id.clone()),
        )
        .unwrap();
        assert_eq!(again.id, m1.id);
        assert_eq!(chat.messages.len(), 1);
    }

    #[test]
    fn legacy_messages_get_stable_synthetic_ids_and_time_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".joy/chats")).unwrap();
        std::fs::write(
            dir.path().join(".joy/chats/aaaa.yaml"),
            "id: aaaa\ntitle: T\ncreated_by: a@x\ncreated: 2026-07-05T02:00:00Z\nupdated: 2026-07-05T02:00:00Z\nparticipants:\n- a@x\nmessages:\n- at: 2026-07-05T02:00:02Z\n  author: a@x\n  text: second\n- at: 2026-07-05T02:00:01Z\n  author: a@x\n  text: first\n",
        )
        .unwrap();
        let chat = load_chat(dir.path(), "aaaa").unwrap().unwrap();
        assert_eq!(chat.messages[0].text, "first");
        assert!(chat.messages[0].id.starts_with("legacy-"));
        // deterministic: a second load derives the SAME ids
        let chat2 = load_chat(dir.path(), "aaaa").unwrap().unwrap();
        assert_eq!(chat.messages[0].id, chat2.messages[0].id);
    }

    #[test]
    fn divergent_appends_merge_without_duplicates() {
        // the joy-yaml driver unions messages by id — simulate the merge
        let base = "id: c\ntitle: T\ncreated_by: a@x\ncreated: 2026-07-05T02:00:00Z\nupdated: 2026-07-05T02:00:00Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T02:00:01Z\n  author: a@x\n  text: hello\n";
        let ours = "id: c\ntitle: T\ncreated_by: a@x\ncreated: 2026-07-05T02:00:00Z\nupdated: 2026-07-05T02:00:02Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T02:00:01Z\n  author: a@x\n  text: hello\n- id: m2\n  at: 2026-07-05T02:00:02Z\n  author: a@x\n  text: ours\n";
        let theirs = "id: c\ntitle: T\ncreated_by: a@x\ncreated: 2026-07-05T02:00:00Z\nupdated: 2026-07-05T02:00:03Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T02:00:01Z\n  author: a@x\n  text: hello\n- id: m3\n  at: 2026-07-05T02:00:03Z\n  author: b@x\n  text: theirs\n";
        let merged = crate::merge::merge_yaml_doc(base, ours, theirs).unwrap();
        let chat: crate::model::chat::Chat = serde_yaml_ng::from_str(&merged).unwrap();
        let ids: Vec<&str> = chat.messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"m2") && ids.contains(&"m3"));
    }

    #[test]
    fn error_kind_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        let mut chat = open_chat(dir.path(), vec![MemberRef::new("a@x")], None, now()).unwrap();
        append_kind_with_id(
            dir.path(),
            &mut chat,
            MemberRef::new("a@x"),
            "joy auth failed: not implemented",
            MessageKind::Error,
            now(),
            None,
        )
        .unwrap();
        let loaded = load_chat(dir.path(), &chat.id).unwrap().unwrap();
        assert_eq!(loaded.messages[0].kind, MessageKind::Error);
    }
}
