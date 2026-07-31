// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Schema migrations for item YAML, mirroring [`super::project_yaml`]:
//! pure on-read transforms applied between YAML parse and the strict
//! typed model. The migrated form persists when the item is next SAVED
//! (`items::save_item` serializes the current schema) — never as a mass
//! rewrite. Removing a migration once its window closes is one step:
//! delete the module file and its line in [`apply`].

mod m_2026_07_drop_stored_history;

use serde_yaml_ng::Value;

/// Apply every item migration in order. Returns the (possibly migrated)
/// value and whether anything changed.
pub fn apply(value: Value) -> (Value, bool) {
    let mut changed = false;
    let (value, c) = m_2026_07_drop_stored_history::migrate(value);
    changed |= c;
    (value, changed)
}
