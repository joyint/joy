// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Anonymous content-key transport for sealed chats (ADR JAPP-002A-30).
//!
//! A chat's per-epoch AES-256-GCM content key (CK) must reach each
//! participant WITHOUT the stored bytes revealing who a slot is for: a
//! keyless repo reader, even one holding every public `verify_key`, must
//! not be able to tell who is in which chat. The current per-member wrap
//! ([`crate::crypt::wrap_for_member`]) prefixes the granter's verify_key
//! and keys the map by recipient id, both of which leak membership.
//!
//! The fix is an EPHEMERAL-sender wrap. Each grant mints a throwaway
//! X25519 keypair, derives the pairwise KEK against the recipient's
//! X25519 public, and discards the ephemeral secret. The stored slot is
//!
//! ```text
//! [0..32]   eph_pub   ephemeral X25519 public (fresh, discarded)
//! [32..108] body      wrap(kek, epoch_id[16] || CK[32]) = nonce||ct||tag
//! ```
//!
//! a uniform 108 bytes carrying no recipient id, no verify_key, and no
//! plaintext epoch. Testing "is this slot for member M" needs
//! `ECDH(eph_secret, X_M)` — the discarded ephemeral secret OR M's
//! secret. A public-key-only adversary has neither, so no slot is
//! attributable and two slots are unlinkable across chats. Decoy slots
//! (wraps to throwaway keys nobody holds) are byte-identical real wraps,
//! so participant COUNT can be padded without a distinguishable marker.
//!
//! The primitives are reused verbatim: [`joy_crypt::pairwise::pairwise_kek`]
//! (HKDF-SHA256 over X25519 ECDH) and [`joy_crypt::wrap`] (AES-256-GCM),
//! only the HKDF `info` prefix and the sealed `epoch_id||CK` payload are
//! new. No new wire format.

use std::collections::BTreeMap;

use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::auth::{IdentityKeypair, PublicKey};
use crate::error::JoyError;

/// HKDF info domain for chat content-key wraps. Binds a slot to its chat
/// (a slot cannot be replayed into another chat) without naming a
/// recipient or an epoch (both sealed inside the wrap body).
const WRAP_INFO_PREFIX: &[u8] = b"chat-anon-v1:";

/// A slot is exactly this many bytes: 32 (eph pub) + 12 (nonce) + 48
/// (ciphertext of epoch_id||CK) + 16 (tag).
pub const SLOT_LEN: usize = 108;

/// A chat content key.
pub type ContentKey = [u8; 32];

fn info(cid: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(WRAP_INFO_PREFIX.len() + cid.len());
    v.extend_from_slice(WRAP_INFO_PREFIX);
    v.extend_from_slice(cid.as_bytes());
    v
}

/// A fresh random epoch id (16 bytes, lowercase hex). Opaque and
/// content-free, so concurrent rotations mint DISTINCT epochs (no
/// positional collision) and the id itself leaks nothing.
pub fn new_epoch_id() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// A fresh random content key.
pub fn new_content_key() -> ContentKey {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

/// The content-addressed filename for a slot (`sha256(slot)[..16]` hex =
/// 32 chars). Identical slots dedupe under keyless union; distinct ones
/// never collide.
pub fn slot_id(slot: &[u8]) -> String {
    hex::encode(&Sha256::digest(slot)[..16])
}

/// Anonymously wrap `(epoch_id, ck)` for a recipient addressed by their
/// Ed25519 `verify_key`. Returns the 108-byte slot; the ephemeral secret
/// is dropped inside this call.
pub fn anon_wrap_slot(
    cid: &str,
    epoch_id: &str,
    ck: &ContentKey,
    recipient_vk: &PublicKey,
) -> Result<[u8; SLOT_LEN], JoyError> {
    let eid = hex::decode(epoch_id).map_err(|e| JoyError::AuthFailed(format!("epoch id: {e}")))?;
    if eid.len() != 16 {
        return Err(JoyError::AuthFailed("epoch id must be 16 bytes".into()));
    }
    let eph = IdentityKeypair::from_random();
    let eph_secret = eph.to_x25519_secret_bytes();
    let eph_pub = eph.public_key().to_x25519_public_bytes();
    let recipient_x = recipient_vk.to_x25519_public_bytes();
    let kek = joy_crypt::pairwise::pairwise_kek(&eph_secret, &recipient_x, &info(cid));

    let mut payload = Vec::with_capacity(48);
    payload.extend_from_slice(&eid);
    payload.extend_from_slice(ck);
    let body = joy_crypt::wrap::wrap(&kek, &payload);
    if body.len() != SLOT_LEN - 32 {
        return Err(JoyError::AuthFailed(format!(
            "wrap body length {} unexpected",
            body.len()
        )));
    }
    let mut slot = [0u8; SLOT_LEN];
    slot[..32].copy_from_slice(&eph_pub);
    slot[32..].copy_from_slice(&body);
    Ok(slot)
}

/// A decoy slot: a real anonymous wrap to a throwaway keypair nobody
/// holds. Byte-indistinguishable from a genuine slot, so it pads the
/// participant count without a marker.
pub fn decoy_slot(cid: &str) -> [u8; SLOT_LEN] {
    let throwaway = IdentityKeypair::from_random();
    anon_wrap_slot(
        cid,
        &new_epoch_id(),
        &new_content_key(),
        &throwaway.public_key(),
    )
    .expect("decoy wrap with valid fresh material never fails")
}

/// Every epoch content key the holder of `my_x_secret` (their X25519
/// secret, from `IdentityKeypair::to_x25519_secret_bytes`) can open from
/// `slots`. A slot that does not open is absence, never an error (same
/// discipline as zone-key resolution). Cost is one X25519 + one AEAD open
/// per slot; only the holder's own slot(s) verify.
pub fn resolve_epoch_keys<'a>(
    cid: &str,
    my_x_secret: &[u8; 32],
    slots: impl IntoIterator<Item = &'a [u8]>,
) -> BTreeMap<String, ContentKey> {
    let info = info(cid);
    let mut out = BTreeMap::new();
    for slot in slots {
        if slot.len() != SLOT_LEN {
            continue;
        }
        let mut eph_pub = [0u8; 32];
        eph_pub.copy_from_slice(&slot[..32]);
        let kek = joy_crypt::pairwise::pairwise_kek(my_x_secret, &eph_pub, &info);
        if let Ok(p) = joy_crypt::wrap::unwrap(&kek, &slot[32..]) {
            if p.len() == 48 {
                let epoch_id = hex::encode(&p[..16]);
                let mut ck = [0u8; 32];
                ck.copy_from_slice(&p[16..]);
                out.insert(epoch_id, ck);
            }
        }
    }
    out
}

