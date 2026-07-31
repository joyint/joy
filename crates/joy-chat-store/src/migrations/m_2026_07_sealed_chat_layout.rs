// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Legacy `<cid>/meta.yaml + messages/` becomes the sealed `keys/ + log/`
//! (JAPP-0135-FD / JOY-021D-F4).
//!
//! A chat written before the sealed event log is still readable by this
//! CLI, but the transport the apps use serves only the sealed shape, so an
//! unmigrated chat is simply absent in both apps. It also still lies in
//! the clear on the forge, which is the other half of why this must not
//! wait for someone to happen to write into that chat.
//!
//! Nobody needs a key for this. Sealing wraps the epoch key for the
//! MEMBERS' public keys; the private half is only ever needed to READ.
//! So whoever holds the repository can convert a chat and still not be
//! able to open it afterwards, the platform included. That is what lets
//! this run by itself when a new version arrives.

use std::path::Path;

use joy_core::error::JoyError;

use super::{Applied, Pending, Skipped};

const WHAT: &str = "seal the chat into keys/ + log/";
const CANNOT: &str =
    "written with the per-message encryption this build no longer reads; left untouched";

/// The legacy chats, found without a key: the old reader only ever
/// answers for a `meta.yaml` subtree, and a sealed chat has none.
fn legacy(root: &Path) -> Result<Vec<joy_chat::model::chat::Chat>, JoyError> {
    crate::chat_ref::load_chats(root)
}

/// Whether a legacy chat can be converted at all.
///
/// Between JOY-0218-E8 and the platform-key removal, messages carried
/// their ciphertext in an `enc` field that today's model does not know.
/// Serde drops it, so such a message reads as empty text, and sealing it
/// would replace a chat nobody can read with an EMPTY chat everybody can
/// read. That is worse than leaving it alone, so it is left alone and
/// named.
fn readable(chat: &joy_chat::model::chat::Chat) -> bool {
    !chat.messages.iter().any(|m| m.text.is_empty())
}

/// The writer tag a migration seals under.
///
/// `seal` derives a short tag from a seed to mark WHO wrote, and needs
/// nothing else from it: recipients are wrapped with their PUBLIC keys.
/// A migration is nobody in particular, so it writes under a fixed tag
/// that says exactly that, and stays as unable to read the result as it
/// was before. Nothing is protected by this value.
const MIGRATION_WRITER: [u8; 32] = [0x6d; 32];

pub(super) fn pending(root: &Path) -> Result<Vec<Pending>, JoyError> {
    Ok(legacy(root)?
        .into_iter()
        .map(|chat| Pending {
            chat_id: chat.id,
            what: WHAT,
        })
        .collect())
}

pub(super) fn apply(root: &Path) -> Result<(Vec<Applied>, Vec<Skipped>), JoyError> {
    let mut done = Vec::new();
    let mut skipped = Vec::new();
    for chat in legacy(root)? {
        if !readable(&chat) {
            skipped.push(Skipped {
                chat_id: chat.id,
                why: CANNOT.to_string(),
            });
            continue;
        }
        let id = chat.id.clone();
        match crate::chat_store::save(root, &chat, &MIGRATION_WRITER) {
            Ok(()) => done.push(Applied {
                chat_id: id,
                what: WHAT,
            }),
            // A project with no member key at all cannot be sealed for
            // anyone; the chat stays as it is rather than half-converted.
            Err(e) => skipped.push(Skipped {
                chat_id: id,
                why: e.to_string(),
            }),
        }
    }
    Ok((done, skipped))
}
