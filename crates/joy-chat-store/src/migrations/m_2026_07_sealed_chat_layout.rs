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
//! The conversion is the ordinary save: [`crate::chat_store::save`] drops
//! the legacy subtree and writes the sealed one in the same commit, so
//! this migration only has to find the chats and hand them over.

use std::path::Path;

use joy_core::error::JoyError;

use super::{Applied, Pending};

const WHAT: &str = "seal the chat into keys/ + log/";

/// The legacy chats, found without a key: the old reader only ever
/// answers for a `meta.yaml` subtree, and a sealed chat has none.
fn legacy(root: &Path) -> Result<Vec<String>, JoyError> {
    Ok(crate::chat_ref::load_chats(root)?
        .into_iter()
        .map(|c| c.id)
        .collect())
}

pub(super) fn pending(root: &Path) -> Result<Vec<Pending>, JoyError> {
    Ok(legacy(root)?
        .into_iter()
        .map(|chat_id| Pending {
            chat_id,
            what: WHAT,
        })
        .collect())
}

pub(super) fn apply(root: &Path, seed: &[u8; 32]) -> Result<Vec<Applied>, JoyError> {
    let mut done = Vec::new();
    for id in legacy(root)? {
        // Read it the way every reader does, so the migration converts
        // exactly what a person sees today and never a half-read chat.
        let Some(chat) = crate::chats::load_chat(root, &id)? else {
            continue;
        };
        crate::chat_store::save(root, &chat, seed)?;
        done.push(Applied {
            chat_id: id,
            what: WHAT,
        });
    }
    Ok(done)
}
