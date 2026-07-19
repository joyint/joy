// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The chat model (git-native, JOY-01F1). A chat is `.joy/chats/<id>.yaml`:
//! its participants (member refs), the ACP session ids of participating AI
//! members (so an AI's thread survives restarts), per-delegator agent
//! permission mode overrides (ADR JAPP-00F3-E8), and the messages. The
//! repo is the source of truth; any real-time delivery layer (platform
//! pub/sub) is an optimization, never the data home.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::member_ref::MemberRef;
use crate::model::agent_mode::AgentMode;

/// What kind of chat this is (JOY-01F4). `General` is the singleton
/// team-wide chat (fixed id `general`, participants = every project
/// member, never deletable or leavable); `Team` is a member-created group;
/// `Direct` a 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChatKind {
    General,
    #[default]
    Team,
    Direct,
}

/// Message kinds: `Text` is a member message; `Notice` is a system line
/// ("@xy left", "@xy was added"), rendered centered and muted; `Error`
/// persists a failed command's answer (the yellow box) so a call never
/// stands answerless after a reload (operator rule, 2026-07-05); `Tool`
/// is a tool's own persisted answer (JAPP-010D-B0): the frozen result
/// snapshot of a command, never attributed to a person on render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageKind {
    #[default]
    Text,
    Notice,
    Error,
    Tool,
}

/// One message in a chat. The author is a member ref and resolves for
/// display via the no-raw-ID rule.
///
/// `id` is the channel's exact identity (ADR JAPP-00C9): the CLIENT mints
/// it (UUIDv7) the moment the message is written, it persists in the
/// file, and every delivery path dedupes on it — no timestamp
/// heuristics. Messages from before the channel load without one and get
/// a deterministic synthetic id (same on every client).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub at: DateTime<Utc>,
    pub author: MemberRef,
    pub text: String,
    #[serde(default, skip_serializing_if = "is_default_kind")]
    pub kind: MessageKind,
    /// AI replies: the human whose delegation this turn ran under —
    /// PERSISTED so the attribution survives reloads (operator rule).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_by: Option<String>,
    /// AI replies: wall time of the turn in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_ms: Option<u32>,
    /// AI replies: number of tool steps the turn took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_steps: Option<u32>,
    /// Tool messages (JAPP-010D-B0): which tool answered, e.g. "/joy".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Tool messages: the frozen result snapshot, an opaque versioned
    /// JSON string minted by the sending client at completion time. The
    /// view NEVER reconstructs a result from the live store (a later
    /// reconstruction shows different content and breaks the discussion
    /// that referred to it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// AI replies: the turn's activity details (thinking, tool steps,
    /// answered permissions) as an opaque versioned JSON string — the
    /// collapsed block must survive a reload like the text (operator
    /// decision 2026-07-16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// ADR JAPP-002A-30: the encrypted content envelope, hex
    /// `nonce(12) || AES-256-GCM(ct)` over the JSON of the sensitive
    /// fields (text, payload, details). When set, those fields are
    /// EMPTY at rest and [`crate::chat_crypt`] restores them in memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc: Option<String>,
    /// The key epoch `enc` was sealed under (index into
    /// [`ChatCrypt::epochs`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u32>,
}

impl ChatMessage {
    /// The deterministic fallback id for pre-channel messages: every
    /// client derives the SAME id from the message's content.
    pub fn synthetic_id(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.at.to_rfc3339().as_bytes());
        hasher.update(b"|");
        hasher.update(self.author.id().as_bytes());
        hasher.update(b"|");
        hasher.update(self.text.as_bytes());
        let digest = hasher.finalize();
        format!("legacy-{}", hex::encode(&digest[..12]))
    }
}

fn is_default_kind(kind: &MessageKind) -> bool {
    *kind == MessageKind::Text
}

fn is_default_chat_kind(kind: &ChatKind) -> bool {
    *kind == ChatKind::Team
}

