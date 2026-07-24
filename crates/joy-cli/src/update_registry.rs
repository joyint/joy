// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Declarative registry of joy-managed artefacts.
//!
//! `joy update` and `joy update --check` iterate this registry instead
//! of hand-rolling per-domain plumbing. Adding a new artefact means
//! writing one [`UpdateItem`] impl and registering it in [`all`].
//! See JOY-0169-6A.
//!
//! The trait keeps two paths separate:
//! - [`UpdateItem::check`] is read-only and reports the current state.
//! - [`UpdateItem::refresh`] writes if needed and returns whether
//!   anything actually changed.
//!
//! The orchestrator (in [`crate::commands::update`]) groups rows by
//! [`UpdateItem::section`] and drives output formatting.

use std::path::Path;

use anyhow::Result;
use joy_core::vcs::Vcs;
use joy_core::{embedded, init};

use crate::commands::{ai, auth};

/// Section labels used to group items in the output.
pub const SECTION_REPO: &str = "Repo state";
pub const SECTION_GIT: &str = "Git extensions";
pub const SECTION_EMBEDDED: &str = "Embedded files";
pub const SECTION_AUTH: &str = "Auth artefacts";
pub const SECTION_AI: &str = "AI tool files";

/// Visual-and-semantic state of a check row.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowMark {
    /// Up to date. Renders with `check_mark`. Does not contribute to
    /// the "stale" exit code.
    Ok,
    /// Needs attention from `joy update`. Renders with `warn_mark` and
    /// makes `joy update --check` exit 2.
    Stale,
    /// Informational only (e.g. "not installed"). Renders with
    /// `empty_mark` and does not contribute to the exit code.
    Info,
}

/// Read-only result of an [`UpdateItem::check`].
pub struct CheckRow {
    /// Display name shown left-aligned in the row.
    pub name: String,
    pub mark: RowMark,
    /// Right-aligned detail (e.g. "up to date", "stale", "(0.14.2)").
    pub detail: String,
}

impl RowMark {
    /// Convenience for items that only distinguish ok/stale.
    pub fn from_ok(ok: bool) -> Self {
        if ok {
            RowMark::Ok
        } else {
            RowMark::Stale
        }
    }
}

