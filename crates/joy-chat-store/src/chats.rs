// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Read/write for git-native chats under `.joy/chats/` (JOY-01F1).
//! Mirrors the item store: one YAML per chat, staged into git on save so
//! the forge stays the source of truth.

use std::path::Path;

use chrono::{DateTime, Utc};

use joy_chat::model::chat::{Chat, ChatKind, ChatMessage, MessageKind};
use joy_core::error::JoyError;
use joy_core::member_ref::MemberRef;
use joy_core::model::config::InteractionLevel;

/// Legacy subdir under `.joy/` where chats lived before they moved to the
/// dedicated ref (ADR JAPP-00DC-FC). Kept only so the one-time migration
/// can find and remove the stale working-tree files.
pub const CHATS_DIR: &str = "chats";

/// A fresh, short, file-safe chat id.
pub fn new_chat_id() -> String {
    // A full 128-bit opaque id (32 hex chars). The tree name is the only
    // plaintext a chat leaks, so it carries no structure a keyless reader
    // could read (ADR JAPP-002A-30).
    uuid::Uuid::new_v4().simple().to_string()
}

/// Save a chat onto the `refs/joy/chats` ref (never the working branch),
/// SEALED (ADR JAPP-002A-30): with a custodian seed set, the content key
/// is ensured, pending messages are sealed and only the at-rest form is
/// written. Without a custodian, writing an encrypted chat is refused,
/// and a project that COULD encrypt (any identity on record) never gets
/// plaintext either: the chat stays ephemeral until someone
/// authenticates. Only a checkout without a Joy project (bare library
/// use) or a project without any identity persists as before.
/// Sealing mutates the CALLER's chat too (crypt header, per-message
/// envelopes next to the opened fields), so a publisher right after the
/// save holds the exact sealed form the wire needs.
pub fn save_chat(root: &Path, chat: &mut Chat) -> Result<(), JoyError> {
    // Sealed whole-file storage (ADR JAPP-002A-30): with a custodian seed
    // and a project that has an identity to wrap for, persist through
    // [`crate::chat_store`] (opaque keys/log tree; migrates a legacy chat
    // in place). The in-memory `chat` stays OPENED for the caller.
    if let Some(seed) = crate::writer::seed() {
        if crate::chat_store::can_seal(root, chat) {
            return crate::chat_store::save(root, chat, &seed);
        }
    }
    // No custodian, or nothing to wrap: a project that COULD encrypt never
    // gets plaintext (it stays ephemeral until someone authenticates);
    // only a project with no identity at all persists in the clear.
    if let Ok(project) = joy_core::store::load_project(root) {
        let encryptable = project.members().any(|(_, m)| m.verify_key.is_some());
        if encryptable {
            return Err(JoyError::AuthFailed(
                "chat not persisted: authenticate first (ADR JAPP-002A-30)".into(),
            ));
        }
    }
    crate::chat_ref::save_chat(root, chat)
}

/// Load one chat by id, opened. New-format (sealed) chats fold through
/// [`crate::chat_store`]; a legacy (unmigrated) chat falls back to the old
/// reader plus the custodian open.
pub fn load_chat(root: &Path, id: &str) -> Result<Option<Chat>, JoyError> {
    if let Some(seed) = crate::writer::seed() {
        if let Some(mut chat) = crate::chat_store::load(root, id, &seed)? {
            normalize(&mut chat);
            return Ok(Some(chat));
        }
    }
    // A project with no identity at all keeps its chats in the clear
    // (ADR JAPP-002A-30); that is the only thing left to read here.
    match crate::chat_ref::load_chat(root, id)? {
        Some(mut chat) => {
            normalize(&mut chat);
            Ok(Some(chat))
        }
        None => Ok(None),
    }
}

/// Load every chat, opened, newest-updated first. Sealed chats come from
/// [`crate::chat_store`]; any legacy chat not yet migrated is added from
/// the old reader.
pub fn load_chats(root: &Path) -> Result<Vec<Chat>, JoyError> {
    let mut out: Vec<Chat> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(seed) = crate::writer::seed() {
        for mut chat in crate::chat_store::load_all(root, &seed)? {
            normalize(&mut chat);
            seen.insert(chat.id.clone());
            out.push(chat);
        }
    }
    for mut chat in crate::chat_ref::load_chats(root)? {
        if seen.contains(&chat.id) {
            continue;
        }
        normalize(&mut chat);
        out.push(chat);
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.updated));
    Ok(out)
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
    save_chat(root, &mut chat)?;
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
        tool: None,
        payload: None,
        details: None,
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
    details: Option<String>,
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
    stored.details = details;
    let enriched = stored.clone();
    save_chat(root, chat)?;
    Ok(enriched)
}

