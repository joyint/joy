// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The host keys the three public forges publish, shipped as data
//! (design D1.4a, decision 23).
//!
//! The file beside this module holds the full key blobs, not
//! fingerprints, because a fingerprint cannot answer "is THIS the key"
//! without the key. Every blob carries the fingerprint its forge
//! publishes and the page that publishes it, and the test
//! `pins_match_the_published_fingerprints` recomputes the fingerprint
//! from the blob, so a blob that was edited by hand fails the build.
//! Codeberg publishes fingerprints only, so its blob was taken once
//! with `ssh-keyscan` and checked against that page; `just
//! check-host-key-pins` takes it again and checks it against the page
//! a second time, which is the build step D1.4a asks for.
//!
//! **Why nothing consults this yet.** Pinning replaces the person's
//! first contact decision with trust in the joy release: it removes the
//! one moment where a man in the middle could be caught on a fresh
//! machine, and it makes joy refuse a legitimate key rotation at these
//! hosts until a new pin ships. That trade is decision 23 and the
//! operator has not answered it, so [`CONSULTED`] is false and an
//! unknown host is refused with `needs_host_trust` on every `Background`
//! and `Delegated` host, exactly as it is for every other host. The
//! data is here so that answering the decision is one constant and no
//! new key hunt.

use std::sync::OnceLock;

use serde::Deserialize;

/// The pin file, shipped inside the release.
const HOST_KEYS: &str = include_str!("../../../data/host-keys.json");

/// Whether joy consults the pins at all (design decision 23, still
/// open). While this is false the pins are data nobody reads, and a
/// host joy has no known_hosts line for is unknown, whatever the pin
/// file holds.
pub const CONSULTED: bool = false;

/// One published key of one host.
#[derive(Debug, Deserialize)]
pub struct PinnedKey {
    /// The known_hosts spelling of the type (`ssh-ed25519`).
    #[serde(rename = "type")]
    pub key_type: String,
    /// The key blob, base64 as the forge publishes it.
    pub key: String,
    /// The fingerprint the forge publishes for that blob.
    pub fingerprint: String,
}

impl PinnedKey {
    /// The raw blob this pin holds.
    pub fn blob(&self) -> Option<Vec<u8>> {
        super::decode_base64(&self.key)
    }
}

/// One pinned host.
#[derive(Debug, Deserialize)]
pub struct PinnedHost {
    pub host: String,
    /// The endpoint the build step scans. A pin is found by host NAME
    /// and not by port: the two GitHub names and the two GitLab names
    /// serve the same keys, and a known_hosts line still wins over a
    /// pin whatever the port is.
    pub port: u16,
    /// The forge's own name, for the sentence a person reads.
    pub forge: String,
    /// Where the blobs were taken from.
    pub source: String,
    /// The page the forge publishes the fingerprints on, which is what
    /// a refusal sentence sends a person to.
    pub published_at: String,
    pub keys: Vec<PinnedKey>,
}

#[derive(Debug, Deserialize)]
struct PinFile {
    hosts: Vec<PinnedHost>,
}

fn parsed() -> &'static [PinnedHost] {
    static PARSED: OnceLock<Vec<PinnedHost>> = OnceLock::new();
    PARSED
        .get_or_init(|| match serde_json::from_str::<PinFile>(HOST_KEYS) {
            Ok(file) => file.hosts,
            Err(e) => {
                // A pin file joy cannot read is a release fault, and
                // the answer is to know no pins rather than to refuse
                // every contact.
                tracing::error!(error = %e, "the pinned host keys could not be read");
                Vec::new()
            }
        })
        .as_slice()
}

/// Every pin the release ships, whether or not they are consulted.
/// Read by the tests and by the build step, never by a contact.
pub fn recorded() -> &'static [PinnedHost] {
    parsed()
}

/// The pin for one host as DATA, ignoring decision 23.
pub fn recorded_for(host: &str) -> Option<&'static PinnedHost> {
    let host = host.trim_end_matches('.');
    parsed()
        .iter()
        .find(|pin| pin.host.eq_ignore_ascii_case(host))
}

/// The pin a contact may use, which is nothing until decision 23 is
/// answered.
pub fn consulted_for(host: &str) -> Option<&'static PinnedHost> {
    CONSULTED.then(|| recorded_for(host)).flatten()
}
