// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! Per-chat content encryption (ADR JAPP-002A-30).
//!
//! Every persisted chat is encrypted; no plaintext chat is ever written
//! to disk. Each chat owns an AES-256-GCM content key that seals the
//! sensitive message fields (text, payload, details). The key is
//! wrapped X25519-pairwise (the existing Crypt primitive) for every
//! participant with a verify_key, plus the reserved "platform"
//! custodian, which persists chats and acts session-scoped for AI
//! participants. Wraps live in the chat object itself; there is no chat
//! Crypt zone and project.yaml stays untouched.
//!
//! Participant changes happen at that one header: adding someone is one
//! new wrap in the active epoch; removing someone appends a NEW epoch
//! (rotation forward). Past messages stay under their old epoch, which
//! the removed party can still open; new messages are sealed under the
//! new key they no longer hold. An identity is the precondition for
//! persistence: without a seed to grant wraps, a chat stays ephemeral.

use std::collections::BTreeMap;

use rand::RngCore;

use crate::auth::IdentityKeypair;
use crate::error::JoyError;
use crate::model::chat::{Chat, ChatCrypt, ChatKeyEpoch, ChatMessage};
use crate::model::project::{Project, PLATFORM_RECIPIENT};

/// The wrap context for one chat epoch; feeds the pairwise KEK info so
/// a wrap can never be replayed for another chat or epoch.
fn wrap_name(chat_id: &str, epoch: u32) -> String {
    format!("chat:{chat_id}#{epoch}")
}

/// AAD binding a sealed message to its chat, epoch and message id.
/// The FORMAT lives in joy-crypt (shared with the browser WASM).
fn aad_for(chat_id: &str, epoch: u32, msg_id: &str) -> Vec<u8> {
    joy_crypt::chat::aad(chat_id, epoch, msg_id)
}

/// The sensitive fields of a message, sealed as one JSON envelope.
#[derive(serde::Serialize, serde::Deserialize)]
struct Envelope {
    #[serde(default)]
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    details: Option<String>,
}

/// The active (last) epoch index, if the chat has a crypt header.
pub fn active_epoch(chat: &Chat) -> Option<u32> {
    let crypt = chat.crypt.as_ref()?;
    (!crypt.epochs.is_empty()).then(|| (crypt.epochs.len() - 1) as u32)
}

/// Everyone the content key is wrapped for: participants with a
/// verify_key on record, plus the platform custodian when the project
/// is registered. AI members rarely hold their own verify_key; the
/// platform wrap covers reading and writing FOR them, session-scoped
/// on the platform side (the container model's custodial pattern).
fn wrap_recipients(project: &Project, chat: &Chat) -> Vec<(String, crate::auth::PublicKey)> {
    let mut out = Vec::new();
    for id in effective_recipient_ids(project, chat) {
        if let Some(public) = recipient_public(project, &id) {
            out.push((id, public));
        }
    }
    if let Some(public) = recipient_public(project, PLATFORM_RECIPIENT) {
        out.push((PLATFORM_RECIPIENT.to_string(), public));
    }
    out
}

/// The member ids the key must be wrapped for: the stored participant
/// list, EXCEPT that an empty team/General list means "everyone in the
/// project" (the chats module's convention).
fn effective_recipient_ids(project: &Project, chat: &Chat) -> Vec<String> {
    use crate::model::chat::ChatKind;
    if chat.participants.is_empty() && matches!(chat.kind, ChatKind::General | ChatKind::Team) {
        project.members().map(|(id, _)| id.clone()).collect()
    } else {
        chat.participants
            .iter()
            .map(|p| p.id().to_string())
            .collect()
    }
}

/// The verify_key on record for a wrap recipient: a member's own key,
/// or the platform's registered key for the reserved custodian id.
fn recipient_public(project: &Project, id: &str) -> Option<crate::auth::PublicKey> {
    let hex = if id == PLATFORM_RECIPIENT {
        project.platform.as_ref().map(|p| p.verify_key.clone())
    } else {
        project.member_by_key(id).and_then(|m| m.verify_key.clone())
    }?;
    crate::auth::PublicKey::from_hex(&hex).ok()
}

