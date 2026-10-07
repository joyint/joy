// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Installs the per-command member resolver (ADR-042).
//!
//! Called once at dispatch. In open mode it installs a pass-through resolver;
//! in anonymous mode it decrypts `members.yaml` (so every output can resolve an
//! opaque id to a name/e-mail) using, in order, the members-zone key cached in
//! the active session (joy-core's `member_ref::resolver_for`, the construction
//! every host shares, JOY-02C3-85), then `JOY_PASSPHRASE`. When neither is available the
//! resolver is "locked": output requests authentication rather than leaking an
//! id (fail-safe). All resolution then flows through `joy_core::member_ref`.

use joy_core::auth;
use joy_core::member_ref::{install, MemberResolver};
use joy_core::members_file::{self, MembersFile, MEMBERS_ZONE};
use joy_core::model::project::Project;
use joy_core::store;
use joy_crypt::zone::{unwrap_for_member, ZoneKey};

/// Build and install the member resolver for the current command.
pub fn install_member_resolver() {
    let resolver = build().unwrap_or_else(MemberResolver::open);
    install(resolver);
}

fn build() -> Option<MemberResolver> {
    let cwd = std::env::current_dir().ok()?;
    let root = store::find_project_root(&cwd)?;
    // The one construction every host shares (JOY-02C3-85): privacy mode,
    // then the session's members-zone key. What the CLI adds is the
    // passphrase on the command line, which no other host has.
    let resolver = joy_core::member_ref::resolver_for(&root);
    if !resolver.locked() {
        return Some(resolver);
    }
    let project = store::load_project(&root).ok()?;
    Some(MemberResolver::anonymous(passphrase_members(
        &root, &project,
    )))
}

/// `members.yaml` decrypted with the key `JOY_PASSPHRASE` derives for the
/// current member. `None` means locked.
fn passphrase_members(root: &std::path::Path, project: &Project) -> Option<MembersFile> {
    let zk = passphrase_members_key(root, project)?;
    members_file::read(root, &zk).ok()
}

/// The members-zone key derived from `JOY_PASSPHRASE` for the current member.
fn passphrase_members_key(root: &std::path::Path, project: &Project) -> Option<ZoneKey> {
    let passphrase = crate::auth_gate::passphrase_flag()?;
    let member_key = joy_core::identity::acting_human_key(root).ok()?;
    let member = project.member_by_key(&member_key)?;
    let unlocked = auth::unlock_identity(member, &passphrase).ok()?;
    let wrap = member.members_wrap.as_deref()?;
    unwrap_for_member(wrap, MEMBERS_ZONE, &unlocked.seed).ok()
}
