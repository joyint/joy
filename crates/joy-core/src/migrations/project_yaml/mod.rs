// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Migrations applied to project.yaml on read.
//!
//! Adding a migration: drop a new `m_<yyyy_mm>_<slug>.rs` next to this
//! file and chain its `migrate` call inside [`apply`]. Removing a
//! migration after its deprecation window: delete the file and the chain
//! entry. No struct-level changes required.

mod m_2026_04_rename_auth_fields;

use serde_yaml_ng::Value;

/// Run every project.yaml migration in order.
///
/// Returns the (possibly transformed) value plus a flag indicating
/// whether any migration produced a change. Callers use the flag to
/// decide whether to emit a deprecation warning.
pub fn apply(value: Value) -> (Value, bool) {
    let mut value = value;
    let mut changed = false;

    let (v, c) = m_2026_04_rename_auth_fields::migrate(value);
    value = v;
    changed |= c;

    (value, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_is_idempotent_on_already_migrated_yaml() {
        let yaml = r#"
members:
  alice@example.com:
    capabilities: all
    verify_key: aa
    kdf_nonce: bb
    enrollment_verifier: cc
    ai_delegations:
      ai:claude@joy:
        delegation_verifier: dd
        created: 2026-04-15T10:00:00Z
"#;
        let value: Value = serde_yaml_ng::from_str(yaml).unwrap();
        let (out, changed) = apply(value.clone());
        assert!(!changed, "no migration should run on current schema");
        assert_eq!(out, value);
    }

    #[test]
    fn apply_runs_on_legacy_yaml() {
        let yaml = r#"
members:
  alice@example.com:
    capabilities: all
    public_key: aa
    salt: bb
"#;
        let value: Value = serde_yaml_ng::from_str(yaml).unwrap();
        let (_out, changed) = apply(value);
        assert!(changed, "legacy schema should trigger migration");
    }
}
