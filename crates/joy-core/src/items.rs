// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::error::JoyError;
use crate::model::item::{item_filename, Item};
use crate::store;

/// Load all items from the .joy/items/ directory.
pub fn load_items(root: &Path) -> Result<Vec<Item>, JoyError> {
    let items_dir = store::joy_dir(root).join(store::ITEMS_DIR);
    if !items_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&items_dir)
        .map_err(|e| JoyError::ReadFile {
            path: items_dir.clone(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let item: Item = store::read_yaml(&entry.path())?;
        items.push(item);
    }

    Ok(items)
}

/// Save an item to .joy/items/{ID}-{slug}.yaml.
pub fn save_item(root: &Path, item: &Item) -> Result<(), JoyError> {
    let items_dir = store::joy_dir(root).join(store::ITEMS_DIR);
    let filename = item_filename(&item.id, &item.title);
    let path = items_dir.join(filename);
    store::write_yaml(&path, item)
}

/// Generate the next item ID by scanning existing files.
/// Returns "ACRONYM-0001" for the first item, increments the highest found.
/// All items share one number space regardless of type.
pub fn next_id(root: &Path, acronym: &str) -> Result<String, JoyError> {
    let prefix = acronym;

    let items_dir = store::joy_dir(root).join(store::ITEMS_DIR);
    if !items_dir.is_dir() {
        return Ok(format!("{prefix}-0001"));
    }

    let mut max_num: u16 = 0;

    let entries = std::fs::read_dir(&items_dir).map_err(|e| JoyError::ReadFile {
        path: items_dir.clone(),
        source: e,
    })?;

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(hex_part) = name.strip_prefix(&format!("{prefix}-")) {
            if let Some(hex_str) = hex_part.get(..4) {
                if let Ok(num) = u16::from_str_radix(hex_str, 16) {
                    max_num = max_num.max(num);
                }
            }
        }
    }

    let next = max_num.checked_add(1).ok_or_else(|| {
        JoyError::Other(format!("{prefix} ID space exhausted (max {prefix}-FFFF)"))
    })?;
    Ok(format!("{prefix}-{next:04X}"))
}

/// Find the file path for an item by its ID.
pub fn find_item_file(root: &Path, id: &str) -> Result<std::path::PathBuf, JoyError> {
    let items_dir = store::joy_dir(root).join(store::ITEMS_DIR);
    let prefix = format!("{}-", id);

    let entries = std::fs::read_dir(&items_dir).map_err(|e| JoyError::ReadFile {
        path: items_dir.clone(),
        source: e,
    })?;

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(&prefix) {
            return Ok(entry.path());
        }
    }

    Err(JoyError::ItemNotFound(id.to_string()))
}

/// Load a single item by ID.
pub fn load_item(root: &Path, id: &str) -> Result<Item, JoyError> {
    let path = find_item_file(root, id)?;
    store::read_yaml(&path)
}

/// Delete an item by ID. Returns the deleted item.
pub fn delete_item(root: &Path, id: &str) -> Result<Item, JoyError> {
    let path = find_item_file(root, id)?;
    let item: Item = store::read_yaml(&path)?;
    std::fs::remove_file(&path).map_err(|e| JoyError::WriteFile { path, source: e })?;
    Ok(item)
}

/// Remove references to a deleted item from other items' deps and parent fields.
pub fn remove_references(root: &Path, deleted_id: &str) -> Result<Vec<String>, JoyError> {
    let items = load_items(root)?;
    let mut updated = Vec::new();
    for mut item in items {
        let mut changed = false;
        if item.deps.contains(&deleted_id.to_string()) {
            item.deps.retain(|d| d != deleted_id);
            changed = true;
        }
        if item.parent.as_deref() == Some(deleted_id) {
            item.parent = None;
            changed = true;
        }
        if changed {
            item.updated = chrono::Utc::now();
            update_item(root, &item)?;
            updated.push(item.id.clone());
        }
    }
    Ok(updated)
}

/// Check if adding a dependency would create a cycle.
/// Returns the cycle path if one exists.
pub fn detect_cycle(
    root: &Path,
    item_id: &str,
    new_dep_id: &str,
) -> Result<Option<Vec<String>>, JoyError> {
    let items = load_items(root)?;
    let mut visited = vec![item_id.to_string()];
    if find_cycle(&items, new_dep_id, &mut visited) {
        visited.push(new_dep_id.to_string());
        Ok(Some(visited))
    } else {
        Ok(None)
    }
}

fn find_cycle(items: &[Item], current: &str, visited: &mut Vec<String>) -> bool {
    if visited.contains(&current.to_string()) {
        return true;
    }
    if let Some(item) = items.iter().find(|i| i.id == current) {
        visited.push(current.to_string());
        for dep in &item.deps {
            if find_cycle(items, dep, visited) {
                return true;
            }
        }
        visited.pop();
    }
    false
}

/// Update an item in place (overwrites its file).
pub fn update_item(root: &Path, item: &Item) -> Result<(), JoyError> {
    let old_path = find_item_file(root, &item.id)?;
    // Write new file first to avoid data loss if write fails
    save_item(root, item)?;
    // Remove old file if the filename changed (title may have changed)
    let new_path = store::joy_dir(root)
        .join(store::ITEMS_DIR)
        .join(item_filename(&item.id, &item.title));
    if old_path != new_path {
        let _ = std::fs::remove_file(&old_path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::item::{ItemType, Priority};
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let joy_dir = dir.join(".joy");
        std::fs::create_dir_all(joy_dir.join("items")).unwrap();
    }

    #[test]
    fn next_id_first_item() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        assert_eq!(next_id(dir.path(), "JOY").unwrap(), "JOY-0001");
    }

    #[test]
    fn next_id_increments() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item = Item::new(
            "JOY-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item).unwrap();

        assert_eq!(next_id(dir.path(), "JOY").unwrap(), "JOY-0002");
    }

    #[test]
    fn next_id_skips_gaps() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item1 = Item::new(
            "JOY-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item1).unwrap();

        let item3 = Item::new(
            "JOY-0003".into(),
            "Third".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item3).unwrap();

        assert_eq!(next_id(dir.path(), "JOY").unwrap(), "JOY-0004");
    }

    #[test]
    fn load_items_empty() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let items = load_items(dir.path()).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn save_and_load_item() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item = Item::new(
            "JOY-0001".into(),
            "Test item".into(),
            ItemType::Story,
            Priority::High,
        );
        save_item(dir.path(), &item).unwrap();

        let items = load_items(dir.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "JOY-0001");
        assert_eq!(items[0].title, "Test item");
    }

    #[test]
    fn load_items_sorted() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item2 = Item::new(
            "JOY-0002".into(),
            "Second".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item2).unwrap();

        let item1 = Item::new(
            "JOY-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item1).unwrap();

        let items = load_items(dir.path()).unwrap();
        assert_eq!(items[0].id, "JOY-0001");
        assert_eq!(items[1].id, "JOY-0002");
    }
}
