// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The member files of joy 0.23 into the shape of format 4 (JOY-02DB-09),
//! a migration under ADR JOY-0244-9E: delete this module and its lines in
//! `migrations` (waiting, update_with, prepare_update, finish_update), in
//! `member_files` (read, write) and in `model::project` (`de_members`) once
//! no project from before is left. Nothing else names the old shape.
//!
//! joy 0.23 wrote a person's address as `email: <address>` at the top of
//! the file and the seed wrap four lines below as `seed_wrap_passphrase`.
//! Secret scanners read that as an address and its password (GitGuardian,
//! 2026-10-17). Format 4 says what a member is under `type` and puts the
//! name it goes by as the one key under `identity`, with the keys below
//! it; `seed_wrap_passphrase` is `seed_wrap`, `email_match` is
//! `email_tag`.
//!
//! An older joy cannot read format 4, so the files are rewritten when a
//! person says yes to the update and at no other moment (JOY-02CC-C1).
//! Until then a project of format 2 or 3 is read and written in the shape
//! it has: [`from_flat`] on every read, [`to_flat`] on every write. No
//! signature covers the shape of a file, so nothing is signed here.

use std::path::Path;

use serde_yaml_ng::{Mapping, Value};

use crate::error::JoyError;
use crate::model::project::{is_ai_member, MemberFileShape, MemberKind, MemberLayout, Project};
use crate::store;

/// The joy that reads format 4: everybody needs it after the update.
pub const NEEDS_JOY: &str = "0.24";

/// What a member of a project from before format 4 is, by the one rule
/// those projects were written with: an AI member is known by its name,
/// a person by an address or an opaque id.
pub fn kind_of_key(key: &str) -> MemberKind {
    if is_ai_member(key) {
        MemberKind::Ai
    } else {
        MemberKind::Person
    }
}

/// The keys that move under `identity`, in their order there, each with
/// its old name.
const IDENTITY_KEYS: &[(&str, &str)] = &[
    ("email_match", "email_tag"),
    ("verify_key", "verify_key"),
    ("kdf_nonce", "kdf_nonce"),
    ("seed_wrap_passphrase", "seed_wrap"),
    ("seed_wrap_recovery", "seed_wrap_recovery"),
    ("enrollment_verifier", "enrollment_verifier"),
];

/// The keys that stay at the top, in their order in format 4.
const TOP_KEYS: &[&str] = &[
    "adapter",
    "model",
    "capabilities",
    "level",
    "granted",
    "origin",
    "members_wrap",
    "crypt_wraps",
    "ai_delegations",
    "updated",
];

/// The order of a joy 0.23 member file, old names included.
const FLAT_KEYS: &[&str] = &[
    "email",
    "name",
    "adapter",
    "model",
    "capabilities",
    "level",
    "granted",
    "origin",
    "verify_key",
    "kdf_nonce",
    "seed_wrap_passphrase",
    "seed_wrap_recovery",
    "enrollment_verifier",
    "email_match",
    "members_wrap",
    "crypt_wraps",
    "ai_delegations",
    "updated",
];

/// A member file of joy 0.23, the one named `id`, in the shape of format
/// 4. A key neither shape knows is carried over at the end.
pub fn from_flat(id: &str, flat: Value) -> Value {
    let Value::Mapping(mut flat) = flat else {
        return flat;
    };
    let (kind, key) = match (flat.remove("email"), flat.remove("name")) {
        (Some(Value::String(address)), _) => (MemberKind::Person, address),
        (_, Some(Value::String(name))) => (MemberKind::Ai, name),
        // an anonymous project's person: the id is the name they go by
        _ => (MemberKind::Person, id.to_string()),
    };
    let mut identity = Mapping::new();
    for (old, new) in IDENTITY_KEYS {
        if let Some(value) = flat.remove(*old) {
            identity.insert((*new).into(), value);
        }
    }
    let mut out = Mapping::new();
    out.insert("type".into(), kind.as_str().into());
    let mut named = Mapping::new();
    named.insert(key.into(), Value::Mapping(identity));
    out.insert("identity".into(), Value::Mapping(named));
    for top in TOP_KEYS {
        if let Some(value) = flat.remove(*top) {
            out.insert((*top).into(), value);
        }
    }
    for (key, value) in flat {
        out.insert(key, value);
    }
    Value::Mapping(out)
}

