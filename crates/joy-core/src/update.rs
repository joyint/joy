// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The update framework: what a joy-managed artefact is, and how a
//! caller asks about it.
//!
//! Moved out of joy-cli (operator decision 2026-07-29) so that everyone
//! who opens a project runs THE SAME reconciles: the CLI at its version
//! sync, the desktop when it opens a project, the platform when it loads
//! one. Before that only the CLI ran them, so a migration written for
//! everybody reached whoever happened to use the terminal.
//!
//! Each layer owns its own items and hands them to [`refresh`] or
//! [`check`]: joy-core the repo artefacts below, joy-chat-store its chat
//! storage, joy-cli the AI tool files. Nobody assembles someone else's.

use std::path::Path;

use crate::init;
use crate::vcs::Vcs;

/// The joy version this build stamps a synced repo with. joy-core and
/// the CLI move in one workspace version, so the marker means the same
/// thing whoever ran the sync.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What an update item may fail with. joy-core has no anyhow, and the
/// callers do not share one error type either, so the trait takes the
/// boxed standard error every layer can turn its own into.
pub type UpdateResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

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

/// Where an item belongs.
///
/// A developer's checkout gets everything joy manages: hooks, ignore
/// blocks, embedded files, the version marker. A server's clone of
/// someone else's repository gets NONE of that — it holds the project's
/// data and must not author repo hygiene into it (operator, 2026-07-29:
/// the platform started installing hooks into every project it served).
/// What both need is the data reconciles: schema, layout, chat storage.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Runs wherever the project is opened, server included.
    Data,
    /// Runs only where a person works on the checkout.
    Checkout,
}

/// One joy-managed artefact (or a small group thereof).
pub trait UpdateItem: Sync + Send {
    fn section(&self) -> &'static str;
    /// Where this item belongs; most artefacts are checkout hygiene.
    fn reach(&self) -> Reach {
        Reach::Checkout
    }
    /// Read-only check; may produce zero or more rows.
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>>;
    /// Write path. Idempotent. Produces one row per touched artefact;
    /// rows with `action: None` mean "already up to date".
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>>;
}

/// The two facts the repo items ask about project.yaml and SECURITY.md:
/// whether SECURITY.md is current, whether the schema needs migrating,
/// the migrated value, and where SECURITY.md sits.
pub fn project_state(
    root: &Path,
) -> UpdateResult<(bool, bool, serde_yaml_ng::Value, std::path::PathBuf)> {
    let project_path = crate::store::joy_dir(root).join(crate::store::PROJECT_FILE);
    let security_path = root.join("SECURITY.md");
    let raw = std::fs::read_to_string(&project_path)?;
    let raw_value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw)?;
    let (migrated_value, applied) = crate::migrations::project_yaml::apply(raw_value);
    let schema_stale = applied.any;
    let security_current = crate::security_md::is_current(&security_path)?;
    Ok((
        security_current,
        schema_stale,
        migrated_value,
        security_path,
    ))
}

/// Persist a project.yaml schema migration produced by [`project_state`].
pub fn write_migrated_project(
    root: &Path,
    migrated_value: serde_yaml_ng::Value,
) -> UpdateResult<()> {
    let project_path = crate::store::joy_dir(root).join(crate::store::PROJECT_FILE);
    let project: crate::model::project::Project = serde_yaml_ng::from_value(migrated_value)?;
    crate::store::write_yaml_preserve(&project_path, &project)?;
    let rel = format!("{}/{}", crate::store::JOY_DIR, crate::store::PROJECT_FILE);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// Whether a managed block with `marker` is present and carries every
/// entry. Shared by the gitignore and gitattributes items.
pub fn block_present(path: &Path, marker: &str, entries: &[&str]) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.contains(marker) && entries.iter().all(|e| s.contains(e)))
        .unwrap_or(false)
}

// -- the repo's own artefacts -------------------------------------------
//
// Everything joy manages in a checkout that needs nothing above this
// crate. The AI tool files live in joy-cli and the chat storage in
// joy-chat-store, each with the code that owns them.

