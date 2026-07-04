// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Turn rules for AI members in chats (JOY-01F9). One implementation for
//! every host (desktop, platform): an AI answers only when @mentioned;
//! since the last human message each AI may post at most
//! [`MAX_AI_TURNS_SINCE_HUMAN`] messages — an address beyond that yields
//! the moderation notice instead of another AI turn. Whether the SENDER
//! is allowed to address the AI (local adapter installed, API key set) is
//! host-specific and checked by the caller.

use crate::model::chat::{Chat, ChatMessage, MessageKind};

/// How many messages one AI may post since the last human message before
/// a human has to moderate on ("ask, then react to the answer").
pub const MAX_AI_TURNS_SINCE_HUMAN: usize = 2;

/// The system line posted when the chain guard trips (notice format).
pub const MODERATION_NOTICE: &str = "the AIs paused — a human has to moderate on";

/// What a host should do for one AI member after a new message landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnDecision {
    /// Run one agent turn and append its reply.
    Respond,
    /// Not addressed (or a notice): stay quiet.
    Silent,
    /// Addressed, but the chain guard tripped: post [`MODERATION_NOTICE`]
    /// (once) instead of a turn.
    NeedsModeration,
}

fn is_ai(id: &str) -> bool {
    id.starts_with("ai:")
}

/// The short alias an AI is @mentioned by: `ai:claude@joy` -> `claude`.
pub fn alias(member_id: &str) -> &str {
    member_id
        .strip_prefix("ai:")
        .and_then(|rest| rest.split('@').next())
        .unwrap_or(member_id)
}

/// Member ids among `candidates` that `text` @mentions, by full ref or by
/// short alias.
pub fn mentions<'a>(text: &str, candidates: &'a [String]) -> Vec<&'a String> {
    let tokens: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '!' || c == '?')
        .filter_map(|w| w.strip_prefix('@'))
        .map(|w| w.trim_end_matches(['.', ':', ')']))
        .filter(|w| !w.is_empty())
        .collect();
    candidates
        .iter()
        .filter(|candidate| {
            tokens
                .iter()
                .any(|t| *t == candidate.as_str() || *t == alias(candidate))
        })
        .collect()
}

/// Decide what `ai_member` should do about the newest message.
pub fn decide(chat: &Chat, newest: &ChatMessage, ai_member: &str) -> TurnDecision {
    if newest.kind == MessageKind::Notice || newest.author.id() == ai_member {
        return TurnDecision::Silent;
    }
    let participant_ids: Vec<String> = chat
        .participants
        .iter()
        .map(|p| p.id().to_string())
        .collect();
    let addressed = mentions(&newest.text, &participant_ids)
        .iter()
        .any(|m| m.as_str() == ai_member);
    if !addressed {
        return TurnDecision::Silent;
    }
    // Humans reset the chain; an AI addressing an AI is bounded.
    if !is_ai(newest.author.id()) {
        return TurnDecision::Respond;
    }
    let turns_since_human = chat
        .messages
        .iter()
        .rev()
        .take_while(|m| is_ai(m.author.id()) || m.kind == MessageKind::Notice)
        .filter(|m| m.kind == MessageKind::Text && m.author.id() == ai_member)
        .count();
    if turns_since_human >= MAX_AI_TURNS_SINCE_HUMAN {
        TurnDecision::NeedsModeration
    } else {
        TurnDecision::Respond
    }
}

