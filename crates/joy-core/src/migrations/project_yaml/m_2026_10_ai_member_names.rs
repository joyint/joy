// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! AI members by their names in project.yaml (JI-019D-46).
//!
//! A project written before AI members were known by their names keys
//! them in the legacy form: in its member map, in each person's
//! delegations and in the zone wraps of the crypt section. Every one of
//! those keys is read through the one rule
//! ([`crate::migrations::ai_member_name`]), first of all migrations, so
//! that the ones after it and the model see names only.
//!
//! Nothing is rewritten for it: the file is read this way for as long as
//! it stands, and the next regular write carries the names.

use serde_yaml_ng::{Mapping, Value};

use crate::migrations::ai_member_name;

/// The mapping with every key read as a member. Where one AI member
/// stands in it in both forms, the entry under the name wins.
fn by_name(map: &mut Mapping) -> bool {
    let legacy: Vec<Value> = map
        .keys()
        .filter(|key| key.as_str().is_some_and(ai_member_name::is_legacy))
        .cloned()
        .collect();
    for key in &legacy {
        let Some(value) = map.remove(key) else {
            continue;
        };
        let name =
            Value::String(ai_member_name::read(key.as_str().unwrap_or_default()).into_owned());
        if !map.contains_key(&name) {
            map.insert(name, value);
        }
    }
    !legacy.is_empty()
}

pub fn migrate(mut value: Value) -> (Value, bool) {
    let mut changed = false;
    if let Some(members) = value.get_mut("members").and_then(Value::as_mapping_mut) {
        changed |= by_name(members);
        for (_, member) in members.iter_mut() {
            if let Some(delegations) = member
                .get_mut("ai_delegations")
                .and_then(Value::as_mapping_mut)
            {
                changed |= by_name(delegations);
            }
        }
    }
    if let Some(wraps) = value
        .get_mut("crypt")
        .and_then(|crypt| crypt.get_mut("delegations"))
        .and_then(Value::as_mapping_mut)
    {
        changed |= by_name(wraps);
    }
    (value, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_place_that_keys_an_ai_member_reads_as_its_name() {
        let legacy = ai_member_name::legacy_form("claude");
        let yaml = format!(
            "members:\n  {legacy}:\n    capabilities: all\n  horst@example.com:\n    capabilities: all\n    ai_delegations:\n      {legacy}:\n        delegation_verifier: aa\ncrypt:\n  delegations:\n    {legacy}:\n      horst@example.com: bb\n"
        );
        let (out, changed) = migrate(serde_yaml_ng::from_str(&yaml).unwrap());
        assert!(changed);
        let written = serde_yaml_ng::to_string(&out).unwrap();
        assert!(!written.contains(&legacy), "{written}");
        let members = out.get("members").unwrap();
        assert!(members.get("claude").is_some());
        assert!(members
            .get("horst@example.com")
            .and_then(|m| m.get("ai_delegations"))
            .and_then(|d| d.get("claude"))
            .is_some());
        assert!(out
            .get("crypt")
            .and_then(|c| c.get("delegations"))
            .and_then(|d| d.get("claude"))
            .is_some());
    }

    #[test]
    fn a_project_that_names_its_ai_members_is_left_alone() {
        let yaml = "members:\n  claude:\n    capabilities: all\n";
        let (_, changed) = migrate(serde_yaml_ng::from_str(yaml).unwrap());
        assert!(!changed);
    }

    #[test]
    fn where_both_forms_stand_the_name_wins() {
        let legacy = ai_member_name::legacy_form("vibe");
        let yaml = format!("members:\n  {legacy}:\n    adapter: then\n  vibe:\n    adapter: now\n");
        let (out, _) = migrate(serde_yaml_ng::from_str(&yaml).unwrap());
        let members = out.get("members").unwrap().as_mapping().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members.get("vibe").unwrap().get("adapter").unwrap(), "now");
    }
}
