// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Schema migrations for item YAML, mirroring [`super::project_yaml`]:
//! pure on-read transforms applied between YAML parse and typed
//! deserialization, so the [`crate::model::item::Item`] model stays
//! strict. Files persist in the current schema through the sync-time
//! repo migration; this layer also covers encrypted items, which no
//! filesystem migration can rewrite without their zone key.

mod m_2026_07_history_backfill;

use serde_yaml_ng::Value;

/// Apply every item migration in order. Returns the (possibly migrated)
/// value and whether anything changed.
pub fn apply(value: Value) -> (Value, bool) {
    let mut changed = false;
    let (value, c) = m_2026_07_history_backfill::migrate(value);
    changed |= c;
    (value, changed)
}
