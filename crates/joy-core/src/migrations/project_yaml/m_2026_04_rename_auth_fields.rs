// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! 2026-04 rename of auth schema field names per ADR-035.
//!
//! Field-level renames applied to each member entry in `project.yaml`:
//!
//! | Old           | New                   |
//! | ------------- | --------------------- |
//! | public_key    | verify_key            |
//! | salt          | kdf_nonce             |
//! | otp_hash      | enrollment_verifier   |
//!
//! And under `ai_delegations.<ai>`:
//!
//! | Old             | New                  |
//! | --------------- | -------------------- |
//! | delegation_key  | delegation_verifier  |
//!
//! `AttestationSignedFields.otp_hash` is intentionally **not** migrated.
//! Its serde key is pinned to the historical name in code so the
//! canonical bytes signed by existing attestations remain bit-identical
//! and signatures stay valid (per ADR-035).
//!
//! Scheduled for removal in v0.13. After removal, the read path will no
//! longer accept the legacy field names; users must run `joy auth update`
//! against v0.12 first.

use serde_yaml_ng::{Mapping, Value};

const MEMBER_FIELD_RENAMES: &[(&str, &str)] = &[
    ("public_key", "verify_key"),
    ("salt", "kdf_nonce"),
    ("otp_hash", "enrollment_verifier"),
];

/// Rename auth schema fields in-place on the given parsed YAML.
///
/// Returns the (possibly transformed) value plus a flag indicating
/// whether any rename was performed.
pub fn migrate(value: Value) -> (Value, bool) {
    let Value::Mapping(mut root) = value else {
        return (value, false);
    };

    let mut changed = false;

    if let Some(Value::Mapping(members)) = root.get_mut(Value::String("members".into())) {
        for (_email, member) in members.iter_mut() {
            if let Value::Mapping(member_map) = member {
                changed |= rename_member_fields(member_map);
                if let Some(Value::Mapping(delegations)) =
                    member_map.get_mut(Value::String("ai_delegations".into()))
                {
                    for (_ai, entry) in delegations.iter_mut() {
                        if let Value::Mapping(entry_map) = entry {
                            changed |= rename_delegation_field(entry_map);
                        }
                    }
                }
            }
        }
    }

    (Value::Mapping(root), changed)
}

fn rename_member_fields(member_map: &mut Mapping) -> bool {
    let mut changed = false;
    for (old, new) in MEMBER_FIELD_RENAMES {
        if let Some(v) = member_map.remove(Value::String((*old).into())) {
            member_map.insert(Value::String((*new).into()), v);
            changed = true;
        }
    }
    changed
}

fn rename_delegation_field(entry_map: &mut Mapping) -> bool {
    if let Some(v) = entry_map.remove(Value::String("delegation_key".into())) {
        entry_map.insert(Value::String("delegation_verifier".into()), v);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Value {
        serde_yaml_ng::from_str(yaml).unwrap()
    }

    #[test]
    fn renames_all_four_member_fields() {
        let yaml = r#"
members:
  alice@example.com:
    capabilities: all
    public_key: aa
    salt: bb
    otp_hash: cc
    ai_delegations:
      ai:claude@joy:
        delegation_key: dd
        created: 2026-04-15T10:00:00Z
"#;
        let (out, changed) = migrate(parse(yaml));
        assert!(changed);
        let alice = out
            .as_mapping()
            .unwrap()
            .get(Value::String("members".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("alice@example.com".into()))
            .unwrap();
        let alice_map = alice.as_mapping().unwrap();
        assert!(alice_map.contains_key(Value::String("verify_key".into())));
        assert!(alice_map.contains_key(Value::String("kdf_nonce".into())));
        assert!(alice_map.contains_key(Value::String("enrollment_verifier".into())));
        assert!(!alice_map.contains_key(Value::String("public_key".into())));
        assert!(!alice_map.contains_key(Value::String("salt".into())));
        assert!(!alice_map.contains_key(Value::String("otp_hash".into())));

        let delegations = alice_map
            .get(Value::String("ai_delegations".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        let claude = delegations
            .get(Value::String("ai:claude@joy".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert!(claude.contains_key(Value::String("delegation_verifier".into())));
        assert!(!claude.contains_key(Value::String("delegation_key".into())));
    }

    #[test]
    fn idempotent_when_already_migrated() {
        let yaml = r#"
members:
  alice@example.com:
    capabilities: all
    verify_key: aa
    kdf_nonce: bb
"#;
        let parsed = parse(yaml);
        let (out, changed) = migrate(parsed.clone());
        assert!(!changed);
        assert_eq!(out, parsed);
    }

    #[test]
    fn handles_mixed_legacy_and_new_fields() {
        // Member that already has verify_key but still has legacy salt.
        let yaml = r#"
members:
  alice@example.com:
    capabilities: all
    verify_key: aa
    salt: bb
"#;
        let (out, changed) = migrate(parse(yaml));
        assert!(changed);
        let alice_map = out
            .as_mapping()
            .unwrap()
            .get(Value::String("members".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("alice@example.com".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert!(alice_map.contains_key(Value::String("verify_key".into())));
        assert!(alice_map.contains_key(Value::String("kdf_nonce".into())));
        assert!(!alice_map.contains_key(Value::String("salt".into())));
    }

    #[test]
    fn does_not_touch_attestation_otp_hash() {
        // The historical otp_hash key inside attestation.signed_fields must
        // remain intact - the canonical-form freeze in AttestationSignedFields
        // depends on it.
        let yaml = r#"
members:
  alice@example.com:
    capabilities: all
    public_key: aa
    attestation:
      attester: horst@example.com
      signed_fields:
        email: alice@example.com
        capabilities: all
        otp_hash: ff
      signed_at: 2026-04-20T10:00:00Z
      signature: aa
"#;
        let (out, _changed) = migrate(parse(yaml));
        let alice_map = out
            .as_mapping()
            .unwrap()
            .get(Value::String("members".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("alice@example.com".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        let signed_fields = alice_map
            .get(Value::String("attestation".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("signed_fields".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert!(signed_fields.contains_key(Value::String("otp_hash".into())));
        assert!(!signed_fields.contains_key(Value::String("enrollment_verifier".into())));
    }

    #[test]
    fn returns_value_unchanged_when_no_members_section() {
        let yaml = r#"
name: empty
created: 2026-01-01T00:00:00Z
"#;
        let parsed = parse(yaml);
        let (out, changed) = migrate(parsed.clone());
        assert!(!changed);
        assert_eq!(out, parsed);
    }
}