struct VersionMarkerItem;

impl UpdateItem for VersionMarkerItem {
    fn section(&self) -> &'static str {
        SECTION_REPO
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
        let current = CURRENT_VERSION;
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let current = CURRENT_VERSION;
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

struct GitattributesBlockItem;

impl UpdateItem for GitattributesBlockItem {
    fn section(&self) -> &'static str {
        SECTION_GIT
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
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
        let vcs = crate::vcs::default_vcs();
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
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let before = self.current(root);
        let vcs = crate::vcs::default_vcs();
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
        let vcs = crate::vcs::default_vcs();
        vcs.config_get(root, "core.hooksPath").ok().as_deref() == Some(".joy/hooks")
    }
}

impl UpdateItem for HooksPathItem {
    fn section(&self) -> &'static str {
        SECTION_GIT
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let before = self.current(root);
        let vcs = crate::vcs::default_vcs();
        if vcs.is_repo(root) {
            vcs.config_set(root, "core.hooksPath", ".joy/hooks")?;
        }
        Ok(vec![RefreshRow {
            name: "core.hooksPath".into(),
            action: if before { None } else { Some("set") },
        }])
    }
}

struct EmbeddedFilesItem {
    section: &'static str,
    files: &'static [crate::embedded::EmbeddedFile],
}

impl UpdateItem for EmbeddedFilesItem {
    fn section(&self) -> &'static str {
        self.section
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
        let diffs = crate::embedded::diff_files(root, self.files)?;
        Ok(diffs
            .into_iter()
            .map(|(target, status)| {
                let ok = matches!(status, crate::embedded::FileStatus::UpToDate);
                let detail = match status {
                    crate::embedded::FileStatus::UpToDate => "up to date",
                    crate::embedded::FileStatus::Outdated => "stale (would be re-rendered)",
                    crate::embedded::FileStatus::Missing => "missing (would be installed)",
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let actions = crate::embedded::sync_files(root, self.files)?;
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

struct SecurityMdItem;

impl UpdateItem for SecurityMdItem {
    fn section(&self) -> &'static str {
        SECTION_AUTH
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
        let (security_current, _, _, _) = project_state(root)?;
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let security_path = root.join("SECURITY.md");
        let changed = crate::security_md::render(&security_path)?;
        if changed {
            crate::git_ops::auto_git_add(root, &["SECURITY.md"]);
        }
        Ok(vec![RefreshRow {
            name: "SECURITY.md".into(),
            action: if changed { Some("rendered") } else { None },
        }])
    }
}

struct ProjectYamlSchemaItem;

impl UpdateItem for ProjectYamlSchemaItem {
    fn reach(&self) -> Reach {
        Reach::Data
    }
    fn section(&self) -> &'static str {
        SECTION_AUTH
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
        let (_, schema_stale, _, _) = project_state(root)?;
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let (_, schema_stale, migrated_value, _) = project_state(root)?;
        if schema_stale {
            write_migrated_project(root, migrated_value)?;
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

/// Filesystem-aware repo-layout migrations (see
/// `crate::migrations::repo`). Currently the one-shot doc-path
/// reconcile for JOY-01C7-CB; removable once that window closes.
struct DocPathsMigrationItem;

impl UpdateItem for DocPathsMigrationItem {
    fn reach(&self) -> Reach {
        Reach::Data
    }
    fn section(&self) -> &'static str {
        SECTION_AUTH
    }
    fn check(&self, root: &Path) -> UpdateResult<Vec<CheckRow>> {
        let pins = crate::migrations::repo::pending(root)?;
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
    fn refresh(&self, root: &Path) -> UpdateResult<Vec<RefreshRow>> {
        let pins = crate::migrations::repo::apply(root)?;
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

/// The items joy-core owns. A caller adds what its own layer owns.
pub fn core_items() -> Vec<Box<dyn UpdateItem>> {
    vec![
        Box::new(VersionMarkerItem),
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
        Box::new(DocPathsMigrationItem),
    ]
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
}
