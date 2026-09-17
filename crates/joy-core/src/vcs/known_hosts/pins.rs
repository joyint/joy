// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The host keys the three public forges publish, shipped as data
//! (design D1.4a, decision 23).
//!
//! **Data in the release, not code in the binary.** The pins live in
//! `host-keys.json` inside the release and joy reads that file at run
//! time: beside the joy binary, or in `<prefix>/share/joy` beside its
//! `bin` directory. D1.4a asks for exactly this, and for one reason: a
//! pinned host that rotates its key would otherwise be unreachable
//! until a new joy is built and installed, which is the failure the
//! pin is supposed to bound rather than cause. Whoever can write into
//! that directory can already replace the joy binary itself, so
//! reading the file there extends no trust; it only makes a rotation a
//! file that is replaced. The copy compiled in below is the fallback
//! for a binary that stands alone (a `cargo run`, a test binary, a
//! manual copy), never the pin file itself.
//!
//! The file holds full key blobs, not fingerprints, because a
//! fingerprint cannot answer "is THIS the key" without the key. Every
//! blob carries the fingerprint its forge publishes and the page that
//! publishes it.
//!
//! **Why a release pins nothing yet.** Pinning replaces the person's
//! first contact decision with trust in the joy release: it removes the
//! one moment where a man in the middle could be caught on a fresh
//! machine, and it makes joy refuse a legitimate key rotation at these
//! hosts until a new pin file arrives. That trade is decision 23 and
//! the operator has not answered it, so the file a release ships holds
//! no host at all and an unknown host is refused with
//! `needs_host_trust` on every `Background` and `Delegated` host,
//! exactly as it is for every other host. The blobs that answering the
//! decision would ship are parked beside the release in
//! `host-keys.published.json`, which no joy binary carries: they are
//! read by the tests and by `just check-host-key-pins`, so answering
//! the decision is a file that is copied and no new key hunt.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// The pin file's name, in the release and beside it.
const FILE_NAME: &str = "host-keys.json";

/// The copy compiled into this binary. It answers only where the
/// release has no pin file of its own, and it ships empty for as long
/// as decision 23 is open.
const BAKED: &str = include_str!("../../../data/host-keys.json");

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

/// Every pin this release holds, which is what the file in the release
/// holds and nothing else. Empty while decision 23 is open.
pub fn shipped() -> &'static [PinnedHost] {
    static SHIPPED: OnceLock<Vec<PinnedHost>> = OnceLock::new();
    SHIPPED
        .get_or_init(|| match release_file() {
            Some((path, text)) => {
                let hosts = hosts_in(&text, &path.display().to_string());
                tracing::debug!(
                    file = %path.display(),
                    hosts = hosts.len(),
                    "the pinned host keys were read from the release"
                );
                hosts
            }
            None => hosts_in(BAKED, "the copy compiled into this binary"),
        })
        .as_slice()
}

/// The pin a contact may consult for one host: the one the release
/// ships, or nothing. A known_hosts line always wins over it, and it is
/// asked only where no file holds any line for the host (D1.4a).
pub fn consulted_for(host: &str) -> Option<&'static PinnedHost> {
    find(shipped(), host)
}

/// The pin file of the running release, read at run time.
fn release_file() -> Option<(PathBuf, String)> {
    let exe = std::env::current_exe().ok()?;
    file_beside(exe.parent()?)
}

/// The pin file a release whose binary sits in `exe_dir` carries, and
/// the text of it.
pub(super) fn file_beside(exe_dir: &Path) -> Option<(PathBuf, String)> {
    candidates(exe_dir)
        .into_iter()
        .find_map(|path| std::fs::read_to_string(&path).ok().map(|text| (path, text)))
}

/// Where a release keeps its pin file, in the order joy looks: beside
/// the binary (which is how the installers lay a release out), then in
/// the share directory of a prefix install, where `bin/joy` has its
/// data under `share/joy`.
pub(super) fn candidates(exe_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![exe_dir.join(FILE_NAME)];
    if let Some(prefix) = exe_dir.parent() {
        paths.push(prefix.join("share").join("joy").join(FILE_NAME));
    }
    paths
}

/// The hosts one pin file names. A file joy cannot read is a release
/// fault, and the answer to it is to know no pins rather than to refuse
/// every contact.
pub(super) fn hosts_in(text: &str, origin: &str) -> Vec<PinnedHost> {
    match serde_json::from_str::<PinFile>(text) {
        Ok(file) => file.hosts,
        Err(e) => {
            tracing::error!(origin, error = %e, "the pinned host keys could not be read");
            Vec::new()
        }
    }
}

fn find<'a>(hosts: &'a [PinnedHost], host: &str) -> Option<&'a PinnedHost> {
    let host = host.trim_end_matches('.');
    hosts.iter().find(|pin| pin.host.eq_ignore_ascii_case(host))
}

/// The blobs taken off the three public forges and checked against the
/// pages they publish, parked beside the release.
///
/// `cfg(test)` and nothing else: this is the material decision 23 would
/// ship, it is compiled into no binary joy releases, and it is here so
/// that the branch which will read a pin is exercised before the
/// decision switches it on.
#[cfg(test)]
pub fn published() -> &'static [PinnedHost] {
    static PUBLISHED: OnceLock<Vec<PinnedHost>> = OnceLock::new();
    PUBLISHED
        .get_or_init(|| {
            hosts_in(
                include_str!("../../../data/host-keys.published.json"),
                "data/host-keys.published.json",
            )
        })
        .as_slice()
}

/// The parked pin for one host, for the tests and for nothing else.
#[cfg(test)]
pub fn published_for(host: &str) -> Option<&'static PinnedHost> {
    find(published(), host)
}