/// The X25519 secret for a member identity seed (convenience for callers
/// resolving their own slots).
pub fn x25519_secret(seed: &[u8; 32]) -> [u8; 32] {
    IdentityKeypair::from_seed(seed).to_x25519_secret_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kp(seed: u8) -> IdentityKeypair {
        IdentityKeypair::from_seed(&[seed; 32])
    }

    #[test]
    fn recipient_opens_its_slot_and_nobody_else_does() {
        let cid = "0123456789abcdef0123456789abcdef";
        let epoch = new_epoch_id();
        let ck = new_content_key();
        let recipient = kp(1);
        let stranger = kp(2);

        let slot = anon_wrap_slot(cid, &epoch, &ck, &recipient.public_key()).unwrap();
        assert_eq!(slot.len(), SLOT_LEN);

        // the recipient resolves exactly this epoch's key
        let mine = resolve_epoch_keys(
            cid,
            &recipient.to_x25519_secret_bytes(),
            std::iter::once(&slot[..]),
        );
        assert_eq!(mine.get(&epoch).copied(), Some(ck));

        // a stranger holding every PUBLIC key still opens nothing
        let theirs = resolve_epoch_keys(
            cid,
            &stranger.to_x25519_secret_bytes(),
            std::iter::once(&slot[..]),
        );
        assert!(theirs.is_empty(), "non-recipient must not resolve any key");
    }

    #[test]
    fn a_slot_bound_to_one_chat_does_not_open_under_another() {
        let epoch = new_epoch_id();
        let ck = new_content_key();
        let recipient = kp(3);
        let slot = anon_wrap_slot("chat-aaaa", &epoch, &ck, &recipient.public_key()).unwrap();
        // same recipient, WRONG chat id in the info -> no open (AAD binding)
        let got = resolve_epoch_keys(
            "chat-bbbb",
            &recipient.to_x25519_secret_bytes(),
            std::iter::once(&slot[..]),
        );
        assert!(got.is_empty());
    }

    #[test]
    fn decoys_are_byte_uniform_and_unattributable() {
        let cid = "cafebabecafebabecafebabecafebabe";
        let real = anon_wrap_slot(
            cid,
            &new_epoch_id(),
            &new_content_key(),
            &kp(4).public_key(),
        )
        .unwrap();
        let decoy = decoy_slot(cid);
        assert_eq!(real.len(), decoy.len());
        assert_eq!(decoy.len(), SLOT_LEN);
        // nobody in the project opens the decoy
        for seed in 1..8u8 {
            let got = resolve_epoch_keys(
                cid,
                &kp(seed).to_x25519_secret_bytes(),
                std::iter::once(&decoy[..]),
            );
            assert!(got.is_empty());
        }
    }

    #[test]
    fn resolve_collects_every_epoch_the_holder_was_granted() {
        let cid = "0000111122223333444455556666777";
        let me = kp(9);
        let (e1, k1) = (new_epoch_id(), new_content_key());
        let (e2, k2) = (new_epoch_id(), new_content_key());
        let s1 = anon_wrap_slot(cid, &e1, &k1, &me.public_key()).unwrap();
        let s2 = anon_wrap_slot(cid, &e2, &k2, &me.public_key()).unwrap();
        let decoy = decoy_slot(cid);
        let slots: Vec<&[u8]> = vec![&s1[..], &decoy[..], &s2[..]];
        let got = resolve_epoch_keys(cid, &me.to_x25519_secret_bytes(), slots);
        assert_eq!(got.get(&e1).copied(), Some(k1));
        assert_eq!(got.get(&e2).copied(), Some(k2));
        assert_eq!(got.len(), 2, "the decoy contributes nothing");
    }

    #[test]
    fn slot_id_is_content_addressed() {
        let cid = "abcabcabcabcabcabcabcabcabcabcab";
        let s = anon_wrap_slot(
            cid,
            &new_epoch_id(),
            &new_content_key(),
            &kp(5).public_key(),
        )
        .unwrap();
        assert_eq!(slot_id(&s), slot_id(&s), "deterministic");
        assert_eq!(slot_id(&s).len(), 32);
    }
}
