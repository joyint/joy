// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Sealing chat events into content-addressed blobs (ADR JAPP-002A-30).
//!
//! Each [`ChatEvent`](crate::chat_events::ChatEvent) is sealed as its own
//! Crypt blob under its epoch's content key, in the SAME on-disk format
//! [`crate::crypt::decrypt_blob`] reads, so `joy crypt open` yields raw
//! event YAML. The blob's AAD zone is `chat:<cid>#<epoch_id>`, binding an
//! event to its chat and epoch (it cannot be replayed elsewhere). The
//! nonce is DERIVED from the plaintext, so re-sealing an unchanged event
//! is byte-identical: the log is content-addressed by
//! [`rid`] (`sha256(blob)`), a re-save adds no git objects, and a keyless
//! union dedups automatically.
//!
//! This module is the crypto seam between [`crate::chat_events`] (the
//! semantic fold) and [`crate::chat_ref`] (git storage). It holds no git
//! and no membership logic — just seal / open / address.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::chat_events::ChatEvent;
use crate::chat_wrap::ContentKey;
use crate::crypt::{self, ZoneKey};
use crate::error::JoyError;

/// The subtree that holds the anonymous key slots.
pub const KEYS_DIR: &str = "keys";
/// The subtree that holds the sealed event log.
pub const LOG_DIR: &str = "log";

/// The AAD zone binding an event to its chat and epoch.
fn event_zone(cid: &str, epoch_id: &str) -> String {
    format!("chat:{cid}#{epoch_id}")
}

/// The epoch id carried in an event blob's zone name, if it is one.
fn epoch_of_zone(zone: &str) -> Option<&str> {
    zone.strip_prefix("chat:")
        .and_then(|rest| rest.rsplit_once('#'))
        .map(|(_, epoch)| epoch)
}

/// A 12-byte nonce derived from the sealed content, so the same event
/// always seals to the same bytes (content addressing / idempotent
/// re-save). Distinct events differ in their stamp/content, hence nonce.
fn det_nonce(zone: &str, plaintext: &[u8]) -> [u8; 12] {
    let mut h = Sha256::new();
    h.update(b"joychat-nonce:");
    h.update(zone.as_bytes());
    h.update([0u8]);
    h.update(plaintext);
    let digest = h.finalize();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&digest[..12]);
    nonce
}

/// Seal one event under `epoch_id`'s content key. The blob is in the
/// standard Crypt format, so `joy crypt open` decrypts it to raw YAML.
pub fn seal_event(
    cid: &str,
    epoch_id: &str,
    ck: &ContentKey,
    event: &ChatEvent,
) -> Result<Vec<u8>, JoyError> {
    let yaml = serde_yaml_ng::to_string(event)?;
    let zone = event_zone(cid, epoch_id);
    let nonce = det_nonce(&zone, yaml.as_bytes());
    Ok(crypt::encrypt_blob_with_nonce(
        &zone,
        &ZoneKey::from_bytes(*ck),
        &nonce,
        yaml.as_bytes(),
    ))
}

/// The content-addressed id (filename) of a sealed blob: `sha256[..16]`
/// hex = 32 chars. Identical blobs dedup; distinct ones never collide.
pub fn rid(blob: &[u8]) -> String {
    hex::encode(&Sha256::digest(blob)[..16])
}

/// Open a sealed event blob given the epoch content keys the reader
/// holds. `None` when no held key opens it (foreign epoch, tampered),
/// which the caller treats as a still-sealed event, never an error.
pub fn open_event(blob: &[u8], keys: &BTreeMap<String, ContentKey>) -> Option<ChatEvent> {
    let (_, plain) = crypt::decrypt_blob(
        |zone| {
            epoch_of_zone(zone)
                .and_then(|epoch| keys.get(epoch))
                .map(|ck| ZoneKey::from_bytes(*ck))
        },
        blob,
    )
    .ok()?;
    serde_yaml_ng::from_slice(&plain).ok()
}

