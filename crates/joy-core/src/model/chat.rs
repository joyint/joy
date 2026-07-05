// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The chat model (git-native, JOY-01F1). A chat is `.joy/chats/<id>.yaml`:
//! its participants (member refs), the ACP session ids of participating AI
//! members (so an AI's thread survives restarts), and the messages. The
//! repo is the source of truth; any real-time delivery layer (platform
//! pub/sub) is an optimization, never the data home.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::member_ref::MemberRef;

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
            messages: Vec::new(),
        }
    }
}