/// Start encryption for a chat that has none yet: mint the content key
/// and wrap it for every recipient. The granter (whoever persists
/// first: the platform, or a person on desktop) contributes their seed.
/// Errors when NOBODY could be wrapped; the caller keeps the chat
/// ephemeral in that case.
pub fn ensure_crypt(
    project: &Project,
    chat: &mut Chat,
    granter_seed: &[u8; 32],
) -> Result<[u8; 32], JoyError> {
    if chat.crypt.is_some() {
        return open_any(chat, granter_seed)
            .ok_or_else(|| JoyError::AuthFailed("no chat wrap opens for this seed".into()));
    }
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let epoch = seal_epoch(project, chat, granter_seed, &key, 0)?;
    chat.crypt = Some(ChatCrypt {
        epochs: vec![epoch],
    });
    Ok(key)
}

/// Build one epoch's wrap map for the current recipients.
fn seal_epoch(
    project: &Project,
    chat: &Chat,
    granter_seed: &[u8; 32],
    key: &[u8; 32],
    epoch: u32,
) -> Result<ChatKeyEpoch, JoyError> {
    let recipients = wrap_recipients(project, chat);
    if recipients.is_empty() {
        return Err(JoyError::AuthFailed(
            "no participant with an identity key; chat stays ephemeral".into(),
        ));
    }
    let granter = IdentityKeypair::from_seed(granter_seed);
    let granter_pk = granter.public_key();
    let name = wrap_name(&chat.id, epoch);
    let zone_key = crate::crypt::ZoneKey::from_bytes(*key);
    let mut wraps = BTreeMap::new();
    for (id, public) in recipients {
        wraps.insert(
            id,
            crate::crypt::wrap_for_member(&zone_key, &name, granter_seed, &granter_pk, &public),
        );
    }
    Ok(ChatKeyEpoch { wraps })
}

/// Open the key of `epoch` with the wrap addressed to `recipient_id`.
pub fn open_key(
    chat: &Chat,
    recipient_id: &str,
    seed: &[u8; 32],
    epoch: u32,
) -> Result<[u8; 32], JoyError> {
    let crypt = chat
        .crypt
        .as_ref()
        .ok_or_else(|| JoyError::AuthFailed("chat has no crypt header".into()))?;
    let entry = crypt
        .epochs
        .get(epoch as usize)
        .and_then(|e| e.wraps.get(recipient_id))
        .ok_or_else(|| JoyError::AuthFailed(format!("no chat wrap for {recipient_id}")))?;
    // the SHARED unwrap (joy-crypt): byte-identical with the browser
    let secret = IdentityKeypair::from_seed(seed).to_x25519_secret_bytes();
    joy_crypt::chat::unwrap_content_key(&secret, entry, &chat.id, epoch)
        .map_err(|e| JoyError::AuthFailed(format!("chat wrap would not open: {e}")))
}

/// Open the key of `epoch` with whatever wrap this seed can unwrap
/// (the recipient id is not always known to the caller, e.g. the
/// platform custodian). None when no wrap opens.
pub fn open_epoch_any(chat: &Chat, seed: &[u8; 32], epoch: u32) -> Option<[u8; 32]> {
    let ids: Vec<String> = chat
        .crypt
        .as_ref()?
        .epochs
        .get(epoch as usize)?
        .wraps
        .keys()
        .cloned()
        .collect();
    ids.iter()
        .find_map(|id| open_key(chat, id, seed, epoch).ok())
}

/// [`open_epoch_any`] for the ACTIVE epoch.
pub fn open_any(chat: &Chat, seed: &[u8; 32]) -> Option<[u8; 32]> {
    open_epoch_any(chat, seed, active_epoch(chat)?)
}

/// The process-wide custodian seed (the platform's key, or the signed-in
/// person's identity seed on desktop): whoever holds it persists chats
/// sealed and reads them opened. The same pattern as the active zone
/// keys in [`crate::crypt`]. None (default) means no persistence surface
/// is authenticated: encrypted chats stay sealed and cannot be written.
static CUSTODIAN_SEED: std::sync::RwLock<Option<[u8; 32]>> = std::sync::RwLock::new(None);

pub fn set_custodian_seed(seed: Option<[u8; 32]>) {
    *CUSTODIAN_SEED.write().unwrap_or_else(|e| e.into_inner()) = seed;
}

