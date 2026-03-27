// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::error::JoyError;
use crate::model::milestone::{milestone_filename, Milestone};
use crate::store;

/// Load all milestones from the .joy/milestones/ directory.
pub fn load_milestones(root: &Path) -> Result<Vec<Milestone>, JoyError> {
    let ms_dir = store::joy_dir(root).join(store::MILESTONES_DIR);
    if !ms_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut milestones = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&ms_dir)
        .map_err(|e| JoyError::ReadFile {
            path: ms_dir.clone(),
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
        let ms: Milestone = store::read_yaml(&entry.path())?;
        milestones.push(ms);
    }

    Ok(milestones)
}

/// Save a milestone to .joy/milestones/{ID}-{slug}.yaml.
pub fn save_milestone(root: &Path, ms: &Milestone) -> Result<(), JoyError> {
    let ms_dir = store::joy_dir(root).join(store::MILESTONES_DIR);
    let filename = milestone_filename(&ms.id, &ms.title);
    let path = ms_dir.join(&filename);
    store::write_yaml(&path, ms)?;
    let rel = format!("{}/{}/{}", store::JOY_DIR, store::MILESTONES_DIR, filename);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// Update a milestone in place (overwrites its file).
/// Removes the old file if the filename changed (e.g. title was edited).
pub fn update_milestone(root: &Path, ms: &Milestone) -> Result<(), JoyError> {
    let old_path = find_milestone_file(root, &ms.id)?;
    save_milestone(root, ms)?;
    let new_path = store::joy_dir(root)
        .join(store::MILESTONES_DIR)
        .join(milestone_filename(&ms.id, &ms.title));
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

/// Generate the next milestone ID by scanning existing files.
/// Returns "ACRONYM-MS-01" for the first milestone.
pub fn next_id(root: &Path, acronym: &str) -> Result<String, JoyError> {
    let prefix = format!("{acronym}-MS-");
    let ms_dir = store::joy_dir(root).join(store::MILESTONES_DIR);
    if !ms_dir.is_dir() {
        return Ok(format!("{prefix}01"));
    }

    let mut max_num: u8 = 0;

    let entries = std::fs::read_dir(&ms_dir).map_err(|e| JoyError::ReadFile {
        path: ms_dir.clone(),
        source: e,
    })?;

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(hex_part) = name.strip_prefix(&prefix) {
            if let Some(hex_str) = hex_part.get(..2) {
                if let Ok(num) = u8::from_str_radix(hex_str, 16) {
                    max_num = max_num.max(num);
                }
            }
        }
    }

    let next = max_num
        .checked_add(1)
        .ok_or_else(|| JoyError::Other(format!("{prefix} ID space exhausted (max {prefix}FF)")))?;
    Ok(format!("{prefix}{next:02X}"))
}

/// Find the file path for a milestone by its ID.
pub fn find_milestone_file(root: &Path, id: &str) -> Result<std::path::PathBuf, JoyError> {
    let ms_dir = store::joy_dir(root).join(store::MILESTONES_DIR);
    let prefix = format!("{}-", id);

    let entries = std::fs::read_dir(&ms_dir).map_err(|e| JoyError::ReadFile {
        path: ms_dir.clone(),
        source: e,
    })?;

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(&prefix) {
            return Ok(entry.path());
        }
    }

    Err(JoyError::MilestoneNotFound(id.to_string()))
}

/// Load a single milestone by ID.
pub fn load_milestone(root: &Path, id: &str) -> Result<Milestone, JoyError> {
    let path = find_milestone_file(root, id)?;
    store::read_yaml(&path)
}

/// Delete a milestone file.
pub fn delete_milestone(root: &Path, id: &str) -> Result<(), JoyError> {
    let path = find_milestone_file(root, id)?;
    let rel = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_string();
    std::fs::remove_file(&path).map_err(|e| JoyError::WriteFile { path, source: e })?;
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let joy_dir = dir.join(".joy");
        std::fs::create_dir_all(joy_dir.join("milestones")).unwrap();
    }

    #[test]
    fn next_id_first() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        assert_eq!(next_id(dir.path(), "JOY").unwrap(), "JOY-MS-01");
    }

    #[test]
    fn save_and_load() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let ms = Milestone::new("JOY-MS-01".into(), "Beta".into());
        save_milestone(dir.path(), &ms).unwrap();

        let loaded = load_milestone(dir.path(), "JOY-MS-01").unwrap();
        assert_eq!(loaded.title, "Beta");
    }

    #[test]
    fn next_id_increments() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let ms = Milestone::new("JOY-MS-01".into(), "First".into());
        save_milestone(dir.path(), &ms).unwrap();

        assert_eq!(next_id(dir.path(), "JOY").unwrap(), "JOY-MS-02");
    }

    #[test]
    fn update_removes_old_file_on_title_change() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let mut ms = Milestone::new("JOY-MS-01".into(), "Beta".into());
        save_milestone(dir.path(), &ms).unwrap();

        let old_path = find_milestone_file(dir.path(), "JOY-MS-01").unwrap();
        assert!(old_path.exists());

        ms.title = "Beta Release".into();
        update_milestone(dir.path(), &ms).unwrap();

        let new_path = find_milestone_file(dir.path(), "JOY-MS-01").unwrap();
        assert!(new_path.exists());
        assert!(!old_path.exists());
        assert_ne!(old_path, new_path);

        let loaded = load_milestone(dir.path(), "JOY-MS-01").unwrap();
        assert_eq!(loaded.title, "Beta Release");
    }

    #[test]
    fn delete_works() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let ms = Milestone::new("JOY-MS-01".into(), "Beta".into());
        save_milestone(dir.path(), &ms).unwrap();
        delete_milestone(dir.path(), "JOY-MS-01").unwrap();

        assert!(load_milestone(dir.path(), "JOY-MS-01").is_err());
    }
}
