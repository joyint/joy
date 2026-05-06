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

    // Register the YAML / log merge driver in .gitattributes and git config.
    ensure_gitattributes(root)?;
    register_merge_driver(root)?;

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
    ensure_gitattributes(root)?;
    register_merge_driver(root)?;
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

pub const GITATTRIBUTES_BLOCK_START: &str = "### joy:start -- managed by joy, do not edit manually";
pub const GITATTRIBUTES_BLOCK_END: &str = "### joy:end";

/// Path-pattern -> Git attribute lines for the joy-managed
/// `.gitattributes` block. The YAML driver covers every Joy YAML file
/// (items, milestones, releases, project metadata). The log entry uses
/// Git's built-in union driver as an interim until JOY-0112 (Merkle-DAG
/// log) ships.
pub const GITATTRIBUTES_BASE_ENTRIES: &[&str] = &[
    ".joy/items/*.yaml merge=joy-yaml",
    ".joy/milestones/*.yaml merge=joy-yaml",
    ".joy/releases/*.yaml merge=joy-yaml",
    ".joy/ai/agents/*.yaml merge=joy-yaml",
    ".joy/ai/jobs/*.yaml merge=joy-yaml",
    ".joy/project.yaml merge=joy-yaml",
    ".joy/config.defaults.yaml merge=joy-yaml",
    ".joy/logs/*.log merge=union",
];

pub const MERGE_DRIVER_NAME_KEY: &str = "merge.joy-yaml.name";
pub const MERGE_DRIVER_NAME_VALUE: &str = "Joy YAML merge driver";
pub const MERGE_DRIVER_CMD_KEY: &str = "merge.joy-yaml.driver";
pub const MERGE_DRIVER_CMD_VALUE: &str =
    "joy merge driver --base %O --current %A --other %B --path %P --ours-rev %X --theirs-rev %Y";