pub fn custodian_seed() -> Option<[u8; 32]> {
    *CUSTODIAN_SEED.read().unwrap_or_else(|e| e.into_inner())
}

/// Open every epoch the custodian seed can unwrap, in place. No-op
/// without a custodian.
pub fn open_with_custodian(chat: &mut Chat) {
    let Some(seed) = custodian_seed() else {
        return;
    };
    let snapshot = chat.clone();
    open_messages(chat, |epoch| open_epoch_any(&snapshot, &seed, epoch));
}

/// Seal every message that is still plaintext under the ACTIVE key.
/// Call before every persist; also THE migration for legacy plaintext
/// chats (they simply have all messages pending).
pub fn seal_messages(chat: &mut Chat, key: &[u8; 32]) {
    let Some(epoch) = active_epoch(chat) else {
        return;
    };
    let chat_id = chat.id.clone();
    for message in &mut chat.messages {
        if message.enc.is_some() {
            continue;
        }
        let id = if message.id.is_empty() {
            message.synthetic_id()
        } else {
            message.id.clone()
        };
        let envelope = Envelope {
            text: std::mem::take(&mut message.text),
            payload: message.payload.take(),
            details: message.details.take(),
        };
        let plain = serde_json::to_vec(&envelope).expect("envelope serializes");
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = joy_crypt::aead::seal(key, &nonce, &aad_for(&chat_id, epoch, &id), &plain)
            .expect("AES-256-GCM seal with a 32-byte key never fails");
        let mut blob = Vec::with_capacity(12 + ct.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        message.enc = Some(hex::encode(blob));
        message.epoch = Some(epoch);
    }
}

/// Restore the sensitive fields of every sealed message the key lookup
/// can serve, IN MEMORY. `enc` and `epoch` stay set: a later persist
/// keeps each message under its ORIGINAL epoch (rotation forward must
/// not re-seal history under the new key) and
/// [`sealed_for_save`] strips the plaintext again. Messages whose epoch
/// yields no key stay locked (empty text, envelope intact).
pub fn open_messages(chat: &mut Chat, key_for_epoch: impl Fn(u32) -> Option<[u8; 32]>) {
    let chat_id = chat.id.clone();
    for message in &mut chat.messages {
        let (Some(enc), Some(epoch)) = (message.enc.clone(), message.epoch) else {
            continue;
        };
        let Some(key) = key_for_epoch(epoch) else {
            continue;
        };
        if let Some(envelope) = open_envelope(&chat_id, message, &enc, epoch, &key) {
            message.text = envelope.text;
            message.payload = envelope.payload;
            message.details = envelope.details;
        }
    }
}

/// The at-rest form of an opened chat: every sealed message carries ONLY
/// its envelope; the in-memory plaintext of [`open_messages`] is
/// stripped. THE guarantee that no plaintext chat reaches disk.
pub fn sealed_for_save(chat: &Chat) -> Chat {
    let mut sealed = chat.clone();
    for message in &mut sealed.messages {
        if message.enc.is_some() {
            message.text = String::new();
            message.payload = None;
            message.details = None;
        }
    }
    sealed
}

fn open_envelope(
    chat_id: &str,
    message: &ChatMessage,
    enc: &str,
    epoch: u32,
    key: &[u8; 32],
) -> Option<Envelope> {
    let blob = hex::decode(enc).ok()?;
    if blob.len() < 12 + 16 {
        return None;
    }
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&blob[..12]);
    let id = if message.id.is_empty() {
        message.synthetic_id()
    } else {
        message.id.clone()
    };
    let plain =
        joy_crypt::aead::open(key, &nonce, &aad_for(chat_id, epoch, &id), &blob[12..]).ok()?;
    serde_json::from_slice(&plain).ok()
}

