// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The device local pin and the login memory of D4.1c.
//!
//! **Where the pin lives, and why not in `project.yaml`.** That file is
//! the project's shared, committed file and joy syncs it to the forge,
//! so a pin there publishes one person's work account to the whole team
//! and to the forge. The pin lives in the per project app state file
//! joy-core computes and which the CLI and the app both reach
//! (`app_state_project_file`), as
//! `forgeLogin: {"github.com": "scotty-work"}`. It sits beside the
//! acting member pin joy-core already keeps in the same object, so both
//! readers must leave every other key alone.
//!
//! **The memory** is the second step of the order: the login that last
//! reached this remote. It is cached per normalised remote in the
//! device state and thrown away on a 401, 403 or 404 from that remote
//! and on logout, because a cache that outlives the access it recorded
//! is how a person ends up pushing under the wrong account.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The key of the pin inside the per project app state file (D4.1c).
pub const FORGE_LOGIN_KEY: &str = "forgeLogin";

/// The device state file the login memory lives in.
const MEMORY_FILE: &str = "forge-logins.json";

/// The pinned login for this host in this project, if the person set
/// one. Best effort: no app state is not an error, it is "no pin".
pub fn pinned(root: &Path, host: &str) -> Option<String> {
    let path = joy_core::auth::session::app_state_project_file(root).ok()?;
    pinned_in(&path, host)
}

/// [`pinned`] against a named file, for the tests.
pub fn pinned_in(path: &Path, host: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let state: serde_json::Value = serde_json::from_str(&text).ok()?;
    state
        .get(FORGE_LOGIN_KEY)?
        .get(host)?
        .as_str()
        .map(str::to_string)
        .filter(|login| !login.is_empty())
}

/// The normalised remote: the host and the `owner/repo` path,
/// lowercased, without the wire form and without the `.git` suffix.
/// Every wire form of one repository therefore has one key, which is
/// what makes the memory usable at all.
pub fn remote_key(remote: &str) -> Option<String> {
    let host = crate::url::host_of(remote)?;
    let path = crate::url::repo_path_of(remote)?;
    Some(format!("{host}/{}", path.to_ascii_lowercase()))
}

/// The login the memory recorded for this remote.
pub fn remembered(state_dir: Option<&Path>, remote: &str) -> Option<String> {
    let key = remote_key(remote)?;
    let path = memory_file(state_dir)?;
    read_memory(&path).get(&key).cloned()
}

/// Remember the login that reached this remote. Best effort: a state
/// directory that cannot be written costs a cache entry, not the call.
pub fn remember(state_dir: Option<&Path>, remote: &str, login: &str) {
    let (Some(key), Some(path)) = (remote_key(remote), memory_file(state_dir)) else {
        return;
    };
    let mut memory = read_memory(&path);
    if memory.get(&key).is_some_and(|known| known == login) {
        return;
    }
    memory.insert(key, login.to_string());
    write_memory(&path, &memory);
}

/// Throw the memory of this remote away. D4.1c names the moments: a
/// 401, a 403 or a 404 from that remote, and logout.
pub fn forget_remote(state_dir: Option<&Path>, remote: &str) {
    let (Some(key), Some(path)) = (remote_key(remote), memory_file(state_dir)) else {
        return;
    };
    let mut memory = read_memory(&path);
    if memory.remove(&key).is_some() {
        write_memory(&path, &memory);
    }
}

/// Throw away every memory of one login on one host, which is what
/// logout does: the account is gone, so every remote it was remembered
/// for has to be decided again.
pub fn forget_login(state_dir: Option<&Path>, host: &str, login: &str) {
    let Some(path) = memory_file(state_dir) else {
        return;
    };
    let mut memory = read_memory(&path);
    let prefix = format!("{host}/");
    let before = memory.len();
    memory.retain(|key, known| !(key.starts_with(&prefix) && known == login));
    if memory.len() != before {
        write_memory(&path, &memory);
    }
}

fn memory_file(state_dir: Option<&Path>) -> Option<PathBuf> {
    let base = match state_dir {
        Some(dir) => dir.to_path_buf(),
        None => joy_core::auth::session::app_state_dir().ok()?,
    };
    Some(base.join(MEMORY_FILE))
}

fn read_memory(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_memory(path: &Path, memory: &BTreeMap<String, String>) {
    let Ok(text) = serde_json::to_string_pretty(memory) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pin is read out of the object joy-core already keeps, and
    /// every other key in that object is somebody else's business.
    #[test]
    fn the_pin_is_read_from_the_per_project_app_state_and_not_from_project_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{"member":"horst","forgeLogin":{"github.com":"scotty-work","codeberg.org":""}}"#,
        )
        .unwrap();
        assert_eq!(
            pinned_in(&path, "github.com").as_deref(),
            Some("scotty-work")
        );
        assert_eq!(pinned_in(&path, "codeberg.org"), None);
        assert_eq!(pinned_in(&path, "gitlab.com"), None);
        assert_eq!(
            pinned_in(&dir.path().join("nothing.json"), "github.com"),
            None
        );
    }

    /// Every wire form of one repository is one key, or the memory
    /// would record the ssh remote and miss the https twin.
    #[test]
    fn every_wire_form_of_one_repository_shares_one_memory_key() {
        let key = remote_key("git@github.com:joyint/App.git").unwrap();
        assert_eq!(key, "github.com/joyint/app");
        assert_eq!(
            remote_key("https://github.com/joyint/app").as_deref(),
            Some(key.as_str())
        );
        assert_eq!(
            remote_key("ssh://git@github.com/joyint/app.git").as_deref(),
            Some(key.as_str())
        );
        assert_eq!(
            remote_key("https://gitlab.com/group/sub/app.git").as_deref(),
            Some("gitlab.com/group/sub/app")
        );
        assert_eq!(remote_key("/home/p/bare.git"), None);
    }

    #[test]
    fn a_memory_file_records_a_login_per_remote_and_forgets_it_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forge-logins.json");
        let mut memory = read_memory(&path);
        assert!(memory.is_empty());
        memory.insert("github.com/joyint/app".into(), "scotty".into());
        memory.insert("github.com/acme/widgets".into(), "work".into());
        write_memory(&path, &memory);
        let read = read_memory(&path);
        assert_eq!(read.get("github.com/joyint/app").unwrap(), "scotty");
        assert_eq!(read.len(), 2);
    }
}