/// A human's @mention of a PROJECT AI that is not a participant yet adds
/// it to the chat (the messenger convention, and the reason a fresh
/// personal chat can talk to @claude at all). Returns whether anyone was
/// added; General needs no adds (everyone is in it).
pub fn add_mentioned_ais(
    root: &std::path::Path,
    chat: &mut Chat,
    newest: &ChatMessage,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, crate::error::JoyError> {
    if chat.kind == crate::model::chat::ChatKind::General
        || chat.read_only
        || newest.kind == MessageKind::Notice
        || is_ai(newest.author.id())
    {
        return Ok(false);
    }
    let project = crate::store::load_project(root)?;
    let project_ais: Vec<String> = project
        .members()
        .map(|(key, _)| key.clone())
        .filter(|key| is_ai(key))
        .collect();
    let mentioned: Vec<String> = mentions(&newest.text, &project_ais)
        .into_iter()
        .cloned()
        .collect();
    let mut added = false;
    for member in mentioned {
        if !chat.participants.iter().any(|p| p.id() == member) {
            crate::chats::add_participant(
                root,
                chat,
                crate::member_ref::MemberRef::new(member),
                &newest.author,
                now,
            )?;
            added = true;
        }
    }
    Ok(added)
}

/// Whether the moderation notice is already the newest notice (post once).
pub fn moderation_already_posted(chat: &Chat) -> bool {
    chat.messages
        .iter()
        .rev()
        .take_while(|m| is_ai(m.author.id()) || m.kind == MessageKind::Notice)
        .any(|m| m.kind == MessageKind::Notice && m.text == MODERATION_NOTICE)
}

/// The prompt for one agent turn: the attributed transcript plus the role
/// instruction. The chat itself IS the context — a fresh session with this
/// prompt has everything either AI has said.
pub fn context_prompt(chat: &Chat, ai_member: &str) -> String {
    let title = chat.title.as_deref().unwrap_or("Team chat");
    let mut prompt = format!(
        "You are {ai_member}, a member of the team chat \"{title}\".\n\
         Below is the full conversation. Reply with your next chat message\n\
         only — no preamble, no markdown headings. Address teammates with\n\
         @name when you need one of them; a human moderates the room.\n\n\
         --- conversation ---\n"
    );
    for message in &chat.messages {
        if message.kind == MessageKind::Notice {
            prompt.push_str(&format!("[notice] {}\n", message.text));
        } else {
            prompt.push_str(&format!("{}: {}\n", message.author.id(), message.text));
        }
    }
    prompt.push_str("--- end ---\n");
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::member_ref::MemberRef;
    use chrono::{TimeZone, Utc};

    fn chat_with(messages: Vec<(&str, &str, MessageKind)>) -> Chat {
        let now = Utc.with_ymd_and_hms(2026, 7, 4, 12, 0, 0).unwrap();
        let mut chat = Chat::new(
            "t1",
            vec![
                MemberRef::new("horst@example.com"),
                MemberRef::new("ai:claude@joy"),
                MemberRef::new("ai:vibe@joy"),
            ],
            now,
        );
        chat.messages = messages
            .into_iter()
            .map(|(author, text, kind)| ChatMessage {
                at: now,
                author: MemberRef::new(author),
                text: text.into(),
                kind,
            })
            .collect();
        chat
    }

    #[test]
    fn mentions_match_alias_and_full_ref() {
        let candidates = vec![
            "ai:claude@joy".to_string(),
            "geordi@example.org".to_string(),
        ];
        let found = mentions("@claude please ask @geordi@example.org.", &candidates);
        assert_eq!(found.len(), 2);
        assert!(mentions("no at all", &candidates).is_empty());
        assert!(mentions("mail me claude@joy", &candidates).is_empty());
    }

    #[test]
    fn only_mentioned_ai_responds() {
        let chat = chat_with(vec![(
            "horst@example.com",
            "@claude check this",
            MessageKind::Text,
        )]);
        let newest = chat.messages.last().unwrap();
        assert_eq!(
            decide(&chat, newest, "ai:claude@joy"),
            TurnDecision::Respond
        );
        assert_eq!(decide(&chat, newest, "ai:vibe@joy"), TurnDecision::Silent);
    }

    #[test]
    fn ai_chain_is_bounded_and_human_resets() {
        // claude asked vibe, vibe answered, claude reacted, vibe answered:
        // each has 2 turns since the human — the next AI address must
        // moderate, a human message resets.
        let chat = chat_with(vec![
            (
                "horst@example.com",
                "@claude kick it off",
                MessageKind::Text,
            ),
            (
                "ai:claude@joy",
                "@vibe what is the state?",
                MessageKind::Text,
            ),
            ("ai:vibe@joy", "@claude two tests fail", MessageKind::Text),
            ("ai:claude@joy", "@vibe which ones?", MessageKind::Text),
            ("ai:vibe@joy", "@claude the auth pair", MessageKind::Text),
        ]);
        let newest = chat.messages.last().unwrap();
        assert_eq!(
            decide(&chat, newest, "ai:claude@joy"),
            TurnDecision::NeedsModeration
        );
        // vibe addressed by claude at this point would also be over budget
        let chat2 = chat_with(vec![
            ("horst@example.com", "@claude go", MessageKind::Text),
            ("ai:claude@joy", "@vibe q1", MessageKind::Text),
            ("ai:vibe@joy", "@claude a1", MessageKind::Text),
            ("ai:claude@joy", "@vibe q2", MessageKind::Text),
        ]);
        let newest2 = chat2.messages.last().unwrap();
        assert_eq!(
            decide(&chat2, newest2, "ai:vibe@joy"),
            TurnDecision::Respond
        );
        // a human speaking resets the chain
        let chat3 = chat_with(vec![
            ("ai:claude@joy", "@vibe q", MessageKind::Text),
            ("ai:vibe@joy", "@claude a", MessageKind::Text),
            ("horst@example.com", "@claude summarize", MessageKind::Text),
        ]);
        let newest3 = chat3.messages.last().unwrap();
        assert_eq!(
            decide(&chat3, newest3, "ai:claude@joy"),
            TurnDecision::Respond
        );
    }

    #[test]
    fn human_mentions_add_project_ais_to_the_chat() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        // a real project with one AI member
        let mut project = crate::model::Project::new("T".to_string(), Some("T".to_string()));
        for member in ["ai:claude@joy", "horst@example.com"] {
            project
                .register_member(
                    member,
                    crate::model::project::Member::new(
                        crate::model::project::MemberCapabilities::All,
                    ),
                )
                .unwrap();
        }
        crate::store::write_yaml(
            &crate::store::joy_dir(dir.path()).join(crate::store::PROJECT_FILE),
            &project,
        )
        .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 7, 4, 19, 0, 0).unwrap();
        let mut chat = crate::chats::open_chat(
            dir.path(),
            vec![MemberRef::new("horst@example.com")],
            Some("New chat".into()),
            now,
        )
        .unwrap();
        let msg = crate::chats::append_message(
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

    #[test]
    fn moderation_notice_posts_once() {
        let mut chat = chat_with(vec![("ai:claude@joy", "@vibe q", MessageKind::Text)]);
        assert!(!moderation_already_posted(&chat));
        chat.messages.push(ChatMessage {
            at: chat.updated,
            author: MemberRef::new("ai:claude@joy"),
            text: MODERATION_NOTICE.into(),
            kind: MessageKind::Notice,
        });
        assert!(moderation_already_posted(&chat));
    }

    #[test]
    fn context_prompt_carries_the_whole_transcript() {
        let chat = chat_with(vec![
            ("horst@example.com", "hello", MessageKind::Text),
            ("ai:vibe@joy", "hi @claude", MessageKind::Text),
        ]);
        let prompt = context_prompt(&chat, "ai:claude@joy");
        assert!(prompt.contains("You are ai:claude@joy"));
        assert!(prompt.contains("horst@example.com: hello"));
        assert!(prompt.contains("ai:vibe@joy: hi @claude"));
    }
}