/// The wrap upkeep every persist runs (ADR JAPP-002A-30, the two
/// participant rules in one place):
/// - someone LEFT (a wrap-holder is no longer a recipient): rotate the
///   key forward; past epochs stay readable to them, new messages not.
/// - someone ARRIVED or enrolled (a recipient has a key but no wrap,
///   e.g. General chats cover every project member, or a member set up
///   their identity after the chat existed): one new wrap, same key.
/// Returns the key new messages seal under.
pub fn maintain_wraps(
    project: &Project,
    chat: &mut Chat,
    granter_seed: &[u8; 32],
    key: [u8; 32],
) -> Result<[u8; 32], JoyError> {
    let Some(epoch) = active_epoch(chat) else {
        return Ok(key);
    };
    let recipients = wrap_recipients(project, chat);
    let recipient_ids: std::collections::BTreeSet<&str> =
        recipients.iter().map(|(id, _)| id.as_str()).collect();
    let holders: Vec<String> = chat.crypt.as_ref().map_or_else(Vec::new, |c| {
        c.epochs[epoch as usize].wraps.keys().cloned().collect()
    });
    if holders
        .iter()
        .any(|id| id != PLATFORM_RECIPIENT && !recipient_ids.contains(id.as_str()))
    {
        return rotate_for_removal(project, chat, granter_seed);
    }
    let granter = IdentityKeypair::from_seed(granter_seed);
    let granter_pk = granter.public_key();
    let name = wrap_name(&chat.id, epoch);
    let zone_key = crate::crypt::ZoneKey::from_bytes(key);
    if let Some(entry) = chat
        .crypt
        .as_mut()
        .and_then(|c| c.epochs.get_mut(epoch as usize))
    {
        for (id, public) in recipients {
            entry.wraps.entry(id).or_insert_with(|| {
                crate::crypt::wrap_for_member(&zone_key, &name, granter_seed, &granter_pk, &public)
            });
        }
    }
    Ok(key)
}

/// Whether a persist would change the active epoch's wraps (the startup
/// sweep uses this to re-save only chats that need it).
pub fn wraps_stale(project: &Project, chat: &Chat) -> bool {
    let Some(epoch) = active_epoch(chat) else {
        return false;
    };
    let Some(entry) = chat
        .crypt
        .as_ref()
        .and_then(|c| c.epochs.get(epoch as usize))
    else {
        return false;
    };
    let recipients = wrap_recipients(project, chat);
    let recipient_ids: std::collections::BTreeSet<&str> =
        recipients.iter().map(|(id, _)| id.as_str()).collect();
    let missing = recipients
        .iter()
        .any(|(id, _)| !entry.wraps.contains_key(id));
    let stale_holder = entry
        .wraps
        .keys()
        .any(|id| id != PLATFORM_RECIPIENT && !recipient_ids.contains(id.as_str()));
    missing || stale_holder
}

/// Add one wrap for a NEW participant to the active epoch (ADR: adding
/// a participant is one new wrap). The granter must hold the key.
pub fn add_participant_wrap(
    project: &Project,
    chat: &mut Chat,
    granter_seed: &[u8; 32],
    member_id: &str,
) -> Result<(), JoyError> {
    let epoch = active_epoch(chat)
        .ok_or_else(|| JoyError::AuthFailed("chat has no crypt header".into()))?;
    let key = open_any(chat, granter_seed)
        .ok_or_else(|| JoyError::AuthFailed("no chat wrap opens for this seed".into()))?;
    let public = recipient_public(project, member_id)
        .ok_or_else(|| JoyError::AuthFailed(format!("{member_id} has no identity key")))?;
    let granter = IdentityKeypair::from_seed(granter_seed);
    let name = wrap_name(&chat.id, epoch);
    let zone_key = crate::crypt::ZoneKey::from_bytes(key);
    let wrap = crate::crypt::wrap_for_member(
        &zone_key,
        &name,
        granter_seed,
        &granter.public_key(),
        &public,
    );
    if let Some(entry) = chat
        .crypt
        .as_mut()
        .and_then(|c| c.epochs.get_mut(epoch as usize))
    {
        entry.wraps.insert(member_id.to_string(), wrap);
    }
    Ok(())
}

