// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Opening and sealing a chat: the two operations that need a key.
//!
//! This is the seam the app runs in its webview (JAPP-0135-FD). Storage
//! hands over what it holds for one chat, [`Sealed`], plus the reader's
//! seed; [`open`] gives back the chat. A save goes the other way: [`seal`]
//! turns the chat the person edited into new slots and blobs, and storage
//! writes those bytes without ever seeing a key.
//!
//! Everything that used to sit between those two acts, the epoch DAG, the
//! coverage bookkeeping, the delta, lives here now. It has no business
//! knowing about git, and git has no business knowing about it.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use joy_crypt::identity::PublicKey;
use sha2::{Digest, Sha256};

use crate::chat_events::{self, ChatEvent};
use crate::chat_seal;
use crate::chat_wrap::{self, ContentKey, SLOT_LEN};
use crate::error::ChatError;
use crate::model::chat::Chat;

/// What storage holds for ONE chat: the anonymous key slots and the sealed
/// event blobs, as bytes. Their names are derived from the bytes
/// ([`chat_wrap::slot_id`], [`chat_seal::rid`]), so the two sides never
/// have to agree on anything but the content itself.
#[derive(Debug, Default, Clone)]
pub struct Sealed {
    pub slots: Vec<Vec<u8>>,
    pub blobs: Vec<Vec<u8>>,
}

/// A chat as a reader who holds a key sees it, plus what that reader needs
/// to seal a change onto it without losing anyone else's.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The folded chat.
    pub chat: Chat,
    /// Every event the reader could open. A foreign epoch stays sealed and
    /// is simply absent, never an error.
    pub events: Vec<ChatEvent>,
    /// The content keys this reader holds, by epoch.
    pub epoch_keys: BTreeMap<String, ContentKey>,
}

/// New bytes a save produces: name -> content, for slots and for blobs.
/// Storage unions them with what it already has; an unchanged save
/// produces nothing, which is what makes a re-save add no git objects.
#[derive(Debug, Default, Clone)]
pub struct Write {
    pub slots: Vec<(String, Vec<u8>)>,
    pub blobs: Vec<(String, Vec<u8>)>,
}

impl Write {
    /// Nothing to store.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty() && self.blobs.is_empty()
    }
}

/// One member of the project as far as sealing is concerned: their id and
/// the identity key on record. A member without a key cannot be a
/// recipient, and is simply absent here.
#[derive(Debug, Clone)]
pub struct Member {
    pub id: String,
    pub verify_key: PublicKey,
}

/// Who a chat's key must reach: its participants, or every project member
/// when the list is empty (the General/Team "everyone" convention).
///
/// Both sides ask THIS function. Storage knows the project from disk and
/// the webview gets it over the wire, but the rule is one rule, or the
/// same chat would be sealed for different people depending on who saved
/// it.
pub fn recipients(chat: &Chat, members: &[Member]) -> Vec<(String, PublicKey)> {
    use crate::model::chat::ChatKind;
    let everyone =
        chat.participants.is_empty() && matches!(chat.kind, ChatKind::General | ChatKind::Team);
    let wanted: Vec<String> = if everyone {
        members.iter().map(|m| m.id.clone()).collect()
    } else {
        chat.participants
            .iter()
            .map(|p| p.id().to_string())
            .collect()
    };
    wanted
        .into_iter()
        .filter_map(|id| {
            members
                .iter()
                .find(|m| m.id == id)
                .map(|m| (id, m.verify_key.clone()))
        })
        .collect()
}

/// Open what storage holds for a chat with one reader's seed.
///
/// A reader who holds no slot gets an [`Opened`] with no events; the
/// caller treats that as "not for me", never as an error, because a chat
/// must not reveal that it exists to someone outside it.
pub fn open(cid: &str, sealed: &Sealed, seed: &[u8; 32]) -> Opened {
    let x = chat_wrap::x25519_secret(seed);
    let epoch_keys = chat_wrap::resolve_epoch_keys(cid, &x, sealed.slots.iter().map(|s| &s[..]));
    let events = chat_seal::open_events(sealed.blobs.iter().map(|b| b.as_slice()), &epoch_keys);
    let created = created_of(&events).unwrap_or_else(Utc::now);
    let chat = chat_events::fold(cid, created, &events);
    Opened {
        chat,
        events,
        epoch_keys,
    }
}

