// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::error::JoyError;
use crate::model::item::{item_filename, Item, ItemType};
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
/// Returns "IT-0001" for the first item, or increments the highest found.
/// For epics, returns "EP-XXXX".
pub fn next_id(root: &Path, item_type: &ItemType) -> Result<String, JoyError> {
    let prefix = match item_type {
        ItemType::Epic => "EP",
        _ => "IT",
    };

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
        // Match files starting with the prefix (e.g. "IT-" or "EP-")
        if let Some(hex_part) = name.strip_prefix(&format!("{prefix}-")) {
            if let Some(hex_str) = hex_part.get(..4) {
                if let Ok(num) = u16::from_str_radix(hex_str, 16) {
                    max_num = max_num.max(num);
                }
            }
        }
    }

    Ok(format!("{prefix}-{:04X}", max_num + 1))
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

/// Update an item in place (overwrites its file).
pub fn update_item(root: &Path, item: &Item) -> Result<(), JoyError> {
    // Remove old file (title may have changed, altering the filename)
    let old_path = find_item_file(root, &item.id)?;
    std::fs::remove_file(&old_path).map_err(|e| JoyError::WriteFile {
        path: old_path,
        source: e,
    })?;
    save_item(root, item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::item::Priority;
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let joy_dir = dir.join(".joy");
        std::fs::create_dir_all(joy_dir.join("items")).unwrap();
    }

    #[test]
    fn next_id_first_item() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        assert_eq!(next_id(dir.path(), &ItemType::Task).unwrap(), "IT-0001");
    }

    #[test]
    fn next_id_first_epic() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        assert_eq!(next_id(dir.path(), &ItemType::Epic).unwrap(), "EP-0001");
    }

    #[test]
    fn next_id_increments() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item = Item::new(
            "IT-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item).unwrap();

        assert_eq!(next_id(dir.path(), &ItemType::Task).unwrap(), "IT-0002");
    }

    #[test]
    fn next_id_skips_gaps() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item1 = Item::new(
            "IT-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item1).unwrap();

        let item3 = Item::new(
            "IT-0003".into(),
            "Third".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item3).unwrap();

        assert_eq!(next_id(dir.path(), &ItemType::Task).unwrap(), "IT-0004");
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
            "IT-0001".into(),
            "Test item".into(),
            ItemType::Story,
            Priority::High,
        );
        save_item(dir.path(), &item).unwrap();

        let items = load_items(dir.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "IT-0001");
        assert_eq!(items[0].title, "Test item");
    }

    #[test]
    fn load_items_sorted() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let item2 = Item::new(
            "IT-0002".into(),
            "Second".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item2).unwrap();

        let item1 = Item::new(
            "IT-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
        );
        save_item(dir.path(), &item1).unwrap();

        let items = load_items(dir.path()).unwrap();
        assert_eq!(items[0].id, "IT-0001");
        assert_eq!(items[1].id, "IT-0002");
    }
}
