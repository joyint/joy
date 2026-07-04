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

/// One message in a chat. The author is a member ref and resolves for
/// display via the no-raw-ID rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub at: DateTime<Utc>,
    pub author: MemberRef,
    pub text: String,
}

/// A persistent chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    /// Short, stable id (also the file stem).
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
            title: None,
            created: now,
            updated: now,
            participants,
            ai_sessions: BTreeMap::new(),
            messages: Vec::new(),
        }
    }
}
