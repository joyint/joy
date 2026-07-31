// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Drop the stored `history` list.
//!
//! The audit trail lives in the event log alone — (timestamp, id,
//! action, actor), append-only, value-free (decision JOY-0175-9B); every
//! display derives the "Updated" trail from it at lookup time. Items
//! written while a stored list existed shed the key here on read, and
//! the file sheds it whenever the item is next saved.

use serde_yaml_ng::Value;

pub fn migrate(mut value: Value) -> (Value, bool) {
    let Some(map) = value.as_mapping_mut() else {
        return (value, false);
    };
    let removed = map.remove(Value::String("history".into())).is_some();
    (value, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_history_is_dropped_and_absence_is_a_no_op() {
        let v: Value = serde_yaml_ng::from_str(
            "id: T-1\ntitle: x\nhistory:\n- date: 2026-01-01T00:00:00Z\n  by: a@x\n",
        )
        .unwrap();
        let (out, changed) = migrate(v);
        assert!(changed);
        assert!(out.get("history").is_none());

        let (again, changed_again) = migrate(out);
        assert!(!changed_again);
        assert_eq!(again.get("title"), Some(&Value::String("x".into())));
    }
}