/// Open every sealed event this reader can, folding foreign ones out.
/// Order-independent: the fold that consumes them is a CRDT.
pub fn open_events<'a>(
    blobs: impl IntoIterator<Item = &'a [u8]>,
    keys: &BTreeMap<String, ContentKey>,
) -> Vec<ChatEvent> {
    blobs
        .into_iter()
        .filter_map(|b| open_event(b, keys))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_events::{diff, fold};
    use crate::chat_wrap::{anon_wrap_slot, new_content_key, new_epoch_id, resolve_epoch_keys};
    use crate::member_ref::MemberRef;
    use crate::model::chat::{Chat, ChatKind, ChatMessage, MessageKind};
    use chrono::{DateTime, Utc};

    fn ts(s: u32) -> DateTime<Utc> {
        format!("2026-07-19T00:00:{s:02}Z").parse().unwrap()
    }
    fn msg(id: &str, s: u32, author: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            at: ts(s),
            author: MemberRef::new(author),
            text: text.into(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
            enc: None,
            epoch: None,
        }
    }
    fn kp(seed: u8) -> crate::auth::IdentityKeypair {
        crate::auth::IdentityKeypair::from_seed(&[seed; 32])
    }

    /// The FULL sealed-storage path minus git: a chat becomes sealed
    /// events + anonymous key slots, and a participant reconstructs it
    /// exactly; a non-participant reconstructs nothing.
    #[test]
    fn a_chat_seals_and_a_participant_reconstructs_it_exactly() {
        let cid = "0123456789abcdef0123456789abcdef";
        let epoch = new_epoch_id();
        let ck = new_content_key();
        let horst = kp(1);
        let stranger = kp(2);

        // build a rich chat
        let base = Chat::new(cid, Vec::new(), ts(0));
        let mut chat = base.clone();
        chat.title = Some("Standup".into());
        chat.kind = ChatKind::Team;
        chat.participants = vec![MemberRef::new("horst@example.com")];
        chat.messages
            .push(msg("m1", 1, "horst@example.com", "secret plan"));
        chat.messages
            .push(msg("m2", 2, "horst@example.com", "part two"));

        // seal every event under the epoch CK
        let events = diff(&base, &chat, "wtag");
        let blobs: Vec<Vec<u8>> = events
            .iter()
            .map(|e| seal_event(cid, &epoch, &ck, e).unwrap())
            .collect();

        // NO plaintext leaks in the sealed bytes
        for b in &blobs {
            let hay = String::from_utf8_lossy(b);
            assert!(!hay.contains("Standup"));
            assert!(!hay.contains("secret plan"));
            assert!(!hay.contains("horst@example.com"));
        }

        // the key slot for horst
        let slot = anon_wrap_slot(cid, &epoch, &ck, &horst.public_key()).unwrap();

        // horst resolves the CK from the slot, opens every event, folds
        let keys = resolve_epoch_keys(
            cid,
            &horst.to_x25519_secret_bytes(),
            std::iter::once(&slot[..]),
        );
        let opened = open_events(blobs.iter().map(|b| b.as_slice()), &keys);
        let rebuilt = fold(cid, ts(0), &opened);
        assert_eq!(rebuilt.title.as_deref(), Some("Standup"));
        assert_eq!(rebuilt.messages.len(), 2);
        assert_eq!(rebuilt.messages[0].text, "secret plan");
        assert!(rebuilt
            .participants
            .iter()
            .any(|p| p.id() == "horst@example.com"));

        // a stranger resolves no key, opens nothing, folds an empty chat
        let none = resolve_epoch_keys(
            cid,
            &stranger.to_x25519_secret_bytes(),
            std::iter::once(&slot[..]),
        );
        let opened_none = open_events(blobs.iter().map(|b| b.as_slice()), &none);
        assert!(opened_none.is_empty());
        let empty = fold(cid, ts(0), &opened_none);
        assert!(empty.title.is_none());
        assert!(empty.messages.is_empty());
    }

    /// Re-sealing the same event is byte-identical: content addressing is
    /// stable, so a re-save adds no objects and union dedups.
    #[test]
    fn sealing_is_deterministic_for_content_addressing() {
        let cid = "cafecafecafecafecafecafecafecafe";
        let epoch = new_epoch_id();
        let ck = new_content_key();
        let base = Chat::new(cid, Vec::new(), ts(0));
        let mut chat = base.clone();
        chat.title = Some("T".into());
        let events = diff(&base, &chat, "w");
        let a = seal_event(cid, &epoch, &ck, &events[0]).unwrap();
        let b = seal_event(cid, &epoch, &ck, &events[0]).unwrap();
        assert_eq!(a, b, "same event -> same bytes");
        assert_eq!(rid(&a), rid(&b));
    }
}