/// A persistent chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    /// Short, stable id (also the file stem).
    pub id: String,
    #[serde(default, skip_serializing_if = "is_default_chat_kind")]
    pub kind: ChatKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<MemberRef>,
    /// Set by delete-for-all: the chat stays readable but frozen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// Members who locally dismissed a delete-for-all chat; once it covers
    /// every participant the file is garbage-collected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_for: Vec<MemberRef>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    /// Everyone in the chat (humans and AI members).
    pub participants: Vec<MemberRef>,
    /// ACP session id per participating AI member (keyed by member id),
    /// so the AI-side conversation thread survives restarts.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai_sessions: BTreeMap<String, String>,
    /// Per-delegator agent permission mode overrides (ADR JAPP-00F3-E8):
    /// outer key = AI participant member id, inner key = delegating
    /// member id (at-rest form), value = the mode that delegator's turns
    /// run under in THIS chat. Nested maps rather than a composite key so
    /// the YAML merge unions entries per delegator. Plain `String` keys
    /// on purpose: `MemberRef`'s Serialize is presentation-aware (see
    /// [`ai_sessions`](Self::ai_sessions)). Resolve a turn's actual mode
    /// with [`crate::model::agent_mode::effective_mode`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<String, BTreeMap<String, AgentMode>>,
    /// ADR JAPP-002A-30: the participant-wrapped content-key header.
    /// None only for a legacy plaintext chat (migrated on next persist)
    /// or a chat that was never persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crypt: Option<ChatCrypt>,
    /// Per-member read watermark (ADR JAPP-002A-30, sealed read markers):
    /// member id -> the instant up to which they have read. Held IN the
    /// chat (a sealed `Read` event per advance), never a server-side DB, so
    /// a desktop clone carries its own markers. Seeded to a member's join
    /// instant so pre-join history is not "unread". A member's EFFECTIVE
    /// watermark also advances to their own last authored message (you have
    /// read what you wrote); use [`Chat::effective_watermark`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub read_markers: BTreeMap<String, DateTime<Utc>>,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

/// The chat's content-key header (ADR JAPP-002A-30): one AES-256-GCM
/// key per epoch, wrapped X25519-pairwise for every participant who can
/// hold one. Lives IN the chat object, never in project.yaml; there is
/// no chat Crypt zone. Removing a participant appends a new epoch
/// (rotation forward): past messages stay under their old epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ChatCrypt {
    /// Key epochs, oldest first; the LAST is the active one.
    pub epochs: Vec<ChatKeyEpoch>,
}

/// One content-key epoch: recipient id (member id or the reserved
/// "platform" custodian) -> hex wrap (granter verify_key prefixed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ChatKeyEpoch {
    pub wraps: std::collections::BTreeMap<String, String>,
}

impl Chat {
    pub fn new(id: impl Into<String>, participants: Vec<MemberRef>, now: DateTime<Utc>) -> Self {
        Self {
            id: id.into(),
            kind: ChatKind::Team,
            title: None,
            subtitle: None,
            created_by: None,
            read_only: false,
            deleted_for: Vec::new(),
            created: now,
            updated: now,
            participants,
            ai_sessions: BTreeMap::new(),
            modes: BTreeMap::new(),
            crypt: None,
            read_markers: BTreeMap::new(),
            messages: Vec::new(),
        }
    }

    /// The instant up to which `member` has read: the max of their sealed
    /// read watermark and their own last authored message (you have read
    /// what you wrote). `None` if they have neither read nor written.
    pub fn effective_watermark(&self, member: &str) -> Option<DateTime<Utc>> {
        let explicit = self.read_markers.get(member).copied();
        let authored = self
            .messages
            .iter()
            .filter(|m| m.author.id() == member)
            .map(|m| m.at)
            .max();
        explicit.into_iter().chain(authored).max()
    }