/// Update the joy-managed block in .gitattributes with the given lines.
/// Replaces the block if it exists, appends otherwise.
pub fn update_gitattributes_block(root: &Path, lines: &[&str]) -> Result<(), JoyError> {
    let path = root.join(".gitattributes");

    let mut joined = String::new();
    for line in lines {
        joined.push_str(line);
        joined.push('\n');
    }
    let block = format!(
        "{}\n{}{}",
        GITATTRIBUTES_BLOCK_START, joined, GITATTRIBUTES_BLOCK_END
    );

    let content = if path.is_file() {
        let existing = std::fs::read_to_string(&path).map_err(|e| JoyError::ReadFile {
            path: path.clone(),
            source: e,
        })?;
        if existing.contains(GITATTRIBUTES_BLOCK_START)
            && existing.contains(GITATTRIBUTES_BLOCK_END)
        {
            let start = existing.find(GITATTRIBUTES_BLOCK_START).unwrap();
            let end =
                existing.find(GITATTRIBUTES_BLOCK_END).unwrap() + GITATTRIBUTES_BLOCK_END.len();
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

    // Idempotency: skip write + auto-stage when content already matches.
    // The lazy-activation hook (called on every joy invocation) relies on
    // this short-circuit to stay cheap and to not dirty the working tree.
    if path.is_file() {
        if let Ok(existing) = std::fs::read_to_string(&path) {
            if existing == content {
                return Ok(());
            }
        }
    }

    std::fs::write(&path, &content).map_err(|e| JoyError::WriteFile { path, source: e })?;
    crate::git_ops::auto_git_add(root, &[".gitattributes"]);
    Ok(())
}

fn ensure_gitattributes(root: &Path) -> Result<(), JoyError> {
    update_gitattributes_block(root, GITATTRIBUTES_BASE_ENTRIES)
}

/// Best-effort registration check, called before every joy invocation
/// that has a project root. Brings `.gitattributes` and the local git
/// merge-driver config in line with the current binary, so users who
/// upgraded joy without re-running `joy init` still get the merge
/// driver. See JOY-0162.
///
/// Idempotent and silent: the file write is skipped when the block is
/// already up to date, and `register_merge_driver` only writes when the
/// stored values differ.
pub fn ensure_lazy_activation(root: &Path) -> Result<(), JoyError> {
    let vcs = default_vcs();
    if !vcs.is_repo(root) {
        return Ok(());
    }
    ensure_gitattributes(root)?;
    register_merge_driver(root)?;
    Ok(())
}

/// Per-clone git config key recording the joy version that last synced
/// this repo. Compared against `env!("CARGO_PKG_VERSION")` to drive the
/// auto-sync hook. See JOY-0164-B5.
pub const LAST_SYNC_VERSION_KEY: &str = "joy.last-sync-version";

/// Read the recorded last-sync version from this clone's git config.
/// `None` if not a repo or the key is unset.
///
/// TODO: route through a `Vcs::config_get` trait method so non-Git
/// backends can implement (ADR-010). Today it's a `GitVcs` inherent
/// method, which constrains the abstraction.
pub fn last_sync_version(root: &Path) -> Option<String> {
    let vcs = default_vcs();
    if !vcs.is_repo(root) {
        return None;
    }
    vcs.config_get(root, LAST_SYNC_VERSION_KEY).ok()
}

/// Stamp the current binary version into this clone's git config.
pub fn set_last_sync_version(root: &Path, version: &str) -> Result<(), JoyError> {
    let vcs = default_vcs();
    if !vcs.is_repo(root) {
        return Ok(());
    }
    vcs.config_set(root, LAST_SYNC_VERSION_KEY, version)
}

/// One-shot full sync of a repo against the current binary. Today this
/// runs `ensure_lazy_activation` and stamps `joy.last-sync-version`.
/// Future work (tracked under JOY-0164-B5): also fold in the work that
/// `joy auth update` and `joy ai update` do, with a unified output
/// protocol.
pub fn run_sync(root: &Path, current_version: &str) -> Result<(), JoyError> {
    ensure_lazy_activation(root)?;
    set_last_sync_version(root, current_version)
}

/// Register the joy-yaml merge driver in the local Git config. Idempotent:
/// repeated calls overwrite with the same value. The config is per-clone
/// (Git does not transmit it through clone), so this is also called from
/// `onboard` to bring fresh clones up to date.
fn register_merge_driver(root: &Path) -> Result<(), JoyError> {
    let vcs = default_vcs();
    if !vcs.is_repo(root) {
        return Ok(());
    }
    if vcs.config_get(root, MERGE_DRIVER_NAME_KEY).ok().as_deref() != Some(MERGE_DRIVER_NAME_VALUE)
    {
        vcs.config_set(root, MERGE_DRIVER_NAME_KEY, MERGE_DRIVER_NAME_VALUE)?;
    }
    if vcs.config_get(root, MERGE_DRIVER_CMD_KEY).ok().as_deref() != Some(MERGE_DRIVER_CMD_VALUE) {
        vcs.config_set(root, MERGE_DRIVER_CMD_KEY, MERGE_DRIVER_CMD_VALUE)?;
    }
    Ok(())
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
    fn init_writes_gitattributes_block_with_joy_yaml_and_union_log() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
            user: None,
            language: None,
        })
        .unwrap();

        let content = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(content.contains(GITATTRIBUTES_BLOCK_START));
        assert!(content.contains(GITATTRIBUTES_BLOCK_END));
        assert!(content.contains(".joy/items/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/milestones/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/releases/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/ai/agents/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/ai/jobs/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/project.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/config.defaults.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/logs/*.log merge=union"));
    }

    #[test]
    fn init_does_not_duplicate_gitattributes_block() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
            user: None,
            language: None,
        })
        .unwrap();
        let first = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();

        super::ensure_gitattributes(dir.path()).unwrap();
        let second = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();

        assert_eq!(first, second);
        assert_eq!(second.matches(GITATTRIBUTES_BLOCK_START).count(), 1);
    }

    #[test]
    fn init_registers_merge_driver_in_git_config() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            root: dir.path().to_path_buf(),
            name: Some("Test".into()),
            acronym: None,
            user: None,
            language: None,
        })
        .unwrap();

        let vcs = default_vcs();
        let name = vcs.config_get(dir.path(), MERGE_DRIVER_NAME_KEY).unwrap();
        let cmd = vcs.config_get(dir.path(), MERGE_DRIVER_CMD_KEY).unwrap();
        assert_eq!(name, MERGE_DRIVER_NAME_VALUE);
        assert_eq!(cmd, MERGE_DRIVER_CMD_VALUE);
        assert!(cmd.contains("--ours-rev %X"));
        assert!(cmd.contains("--theirs-rev %Y"));
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
