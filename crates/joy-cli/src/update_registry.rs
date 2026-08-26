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

use joy_core::init;

use crate::commands::ai;

pub use joy_core::update::{
    block_present, marker_ahead_of, CheckRow, RefreshRow, RowMark, UpdateItem, UpdateResult,
    SECTION_AI, SECTION_AUTH, SECTION_EMBEDDED, SECTION_GIT, SECTION_REPO,
};

/// Build the registry: what joy-core owns, what joy-chat-store owns, and
/// what only the CLI has (the AI tool files). Order defines section and
/// row ordering in the displayed output.
pub fn all() -> Vec<Box<dyn UpdateItem>> {
    let mut v: Vec<Box<dyn UpdateItem>> = joy_chat_store::update::project_items();
    v.push(Box::new(LegacyAiArtifactsItem));
    v.push(Box::new(GitignoreBlockItem));
    v.push(Box::new(AiMemberAdapterItem));
    for id in ai::tool_ids() {
        v.push(Box::new(AiToolItem { id }));
    }
    v
}

// -- Repo state ---------------------------------------------------------

/// Removes dead pre-ADR-024 artefacts under `.joy/` (legacy AI
/// instruction/skill/capability files). See
/// [`joy_core::init::LEGACY_AI_ARTIFACTS`] and [`ai::remove_legacy_ai_artifacts`].
struct LegacyAiArtifactsItem;

impl UpdateItem for LegacyAiArtifactsItem {
    fn section(&self) -> &'static str {
        SECTION_REPO
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
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

struct GitignoreBlockItem;

impl GitignoreBlockItem {
    /// The entries the managed block should carry for this repo. All or
    /// nothing, never per-tool: once the project has any AI member the block
    /// holds the full fixed set (base entries plus every tool's ignore
    /// entries), so `joy update` stops stripping the per-tool lines
    /// `joy ai init` writes; with no AI member it stays the base-only set
    /// `joy init` produces (JOY-01FE-98). The signal is `ai:*` membership in
    /// `.joy/project.yaml`, not the machine-local marker files: those are
    /// git-ignored, so a fresh checkout would otherwise read "nothing
    /// configured" and strip the committed tool lines (JOY-0264-89).
    fn expected_entries(root: &Path) -> Vec<(&'static str, &'static str)> {
        if joy_ai::ai_setup::has_ai_member(root) {
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
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let before = matches!(self.check(root)?[0].mark, RowMark::Ok);
        init::update_gitignore_block(root, &Self::expected_entries(root))?;
        Ok(vec![RefreshRow {
            name: ".gitignore block".into(),
            action: if before { None } else { Some("registered") },
        }])
    }
}

// -- Embedded files -----------------------------------------------------

// -- Auth artefacts -----------------------------------------------------

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
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
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
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
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

    /// Minimal project.yaml under `root/.joy/` with the given members block.
    fn seed_project_yaml(root: &Path, members_yaml: &str) {
        let dir = root.join(".joy");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("project.yaml"),
            format!("name: Test\ncreated: 2026-01-01T00:00:00Z\n{members_yaml}"),
        )
        .unwrap();
    }

    #[test]
    fn gitignore_block_full_set_from_membership_not_local_markers() {
        // Fresh-clone shape: an AI member in project.yaml, no local marker
        // files. The expected block must still be the full fixed set, so
        // `joy update` does not strip the committed tool entries
        // (JOY-0264-89).
        let tmp = tempfile::tempdir().unwrap();
        seed_project_yaml(
            tmp.path(),
            "members:\n  \"ai:claude@joy\":\n    capabilities: all\n",
        );
        let entries = GitignoreBlockItem::expected_entries(tmp.path());
        assert_eq!(entries, joy_ai::ai_setup::managed_gitignore_entries());
    }

    #[test]
    fn gitignore_block_base_only_without_ai_members() {
        let tmp = tempfile::tempdir().unwrap();
        seed_project_yaml(
            tmp.path(),
            "members:\n  horst@joy:\n    capabilities: all\n",
        );
        let entries = GitignoreBlockItem::expected_entries(tmp.path());
        assert_eq!(entries, init::GITIGNORE_BASE_ENTRIES.to_vec());
    }
}
