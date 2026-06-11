// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Installs the per-command member resolver (ADR-042).
//!
//! Called once at dispatch. In open mode it installs a pass-through resolver;
//! in anonymous mode it decrypts `members.yaml` (so every output can resolve an
//! opaque id to a name/e-mail) using, in order, the members-zone key cached in
//! the active session, then `JOY_PASSPHRASE`. When neither is available the
//! resolver is "locked": output requests authentication rather than leaking an
//! id (fail-safe). All resolution then flows through `joy_core::member_ref`.

use joy_core::auth;
use joy_core::crypt::{self, ZoneKey};
use joy_core::member_ref::{install, MemberResolver};
use joy_core::members_file::{self, MembersFile, MEMBERS_ZONE};
use joy_core::model::project::{PrivacyMode, Project};
use joy_core::store;
use joy_core::vcs::Vcs;

/// Build and install the member resolver for the current command.
pub fn install_member_resolver() {
    let resolver = build().unwrap_or_else(MemberResolver::open);
    install(resolver);
}

fn build() -> Option<MemberResolver> {
    let cwd = std::env::current_dir().ok()?;
    let root = store::find_project_root(&cwd)?;
    let project = store::read_project(&store::joy_dir(&root).join(store::PROJECT_FILE)).ok()?;
    if project.privacy_mode() != PrivacyMode::Anonymous {
        return Some(MemberResolver::open());
    }
    Some(MemberResolver::anonymous(decrypt_members(&root, &project)))
}

/// Best-effort decryption of `members.yaml`: session cache first (no passphrase
/// prompt), then `JOY_PASSPHRASE`. `None` means locked.
fn decrypt_members(root: &std::path::Path, project: &Project) -> Option<MembersFile> {
    if let Some(zk) = session_members_key(root) {
        if let Ok(mf) = members_file::read(root, &zk) {
            return Some(mf);
        }
    }
    if let Some(zk) = passphrase_members_key(root, project) {
        if let Ok(mf) = members_file::read(root, &zk) {
            return Some(mf);
        }
    }
    None
}

/// The members-zone key cached in the current member's active session.
fn session_members_key(root: &std::path::Path) -> Option<ZoneKey> {
    let identity = joy_core::identity::resolve_identity(root).ok()?;
    let project_id = auth::session::project_id(root).ok()?;
    let token = auth::session::load_session(&project_id, &identity.member).ok()??;
    if token.claims.expires <= chrono::Utc::now() {
        return None;
    }
    zone_key_from_hex(token.members_zone_key.as_deref()?)
}

/// The members-zone key derived from `JOY_PASSPHRASE` for the current member.
fn passphrase_members_key(root: &std::path::Path, project: &Project) -> Option<ZoneKey> {
    let passphrase = std::env::var("JOY_PASSPHRASE")
        .ok()
        .filter(|s| !s.is_empty())?;
    let email = joy_core::vcs::default_vcs().user_email().ok()?;
    let member_key = joy_core::privacy::member_key_for_email(project, &email)?;
    let member = project.members.get(&member_key)?;
    let unlocked = auth::unlock_identity(member, &passphrase).ok()?;
    let wrap = member.members_wrap.as_deref()?;
    let _ = root;
    crypt::unwrap_for_member(wrap, MEMBERS_ZONE, &unlocked.seed).ok()
}

fn zone_key_from_hex(hex_str: &str) -> Option<ZoneKey> {
    let bytes = hex::decode(hex_str).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(ZoneKey::from_bytes(arr))
}