/// Rotate the content key forward after a participant left (ADR:
/// removing rotates). A NEW epoch with a fresh key is wrapped for the
/// REMAINING recipients; past epochs stay untouched, so old messages
/// remain readable to the removed party — intrinsic to versioned
/// client-encrypted data.
pub fn rotate_for_removal(
    project: &Project,
    chat: &mut Chat,
    granter_seed: &[u8; 32],
) -> Result<[u8; 32], JoyError> {
    let next = chat
        .crypt
        .as_ref()
        .map(|c| c.epochs.len() as u32)
        .ok_or_else(|| JoyError::AuthFailed("chat has no crypt header".into()))?;
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let epoch = seal_epoch(project, chat, granter_seed, &key, next)?;
    if let Some(crypt) = chat.crypt.as_mut() {
        crypt.epochs.push(epoch);
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::member_ref::MemberRef;

    fn project_with_keys(tag: &str) -> (Project, [u8; 32], [u8; 32]) {
        let dir = std::env::temp_dir().join(format!("jc-chatcrypt-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        crate::init::init(crate::init::InitOptions {
            root: dir.clone(),
            name: Some("ChatCrypt".into()),
            acronym: Some("CC".into()),
            user: Some("horst@example.com".into()),
            language: None,
        })
        .unwrap();
        let mut project = crate::store::load_project(&dir).unwrap();
        let horst_seed = [1u8; 32];
        let platform_seed = [2u8; 32];
        project
            .member_by_key_mut("horst@example.com")
            .unwrap()
            .verify_key = Some(
            IdentityKeypair::from_seed(&horst_seed)
                .public_key()
                .to_hex(),
        );
        project.platform = Some(crate::model::project::PlatformInfo {
            verify_key: IdentityKeypair::from_seed(&platform_seed)
                .public_key()
                .to_hex(),
            registered: chrono::Utc::now(),
        });
        project
            .register_member(
                "ai:vibe@joy",
                crate::model::project::Member::new(crate::model::project::MemberCapabilities::All),
            )
            .unwrap();
        std::fs::remove_dir_all(&dir).ok();
        (project, horst_seed, platform_seed)
    }

    fn chat_with_message() -> Chat {
        let now = chrono::Utc::now();
        let mut chat = Chat::new(
            "c1",
            vec![
                MemberRef::new("horst@example.com"),
                MemberRef::new("ai:vibe@joy"),
            ],
            now,
        );
        chat.messages.push(ChatMessage {
            id: "m1".into(),
            at: now,
            author: MemberRef::new("horst@example.com"),
            text: "the secret plan".into(),
            kind: Default::default(),
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: Some("{\"v\":1}".into()),
            enc: None,
            epoch: None,
        });
        chat
    }

    #[test]
    fn seal_open_roundtrip_covers_every_participant_and_the_platform() {
        let (project, horst_seed, platform_seed) = project_with_keys("roundtrip");
        let mut chat = chat_with_message();
        let key = ensure_crypt(&project, &mut chat, &platform_seed).unwrap();
        // the AI member has no verify_key: covered by the platform wrap
        let wraps = &chat.crypt.as_ref().unwrap().epochs[0].wraps;
        assert!(wraps.contains_key("horst@example.com"));
        assert!(wraps.contains_key(PLATFORM_RECIPIENT));
        assert!(!wraps.contains_key("ai:vibe@joy"), "no key, no wrap");

        seal_messages(&mut chat, &key);
        assert_eq!(chat.messages[0].text, "", "no plaintext at rest");
        assert!(chat.messages[0].details.is_none());
        assert!(chat.messages[0].enc.is_some());

        // ...and NOTHING plaintext survives serialization
        let yaml = serde_yaml_ng::to_string(&chat).unwrap();
        assert!(!yaml.contains("secret plan"), "{yaml}");

        // the participant opens with their own wrap
        let horst_key = open_key(&chat, "horst@example.com", &horst_seed, 0).unwrap();
        let mut readable = chat.clone();
        open_messages(&mut readable, |_| Some(horst_key));
        assert_eq!(readable.messages[0].text, "the secret plan");
        assert_eq!(readable.messages[0].details.as_deref(), Some("{\"v\":1}"));

        // the platform opens via open_any (custodian for the AI session)
        assert!(open_any(&chat, &platform_seed).is_some());
        // a stranger seed opens nothing
        assert!(open_any(&chat, &[9u8; 32]).is_none());
    }

    #[test]
    fn removal_rotates_forward_and_the_new_epoch_excludes_nobody_by_accident() {
        let (mut project, horst_seed, platform_seed) = project_with_keys("rotate");
        // second person, so removal leaves someone besides the platform
        let anna_seed = [3u8; 32];
        let mut anna =
            crate::model::project::Member::new(crate::model::project::MemberCapabilities::All);
        anna.verify_key = Some(IdentityKeypair::from_seed(&anna_seed).public_key().to_hex());
        project.register_member("anna@example.com", anna).unwrap();
        let mut chat = chat_with_message();
        chat.participants.push(MemberRef::new("anna@example.com"));

        let key0 = ensure_crypt(&project, &mut chat, &platform_seed).unwrap();
        seal_messages(&mut chat, &key0);

        // anna leaves: rotate forward, then a new message under epoch 1
        chat.participants.retain(|p| p.id() != "anna@example.com");
        let key1 = rotate_for_removal(&project, &mut chat, &platform_seed).unwrap();
        chat.messages.push(ChatMessage {
            id: "m2".into(),
            at: chrono::Utc::now(),
            author: MemberRef::new("horst@example.com"),
            text: "after the rotation".into(),
            kind: Default::default(),
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
            enc: None,
            epoch: None,
        });
        seal_messages(&mut chat, &key1);

        // anna still opens epoch 0 (intrinsic), NOT epoch 1
        assert!(open_key(&chat, "anna@example.com", &anna_seed, 0).is_ok());
        assert!(open_key(&chat, "anna@example.com", &anna_seed, 1).is_err());

        // horst opens both epochs and reads everything
        let mut readable = chat.clone();
        let horst = |epoch: u32| open_key(&chat, "horst@example.com", &horst_seed, epoch).ok();
        open_messages(&mut readable, horst);
        assert_eq!(readable.messages[0].text, "the secret plan");
        assert_eq!(readable.messages[1].text, "after the rotation");

        // anna's view: epoch-0 message opens, the epoch-1 one stays sealed
        let mut annas = chat.clone();
        open_messages(&mut annas, |epoch| {
            open_key(&chat, "anna@example.com", &anna_seed, epoch).ok()
        });
        assert_eq!(annas.messages[0].text, "the secret plan");
        assert_eq!(annas.messages[1].text, "");
        assert!(annas.messages[1].enc.is_some());
    }

    #[test]
    fn adding_a_participant_is_one_new_wrap() {
        let (mut project, _horst, platform_seed) = project_with_keys("add");
        let mut chat = chat_with_message();
        let _ = ensure_crypt(&project, &mut chat, &platform_seed).unwrap();

        let ben_seed = [4u8; 32];
        let mut ben =
            crate::model::project::Member::new(crate::model::project::MemberCapabilities::All);
        ben.verify_key = Some(IdentityKeypair::from_seed(&ben_seed).public_key().to_hex());
        project.register_member("ben@example.com", ben).unwrap();
        chat.participants.push(MemberRef::new("ben@example.com"));

        add_participant_wrap(&project, &mut chat, &platform_seed, "ben@example.com").unwrap();
        assert!(open_key(&chat, "ben@example.com", &ben_seed, 0).is_ok());
    }

    #[test]
    fn the_custodian_persists_sealed_and_reads_opened() {
        // The platform flow end to end: custodian seed set, a project
        // with identities on disk, save through chats::save_chat seals
        // at rest, load opens back. Exactly the path that answered
        // "authentication failed, chat not persisted" when the platform
        // had not set its seed yet (operator 2026-07-18).
        let dir = std::env::temp_dir().join(format!("jc-custodian-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        crate::init::init(crate::init::InitOptions {
            root: dir.clone(),
            name: Some("Custodian".into()),
            acronym: Some("CU".into()),
            user: Some("horst@example.com".into()),
            language: None,
        })
        .unwrap();
        let platform_seed = [7u8; 32];
        let mut project = crate::store::load_project(&dir).unwrap();
        project.platform = Some(crate::model::project::PlatformInfo {
            verify_key: IdentityKeypair::from_seed(&platform_seed)
                .public_key()
                .to_hex(),
            registered: chrono::Utc::now(),
        });
        crate::store::write_yaml(
            &crate::store::joy_dir(&dir).join(crate::store::PROJECT_FILE),
            &project,
        )
        .unwrap();

        // without the custodian: an identity-bearing project refuses
        // plaintext persistence (the ADR's ephemeral rule)
        set_custodian_seed(None);
        let mut chat = chat_with_message();
        assert!(crate::chats::save_chat(&dir, &mut chat).is_err());

        set_custodian_seed(Some(platform_seed));
        crate::chats::save_chat(&dir, &mut chat).unwrap();
        // reflect what persistence added (the caller reloads in product
        // paths); at rest the text is sealed
        let raw = crate::chat_ref::load_chat(&dir, &chat.id).unwrap().unwrap();
        assert_eq!(raw.messages[0].text, "", "sealed at rest");
        assert!(raw.messages[0].enc.is_some());
        // ...and the custodian load opens it again
        let opened = crate::chats::load_chat(&dir, &chat.id).unwrap().unwrap();
        assert_eq!(opened.messages[0].text, "the secret plan");
        chat = opened;
        assert!(chat.crypt.is_some());
        set_custodian_seed(None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_general_chat_wraps_every_project_member() {
        let (mut project, horst_seed, platform_seed) = project_with_keys("general");
        let anna_seed = [3u8; 32];
        let mut anna =
            crate::model::project::Member::new(crate::model::project::MemberCapabilities::All);
        anna.verify_key = Some(IdentityKeypair::from_seed(&anna_seed).public_key().to_hex());
        project.register_member("anna@example.com", anna).unwrap();
        // General: EMPTY participant list means everyone
        let now = chrono::Utc::now();
        let mut chat = Chat::new("general", Vec::new(), now);
        chat.kind = crate::model::chat::ChatKind::General;
        let _ = ensure_crypt(&project, &mut chat, &platform_seed).unwrap();
        assert!(open_key(&chat, "horst@example.com", &horst_seed, 0).is_ok());
        assert!(open_key(&chat, "anna@example.com", &anna_seed, 0).is_ok());
    }

    #[test]
    fn late_enrollment_backfills_and_leaving_rotates() {
        let (mut project, _horst, platform_seed) = project_with_keys("upkeep");
        let mut chat = chat_with_message();
        chat.participants
            .push(crate::member_ref::MemberRef::new("ben@example.com"));
        // ben exists but has NO key yet: no wrap for him
        project
            .register_member(
                "ben@example.com",
                crate::model::project::Member::new(crate::model::project::MemberCapabilities::All),
            )
            .unwrap();
        let key = ensure_crypt(&project, &mut chat, &platform_seed).unwrap();
        assert!(!wraps_stale(&project, &chat), "nothing to do yet");

        // ben enrolls: the next upkeep backfills his wrap, SAME key
        let ben_seed = [4u8; 32];
        project
            .member_by_key_mut("ben@example.com")
            .unwrap()
            .verify_key = Some(IdentityKeypair::from_seed(&ben_seed).public_key().to_hex());
        assert!(wraps_stale(&project, &chat));
        let kept = maintain_wraps(&project, &mut chat, &platform_seed, key).unwrap();
        assert_eq!(kept, key);
        assert!(open_key(&chat, "ben@example.com", &ben_seed, 0).is_ok());

        // ben leaves: the upkeep rotates forward
        chat.participants.retain(|p| p.id() != "ben@example.com");
        assert!(wraps_stale(&project, &chat));
        let rotated = maintain_wraps(&project, &mut chat, &platform_seed, kept).unwrap();
        assert_ne!(rotated, kept);
        assert_eq!(active_epoch(&chat), Some(1));
        assert!(open_key(&chat, "ben@example.com", &ben_seed, 1).is_err());
        assert!(
            open_key(&chat, "ben@example.com", &ben_seed, 0).is_ok(),
            "past stays"
        );
    }

    #[test]
    fn no_identity_no_persistence() {
        let (mut project, _seed, platform_seed) = project_with_keys("ephemeral");
        // strip every key: nobody can be wrapped
        project.platform = None;
        let ids: Vec<String> = project.members().map(|(id, _)| id.clone()).collect();
        for id in ids {
            project.member_by_key_mut(&id).unwrap().verify_key = None;
        }
        let mut chat = chat_with_message();
        assert!(ensure_crypt(&project, &mut chat, &platform_seed).is_err());
        assert!(chat.crypt.is_none(), "stays ephemeral");
    }
}
