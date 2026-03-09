// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::JoyError;
use crate::model::config::Config;
use crate::model::project::{derive_acronym, Project};
use crate::store;

pub struct InitOptions {
    pub root: PathBuf,
    pub name: Option<String>,
    pub acronym: Option<String>,
}

#[derive(Debug)]
pub struct InitResult {
    pub project_dir: PathBuf,
    pub git_initialized: bool,
    pub git_existed: bool,
}

pub fn init(options: InitOptions) -> Result<InitResult, JoyError> {
    let root = &options.root;
    let joy_dir = store::joy_dir(root);

    if store::is_initialized(root) {
        return Err(JoyError::AlreadyInitialized(joy_dir));
    }

    // Detect or initialize git
    let git_existed = is_git_repo(root);
    let mut git_initialized = false;
    if !git_existed {
        run_git_init(root)?;
        git_initialized = true;
    }

    // Create directory structure
    let dirs = [
        store::ITEMS_DIR,
        store::MILESTONES_DIR,
        store::AI_AGENTS_DIR,
        store::AI_JOBS_DIR,
        store::LOG_DIR,
    ];
    for dir in &dirs {
        let path = joy_dir.join(dir);
        std::fs::create_dir_all(&path).map_err(|e| JoyError::CreateDir {
            path: path.clone(),
            source: e,
        })?;
    }

    // Derive project name and acronym
    let name = options.name.unwrap_or_else(|| {
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string()
    });
    let acronym = options.acronym.unwrap_or_else(|| derive_acronym(&name));

    // Write config and project files
    let config = Config::default();
    store::write_yaml(&joy_dir.join(store::CONFIG_FILE), &config)?;

    let project = Project::new(name, Some(acronym));
    store::write_yaml(&joy_dir.join(store::PROJECT_FILE), &project)?;

    // Ensure .joy/credentials.yaml is in .gitignore
    ensure_gitignore(root)?;

    Ok(InitResult {
        project_dir: joy_dir,
        git_initialized,
        git_existed,
    })
}

fn is_git_repo(root: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_git_init(root: &Path) -> Result<(), JoyError> {
    let status = Command::new("git")
        .args(["init"])
        .current_dir(root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| JoyError::Git(format!("failed to run git init: {e}")))?;

    if !status.success() {
        return Err(JoyError::Git("git init failed".into()));
    }
    Ok(())
}

fn ensure_gitignore(root: &Path) -> Result<(), JoyError> {
    let gitignore_path = root.join(".gitignore");
    let entry = ".joy/credentials.yaml";

    let content = if gitignore_path.is_file() {
        std::fs::read_to_string(&gitignore_path).map_err(|e| JoyError::ReadFile {
            path: gitignore_path.clone(),
            source: e,
        })?
    } else {
        String::new()
    };

    if content.lines().any(|line| line.trim() == entry) {
        return Ok(());
    }

    let new_content = if content.is_empty() {
        format!("{entry}\n")
    } else if content.ends_with('\n') {
        format!("{content}{entry}\n")
    } else {
        format!("{content}\n{entry}\n")
    };

    std::fs::write(&gitignore_path, new_content).map_err(|e| JoyError::WriteFile {
        path: gitignore_path,
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_creates_directory_structure() {
        let dir = tempdir().unwrap();
        let result = init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test Project".into()),
            acronym: Some("TP".into()),
        })
        .unwrap();

        assert!(result.project_dir.join("items").is_dir());
        assert!(result.project_dir.join("milestones").is_dir());
        assert!(result.project_dir.join("ai/agents").is_dir());
        assert!(result.project_dir.join("ai/jobs").is_dir());
        assert!(result.project_dir.join("log").is_dir());
        assert!(result.project_dir.join("config.yaml").is_file());
        assert!(result.project_dir.join("project.yaml").is_file());
    }

    #[test]
    fn init_writes_project_metadata() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("My App".into()),
            acronym: Some("MA".into()),
        })
        .unwrap();

        let project: Project =
            store::read_yaml(&store::joy_dir(dir.path()).join(store::PROJECT_FILE)).unwrap();
        assert_eq!(project.name, "My App");
        assert_eq!(project.acronym.as_deref(), Some("MA"));
    }

    #[test]
    fn init_derives_name_from_directory() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: None,
            acronym: None,
        })
        .unwrap();

        let project: Project =
            store::read_yaml(&store::joy_dir(dir.path()).join(store::PROJECT_FILE)).unwrap();
        // tempdir names vary, just check it's not empty
        assert!(!project.name.is_empty());
        assert!(project.acronym.is_some());
    }

    #[test]
    fn init_fails_if_already_initialized() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
        })
        .unwrap();

        let err = init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
        })
        .unwrap_err();

        assert!(matches!(err, JoyError::AlreadyInitialized(_)));
    }

    #[test]
    fn init_creates_gitignore_with_credentials_entry() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
        })
        .unwrap();

        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".joy/credentials.yaml"));
    }

    #[test]
    fn init_does_not_duplicate_gitignore_entry() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), ".joy/credentials.yaml\n").unwrap();

        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
        })
        .unwrap();

        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(content.matches(".joy/credentials.yaml").count(), 1);
    }

    #[test]
    fn init_initializes_git_if_needed() {
        let dir = tempdir().unwrap();
        let result = init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
        })
        .unwrap();

        assert!(result.git_initialized);
        assert!(!result.git_existed);
        assert!(dir.path().join(".git").is_dir());
    }
}
