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
/// stands answerless after a reload (operator rule, 2026-07-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageKind {
    #[default]
    Text,
    Notice,
    Error,
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
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
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
            messages: Vec::new(),
        }
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
}