    /// The participant member ids (humans AND AI) who have read `msg`:
    /// their effective watermark is at or past the message instant.
    pub fn read_by(&self, msg: &ChatMessage) -> Vec<String> {
        self.participants
            .iter()
            .map(|p| p.id().to_string())
            .filter(|id| self.effective_watermark(id).is_some_and(|w| w >= msg.at))
            .collect()
    }

    /// How many messages `member` has not yet read: those strictly after
    /// their effective watermark. Pre-join history does not count because
    /// joining seeds the watermark to the join instant.
    pub fn unread_count(&self, member: &str) -> usize {
        let watermark = self.effective_watermark(member);
        self.messages
            .iter()
            .filter(|m| watermark.is_none_or(|w| m.at > w))
            .count()
    }

    /// The stored per-chat mode override for the (`agent`, `delegator`)
    /// member-id pair, if any. Feed the result to
    /// [`crate::model::agent_mode::effective_mode`].
    pub fn mode_override(&self, agent: &str, delegator: &str) -> Option<AgentMode> {
        self.modes
            .get(agent)
            .and_then(|per_delegator| per_delegator.get(delegator))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_map_round_trips_and_is_skipped_when_empty() {
        let now = "2026-07-11T00:00:00Z".parse().unwrap();
        let mut chat = Chat::new("c", vec![MemberRef::new("horst@example.com")], now);
        let yaml = serde_yaml_ng::to_string(&chat).unwrap();
        assert!(!yaml.contains("modes"), "empty map must not serialize");

        chat.modes
            .entry("ai:claude@joy".to_string())
            .or_default()
            .insert("horst@example.com".to_string(), AgentMode::AcceptEdits);
        let yaml = serde_yaml_ng::to_string(&chat).unwrap();
        assert!(yaml.contains("modes:"));
        assert!(yaml.contains("accept-edits"), "kebab-case on the wire");

        let back: Chat = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(back, chat);
        assert_eq!(
            back.mode_override("ai:claude@joy", "horst@example.com"),
            Some(AgentMode::AcceptEdits)
        );
        assert_eq!(
            back.mode_override("ai:claude@joy", "geordi@example.org"),
            None
        );
        assert_eq!(
            back.mode_override("ai:copilot@joy", "horst@example.com"),
            None
        );
    }

    #[test]
    fn read_by_and_unread_use_watermark_and_authorship() {
        let now: DateTime<Utc> = "2026-07-11T00:00:00Z".parse().unwrap();
        let mut chat = Chat::new(
            "c",
            vec![
                MemberRef::new("a@e"),
                MemberRef::new("b@e"),
                MemberRef::new("ai:v@joy"),
            ],
            now,
        );
        let mk = |sec: u32, who: &str| ChatMessage {
            id: format!("m{sec}"),
            at: format!("2026-07-11T00:00:0{sec}Z").parse().unwrap(),
            author: MemberRef::new(who),
            text: "x".into(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
            enc: None,
            epoch: None,
        };
        chat.messages = vec![mk(1, "a@e"), mk(2, "b@e"), mk(3, "ai:v@joy")];
        // b@e has an explicit read marker up to t2; a@e and the AI only have
        // authorship (a authored t1, the AI authored the last at t3).
        chat.read_markers.insert("b@e".into(), chat.messages[1].at);

        // effective watermark = max(explicit, own last authored)
        assert_eq!(chat.effective_watermark("a@e"), Some(chat.messages[0].at));
        assert_eq!(chat.effective_watermark("b@e"), Some(chat.messages[1].at));
        assert_eq!(
            chat.effective_watermark("ai:v@joy"),
            Some(chat.messages[2].at)
        );

        // only the AI (author of the last, t3) has read the latest message
        let last = chat.messages.last().unwrap().clone();
        assert_eq!(chat.read_by(&last), vec!["ai:v@joy".to_string()]);

        // unread = messages strictly after the effective watermark
        assert_eq!(chat.unread_count("a@e"), 2); // m2, m3
        assert_eq!(chat.unread_count("b@e"), 1); // m3
        assert_eq!(chat.unread_count("ai:v@joy"), 0);
    }
}