/// Seal the change from `opened.chat` to `next` for `recipients`.
///
/// `recipients` is (member id, verify_key) for everyone the chat is for;
/// the caller reads that from the project, because who belongs to a
/// project is not this crate's business. An empty list is refused: a chat
/// with no recipient could only be sealed to nobody.
///
/// The reader's own view is the baseline, so two writers who cannot see
/// each other's epochs still converge: the events are a CRDT and the
/// storage side unions the blobs.
pub fn seal(
    cid: &str,
    opened: &Opened,
    next: &Chat,
    recipients: &[(String, PublicKey)],
    seed: &[u8; 32],
) -> Result<Write, ChatError> {
    if recipients.is_empty() {
        return Err(ChatError::auth(
            "no participant with an identity key; chat stays ephemeral",
        ));
    }
    let writer_tag = writer_tag(seed);
    let cov = coverage(&opened.events);
    let mut epoch_keys = opened.epoch_keys.clone();
    let mut new_events: Vec<(String, ChatEvent)> = Vec::new();
    let mut new_slots: Vec<[u8; SLOT_LEN]> = Vec::new();

    // --- active epoch: mint on a new chat, rotate on a removal -----------
    let mut active = active_epoch(&opened.events);
    if active.is_none() {
        let e0 = chat_wrap::new_epoch_id();
        let ck0 = chat_wrap::new_content_key();
        epoch_keys.insert(e0.clone(), ck0);
        new_events.push((
            e0.clone(),
            ChatEvent::Epoch {
                epoch_id: e0.clone(),
                parents: Vec::new(),
                created: chat_events::hlc_at(next.created),
                reason: "init".into(),
            },
        ));
        active = Some(e0);
    }
    let recip_ids: BTreeSet<&str> = recipients.iter().map(|(m, _)| m.as_str()).collect();
    let mut active = active.expect("active epoch set");
    let removed = cov
        .keys()
        .any(|(e, m)| e == &active && !recip_ids.contains(m.as_str()));
    if removed {
        let en = chat_wrap::new_epoch_id();
        let ckn = chat_wrap::new_content_key();
        epoch_keys.insert(en.clone(), ckn);
        new_events.push((
            en.clone(),
            ChatEvent::Epoch {
                epoch_id: en.clone(),
                parents: vec![active.clone()],
                created: chat_events::now_hlc(),
                reason: "rotate".into(),
            },
        ));
        active = en;
    }
    let ck_active = *epoch_keys
        .get(&active)
        .ok_or_else(|| ChatError::auth("active epoch key missing"))?;

    // --- coverage: each recipient in the active epoch, plus a re-wrap of
    //     any older epoch they already held under a now-changed key ------
    let is_covered = |cov: &BTreeMap<(String, String), BTreeSet<String>>,
                      pending: &[(String, ChatEvent)],
                      epoch: &str,
                      member: &str,
                      vk_hex: &str|
     -> bool {
        if cov
            .get(&(epoch.to_string(), member.to_string()))
            .is_some_and(|s| s.contains(vk_hex))
        {
            return true;
        }
        pending.iter().any(|(_, ev)| {
            matches!(ev, ChatEvent::Cover { epoch_id, member: m, vk_hex: v }
                if epoch_id == epoch && m == member && v == vk_hex)
        })
    };
    let grant = |epoch: &str,
                 ck: &ContentKey,
                 member: &str,
                 vk: &PublicKey,
                 ne: &mut Vec<(String, ChatEvent)>,
                 ns: &mut Vec<[u8; SLOT_LEN]>|
     -> Result<(), ChatError> {
        let vk_hex = vk.to_hex();
        if is_covered(&cov, ne, epoch, member, &vk_hex) {
            return Ok(());
        }
        ns.push(chat_wrap::anon_wrap_slot(cid, epoch, ck, vk)?);
        ne.push((
            epoch.to_string(),
            ChatEvent::Cover {
                epoch_id: epoch.to_string(),
                member: member.to_string(),
                vk_hex,
            },
        ));
        Ok(())
    };

    for (member, vk) in recipients {
        grant(
            &active,
            &ck_active,
            member,
            vk,
            &mut new_events,
            &mut new_slots,
        )?;
        let vk_hex = vk.to_hex();
        let held: Vec<String> = cov
            .keys()
            .filter(|(_, m)| m == member)
            .map(|(e, _)| e.clone())
            .collect();
        for e in held {
            if let Some(ck) = epoch_keys.get(&e) {
                if !is_covered(&cov, &new_events, &e, member, &vk_hex) {
                    grant(&e, ck, member, vk, &mut new_events, &mut new_slots)?;
                }
            }
        }
    }

    // --- semantic delta, sealed under the active epoch -------------------
    for ev in chat_events::diff(&opened.chat, next, &writer_tag) {
        new_events.push((active.clone(), ev));
    }

    // --- name the bytes --------------------------------------------------
    let mut write = Write::default();
    for slot in &new_slots {
        write.slots.push((chat_wrap::slot_id(slot), slot.to_vec()));
    }
    for (epoch, ev) in &new_events {
        let ck = epoch_keys
            .get(epoch)
            .ok_or_else(|| ChatError::auth("seal epoch key missing"))?;
        let blob = chat_seal::seal_event(cid, epoch, ck, ev)?;
        write.blobs.push((chat_seal::rid(&blob), blob));
    }
    Ok(write)
}