/// A member file of format 4 in the shape joy 0.23 writes, for a project
/// whose person has not said yes to the update yet. A platform member has
/// no such shape: it needs the update first.
pub fn to_flat(file: Value) -> Result<Value, JoyError> {
    let Value::Mapping(mut file) = file else {
        return Ok(file);
    };
    let kind = file.remove("type");
    let identity = file.remove("identity");
    let (key, mut fields) = match identity {
        Some(Value::Mapping(named)) => match named.into_iter().next() {
            Some((Value::String(key), Value::Mapping(fields))) => (key, fields),
            Some((Value::String(key), _)) => (key, Mapping::new()),
            _ => (String::new(), Mapping::new()),
        },
        _ => (String::new(), Mapping::new()),
    };
    let mut flat = Mapping::new();
    match kind.as_ref().and_then(Value::as_str) {
        Some("ai") => {
            flat.insert("name".into(), key.into());
        }
        Some("person") if key.contains('@') => {
            flat.insert("email".into(), key.into());
        }
        Some("person") => {}
        Some("platform") => {
            return Err(JoyError::Other(crate::migrations::needs_an_update(
                "A platform member",
            )))
        }
        other => {
            return Err(JoyError::Other(format!(
                "a member file says no type joy knows: {other:?}"
            )))
        }
    }
    for (old, new) in IDENTITY_KEYS {
        if let Some(value) = fields.remove(*new) {
            file.insert((*old).into(), value);
        }
    }
    for key in FLAT_KEYS {
        if let Some(value) = file.remove(*key) {
            flat.insert((*key).into(), value);
        }
    }
    for (key, value) in file {
        flat.insert(key, value);
    }
    Ok(Value::Mapping(flat))
}

/// Whether the project at `root` keeps member files of joy 0.23.
pub fn pending(root: &Path) -> bool {
    store::load_project(root).is_ok_and(|project| is_pending(&project))
}

fn is_pending(project: &Project) -> bool {
    project.member_layout() == MemberLayout::Files
        && project.member_file_shape() == MemberFileShape::Flat
        && project.has_members()
}

/// The project at `root` with its member files to be written in format 4,
/// when they wait for it and `signer_key` is a person of the project with
/// a key; `None` otherwise (an AI session never updates a project).
pub fn prepare(root: &Path, signer_key: &str) -> Result<Option<Project>, JoyError> {
    let mut project = store::load_project(root)?;
    if !is_pending(&project) {
        return Ok(None);
    }
    let a_person_with_a_key = project
        .member_by_key(signer_key)
        .is_some_and(|m| m.kind == MemberKind::Person && m.verify_key.is_some());
    if !a_person_with_a_key {
        return Ok(None);
    }
    project.set_member_file_shape(MemberFileShape::Identity);
    Ok(Some(project))
}

/// Write what [`prepare`] made: every member file in format 4, and
/// project.yaml saying so. Nothing else is touched.
pub fn finish(root: &Path, project: &Project) -> Result<(), JoyError> {
    store::save_project(root, project)
}

