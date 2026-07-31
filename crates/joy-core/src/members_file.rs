// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Encrypted `members.yaml` (ADR-042 anonymous mode).
//!
//! Maps each opaque member id (`m-<short>`) to its human-readable resolution
//! data: e-mail and optional display name. On disk the file exists only as a
//! `JOYCRYPT` blob, encrypted under a dedicated Crypt zone whose key is wrapped
//! per member against their `verify_key` (the same machinery as any Crypt zone,
//! ADR-038 / ADR-039). Plaintext member e-mail therefore never hits disk.
//!
//! `name` is optional and not populated yet (the first cut sources nothing into
//! it); display degrades to the e-mail when it is absent. Adding a name source
//! later is purely additive: populate the field, no format change.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::JoyError;
use crate::store;
use joy_crypt::zone::{decrypt_blob, encrypt_blob, ZoneKey};

/// Reserved Crypt zone for the members file. The double underscores keep it out
/// of the user-facing `joy crypt` zone namespace.
pub const MEMBERS_ZONE: &str = "__members__";
/// On-disk filename, under `.joy/`.
pub const MEMBERS_FILE: &str = "members.yaml";

/// Decrypted contents of `members.yaml`: opaque id -> resolution data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MembersFile {
    #[serde(default)]
    pub members: BTreeMap<String, MemberInfo>,
}

/// Per-member human-readable data. `name` is optional; display falls back to
/// the e-mail when it is absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl MembersFile {
    /// The e-mail for an opaque id, if present.
    pub fn email_for(&self, id: &str) -> Option<&str> {
        self.members.get(id).map(|m| m.email.as_str())
    }

    /// Display string for an opaque id: name if set, else e-mail. `None` when
    /// the id is not in the file (the caller then keeps showing nothing rather
    /// than a raw id, or requests authentication).
    pub fn display_for(&self, id: &str) -> Option<String> {
        self.members
            .get(id)
            .map(|m| m.name.clone().unwrap_or_else(|| m.email.clone()))
    }
}

/// Path to the (encrypted) `members.yaml` under `.joy/`.
pub fn members_path(root: &Path) -> PathBuf {
    store::joy_dir(root).join(MEMBERS_FILE)
}

/// Whether an (encrypted) `members.yaml` exists on disk.
pub fn exists(root: &Path) -> bool {
    members_path(root).exists()
}

/// Decrypt and parse `members.yaml` using the members-zone key.
pub fn read(root: &Path, zone_key: &ZoneKey) -> Result<MembersFile, JoyError> {
    let blob = std::fs::read(members_path(root))
        .map_err(|e| JoyError::Other(format!("read members.yaml: {e}")))?;
    let (_zone, plain) = decrypt_blob(
        |z| {
            if z == MEMBERS_ZONE {
                Some(ZoneKey::from_bytes(*zone_key.as_bytes()))
            } else {
                None
            }
        },
        &blob,
    )?;
    let text = String::from_utf8(plain)
        .map_err(|_| JoyError::Other("members.yaml is not valid UTF-8".into()))?;
    let mf: MembersFile = serde_yaml_ng::from_str(&text)?;
    Ok(mf)
}

/// Serialize, encrypt, and write `members.yaml` with the members-zone key.
pub fn write(root: &Path, zone_key: &ZoneKey, mf: &MembersFile) -> Result<(), JoyError> {
    let yaml = serde_yaml_ng::to_string(mf)?;
    let blob = encrypt_blob(MEMBERS_ZONE, zone_key, yaml.as_bytes());
    std::fs::write(members_path(root), blob)
        .map_err(|e| JoyError::Other(format!("write members.yaml: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use joy_crypt::zone::looks_like_blob;

    fn info(email: &str, name: Option<&str>) -> MemberInfo {
        MemberInfo {
            email: email.to_string(),
            name: name.map(str::to_string),
        }
    }

    #[test]
    fn roundtrip_is_encrypted_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(store::joy_dir(dir.path())).unwrap();
        let zk = ZoneKey::generate();

        let mut mf = MembersFile::default();
        mf.members
            .insert("m-aaaa".into(), info("horst@joydev.com", None));
        mf.members.insert(
            "m-bbbb".into(),
            info("geordi@example.org", Some("Geordi LaForge")),
        );
        write(dir.path(), &zk, &mf).unwrap();

        let raw = std::fs::read(members_path(dir.path())).unwrap();
        assert!(
            looks_like_blob(&raw),
            "members.yaml must be a JOYCRYPT blob"
        );
        assert!(
            !String::from_utf8_lossy(&raw).contains("horst@joydev.com"),
            "e-mail must not appear in the on-disk blob"
        );

        let back = read(dir.path(), &zk).unwrap();
        assert_eq!(back, mf);
    }

    #[test]
    fn wrong_key_fails_to_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(store::joy_dir(dir.path())).unwrap();
        let zk = ZoneKey::generate();
        write(dir.path(), &zk, &MembersFile::default()).unwrap();
        let other = ZoneKey::generate();
        assert!(read(dir.path(), &other).is_err());
    }

    #[test]
    fn display_prefers_name_then_email() {
        let mut mf = MembersFile::default();
        mf.members
            .insert("m-1".into(), info("a@x.com", Some("Alice")));
        mf.members.insert("m-2".into(), info("b@x.com", None));
        assert_eq!(mf.display_for("m-1").as_deref(), Some("Alice"));
        assert_eq!(mf.display_for("m-2").as_deref(), Some("b@x.com"));
        assert_eq!(mf.email_for("m-1"), Some("a@x.com"));
        assert_eq!(mf.display_for("m-unknown"), None);
    }
}