/// The content key of one epoch as this seed resolves it. Used where a
/// caller needs to read a specific epoch rather than the whole chat.
pub fn epoch_key(cid: &str, sealed: &Sealed, seed: &[u8; 32], epoch: &str) -> Option<ContentKey> {
    let x = chat_wrap::x25519_secret(seed);
    chat_wrap::resolve_epoch_keys(cid, &x, sealed.slots.iter().map(|s| &s[..]))
        .get(epoch)
        .copied()
}

/// When the chat was created, from its own events.
fn created_of(events: &[ChatEvent]) -> Option<DateTime<Utc>> {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Epoch { created, .. } => Some(*created),
            _ => None,
        })
        .min()
        .map(chat_events::hlc_to_datetime)
}

/// The active epoch id: the tip (an epoch no other epoch lists as a
/// parent) with the greatest `(created, id)`. Falls back to the greatest
/// epoch overall if the DAG has no clean tip.
fn active_epoch(events: &[ChatEvent]) -> Option<String> {
    let mut created: BTreeMap<&str, chat_events::Hlc> = BTreeMap::new();
    let mut children: BTreeSet<&str> = BTreeSet::new();
    for e in events {
        if let ChatEvent::Epoch {
            epoch_id,
            parents,
            created: c,
            ..
        } = e
        {
            created.insert(epoch_id.as_str(), *c);
            for p in parents {
                children.insert(p.as_str());
            }
        }
    }
    if created.is_empty() {
        return None;
    }
    let pick = |ids: Vec<&str>| -> Option<String> {
        ids.into_iter()
            .max_by(|a, b| created.get(a).cmp(&created.get(b)).then_with(|| a.cmp(b)))
            .map(str::to_string)
    };
    let tips: Vec<&str> = created
        .keys()
        .copied()
        .filter(|id| !children.contains(id))
        .collect();
    if !tips.is_empty() {
        pick(tips)
    } else {
        pick(created.keys().copied().collect())
    }
}

/// (epoch_id, member) -> the set of verify_keys the CK was wrapped for.
/// A key change adds a second vk for the same (epoch, member); a reader
/// holding either opens it.
fn coverage(events: &[ChatEvent]) -> BTreeMap<(String, String), BTreeSet<String>> {
    let mut cov: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for e in events {
        if let ChatEvent::Cover {
            epoch_id,
            member,
            vk_hex,
        } = e
        {
            cov.entry((epoch_id.clone(), member.clone()))
                .or_default()
                .insert(vk_hex.clone());
        }
    }
    cov
}