/// Rewrite the member files of the project at `root` in format 4, asked
/// by `signer_key`. `Ok(false)`: nothing waited, or they are no person of
/// the project with a key.
pub fn migrate(root: &Path, signer_key: &str) -> Result<bool, JoyError> {
    let Some(project) = prepare(root, signer_key)? else {
        return Ok(false);
    };
    finish(root, &project)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    fn text(value: &Value) -> String {
        serde_yaml_ng::to_string(value).unwrap()
    }

    const PERSON_023: &str = "\
email: alice@example.com
capabilities: all
verify_key: e7f6
kdf_nonce: e20c
seed_wrap_passphrase: 07a4
seed_wrap_recovery: a370
ai_delegations:
  claude:
    delegation_verifier: 5b23
    delegation_salt: '7071'
    created: 2026-04-21T15:59:22Z
updated: 2026-10-16T19:01:32Z
";

    const PERSON_4: &str = "\
type: person
identity:
  alice@example.com:
    verify_key: e7f6
    kdf_nonce: e20c
    seed_wrap: 07a4
    seed_wrap_recovery: a370
capabilities: all
ai_delegations:
  claude:
    delegation_verifier: 5b23
    delegation_salt: '7071'
    created: 2026-04-21T15:59:22Z
updated: 2026-10-16T19:01:32Z
";

    const ANONYMOUS_023: &str = "\
capabilities: all
verify_key: 961e
kdf_nonce: b835
seed_wrap_passphrase: af6d
seed_wrap_recovery: e071
email_match: bc68
members_wrap: 61d0f3
updated: 2026-10-18T06:47:08Z
";

    const ANONYMOUS_4: &str = "\
type: person
identity:
  m-mj2ytsnhhj:
    email_tag: bc68
    verify_key: 961e
    kdf_nonce: b835
    seed_wrap: af6d
    seed_wrap_recovery: e071
capabilities: all
members_wrap: 61d0f3
updated: 2026-10-18T06:47:08Z
";

    const AI_023: &str = "\
name: claude
adapter: claude
capabilities:
- plan
- implement
level: confirmed
verify_key: 3f9c
updated: 2026-10-16T19:01:32Z
";

    const AI_4: &str = "\
type: ai
identity:
  claude:
    verify_key: 3f9c
adapter: claude
capabilities:
- plan
- implement
level: confirmed
updated: 2026-10-16T19:01:32Z
";

    #[test]
    fn a_person_moves_under_their_address_with_the_new_names() {
        assert_eq!(text(&from_flat("m-aaaaaaaaaa", yaml(PERSON_023))), PERSON_4);
    }

    #[test]
    fn an_anonymous_person_goes_by_the_file_id() {
        assert_eq!(
            text(&from_flat("m-mj2ytsnhhj", yaml(ANONYMOUS_023))),
            ANONYMOUS_4
        );
    }

    #[test]
    fn an_ai_member_goes_by_its_name() {
        assert_eq!(text(&from_flat("m-bbbbbbbbbb", yaml(AI_023))), AI_4);
    }

    #[test]
    fn written_back_each_file_is_byte_for_byte_what_joy_0_23_wrote() {
        for (old, new) in [
            (PERSON_023, PERSON_4),
            (ANONYMOUS_023, ANONYMOUS_4),
            (AI_023, AI_4),
        ] {
            assert_eq!(text(&to_flat(yaml(new)).unwrap()), old);
        }
    }

    #[test]
    fn a_platform_member_has_no_shape_of_joy_0_23() {
        let platform = yaml("type: platform\nidentity:\n  joyint.com: {}\ncapabilities: all\n");
        let err = to_flat(platform).unwrap_err();
        assert!(err.to_string().contains("needs an update"), "{err}");
    }

    #[test]
    fn a_key_neither_shape_knows_is_carried_both_ways() {
        let old = format!("{PERSON_023}later: 1\n");
        let new = from_flat("m-aaaaaaaaaa", yaml(&old));
        assert_eq!(new.get("later"), Some(&Value::from(1)));
        assert_eq!(text(&to_flat(new).unwrap()), old);
    }

    #[test]
    fn the_kind_of_a_key_follows_the_rule_projects_were_written_with() {
        assert_eq!(kind_of_key("alice@example.com"), MemberKind::Person);
        assert_eq!(kind_of_key("m-mj2ytsnhhj"), MemberKind::Person);
        assert_eq!(kind_of_key("claude"), MemberKind::Ai);
        let older = crate::migrations::ai_member_name::legacy_form("claude");
        assert_eq!(kind_of_key(&older), MemberKind::Ai);
    }
}
