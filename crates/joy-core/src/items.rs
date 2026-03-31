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
    let path = items_dir.join(&filename);
    store::write_yaml(&path, item)?;
    let rel = format!("{}/{}/{}", store::JOY_DIR, store::ITEMS_DIR, filename);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// Generate the next item ID by scanning existing files.
/// Returns "ACRONYM-0001" for the first item, increments the highest found.
/// All items share one number space regardless of type.
///
/// Legacy format (existing items): ACRONYM-XXXX (4 hex digits)
/// New format (ADR-027): ACRONYM-XXXX-YY (4 hex digits + 2 hex title hash)
pub fn next_id(root: &Path, acronym: &str, title: &str) -> Result<String, JoyError> {
    let prefix = acronym;

    let items_dir = store::joy_dir(root).join(store::ITEMS_DIR);
    if !items_dir.is_dir() {
        let suffix = title_hash_suffix(title);
        return Ok(format!("{prefix}-0001-{suffix}"));
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
    let suffix = title_hash_suffix(title);
    Ok(format!("{prefix}-{next:04X}-{suffix}"))
}

/// Generate 2 hex digits from the title for collision-safe IDs (ADR-027).
pub fn title_hash_suffix(title: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    let hash = hasher.finalize();
    format!("{:02X}", hash[0])
}

/// Find the file path for an item by its ID.
/// Accepts both full IDs (JOY-0042-A3) and short-form (JOY-0042).
/// Short-form returns an error if ambiguous (multiple matches).
pub fn find_item_file(root: &Path, id: &str) -> Result<std::path::PathBuf, JoyError> {
    let items_dir = store::joy_dir(root).join(store::ITEMS_DIR);

    // Normalize: uppercase the ID for matching
    let id_upper = id.to_uppercase();

    let entries: Vec<_> = std::fs::read_dir(&items_dir)
        .map_err(|e| JoyError::ReadFile {
            path: items_dir.clone(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .collect();

    // First try exact match (full ID)
    let exact_prefix = format!("{}-", id_upper);
    for entry in &entries {
        let name = entry.file_name();
        let name_upper = name.to_string_lossy().to_uppercase();
        if name_upper.starts_with(&exact_prefix) {
            return Ok(entry.path());
        }
    }

    // Then try short-form match (prefix without suffix)
    // JOY-0042 matches JOY-0042-A3-some-title.yaml
    let short_prefix = format!("{}-", id_upper);
    let mut matches: Vec<std::path::PathBuf> = Vec::new();
    for entry in &entries {
        let name = entry.file_name();
        let name_upper = name.to_string_lossy().to_uppercase();
        if name_upper.starts_with(&short_prefix) {
            matches.push(entry.path());
        }
    }

    match matches.len() {
        0 => Err(JoyError::ItemNotFound(id.to_string())),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            // Extract full IDs from filenames for the error message
            let ids: Vec<String> = matches
                .iter()
                .filter_map(|p| {
                    let name = p.file_name()?.to_string_lossy().to_string();
                    extract_full_id(&name)
                })
                .collect();
            Err(JoyError::Other(format!("ambiguous ID: {}", ids.join(", "))))
        }
    }
}

/// Extract the full item ID from a filename.
/// "JOY-0042-A3-fix-login.yaml" -> "JOY-0042-A3"
/// "JOY-0042-fix-login.yaml" -> "JOY-0042" (legacy)
fn extract_full_id(filename: &str) -> Option<String> {
    // Strip .yaml extension
    let name = filename
        .strip_suffix(".yaml")
        .or_else(|| filename.strip_suffix(".yml"))?;
    // Find acronym-XXXX pattern
    let parts: Vec<&str> = name.splitn(2, '-').collect();
    if parts.len() < 2 {
        return None;
    }
    let acronym = parts[0];
    let rest = parts[1];

    // Check if it's new format: XXXX-YY-slug or legacy: XXXX-slug
    if rest.len() >= 7 && rest.as_bytes()[4] == b'-' {
        // Could be XXXX-YY-slug (new) or XXXX-slug with short slug
        let hex4 = &rest[..4];
        let maybe_suffix = &rest[5..7];
        if u16::from_str_radix(hex4, 16).is_ok()
            && maybe_suffix.len() == 2
            && u8::from_str_radix(maybe_suffix, 16).is_ok()
            && (rest.len() == 7 || rest.as_bytes()[7] == b'-')
        {
            return Some(format!("{}-{}-{}", acronym, hex4, maybe_suffix).to_uppercase());
        }
    }

    // Legacy format: XXXX-slug
    let hex4 = &rest[..4.min(rest.len())];
    if hex4.len() == 4 && u16::from_str_radix(hex4, 16).is_ok() {
        return Some(format!("{}-{}", acronym, hex4).to_uppercase());
    }

    None
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
    let rel = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_string();
    std::fs::remove_file(&path).map_err(|e| JoyError::WriteFile { path, source: e })?;
    crate::git_ops::auto_git_add(root, &[&rel]);
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
        let old_rel = old_path
            .strip_prefix(root)
            .unwrap_or(&old_path)
            .to_string_lossy()
            .to_string();
        crate::git_ops::auto_git_add(root, &[&old_rel]);
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
        let id = next_id(dir.path(), "JOY", "Test item").unwrap();
        assert!(id.starts_with("JOY-0001-"), "got: {id}");
        assert_eq!(id.len(), 11); // JOY-0001-XX
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
            vec![],
        );
        save_item(dir.path(), &item).unwrap();

        let id = next_id(dir.path(), "JOY", "Second item").unwrap();
        assert!(id.starts_with("JOY-0002-"), "got: {id}");
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
            vec![],
        );
        save_item(dir.path(), &item1).unwrap();

        let item3 = Item::new(
            "JOY-0003".into(),
            "Third".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item3).unwrap();

        let id = next_id(dir.path(), "JOY", "Fourth item").unwrap();
        assert!(id.starts_with("JOY-0004-"), "got: {id}");
    }

    #[test]
    fn next_id_same_title_same_suffix() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let id1 = next_id(dir.path(), "JOY", "Same title").unwrap();
        let suffix1 = &id1[9..];
        let id2_suffix = title_hash_suffix("Same title");
        assert_eq!(suffix1, id2_suffix);
    }

    #[test]
    fn next_id_different_titles_different_suffixes() {
        let suffix_a = title_hash_suffix("Fix login bug");
        let suffix_b = title_hash_suffix("Add roadmap feature");
        // Not guaranteed different, but astronomically unlikely to be equal
        // for these specific strings. If this test fails, the hash function
        // has a collision on these inputs (1:256 chance).
        assert_ne!(suffix_a, suffix_b);
    }

    #[test]
    fn next_id_increments_past_new_format() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        // Save an item with new format ID
        let item = Item::new(
            "JOY-0005-A3".into(),
            "New format".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item).unwrap();

        let id = next_id(dir.path(), "JOY", "Next item").unwrap();
        assert!(id.starts_with("JOY-0006-"), "got: {id}");
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
            vec![],
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
            vec![],
        );
        save_item(dir.path(), &item2).unwrap();

        let item1 = Item::new(
            "JOY-0001".into(),
            "First".into(),
            ItemType::Task,
            Priority::Low,
            vec![],
        );
        save_item(dir.path(), &item1).unwrap();

        let items = load_items(dir.path()).unwrap();
        assert_eq!(items[0].id, "JOY-0001");
        assert_eq!(items[1].id, "JOY-0002");
    }
}