/// A stable, opaque per-writer tag for LWW tiebreaks: `sha256(verify_key)`
/// truncated. Sealed with each event, so it leaks nothing.
fn writer_tag(seed: &[u8; 32]) -> String {
    let vk = joy_crypt::identity::Keypair::from_seed(seed)
        .public_key()
        .to_hex();
    hex::encode(&Sha256::digest(vk.as_bytes())[..6])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chat::{ChatKind, ChatMessage, MessageKind};
    use joy_model::MemberRef;

    const CID: &str = "0123456789abcdef0123456789abcdef";

    fn kp(seed: u8) -> joy_crypt::identity::Keypair {
        joy_crypt::identity::Keypair::from_seed(&[seed; 32])
    }

    fn member(id: &str, seed: u8) -> Member {
        Member {
            id: id.to_string(),
            verify_key: kp(seed).public_key(),
        }
    }

    /// What storage does with a save: union the new bytes in.
    fn store(sealed: &mut Sealed, write: Write) {
        for (_name, bytes) in write.slots {
            sealed.slots.push(bytes);
        }
        for (_name, bytes) in write.blobs {
            sealed.blobs.push(bytes);
        }
    }

    fn line(id: &str, author: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            at: Utc::now(),
            author: MemberRef::new(author),
            text: text.to_string(),
            kind: MessageKind::Text,
            delegated_by: None,
            turn_ms: None,
            tool_steps: None,
            tool: None,
            payload: None,
            details: None,
        }
    }

    #[test]
    fn an_addressed_ai_reads_the_line_that_addressed_it() {
        // The operator's case, end to end (JAPP-0161-DC): a chat that
        // belongs to one person, then a line that addresses an AI. The
        // client takes the AI along as a participant when it seals, and
        // from that moment the AI's own key opens the chat. Without the
        // participant it opens nothing and answers nothing.
        let horst = [1u8; 32];
        let vibe = [2u8; 32];
        let members = vec![member("horst@example.com", 1), member("ai:vibe@joy", 2)];

        let mut stored = Sealed::default();
        let mut chat = Chat::new(CID, vec![MemberRef::new("horst@example.com")], Utc::now());
        chat.kind = ChatKind::Direct;

        // the person writes to themselves first
        let opened = open(CID, &stored, &horst);
        chat.messages.push(line("m1", "horst@example.com", "nur ich"));
        let write = seal(CID, &opened, &chat, &recipients(&chat, &members), &horst).unwrap();
        store(&mut stored, write);

        // nothing for the AI yet: it is not in the chat
        assert!(open(CID, &stored, &vibe).chat.messages.is_empty());

        // now the line that addresses it, sealed WITH the AI as participant
        let opened = open(CID, &stored, &horst);
        let mut next = opened.chat.clone();
        next.participants.push(MemberRef::new("ai:vibe@joy"));
        next.messages.push(line("m2", "horst@example.com", "@vibe ping"));
        let write = seal(CID, &opened, &next, &recipients(&next, &members), &horst).unwrap();
        store(&mut stored, write);

        // the AI opens the chat and SEES the line that addressed it
        let seen = open(CID, &stored, &vibe).chat;
        assert!(
            seen.messages.iter().any(|m| m.text == "@vibe ping"),
            "the addressed AI must read the line that addressed it: {:?}",
            seen.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_member_outside_the_chat_reads_nothing() {
        let horst = [1u8; 32];
        let mate = [3u8; 32];
        let members = vec![member("horst@example.com", 1), member("mate@example.com", 3)];

        let mut stored = Sealed::default();
        let mut chat = Chat::new(CID, vec![MemberRef::new("horst@example.com")], Utc::now());
        chat.kind = ChatKind::Direct;
        chat.messages.push(line("m1", "horst@example.com", "privat"));
        let opened = open(CID, &stored, &horst);
        let write = seal(CID, &opened, &chat, &recipients(&chat, &members), &horst).unwrap();
        store(&mut stored, write);

        let outside = open(CID, &stored, &mate).chat;
        assert!(outside.messages.is_empty(), "a chat must not leak to a project member who is not in it");
    }

    #[test]
    fn an_empty_participant_list_reaches_every_member() {
        // General/Team convention: no list means everyone, and that
        // already covers an AI member without naming it.
        let members = vec![member("horst@example.com", 1), member("ai:vibe@joy", 2)];
        let mut chat = Chat::new(CID, Vec::new(), Utc::now());
        chat.kind = ChatKind::General;
        let ids: Vec<String> = recipients(&chat, &members).into_iter().map(|(m, _)| m).collect();
        assert_eq!(ids, vec!["horst@example.com".to_string(), "ai:vibe@joy".to_string()]);
    }

    #[test]
    fn a_participant_without_a_key_on_record_is_no_recipient() {
        // A member the project knows no identity key for cannot hold a
        // slot; sealing must not fail over it, it is simply not covered.
        let members = vec![member("horst@example.com", 1)];
        let mut chat = Chat::new(CID, Vec::new(), Utc::now());
        chat.kind = ChatKind::Team;
        chat.participants = vec![
            MemberRef::new("horst@example.com"),
            MemberRef::new("keyless@example.com"),
        ];
        let ids: Vec<String> = recipients(&chat, &members).into_iter().map(|(m, _)| m).collect();
        assert_eq!(ids, vec!["horst@example.com".to_string()]);
    }

    #[test]
    fn sealing_for_nobody_is_refused() {
        let horst = [1u8; 32];
        let chat = Chat::new(CID, vec![MemberRef::new("horst@example.com")], Utc::now());
        let opened = open(CID, &Sealed::default(), &horst);
        assert!(seal(CID, &opened, &chat, &[], &horst).is_err());
    }

    #[test]
    fn a_second_writer_keeps_what_the_first_one_wrote() {
        // Two participants save in turn; the union of their bytes must
        // fold into one chat with both lines, or a message would vanish
        // when two people write at once.
        let horst = [1u8; 32];
        let mate = [3u8; 32];
        let members = vec![member("horst@example.com", 1), member("mate@example.com", 3)];
        let participants = vec![
            MemberRef::new("horst@example.com"),
            MemberRef::new("mate@example.com"),
        ];

        let mut stored = Sealed::default();
        let mut chat = Chat::new(CID, participants.clone(), Utc::now());
        chat.kind = ChatKind::Team;
        chat.messages.push(line("m1", "horst@example.com", "erste"));
        let opened = open(CID, &stored, &horst);
        let write = seal(CID, &opened, &chat, &recipients(&chat, &members), &horst).unwrap();
        store(&mut stored, write);

        let opened = open(CID, &stored, &mate);
        let mut next = opened.chat.clone();
        next.messages.push(line("m2", "mate@example.com", "zweite"));
        let write = seal(CID, &opened, &next, &recipients(&next, &members), &mate).unwrap();
        store(&mut stored, write);

        let both = open(CID, &stored, &horst).chat;
        let texts: Vec<&str> = both.messages.iter().map(|m| m.text.as_str()).collect();
        assert!(texts.contains(&"erste") && texts.contains(&"zweite"), "{texts:?}");
    }

    #[test]
    fn saving_the_same_chat_twice_stores_nothing_new() {
        // Content addressing: a save that changes nothing must not add
        // objects, or every poll would grow the repository.
        let horst = [1u8; 32];
        let members = vec![member("horst@example.com", 1)];
        let mut stored = Sealed::default();
        let mut chat = Chat::new(CID, vec![MemberRef::new("horst@example.com")], Utc::now());
        chat.messages.push(line("m1", "horst@example.com", "einmal"));
        let opened = open(CID, &stored, &horst);
        let write = seal(CID, &opened, &chat, &recipients(&chat, &members), &horst).unwrap();
        store(&mut stored, write);

        let opened = open(CID, &stored, &horst);
        let again = seal(CID, &opened, &opened.chat.clone(), &recipients(&chat, &members), &horst)
            .unwrap();
        assert!(again.is_empty(), "a re-save must produce no new bytes");
    }
}
