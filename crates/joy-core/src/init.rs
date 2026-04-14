// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use crate::embedded::{self, EmbeddedFile};
use crate::error::JoyError;
use crate::model::project::{derive_acronym, Project};
use crate::store;
use crate::vcs::{default_vcs, Vcs};

pub const HOOK_FILES: &[EmbeddedFile] = &[EmbeddedFile {
    content: include_str!("../data/hooks/commit-msg"),
    target: "hooks/commit-msg",
    executable: true,
}];

pub const CONFIG_FILES: &[EmbeddedFile] = &[EmbeddedFile {
    content: include_str!("../data/config.defaults.yaml"),
    target: "config.defaults.yaml",
    executable: false,
}];

pub const PROJECT_FILES: &[EmbeddedFile] = &[EmbeddedFile {
    content: include_str!("../data/project.defaults.yaml"),
    target: "project.defaults.yaml",
    executable: false,
}];

pub struct InitOptions {
    pub root: PathBuf,
    pub name: Option<String>,
    pub acronym: Option<String>,
    /// Override the creator member email. Falls back to git config user.email.
    pub user: Option<String>,
    /// Project language code (ISO 639-1, e.g. "en", "de"). Defaults to "en".
    pub language: Option<String>,
}

#[derive(Debug)]
pub struct InitResult {
    pub project_dir: PathBuf,
    pub git_initialized: bool,
    pub git_existed: bool,
}

pub struct OnboardResult {
    pub hooks_installed: bool,
    pub hooks_already_set: bool,
}

