// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;

use crate::error::JoyError;
use crate::model::release::Release;
use crate::store;

/// Save a release to .joy/releases/ACRONYM-vX.Y.Z.yaml.
pub fn save_release(root: &Path, acronym: &str, release: &Release) -> Result<(), JoyError> {
    let releases_dir = store::joy_dir(root).join(store::RELEASES_DIR);
    fs::create_dir_all(&releases_dir).map_err(|e| JoyError::CreateDir {
        path: releases_dir.clone(),
        source: e,
    })?;

    let filename = format!("{}-{}.yaml", acronym, release.version);
    let path = releases_dir.join(&filename);
    store::write_yaml(&path, release)?;
    let rel = format!("{}/{}/{}", store::JOY_DIR, store::RELEASES_DIR, filename);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// Load a specific release by version.
pub fn load_release(root: &Path, acronym: &str, version: &str) -> Result<Release, JoyError> {
    let version = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    let releases_dir = store::joy_dir(root).join(store::RELEASES_DIR);
    let filename = format!("{}-{}.yaml", acronym, version);
    let path = releases_dir.join(filename);
    store::read_yaml(&path)
}

/// Load all releases, sorted by version descending (newest first).
pub fn load_releases(root: &Path) -> Result<Vec<Release>, JoyError> {
    let releases_dir = store::joy_dir(root).join(store::RELEASES_DIR);
    if !releases_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut releases: Vec<Release> = Vec::new();
    for entry in fs::read_dir(&releases_dir).map_err(|e| JoyError::ReadFile {
        path: releases_dir.clone(),
        source: e,
    })? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "yaml") {
            match store::read_yaml::<Release>(&path) {
                Ok(release) => releases.push(release),
                Err(_) => continue,
            }
        }
    }

    // Sort by parsed semver descending (newest first). A lexicographic
    // compare would put "v0.9.0" above "v0.10.0".
    releases.sort_by_key(|r| std::cmp::Reverse(semver_key(&r.version)));
    Ok(releases)
}

/// Turn a version string like "v0.10.0" or "1.2.3" into a tuple of
/// integers for numeric ordering. Non-numeric parts sort as 0.
fn semver_key(v: &str) -> (u64, u64, u64) {
    let trimmed = v.strip_prefix('v').unwrap_or(v);
    // Drop pre-release suffixes ("-rc1", "+build") for the primary ordering.
    let core = trimmed.split(['-', '+']).next().unwrap_or(trimmed);
    let mut parts = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major, minor, patch)
}

/// Get the latest release version, if any.
pub fn latest_version(root: &Path) -> Result<Option<String>, JoyError> {
    let releases = load_releases(root)?;
    Ok(releases.first().map(|r| r.version.clone()))
}

/// Check if an item ID appears in any release. Returns the version if found.
pub fn item_in_release(root: &Path, item_id: &str) -> Result<Option<String>, JoyError> {
    let releases = load_releases(root)?;
    for release in &releases {
        let all_items = [
            &release.items.epics,
            &release.items.stories,
            &release.items.tasks,
            &release.items.bugs,
            &release.items.reworks,
            &release.items.decisions,
            &release.items.ideas,
        ];
        for group in all_items {
            if group.iter().any(|i| i.id == item_id) {
                return Ok(Some(release.version.clone()));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::release::{ReleaseItem, ReleaseItems};
    use chrono::NaiveDate;
    use tempfile::tempdir;

    fn setup_project(dir: &Path) {
        let joy_dir = dir.join(".joy");
        fs::create_dir_all(joy_dir.join("releases")).unwrap();
        fs::write(joy_dir.join("project.yaml"), "name: test\nacronym: TP\n").unwrap();
        fs::write(joy_dir.join("config.defaults.yaml"), "version: 1\n").unwrap();
    }

    #[test]
    fn save_and_load_release() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let release = Release {
            version: "v0.1.0".into(),
            title: Some("First release".into()),
            description: None,
            date: NaiveDate::from_ymd_opt(2026, 3, 22).unwrap(),
            previous: None,
            contributors: Vec::new(),
            items: ReleaseItems::default(),
        };

        save_release(dir.path(), "TP", &release).unwrap();
        let loaded = load_release(dir.path(), "TP", "v0.1.0").unwrap();
        assert_eq!(release, loaded);
    }

    #[test]
    fn load_release_without_v_prefix() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let release = Release {
            version: "v0.2.0".into(),
            title: None,
            description: None,
            date: NaiveDate::from_ymd_opt(2026, 3, 22).unwrap(),
            previous: None,
            contributors: Vec::new(),
            items: ReleaseItems::default(),
        };

        save_release(dir.path(), "TP", &release).unwrap();
        let loaded = load_release(dir.path(), "TP", "0.2.0").unwrap();
        assert_eq!(loaded.version, "v0.2.0");
    }

    #[test]
    fn latest_version_empty() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        assert_eq!(latest_version(dir.path()).unwrap(), None);
    }

    #[test]
    fn latest_version_picks_newest() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        for v in ["v0.1.0", "v0.3.0", "v0.2.0"] {
            let release = Release {
                version: v.into(),
                title: None,
                description: None,
                date: NaiveDate::from_ymd_opt(2026, 3, 22).unwrap(),
                previous: None,
                contributors: Vec::new(),
                items: ReleaseItems::default(),
            };
            save_release(dir.path(), "TP", &release).unwrap();
        }

        assert_eq!(latest_version(dir.path()).unwrap(), Some("v0.3.0".into()));
    }

    #[test]
    fn item_in_release_found() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let release = Release {
            version: "v0.1.0".into(),
            title: None,
            description: None,
            date: NaiveDate::from_ymd_opt(2026, 3, 22).unwrap(),
            previous: None,
            contributors: Vec::new(),
            items: ReleaseItems {
                bugs: vec![ReleaseItem {
                    id: "TP-0001".into(),
                    title: "fix".into(),
                }],
                ..Default::default()
            },
        };
        save_release(dir.path(), "TP", &release).unwrap();

        assert_eq!(
            item_in_release(dir.path(), "TP-0001").unwrap(),
            Some("v0.1.0".into())
        );
        assert_eq!(item_in_release(dir.path(), "TP-9999").unwrap(), None);
    }
}
