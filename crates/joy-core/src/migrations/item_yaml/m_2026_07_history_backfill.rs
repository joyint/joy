// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Give every item the `history` audit list.
//!
//! Items written before the audit footer shipped carry no `history` key;
//! the model requires one (an item without an audit list is not a valid
//! product item). An absent key becomes the empty list: the audit trail
//! of such an item genuinely starts empty, its earlier mutations were
//! never recorded anywhere.

use serde_yaml_ng::Value;

pub fn migrate(mut value: Value) -> (Value, bool) {
    let Some(map) = value.as_mapping_mut() else {
        return (value, false);
    };
    let key = Value::String("history".into());
    if map.contains_key(&key) {
        return (value, false);
    }
    map.insert(key, Value::Sequence(Vec::new()));
    (value, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_history_becomes_the_empty_list_and_is_idempotent() {
        let v: Value = serde_yaml_ng::from_str("id: T-1\ntitle: x\n").unwrap();
        let (out, changed) = migrate(v);
        assert!(changed);
        assert_eq!(out.get("history"), Some(&Value::Sequence(Vec::new())));
        let (_again, changed_again) = migrate(out);
        assert!(!changed_again);
    }

    #[test]
    fn a_recorded_history_is_untouched() {
        let v: Value =
            serde_yaml_ng::from_str("id: T-1\nhistory:\n- date: 2026-01-01T00:00:00Z\n  by: a\n")
                .unwrap();
        let (out, changed) = migrate(v.clone());
        assert!(!changed);
        assert_eq!(out, v);
    }
}