pub fn init(options: InitOptions) -> Result<InitResult, JoyError> {
    let root = &options.root;
    let joy_dir = store::joy_dir(root);

    if store::is_initialized(root) {
        return Err(JoyError::AlreadyInitialized(joy_dir));
    }

    // Detect or initialize git
    let vcs = default_vcs();
    let git_existed = vcs.is_repo(root);
    let mut git_initialized = false;
    if !git_existed {
        vcs.init_repo(root)?;
        git_initialized = true;
    }

    // Create directory structure
    let dirs = [
        store::ITEMS_DIR,
        store::MILESTONES_DIR,
        store::RELEASES_DIR,
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

    // Write config and project defaults (embedded files)
    embedded::sync_files(root, CONFIG_FILES)?;
    embedded::sync_files(root, PROJECT_FILES)?;

    let mut project = Project::new(name, Some(acronym));
    if let Some(lang) = options.language.filter(|s| !s.is_empty()) {
        project.language = lang;
    }

    // Register the project creator as a member with all capabilities.
    // Prefer an explicit override, fall back to git config user.email.
    let creator_email = options
        .user
        .filter(|s| !s.is_empty())
        .or_else(|| vcs.user_email().ok().filter(|s| !s.is_empty()));
    if let Some(email) = creator_email {
        project.members.insert(
            email,
            crate::model::project::Member::new(crate::model::project::MemberCapabilities::All),
        );
    }

    store::write_yaml(&joy_dir.join(store::PROJECT_FILE), &project)?;
    let project_rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    let defaults_rel = format!("{}/{}", store::JOY_DIR, store::CONFIG_DEFAULTS_FILE);
    crate::git_ops::auto_git_add(root, &[&project_rel, &defaults_rel]);

    // Ensure .joy/credentials.yaml is in .gitignore
    ensure_gitignore(root)?;

    // Install hooks
    install_hooks(root)?;

    Ok(InitResult {
        project_dir: joy_dir,
        git_initialized,
        git_existed,
    })
}

/// Onboard an existing project: set up local environment (hooks, etc.).
pub fn onboard(root: &Path) -> Result<OnboardResult, JoyError> {
    embedded::sync_files(root, CONFIG_FILES)?;
    embedded::sync_files(root, PROJECT_FILES)?;
    install_hooks(root)
}

/// Sync hook files and set core.hooksPath.
fn install_hooks(root: &Path) -> Result<OnboardResult, JoyError> {
    let actions = embedded::sync_files(root, HOOK_FILES)?;
    let hooks_installed = actions.iter().any(|a| a.action != "up to date");

    // Set core.hooksPath if not already pointing to .joy/hooks
    let vcs = default_vcs();
    let current = vcs.config_get(root, "core.hooksPath").unwrap_or_default();
    let already_set = current == ".joy/hooks";

    if !already_set {
        vcs.config_set(root, "core.hooksPath", ".joy/hooks")?;
    }

    Ok(OnboardResult {
        hooks_installed,
        hooks_already_set: already_set,
    })
}

pub const GITIGNORE_BLOCK_START: &str = "### joy:start -- managed by joy, do not edit manually";
pub const GITIGNORE_BLOCK_END: &str = "### joy:end";

pub const GITIGNORE_BASE_ENTRIES: &[(&str, &str)] = &[
    (".joy/config.yaml", "personal config"),
    (".joy/credentials.yaml", "secrets"),
    (".joy/hooks/", "git hooks"),
    (".joy/project.defaults.yaml", "embedded project defaults"),
];

/// Update the joy-managed block in .gitignore with the given entries.
/// Each entry is (path, comment). Replaces the block if it exists, appends otherwise.
pub fn update_gitignore_block(root: &Path, entries: &[(&str, &str)]) -> Result<(), JoyError> {
    let gitignore_path = root.join(".gitignore");

    let mut lines = String::new();
    for (path, _comment) in entries {
        lines.push_str(path);
        lines.push('\n');
    }
    let block = format!(
        "{}\n{}{}",
        GITIGNORE_BLOCK_START, lines, GITIGNORE_BLOCK_END
    );

    let content = if gitignore_path.is_file() {
        let existing =
            std::fs::read_to_string(&gitignore_path).map_err(|e| JoyError::ReadFile {
                path: gitignore_path.clone(),
                source: e,
            })?;
        if existing.contains(GITIGNORE_BLOCK_START) && existing.contains(GITIGNORE_BLOCK_END) {
            let start = existing.find(GITIGNORE_BLOCK_START).unwrap();
            let end = existing.find(GITIGNORE_BLOCK_END).unwrap() + GITIGNORE_BLOCK_END.len();
            let mut updated = String::new();
            updated.push_str(&existing[..start]);
            updated.push_str(&block);
            updated.push_str(&existing[end..]);
            updated
        } else {
            let trimmed = existing.trim_end();
            if trimmed.is_empty() {
                format!("{}\n", block)
            } else {
                format!("{}\n\n{}\n", trimmed, block)
            }
        }
    } else {
        format!("{}\n", block)
    };

    std::fs::write(&gitignore_path, &content).map_err(|e| JoyError::WriteFile {
        path: gitignore_path,
        source: e,
    })?;
    crate::git_ops::auto_git_add(root, &[".gitignore"]);
    Ok(())
}

fn ensure_gitignore(root: &Path) -> Result<(), JoyError> {
    update_gitignore_block(root, GITIGNORE_BASE_ENTRIES)
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
            user: None,
            language: None,
        })
        .unwrap();

        assert!(result.project_dir.join("items").is_dir());
        assert!(result.project_dir.join("milestones").is_dir());
        assert!(result.project_dir.join("ai/agents").is_dir());
        assert!(result.project_dir.join("ai/jobs").is_dir());
        assert!(result.project_dir.join("logs").is_dir());
        assert!(result.project_dir.join("config.defaults.yaml").is_file());
        assert!(result.project_dir.join("project.yaml").is_file());
    }

    #[test]
    fn init_writes_project_metadata() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("My App".into()),
            acronym: Some("MA".into()),
            user: None,
            language: None,
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
            user: None,
            language: None,
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
            user: None,
            language: None,
        })
        .unwrap();

        let err = init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
            user: None,
            language: None,
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
            user: None,
            language: None,
        })
        .unwrap();

        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".joy/credentials.yaml"));
        assert!(content.contains(".joy/config.yaml"));
    }

    #[test]
    fn init_does_not_duplicate_gitignore_block() {
        let dir = tempdir().unwrap();
        // First init creates the block
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
            user: None,
            language: None,
        })
        .unwrap();
        let first = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        // Re-running ensure_gitignore should not duplicate
        super::ensure_gitignore(dir.path()).unwrap();
        let second = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        assert_eq!(first, second);
        assert_eq!(second.matches(GITIGNORE_BLOCK_START).count(), 1);
    }

    #[test]
    fn init_initializes_git_if_needed() {
        let dir = tempdir().unwrap();
        let result = init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
            user: None,
            language: None,
        })
        .unwrap();

        assert!(result.git_initialized);
        assert!(!result.git_existed);
        assert!(dir.path().join(".git").is_dir());
    }
}