/// Persist a tool's own answer (JAPP-010D-B0): the frozen result snapshot
/// of a command. The author stays the initiating member (audit: WHO ran
/// it); rendering keys on the kind and never shows it as a person's
/// message. `text` is the plain-text fallback for renderers that do not
/// understand the payload.
#[allow(clippy::too_many_arguments)]
pub fn append_tool_result(
    root: &Path,
    chat: &mut Chat,
    author: MemberRef,
    tool: impl Into<String>,
    payload: impl Into<String>,
    text: impl Into<String>,
    now: DateTime<Utc>,
    id: Option<String>,
) -> Result<ChatMessage, JoyError> {
    if chat.read_only {
        return Err(JoyError::GuardDenied(format!(
            "chat {} was deleted for everyone and is read-only",
            chat.id
        )));
    }
    let message = append_kind_with_id(root, chat, author, text, MessageKind::Tool, now, id)?;
    let stored = chat
        .messages
        .iter_mut()
        .find(|m| m.id == message.id)
        .expect("just appended");
    stored.tool = Some(tool.into());
    stored.payload = Some(payload.into());
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
    save_chat(root, &mut chat)?;
    Ok(chat)
}

/// The chat's effective participants: General AND team chats carry an
/// empty list that means "every project member" (a team chat belongs to
/// the team — new members see it automatically) — resolve it for
/// display and turn logic. Direct chats list their members explicitly.
pub fn effective_participants(
    root: &Path,
    chat: &Chat,
) -> Result<Vec<joy_core::member_ref::MemberRef>, JoyError> {
    if matches!(chat.kind, ChatKind::General | ChatKind::Team) && chat.participants.is_empty() {
        let project = joy_core::store::load_project(root)?;
        return Ok(project
            .members()
            .map(|(key, _)| joy_core::member_ref::MemberRef::new(key.clone()))
            .collect());
    }
    Ok(chat.participants.clone())
}

/// Whether a member sees this chat in their list: team and General
/// chats are visible to EVERY project member (the team owns them);
/// direct chats only to their participants. Deleting for yourself hides
/// any chat until an @mention pulls you back (see
/// [`readd_mentioned_humans`]).
pub fn visible_to(chat: &Chat, member: &joy_core::member_ref::MemberRef) -> bool {
    if chat.deleted_for.iter().any(|m| m.id() == member.id()) {
        return false;
    }
    match chat.kind {
        ChatKind::General | ChatKind::Team => true,
        ChatKind::Direct => chat.participants.iter().any(|p| p.id() == member.id()),
    }
}

