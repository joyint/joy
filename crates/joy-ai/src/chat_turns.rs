// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The one storage-bound part of the turn rules: pulling a mentioned AI
//! into the chat writes to the project, so it lives here and not in the
//! pure rules.
//!
//! Everything else a turn decides (who is addressed, what the prompt
//! says, what a reply may contain) is in `joy_chat::turns`, because the
//! app has to answer the same questions in the webview, where there is
//! no storage at all (JI-0179-4F).

use joy_chat::model::chat::{Chat, ChatMessage, MessageKind};

pub use joy_chat::turns::*;

/// Same question as the pure rules ask, one line, no import dance.
fn is_ai(id: &str) -> bool {
    id.starts_with("ai:")
}

/// A human's LEADING @mention of a PROJECT AI that is not a participant
/// yet adds it to the chat (the messenger convention, and the reason a
/// fresh personal chat can talk to @claude at all). Returns whether anyone
/// was added; General needs no adds (everyone is in it).
///
/// Leading, like [`decide`]: an @name further in the sentence refers to
/// someone, it does not pull them into the room.
pub fn add_mentioned_ais(
    root: &std::path::Path,
    chat: &mut Chat,
    newest: &ChatMessage,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, joy_core::error::JoyError> {
    if matches!(
        chat.kind,
        joy_chat::model::chat::ChatKind::General | joy_chat::model::chat::ChatKind::Team
    ) && chat.participants.is_empty()
        || chat.read_only
        || newest.kind == MessageKind::Notice
        || is_ai(newest.author.id())
    {
        // empty team/General lists mean "everyone is already here"
        return Ok(false);
    }
    let project = joy_core::store::load_project(root)?;
    let project_ais: Vec<String> = project
        .members()
        .map(|(key, _)| key.clone())
        .filter(|key| is_ai(key))
        .collect();
    let mentioned: Vec<String> = leading_mentions(&newest.text, &project_ais)
        .into_iter()
        .cloned()
        .collect();
    let mut added = false;
    for member in mentioned {
        if !chat.participants.iter().any(|p| p.id() == member) {
            joy_chat_store::chats::add_participant(
                root,
                chat,
                joy_core::member_ref::MemberRef::new(member),
                &newest.author,
                now,
            )?;
            added = true;
        }
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use joy_core::member_ref::MemberRef;

    #[test]
    fn human_mentions_add_project_ais_to_the_chat() {
        let dir = tempfile::tempdir().unwrap();
        // chats live on refs/joy/chats now, so persistence needs a repo
        git2::Repository::init(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        // a real project with one AI member
        let mut project = joy_core::model::Project::new("T".to_string(), Some("T".to_string()));
        // chats are always sealed now, so the members carry identities and
        // the writer's seed is installed for this thread
        for (member, seed_byte) in [("ai:claude@joy", 7u8), ("horst@example.com", 5u8)] {
            let mut m = joy_core::model::project::Member::new(
                joy_core::model::project::MemberCapabilities::All,
            );
            m.verify_key = Some(
                joy_core::auth::IdentityKeypair::from_seed(&[seed_byte; 32])
                    .public_key()
                    .to_hex(),
            );
            project.register_member(member, m).unwrap();
        }
        joy_chat_store::writer::set_thread_seed(Some(Some([5u8; 32])));
        joy_core::store::write_yaml(
            &joy_core::store::joy_dir(dir.path()).join(joy_core::store::PROJECT_FILE),
            &project,
        )
        .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 7, 4, 19, 0, 0).unwrap();
        let mut chat = joy_chat_store::chats::open_chat(
            dir.path(),
            vec![MemberRef::new("horst@example.com")],
            Some("New chat".into()),
            now,
        )
        .unwrap();
        let msg = joy_chat_store::chats::append_message(
            dir.path(),
            &mut chat,
            MemberRef::new("horst@example.com"),
            "@claude how many items?",
            now,
        )
        .unwrap();
        let added = add_mentioned_ais(dir.path(), &mut chat, &msg, now).unwrap();
        assert!(added);
        assert!(chat.participants.iter().any(|p| p.id() == "ai:claude@joy"));
        assert_eq!(
            decide(&chat, &msg, "ai:claude@joy"),
            TurnDecision::Respond,
            "the added AI answers the very message that added it"
        );
        // idempotent
        assert!(!add_mentioned_ais(dir.path(), &mut chat, &msg, now).unwrap());
    }
}
