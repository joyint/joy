// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The parity script (JI-0179-4F step 5): ONE scripted conversation and
//! ONE normalized expectation for what the persisted chat must look like
//! after the engine ran it, whichever host ran it.
//!
//! The engine is shared, so a parity break can only come from a HOST:
//! its append transaction, its capability answers, its record fields.
//! Each host therefore gets a runner against this one module: the
//! desktop's runs always (app repo, cargo test over the engine harness);
//! the platform's beats live in the gated web e2e (chat.e2e.ts), which
//! drives the same conversation against the running stack. Reviews alone
//! demonstrably did not hold the line; this file is the machine's copy
//! of the rule.

use joy_chat::model::chat::{Chat, MessageKind};

/// The human lines of the scripted conversation, in order. They exercise
/// the turn classes that diverged historically: an @mention (pulls the
/// AI in, first turn), a plain follow-up (the messenger convention), and
/// a slash line (a TOOL address: never a turn).
pub const SCRIPT: &[&str] = &[
    "@claude welches modell nutzt du?",
    "danke, und wie spät ist es?",
    "/joy ls",
];

/// The reply the scripted agent gives per turn (deterministic, so the
/// golden can name it). Runners wire their agent to answer exactly this.
pub const REPLIES: &[&str] = &["Opus 5", "kurz nach Mitternacht"];

/// The activity every scripted turn produces: one thought and one
/// completed tool call. The platform's mock agent emits exactly this on
/// every prompt; the desktop runner injects the same.
///
/// It is part of the script because activity is part of the RECORD
/// (JAPP-014D-6E): the desktop persisted none of it for weeks while the
/// platform persisted it correctly, and a golden that only looked at the
/// text could not see the difference.
pub fn activity() -> crate::activity::Activity {
    crate::activity::Activity {
        thoughts: "reading the backlog".into(),
        tools: vec![("joy ls".into(), "completed".into())],
        permissions: Vec::new(),
    }
}

/// One normalized row per persisted message: what parity is ABOUT, and
/// nothing that legitimately differs (ids, timestamps, budget fields,
/// and WHO the human is: every human author normalizes to `human`).
///
/// Shape: `kind|author|text` plus, for AI replies, `|by=<set?>` (the
/// attribution exists), `|level=<set?>` (the execution record carries the
/// interaction level) and `|act=<thoughts?>,<n>tools` (the activity block
/// survived into the record).
pub fn normalize(chat: &Chat) -> Vec<String> {
    chat.messages
        .iter()
        .map(|m| {
            let kind = match m.kind {
                MessageKind::Text => "text",
                MessageKind::Notice => "notice",
                MessageKind::Error => "error",
                MessageKind::Tool => "tool",
            };
            let author = if m.author.id().starts_with("ai:") {
                m.author.id().to_string()
            } else {
                "human".to_string()
            };
            let mut row = format!("{kind}|{author}|{}", m.text);
            if m.author.id().starts_with("ai:") && m.kind == MessageKind::Text {
                row.push_str(if m.delegated_by.is_some() {
                    "|by=set"
                } else {
                    "|by=unset"
                });
                let details = m
                    .details
                    .as_deref()
                    .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok());
                let has_level = details
                    .as_ref()
                    .map(|v| v.get("interactionLevel").is_some())
                    .unwrap_or(false);
                row.push_str(if has_level {
                    "|level=set"
                } else {
                    "|level=unset"
                });
                // What the turn DID, as the thread will render it. A host
                // that streams its activity but persists none lands on
                // `-,0tools` here and fails the golden.
                let thoughts = details
                    .as_ref()
                    .and_then(|v| v.get("thoughts"))
                    .and_then(|t| t.as_str())
                    .map(|t| !t.trim().is_empty())
                    .unwrap_or(false);
                let tools = details
                    .as_ref()
                    .and_then(|v| v.get("tools"))
                    .and_then(|t| t.as_array())
                    .map(Vec::len)
                    .unwrap_or(0);
                row.push_str(&format!(
                    "|act={},{tools}tools",
                    if thoughts { "t" } else { "-" }
                ));
            }
            row
        })
        .collect()
}

/// The expected sequence after the engine ran [`SCRIPT`] in a chat that
/// held only the human, with `ai:claude@joy` a registered project member.
/// The slash line persists (it is a message) but triggers NO turn.
pub fn golden() -> Vec<String> {
    vec![
        format!("text|human|{}", SCRIPT[0]),
        // the ENGINE pulls the mentioned AI in; the add-notice is
        // authored by the human who mentioned it
        "notice|human|@ai:claude@joy was added".to_string(),
        format!(
            "text|ai:claude@joy|{}|by=set|level=set|act=t,1tools",
            REPLIES[0]
        ),
        format!("text|human|{}", SCRIPT[1]),
        format!(
            "text|ai:claude@joy|{}|by=set|level=set|act=t,1tools",
            REPLIES[1]
        ),
        format!("text|human|{}", SCRIPT[2]),
    ]
}