/// An @mention pulls a human back: it clears their delete-for-me mark
/// (team chats) and re-adds them to a direct chat's participants. AI
/// mentions are handled by `chat_turns::add_mentioned_ais` (joy-ai).
pub fn readd_mentioned_humans(
    root: &Path,
    chat: &mut Chat,
    text: &str,
    by: &joy_core::member_ref::MemberRef,
    now: DateTime<Utc>,
) -> Result<bool, JoyError> {
    if chat.read_only {
        return Ok(false);
    }
    let project = joy_core::store::load_project(root)?;
    let humans: Vec<String> = project
        .members()
        .map(|(key, _)| key.clone())
        .filter(|key| !key.starts_with("ai:"))
        .collect();
    let mentioned: Vec<String> = joy_chat::mentions::mentions(text, &humans)
        .into_iter()
        .cloned()
        .collect();
    let mut changed = false;
    for member in mentioned {
        let before = chat.deleted_for.len();
        chat.deleted_for.retain(|m| m.id() != member);
        if chat.deleted_for.len() != before {
            changed = true;
        }
        if chat.kind == ChatKind::Direct && !chat.participants.iter().any(|p| p.id() == member) {
            add_participant(
                root,
                chat,
                joy_core::member_ref::MemberRef::new(member),
                by,
                now,
            )?;
            changed = true;
        }
    }
    if changed {
        chat.updated = now;
        save_chat(root, chat)?;
    }
    Ok(changed)
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

/// Advance `member`'s sealed read watermark to `now` (the "read up to
/// here" marker held IN the chat, ADR JAPP-002A-30). No-op that skips the
/// save when the marker would not move forward. Reading is not activity,
/// so `updated` is untouched and the chat never bumps up the recency list.
pub fn mark_read(
    root: &Path,
    chat: &mut Chat,
    member: &MemberRef,
    now: DateTime<Utc>,
) -> Result<(), JoyError> {
    let advances = chat
        .read_markers
        .get(member.id())
        .is_none_or(|w| now.timestamp_millis() > w.timestamp_millis());
    if !advances {
        return Ok(());
    }
    chat.read_markers.insert(member.id().to_string(), now);
    save_chat(root, chat)
}

/// Rename a chat (General keeps its identity).
pub fn rename(root: &Path, chat: &mut Chat, title: impl Into<String>) -> Result<(), JoyError> {
    guard_not_general(chat, "renamed")?;
    chat.title = Some(title.into());
    // `updated` is MESSAGE activity (the recency sort key): renaming must
    // not push a chat up the list (operator 2026-07-18)
    save_chat(root, chat)
}

/// Set the subtitle (General keeps its identity). Like [`rename`], not
/// an activity bump.
pub fn set_subtitle(
    root: &Path,
    chat: &mut Chat,
    subtitle: impl Into<String>,
) -> Result<(), JoyError> {
    guard_not_general(chat, "changed")?;
    chat.subtitle = Some(subtitle.into());
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
    append_notice(
        root,
        chat,
        by.clone(),
        format!("@{} deleted this chat", by.id()),
        now,
    )?;
    chat.read_only = true;
    // the deleter never sees the chat again (operator rule): deleting for
    // everyone deletes for me too; the OTHERS keep it read-only until
    // each removed it for themselves
    if !chat.deleted_for.iter().any(|m| m.id() == by.id()) {
        chat.deleted_for.push(by.clone());
    }
    chat.updated = now;
    save_chat(root, chat)?;
    collect_if_everyone_deleted(root, chat);
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
    if !chat.deleted_for.iter().any(|m| m.id() == member.id()) {
        // the others learn about it, like "deleted this chat" for
        // everyone-deletes (operator rule); frozen chats take no writes
        if !chat.read_only {
            append_notice(
                root,
                chat,
                member.clone(),
                format!("@{} left this chat", member.id()),
                now,
            )?;
        }
        chat.deleted_for.push(member.clone());
        chat.updated = now;
        save_chat(root, chat)?;
    }
    collect_if_everyone_deleted(root, chat);
    Ok(())
}

/// Garbage collection: once every HUMAN of the chat's EFFECTIVE
/// membership deleted it for themselves, the file leaves the repo. The
/// effective view matters — a team chat's empty participant list means
/// the whole project (the raw list made all() vacuously true and the
/// FIRST delete-for-me removed the file for everyone). An unreadable
/// project only skips the check (conservative: the file stays).
fn collect_if_everyone_deleted(root: &Path, chat: &Chat) {
    let Ok(members) = effective_participants(root, chat) else {
        return;
    };
    let humans: Vec<_> = members
        .iter()
        .filter(|p| !p.id().starts_with("ai:"))
        .collect();
    let everyone_done = !humans.is_empty()
        && humans
            .iter()
            .all(|p| chat.deleted_for.iter().any(|m| m.id() == p.id()));
    if everyone_done {
        let _ = crate::chat_ref::remove_chat(root, &chat.id);
    }
}

/// Set (`Some`) or clear (`None`) the interaction level that
/// `delegator`'s turns of `agent` run under in this chat (ADR
/// JAPP-00F3-E8 as revised by JI-0166-D8 §5). The override is the
/// delegator's PRIVATE working-style preference — it binds only their
/// own turns, so no notice is posted and `updated` does not move (the
/// rail must not resort and nothing becomes unread over someone else's
/// setting; JOY-0229-B3). Governance transparency lives at the turn
/// itself: every AI reply carries its effective level in the header.
///
/// Refused on a frozen (delete-for-all) chat, like [`append_message`]:
/// a level change is chat CONFIGURATION and a dead chat takes no
/// configuration. Setting the value that is already stored is a
/// successful no-op without a write.
pub fn set_interaction_level(
    root: &Path,
    chat: &mut Chat,
    member: &MemberRef,
    delegator: &MemberRef,
    level: Option<InteractionLevel>,
) -> Result<(), JoyError> {
    if chat.read_only {
        return Err(JoyError::GuardDenied(format!(
            "chat {} was deleted for everyone and is read-only",
            chat.id
        )));
    }
    if chat.interaction_level_override(member.id(), delegator.id()) == level {
        return Ok(());
    }
    match level {
        Some(level) => {
            chat.interaction_levels
                .entry(member.id().to_string())
                .or_default()
                .insert(delegator.id().to_string(), level);
        }
        None => {
            if let Some(per_delegator) = chat.interaction_levels.get_mut(member.id()) {
                per_delegator.remove(delegator.id());
                if per_delegator.is_empty() {
                    chat.interaction_levels.remove(member.id());
                }
            }
        }
    }
    save_chat(root, chat)
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

    /// A tempdir that is a real git repo. Chats live on `refs/joy/chats`
    /// now (ADR JAPP-00DC-FC), so persistence needs a repo, not just a
    /// directory.
    fn repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        git2::Repository::init(dir.path()).unwrap();
        dir
    }

    #[test]
    fn chat_roundtrip_append_and_ai_session() {
        let dir = repo();
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
        let dir = repo();
        let g = ensure_general(dir.path(), ts(0)).unwrap();
        assert_eq!(g.id, GENERAL_CHAT_ID);
        assert_eq!(g.subtitle.as_deref(), Some("for all team members"));
        // idempotent
        let again = ensure_general(dir.path(), ts(1)).unwrap();
        assert_eq!(again.created, g.created);

        let mut g = again;
        let horst = MemberRef::new("horst@example.com");
        assert!(leave(dir.path(), &mut g, &horst, ts(2)).is_err());
        assert!(rename(dir.path(), &mut g, "X").is_err());
        // deleting IS allowed since 2026-07 (operator): for-me hides it per
        // member, for-all freezes it; ensure_general recreates only after GC
        delete_for_me(dir.path(), &mut g, &horst, ts(2)).unwrap();
        assert!(g.deleted_for.iter().any(|m| m.id() == horst.id()));
        delete_for_all(dir.path(), &mut g, &horst, ts(3)).unwrap();
        assert!(g.read_only);
    }

    #[test]
    fn team_chats_are_visible_to_everyone_until_self_deleted() {
        let dir = repo();
        let horst = MemberRef::new("horst@example.com");
        let geordi = MemberRef::new("geordi@example.org");
        // a team chat: empty participants = the whole team, dynamically
        let mut team = open_chat(dir.path(), Vec::new(), Some("Warp".into()), ts(0)).unwrap();
        assert!(visible_to(&team, &horst));
        assert!(visible_to(&team, &geordi));
        delete_for_me(dir.path(), &mut team, &geordi, ts(1)).unwrap();
        assert!(visible_to(&team, &horst));
        assert!(!visible_to(&team, &geordi));
        // a direct chat is participants-only
        let direct = {
            let mut c = open_chat(dir.path(), vec![horst.clone()], None, ts(2)).unwrap();
            c.kind = ChatKind::Direct;
            save_chat(dir.path(), &mut c).unwrap();
            c
        };
        assert!(visible_to(&direct, &horst));
        assert!(!visible_to(&direct, &geordi));
    }

    #[test]
    fn delete_semantics_deleter_vanishes_others_keep_read_only() {
        let dir = repo();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        let mut project = joy_core::model::Project::new("T".to_string(), Some("T".to_string()));
        for member in ["horst@example.com", "geordi@example.org", "ai:claude@joy"] {
            project
                .register_member(
                    member,
                    joy_core::model::project::Member::new(
                        joy_core::model::project::MemberCapabilities::All,
                    ),
                )
                .unwrap();
        }
        joy_core::store::write_yaml(
            &joy_core::store::joy_dir(dir.path()).join(joy_core::store::PROJECT_FILE),
            &project,
        )
        .unwrap();
        let horst = MemberRef::new("horst@example.com");
        let geordi = MemberRef::new("geordi@example.org");

        // a TEAM chat (empty list = everyone)
        let mut team = open_chat(dir.path(), Vec::new(), Some("Warp".into()), ts(0)).unwrap();

        // delete for me posts the "left" notice and hides only me
        delete_for_me(dir.path(), &mut team, &geordi, ts(1)).unwrap();
        assert!(team
            .messages
            .iter()
            .any(|m| m.text.contains("left this chat")));
        assert!(!visible_to(&team, &geordi));
        assert!(visible_to(&team, &horst));
        // the file survives: horst has not deleted yet (the raw empty
        // participant list once made the FIRST delete remove the file)
        assert!(load_chat(dir.path(), &team.id).unwrap().is_some());

        // delete for everyone: freezes AND vanishes for the deleter
        delete_for_all(dir.path(), &mut team, &horst, ts(2)).unwrap();
        assert!(team.read_only);
        assert!(!visible_to(&team, &horst));
        assert!(team
            .messages
            .iter()
            .any(|m| m.text.contains("deleted this chat")));

        // geordi already deleted for himself; horst is marked by the
        // for-all: every human done -> the file is gone
        delete_for_me(dir.path(), &mut team, &horst, ts(3)).unwrap();
        assert!(load_chat(dir.path(), &team.id).unwrap().is_none());
    }

    #[test]
    fn mention_pulls_a_self_deleted_human_back() {
        let dir = repo();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        let mut project = joy_core::model::Project::new("T".to_string(), Some("T".to_string()));
        for member in ["horst@example.com", "geordi@example.org"] {
            project
                .register_member(
                    member,
                    joy_core::model::project::Member::new(
                        joy_core::model::project::MemberCapabilities::All,
                    ),
                )
                .unwrap();
        }
        joy_core::store::write_yaml(
            &joy_core::store::joy_dir(dir.path()).join(joy_core::store::PROJECT_FILE),
            &project,
        )
        .unwrap();
        let horst = MemberRef::new("horst@example.com");
        let geordi = MemberRef::new("geordi@example.org");
        let mut team = open_chat(dir.path(), Vec::new(), Some("Warp".into()), ts(0)).unwrap();
        delete_for_me(dir.path(), &mut team, &geordi, ts(1)).unwrap();
        assert!(!visible_to(&team, &geordi));
        let changed = readd_mentioned_humans(
            dir.path(),
            &mut team,
            "@geordi@example.org schau mal",
            &horst,
            ts(2),
        )
        .unwrap();
        assert!(changed);
        assert!(visible_to(&team, &geordi));
    }

    #[test]
    fn leave_readd_and_delete_semantics() {
        let dir = repo();
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
        save_chat(dir.path(), &mut chat).unwrap();

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
    fn set_interaction_level_is_silent_persistent_and_idempotent() {
        let dir = repo();
        let horst = MemberRef::new("horst@example.com");
        let claude = MemberRef::new("ai:claude@joy");
        let mut chat =
            open_chat(dir.path(), vec![horst.clone(), claude.clone()], None, ts(0)).unwrap();

        set_interaction_level(
            dir.path(),
            &mut chat,
            &claude,
            &horst,
            Some(InteractionLevel::Confirmed),
        )
        .unwrap();
        assert_eq!(
            chat.interaction_level_override("ai:claude@joy", "horst@example.com"),
            Some(InteractionLevel::Confirmed)
        );
        // A private preference: NO notice, no message, no updated bump
        // (JOY-0229-B3 / JI-0166-D8 §5 revision).
        assert!(chat.messages.is_empty());
        assert_eq!(chat.updated, ts(0));

        // setting the SAME value again stays a no-op
        set_interaction_level(
            dir.path(),
            &mut chat,
            &claude,
            &horst,
            Some(InteractionLevel::Confirmed),
        )
        .unwrap();
        assert!(chat.messages.is_empty());

        // the override round-trips through the ref
        let loaded = load_chat(dir.path(), &chat.id).unwrap().unwrap();
        assert_eq!(loaded, chat);

        // clearing removes the entry and prunes the empty inner map,
        // still without a message
        set_interaction_level(dir.path(), &mut chat, &claude, &horst, None).unwrap();
        assert!(chat.interaction_levels.is_empty());
        assert!(chat.messages.is_empty());

        // clearing what is not stored is a silent no-op
        set_interaction_level(dir.path(), &mut chat, &claude, &horst, None).unwrap();
        assert!(chat.messages.is_empty());
    }

    #[test]
    fn set_interaction_level_keeps_other_delegators_and_refuses_frozen_chats() {
        let dir = repo();
        let horst = MemberRef::new("horst@example.com");
        let geordi = MemberRef::new("geordi@example.org");
        let claude = MemberRef::new("ai:claude@joy");
        let mut chat = open_chat(
            dir.path(),
            vec![horst.clone(), geordi.clone(), claude.clone()],
            None,
            ts(0),
        )
        .unwrap();

        set_interaction_level(
            dir.path(),
            &mut chat,
            &claude,
            &horst,
            Some(InteractionLevel::Autonomous),
        )
        .unwrap();
        set_interaction_level(
            dir.path(),
            &mut chat,
            &claude,
            &geordi,
            Some(InteractionLevel::Proposing),
        )
        .unwrap();
        // clearing one delegator's override leaves the other's intact
        set_interaction_level(dir.path(), &mut chat, &claude, &horst, None).unwrap();
        assert_eq!(
            chat.interaction_level_override("ai:claude@joy", "horst@example.com"),
            None
        );
        assert_eq!(
            chat.interaction_level_override("ai:claude@joy", "geordi@example.org"),
            Some(InteractionLevel::Proposing)
        );

        // a frozen (delete-for-all) chat takes no level changes
        delete_for_all(dir.path(), &mut chat, &horst, ts(4)).unwrap();
        let denied = set_interaction_level(
            dir.path(),
            &mut chat,
            &claude,
            &geordi,
            Some(InteractionLevel::Autonomous),
        );
        assert!(denied.is_err());
    }

    #[test]
    fn rename_and_subtitle() {
        let dir = repo();
        let horst = MemberRef::new("horst@example.com");
        let mut chat = open_chat(dir.path(), vec![horst], None, ts(0)).unwrap();
        rename(dir.path(), &mut chat, "Sprint 13").unwrap();
        set_subtitle(dir.path(), &mut chat, "countdown").unwrap();
        let loaded = load_chat(dir.path(), &chat.id).unwrap().unwrap();
        assert_eq!(loaded.title.as_deref(), Some("Sprint 13"));
        assert_eq!(loaded.subtitle.as_deref(), Some("countdown"));
        // renaming is NOT activity: the recency sort key stays put
        // (operator 2026-07-18)
        assert_eq!(loaded.updated, ts(0));
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 5, 3, 0, 0).unwrap()
    }

    /// A tempdir that is a real git repo (chats live on a ref now).
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        dir
    }

    #[test]
    fn append_mints_a_uuid_and_is_idempotent_per_id() {
        let dir = repo();
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
        // A message stored without an id (pre-channel) gets a
        // deterministic synthetic id on load and the timeline stays
        // time-ordered on every client.
        let dir = repo();
        let mut chat = open_chat(dir.path(), vec![MemberRef::new("a@x")], None, now()).unwrap();
        let mk = |sec: u32, text: &str| ChatMessage {
            id: String::new(),
            at: format!("2026-07-05T02:00:{sec:02}Z").parse().unwrap(),
            author: MemberRef::new("a@x"),
            text: text.into(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
        };
        chat.messages.push(mk(2, "second"));
        chat.messages.push(mk(1, "first"));
        save_chat(dir.path(), &mut chat).unwrap();

        let chat = load_chat(dir.path(), &chat.id).unwrap().unwrap();
        assert_eq!(chat.messages[0].text, "first");
        assert!(chat.messages[0].id.starts_with("legacy-"));
        // deterministic: a second load derives the SAME ids
        let chat2 = load_chat(dir.path(), &chat.id).unwrap().unwrap();
        assert_eq!(chat.messages[0].id, chat2.messages[0].id);
    }

    #[test]
    fn divergent_appends_merge_without_duplicates() {
        // the joy-yaml driver unions messages by id — simulate the merge
        let base = "id: c\ntitle: T\ncreated_by: a@x\ncreated: 2026-07-05T02:00:00Z\nupdated: 2026-07-05T02:00:00Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T02:00:01Z\n  author: a@x\n  text: hello\n";
        let ours = "id: c\ntitle: T\ncreated_by: a@x\ncreated: 2026-07-05T02:00:00Z\nupdated: 2026-07-05T02:00:02Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T02:00:01Z\n  author: a@x\n  text: hello\n- id: m2\n  at: 2026-07-05T02:00:02Z\n  author: a@x\n  text: ours\n";
        let theirs = "id: c\ntitle: T\ncreated_by: a@x\ncreated: 2026-07-05T02:00:00Z\nupdated: 2026-07-05T02:00:03Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T02:00:01Z\n  author: a@x\n  text: hello\n- id: m3\n  at: 2026-07-05T02:00:03Z\n  author: b@x\n  text: theirs\n";
        let merged = joy_core::merge::merge_yaml_doc(base, ours, theirs).unwrap();
        let chat: joy_chat::model::chat::Chat = serde_yaml_ng::from_str(&merged).unwrap();
        let ids: Vec<&str> = chat.messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"m2") && ids.contains(&"m3"));
    }

    #[test]
    fn error_kind_round_trips() {
        let dir = repo();
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