/// Best-effort semver-style version compare. Parses leading "X.Y.Z"
/// triples and compares numerically; falls back to lexical compare
/// when either side is not parseable.
pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    fn triple(v: &str) -> Option<(u32, u32, u32)> {
        let mut parts = v.split('.');
        let x: u32 = parts.next()?.parse().ok()?;
        let y: u32 = parts.next()?.parse().ok()?;
        let z_raw = parts.next()?;
        let z_digits: String = z_raw.chars().take_while(|c| c.is_ascii_digit()).collect();
        let z: u32 = z_digits.parse().ok()?;
        Some((x, y, z))
    }
    match (triple(a), triple(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

/// Returns the recorded `joy.last-sync-version` when it is newer than
/// `current_version`; `None` otherwise. Used by the top-level
/// downgrade guard in `main.rs` and by [`VersionMarkerItem`] to refuse
/// rolling repo state back to an older binary's templates.
pub fn marker_ahead_of(root: &std::path::Path, current_version: &str) -> Option<String> {
    let marker = init::last_sync_version(root)?;
    if cmp_version(&marker, current_version) == std::cmp::Ordering::Greater {
        Some(marker)
    } else {
        None
    }
}

/// Write result of an [`UpdateItem::refresh`].
pub struct RefreshRow {
    pub name: String,
    /// `Some(verb)` when the item was actually changed (e.g.
    /// "rendered", "registered", "refreshed"). `None` when already up
    /// to date and therefore not displayed in the terse `joy update`
    /// output.
    pub action: Option<&'static str>,
}

/// One joy-managed artefact (or a small group thereof).
pub trait UpdateItem: Sync + Send {
    fn section(&self) -> &'static str;
    /// Read-only check; may produce zero or more rows.
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>>;
    /// Write path. Idempotent. Produces one row per touched artefact;
    /// rows with `action: None` mean "already up to date".
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>>;
}

/// Build the registry. Order in this list defines section ordering and
/// row ordering within a section in the displayed output.
pub fn all() -> Vec<Box<dyn UpdateItem>> {
    let mut v: Vec<Box<dyn UpdateItem>> = vec![
        Box::new(VersionMarkerItem),
        Box::new(LegacyAiArtifactsItem),
        Box::new(GitignoreBlockItem),
        Box::new(GitattributesBlockItem),
        Box::new(MergeDriverItem),
        Box::new(HooksPathItem),
        Box::new(EmbeddedFilesItem {
            section: SECTION_EMBEDDED,
            files: init::HOOK_FILES,
        }),
        Box::new(EmbeddedFilesItem {
            section: SECTION_EMBEDDED,
            files: init::CONFIG_FILES,
        }),
        Box::new(EmbeddedFilesItem {
            section: SECTION_EMBEDDED,
            files: init::PROJECT_FILES,
        }),
        Box::new(SecurityMdItem),
        Box::new(ProjectYamlSchemaItem),
        Box::new(AiMemberAdapterItem),
        Box::new(DocPathsMigrationItem),
    ];
    for id in ai::tool_ids() {
        v.push(Box::new(AiToolItem { id }));
    }
    v
}

// -- Repo state ---------------------------------------------------------

struct VersionMarkerItem;

impl UpdateItem for VersionMarkerItem {
    fn section(&self) -> &'static str {
        SECTION_REPO
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let current = crate::commands::update::CURRENT_VERSION;
        let (mark, detail) = match init::last_sync_version(root) {
            Some(v) if v == current => (RowMark::Ok, format!("({current})")),
            Some(v) => match cmp_version(&v, current) {
                std::cmp::Ordering::Greater => (
                    RowMark::Stale,
                    format!("binary older than repo (last {v}, running {current}); update joy"),
                ),
                _ => (
                    RowMark::Stale,
                    format!("stale (last {v}, current {current})"),
                ),
            },
            None => (RowMark::Stale, "never synced".to_string()),
        };
        Ok(vec![CheckRow {
            name: "version marker".into(),
            mark,
            detail,
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let current = crate::commands::update::CURRENT_VERSION;
        // Refuse to roll the marker back: a newer marker means a more
        // recent joy version touched this repo. Overwriting would
        // claim downgrade work that did not happen.
        if marker_ahead_of(root, current).is_some() {
            return Ok(vec![RefreshRow {
                name: "version marker".into(),
                action: None,
            }]);
        }
        let was_current = init::last_sync_version(root).as_deref() == Some(current);
        init::set_last_sync_version(root, current)?;
        Ok(vec![RefreshRow {
            name: "version marker".into(),
            action: if was_current { None } else { Some("stamped") },
        }])
    }
}

/// Removes dead pre-ADR-024 artefacts under `.joy/` (legacy AI
/// instruction/skill/capability files). See
/// [`joy_core::init::LEGACY_AI_ARTIFACTS`] and [`ai::remove_legacy_ai_artifacts`].
struct LegacyAiArtifactsItem;

impl UpdateItem for LegacyAiArtifactsItem {
    fn section(&self) -> &'static str {
        SECTION_REPO
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let present = ai::legacy_ai_artifacts_present(root);
        Ok(vec![CheckRow {
            name: "legacy AI artefacts".into(),
            mark: RowMark::from_ok(!present),
            detail: if present {
                "pre-ADR-024 files present (would be removed)".into()
            } else {
                "none".into()
            },
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let removed = joy_ai::ai_setup::remove_legacy_ai_artifacts(root);
        Ok(vec![RefreshRow {
            name: "legacy AI artefacts".into(),
            action: if removed.is_empty() {
                None
            } else {
                Some("removed")
            },
        }])
    }
}

// -- Git extensions -----------------------------------------------------

fn block_present(path: &Path, marker: &str, entries: &[&str]) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.contains(marker) && entries.iter().all(|e| s.contains(e)))
        .unwrap_or(false)
}

struct GitignoreBlockItem;

impl GitignoreBlockItem {
    /// The entries the managed block should carry for this repo. All or
    /// nothing, never per-tool: once any AI tool is configured the block holds
    /// the full fixed set (base entries plus every tool's ignore entries), so
    /// `joy update` stops stripping the per-tool lines `joy ai init` writes;
    /// with no AI tool configured it stays the base-only set `joy init`
    /// produces (JOY-01FE-98).
    fn expected_entries(root: &Path) -> Vec<(&'static str, &'static str)> {
        if ai::tool_ids()
            .iter()
            .any(|&id| ai::is_tool_configured_pub(root, id))
        {
            joy_ai::ai_setup::managed_gitignore_entries()
        } else {
            init::GITIGNORE_BASE_ENTRIES.to_vec()
        }
    }
}

impl UpdateItem for GitignoreBlockItem {
    fn section(&self) -> &'static str {
        SECTION_GIT
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let entries = Self::expected_entries(root);
        let paths: Vec<&str> = entries.iter().map(|(p, _)| *p).collect();
        let ok = block_present(
            &root.join(".gitignore"),
            init::GITIGNORE_BLOCK_START,
            &paths,
        );
        Ok(vec![CheckRow {
            name: ".gitignore block".into(),
            mark: RowMark::from_ok(ok),
            detail: if ok {
                "managed block present".into()
            } else {
                "missing or out of date".into()
            },
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let before = matches!(self.check(root)?[0].mark, RowMark::Ok);
        init::update_gitignore_block(root, &Self::expected_entries(root))?;
        Ok(vec![RefreshRow {
            name: ".gitignore block".into(),
            action: if before { None } else { Some("registered") },
        }])
    }
}

struct GitattributesBlockItem;

impl UpdateItem for GitattributesBlockItem {
    fn section(&self) -> &'static str {
        SECTION_GIT
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let ok = block_present(
            &root.join(".gitattributes"),
            init::GITATTRIBUTES_BLOCK_START,
            init::GITATTRIBUTES_BASE_ENTRIES,
        );
        Ok(vec![CheckRow {
            name: ".gitattributes block".into(),
            mark: RowMark::from_ok(ok),
            detail: if ok {
                "managed block present".into()
            } else {
                "missing or out of date".into()
            },
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let before = matches!(self.check(root)?[0].mark, RowMark::Ok);
        init::update_gitattributes_block(root, init::GITATTRIBUTES_BASE_ENTRIES)?;
        Ok(vec![RefreshRow {
            name: ".gitattributes block".into(),
            action: if before { None } else { Some("registered") },
        }])
    }
}

struct MergeDriverItem;

impl MergeDriverItem {
    fn current(&self, root: &Path) -> bool {
        let vcs = joy_core::vcs::default_vcs();
        if !vcs.is_repo(root) {
            return false;
        }
        vcs.config_get(root, init::MERGE_DRIVER_NAME_KEY)
            .ok()
            .as_deref()
            == Some(init::MERGE_DRIVER_NAME_VALUE)
            && vcs
                .config_get(root, init::MERGE_DRIVER_CMD_KEY)
                .ok()
                .as_deref()
                == Some(init::MERGE_DRIVER_CMD_VALUE)
    }
}

impl UpdateItem for MergeDriverItem {
    fn section(&self) -> &'static str {
        SECTION_GIT
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let ok = self.current(root);
        Ok(vec![CheckRow {
            name: "merge.joy-yaml.driver".into(),
            mark: RowMark::from_ok(ok),
            detail: if ok {
                "registered".into()
            } else {
                "missing or out of date".into()
            },
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let before = self.current(root);
        let vcs = joy_core::vcs::default_vcs();
        if vcs.is_repo(root) {
            vcs.config_set(
                root,
                init::MERGE_DRIVER_NAME_KEY,
                init::MERGE_DRIVER_NAME_VALUE,
            )?;
            vcs.config_set(
                root,
                init::MERGE_DRIVER_CMD_KEY,
                init::MERGE_DRIVER_CMD_VALUE,
            )?;
        }
        Ok(vec![RefreshRow {
            name: "merge.joy-yaml.driver".into(),
            action: if before { None } else { Some("registered") },
        }])
    }
}

struct HooksPathItem;

impl HooksPathItem {
    fn current(&self, root: &Path) -> bool {
        let vcs = joy_core::vcs::default_vcs();
        vcs.config_get(root, "core.hooksPath").ok().as_deref() == Some(".joy/hooks")
    }
}

impl UpdateItem for HooksPathItem {
    fn section(&self) -> &'static str {
        SECTION_GIT
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let ok = self.current(root);
        Ok(vec![CheckRow {
            name: "core.hooksPath".into(),
            mark: RowMark::from_ok(ok),
            detail: if ok {
                ".joy/hooks".into()
            } else {
                "not pointing at .joy/hooks".into()
            },
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let before = self.current(root);
        let vcs = joy_core::vcs::default_vcs();
        if vcs.is_repo(root) {
            vcs.config_set(root, "core.hooksPath", ".joy/hooks")?;
        }
        Ok(vec![RefreshRow {
            name: "core.hooksPath".into(),
            action: if before { None } else { Some("set") },
        }])
    }
}

// -- Embedded files -----------------------------------------------------

struct EmbeddedFilesItem {
    section: &'static str,
    files: &'static [embedded::EmbeddedFile],
}

impl UpdateItem for EmbeddedFilesItem {
    fn section(&self) -> &'static str {
        self.section
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let diffs = embedded::diff_files(root, self.files)?;
        Ok(diffs
            .into_iter()
            .map(|(target, status)| {
                let ok = matches!(status, embedded::FileStatus::UpToDate);
                let detail = match status {
                    embedded::FileStatus::UpToDate => "up to date",
                    embedded::FileStatus::Outdated => "stale (would be re-rendered)",
                    embedded::FileStatus::Missing => "missing (would be installed)",
                }
                .to_string();
                CheckRow {
                    name: target.to_string(),
                    mark: RowMark::from_ok(ok),
                    detail,
                }
            })
            .collect())
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let actions = embedded::sync_files(root, self.files)?;
        Ok(actions
            .into_iter()
            .map(|a| {
                let action = match a.action {
                    "up to date" => None,
                    "created" => Some("installed"),
                    "updated" => Some("refreshed"),
                    other => Some(Box::leak(other.to_string().into_boxed_str()) as &'static str),
                };
                RefreshRow {
                    name: a.target.to_string(),
                    action,
                }
            })
            .collect())
    }
}

// -- Auth artefacts -----------------------------------------------------

struct SecurityMdItem;

impl UpdateItem for SecurityMdItem {
    fn section(&self) -> &'static str {
        SECTION_AUTH
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let (security_current, _, _, _) = auth::auth_state_pub(root)?;
        Ok(vec![CheckRow {
            name: "SECURITY.md".into(),
            mark: RowMark::from_ok(security_current),
            detail: if security_current {
                "up to date".into()
            } else {
                "stale".into()
            },
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let security_path = root.join("SECURITY.md");
        let changed = joy_core::security_md::render(&security_path)?;
        if changed {
            joy_core::git_ops::auto_git_add(root, &["SECURITY.md"]);
        }
        Ok(vec![RefreshRow {
            name: "SECURITY.md".into(),
            action: if changed { Some("rendered") } else { None },
        }])
    }
}

struct ProjectYamlSchemaItem;

impl UpdateItem for ProjectYamlSchemaItem {
    fn section(&self) -> &'static str {
        SECTION_AUTH
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let (_, schema_stale, _, _) = auth::auth_state_pub(root)?;
        Ok(vec![CheckRow {
            name: "project.yaml schema".into(),
            mark: RowMark::from_ok(!schema_stale),
            detail: if schema_stale {
                "stale".into()
            } else {
                "up to date".into()
            },
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let (_, schema_stale, migrated_value, _) = auth::auth_state_pub(root)?;
        if schema_stale {
            auth::write_migrated_project(root, migrated_value)?;
        }
        Ok(vec![RefreshRow {
            name: "project.yaml schema".into(),
            action: if schema_stale {
                Some("normalised")
            } else {
                None
            },
        }])
    }
}

/// Backfill the ACP adapter onto project.yaml AI members that predate the
/// adapter moving there (JI-0164). Members registered before that carry no
/// adapter, so the platform cannot route their turns; `joy update` sets it
/// from the member's tool id. Removable once no such members remain.
struct AiMemberAdapterItem;

impl AiMemberAdapterItem {
    /// AI members missing an adapter that a known tool id supplies.
    fn missing(root: &Path) -> Vec<String> {
        let Ok(project) = joy_core::store::load_project(root) else {
            return Vec::new();
        };
        project
            .members()
            .filter(|(key, m)| {
                key.starts_with("ai:")
                    && m.adapter.as_deref().unwrap_or("").trim().is_empty()
                    && Self::adapter_for(key).is_some()
            })
            .map(|(key, _)| key.clone())
            .collect()
    }
    /// The adapter for an `ai:<tool>@joy` member key, if the tool is known.
    fn adapter_for(member_key: &str) -> Option<&'static str> {
        let tool = member_key.strip_prefix("ai:")?.split('@').next()?;
        joy_ai::naming::tool_adapter(tool)
    }
}

impl UpdateItem for AiMemberAdapterItem {
    fn section(&self) -> &'static str {
        SECTION_AI
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let missing = Self::missing(root);
        Ok(vec![CheckRow {
            name: "AI member adapters".into(),
            mark: RowMark::from_ok(missing.is_empty()),
            detail: if missing.is_empty() {
                "set on project.yaml members".into()
            } else {
                format!("{} member(s) missing an adapter", missing.len())
            },
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let missing = Self::missing(root);
        if !missing.is_empty() {
            let mut project = joy_core::store::load_project(root)?;
            for key in &missing {
                if let (Some(adapter), Some(m)) =
                    (Self::adapter_for(key), project.member_by_key_mut(key))
                {
                    m.adapter = Some(adapter.to_string());
                }
            }
            joy_core::store::write_yaml_preserve(
                &joy_core::store::joy_dir(root).join(joy_core::store::PROJECT_FILE),
                &project,
            )?;
        }
        Ok(vec![RefreshRow {
            name: "AI member adapters".into(),
            action: if missing.is_empty() {
                None
            } else {
                Some("backfilled")
            },
        }])
    }
}

/// Filesystem-aware repo-layout migrations (see
/// `joy_core::migrations::repo`). Currently the one-shot doc-path
/// reconcile for JOY-01C7-CB; removable once that window closes.
struct DocPathsMigrationItem;

impl UpdateItem for DocPathsMigrationItem {
    fn section(&self) -> &'static str {
        SECTION_AUTH
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let pins = joy_core::migrations::repo::pending(root)?;
        if pins.is_empty() {
            return Ok(vec![CheckRow {
                name: "doc paths".into(),
                mark: RowMark::from_ok(true),
                detail: "up to date".into(),
            }]);
        }
        Ok(pins
            .into_iter()
            .map(|p| CheckRow {
                name: p.key.to_string(),
                mark: RowMark::from_ok(false),
                detail: format!("-> {}", p.to),
            })
            .collect())
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        let pins = joy_core::migrations::repo::apply(root)?;
        if pins.is_empty() {
            return Ok(vec![RefreshRow {
                name: "doc paths".into(),
                action: None,
            }]);
        }
        Ok(pins
            .into_iter()
            .map(|p| RefreshRow {
                name: format!("{} -> {}", p.key, p.to),
                action: Some("pinned"),
            })
            .collect())
    }
}

// -- AI tool files ------------------------------------------------------

struct AiToolItem {
    id: &'static str,
}

impl AiToolItem {
    fn display_name(&self) -> &'static str {
        ai::tool_display_name(self.id).unwrap_or(self.id)
    }
    fn member_id(&self) -> String {
        format!("ai:{}@joy", self.id)
    }
}

impl UpdateItem for AiToolItem {
    fn section(&self) -> &'static str {
        SECTION_AI
    }
    fn check(&self, root: &Path) -> Result<Vec<CheckRow>> {
        let installed = ai::is_tool_installed(self.id);
        let configured = ai::is_tool_configured_pub(root, self.id);
        // joy update only refreshes configured tools; "installed but
        // not configured" and "not installed" are informational, not
        // stale (the user must run joy ai init to opt in).
        let (mark, detail) = if configured {
            let stale = ai::is_tool_stale_pub(root, self.id, &self.member_id())?;
            if stale {
                (RowMark::Stale, "outdated".to_string())
            } else {
                (RowMark::Ok, "up to date".to_string())
            }
        } else if installed {
            (
                RowMark::Info,
                "installed, not configured (run joy ai init)".to_string(),
            )
        } else {
            (RowMark::Info, "not installed".to_string())
        };
        Ok(vec![CheckRow {
            name: self.display_name().to_string(),
            mark,
            detail,
        }])
    }
    fn refresh(&self, root: &Path) -> Result<Vec<RefreshRow>> {
        if !ai::is_tool_configured_pub(root, self.id) {
            // Not configured -> not joy's job to install; skip silently.
            return Ok(Vec::new());
        }
        let changed = ai::refresh_tool_by_id(root, self.id, &self.member_id())?;
        // Keep the configured-tools .gitignore entries fresh too.
        let _ = ai::sync_gitignore_for_configured_tools(root);
        Ok(vec![RefreshRow {
            name: self.display_name().to_string(),
            action: if changed { Some("refreshed") } else { None },
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cmp::Ordering;

    #[test]
    fn cmp_version_orders_numerically() {
        assert_eq!(cmp_version("0.14.2", "0.14.2"), Ordering::Equal);
        assert_eq!(cmp_version("0.14.2", "0.15.0"), Ordering::Less);
        assert_eq!(cmp_version("0.15.0", "0.14.2"), Ordering::Greater);
        // Numeric, not lexical: 9 < 10.
        assert_eq!(cmp_version("0.9.0", "0.10.0"), Ordering::Less);
        assert_eq!(cmp_version("1.0.0", "0.99.99"), Ordering::Greater);
    }

    #[test]
    fn cmp_version_tolerates_pre_release_suffix_on_patch() {
        // We strip non-digits from the patch component, so "rc1"
        // suffixes do not break the compare.
        assert_eq!(cmp_version("0.14.2-rc1", "0.14.2"), Ordering::Equal);
        assert_eq!(cmp_version("0.14.2-rc1", "0.14.3"), Ordering::Less);
    }

    /// Sanity: every section name referenced in [`all`] is one of the
    /// declared section constants. This stops a typo from silently
    /// inventing a new section.
    #[test]
    fn registered_items_use_known_sections() {
        let known = [
            SECTION_REPO,
            SECTION_GIT,
            SECTION_EMBEDDED,
            SECTION_AUTH,
            SECTION_AI,
        ];
        for item in all() {
            assert!(
                known.contains(&item.section()),
                "unknown section: {}",
                item.section()
            );
        }
    }
}
