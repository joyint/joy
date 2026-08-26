// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;

use crate::error::JoyError;
use crate::model::item;
use crate::model::release::{Bump, Release, ReleaseItem};
use crate::store;
use crate::version_bump;

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

fn plural(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

/// Render a release as Markdown -- the notes for the annotated git tag
/// and the forge release, and `joy release show --markdown`'s source of
/// truth. Contributor ids stay raw (`c.id.id()`), never the resolved
/// name/e-mail that Display would produce: published notes stay
/// anonymous (ADR-042).
pub fn render_release_markdown(release: &Release) -> String {
    let mut out = String::new();
    let title_str = release
        .title
        .as_deref()
        .map(|t| format!(" - {t}"))
        .unwrap_or_default();
    out.push_str(&format!("# {}{}\n\n", release.version, title_str));
    out.push_str(&format!("**Date:** {}\n", release.date));
    if let Some(ref prev) = release.previous {
        out.push_str(&format!("**Previous:** {prev}\n"));
    }
    if let Some(ref desc) = release.description {
        out.push_str(&format!("\n{desc}\n"));
    }
    if !release.contributors.is_empty() {
        out.push_str("\n## Contributors\n\n");
        for c in &release.contributors {
            out.push_str(&format!(
                "- {} ({} events on {} items)\n",
                c.id.id(),
                c.events,
                c.items
            ));
        }
    }
    let type_groups: &[(&str, &[ReleaseItem])] = &[
        ("Epics", &release.items.epics),
        ("Stories", &release.items.stories),
        ("Tasks", &release.items.tasks),
        ("Bugs", &release.items.bugs),
        ("Reworks", &release.items.reworks),
        ("Decisions", &release.items.decisions),
        ("Ideas", &release.items.ideas),
    ];
    let total: usize = type_groups.iter().map(|(_, items)| items.len()).sum();
    for (label, items) in type_groups {
        if items.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {label}\n\n"));
        for ri in *items {
            let filename = item::item_filename(&ri.id, &ri.title);
            out.push_str(&format!(
                "- [{}](.joy/items/{}) {}\n",
                ri.id, filename, ri.title
            ));
        }
    }
    if total > 0 {
        out.push_str(&format!("\n---\n*{}*\n", plural(total, "item")));
    }
    out
}

/// Why [`resolve_version`] failed: a store error while reading the
/// release ledger, or a bump argument that is neither an explicit
/// version nor a bump keyword. Split into variants so each surface
/// (CLI, desktop, platform) maps the two cases onto its own error type
/// without re-parsing message text.
#[derive(Debug)]
pub enum ResolveVersionError {
    /// Reading the release ledger failed.
    Store(JoyError),
    /// The bump argument is invalid (the `Bump` parse error, verbatim).
    InvalidBump(String),
}

impl std::fmt::Display for ResolveVersionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(e) => e.fmt(f),
            Self::InvalidBump(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ResolveVersionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(e) => Some(e),
            Self::InvalidBump(_) => None,
        }
    }
}

fn looks_like_explicit(s: &str) -> bool {
    matches!(s.chars().next(), Some(c) if c.is_ascii_digit()) || s.starts_with('v')
}

/// Compute `(current, next)` version from the bump argument (`patch`,
/// `minor`, `major`, an explicit `X.Y.Z`/`vX.Y.Z`, or None = patch) and
/// the previous release. Deterministic: `joy release bump` and
/// `joy release record` call this with the same argument and land on
/// the same version. When `baseline_override` is set, that value
/// replaces the ledger / tag lookup (used by `bump --adopt`).
///
/// `latest_tag_fallback` supplies the newest local `v*` version tag and
/// is only consulted when the release ledger has no entry: the CLI and
/// the desktop shell out through `vcs::default_vcs()`, the platform
/// answers from its git2 layer.
pub fn resolve_version(
    root: &Path,
    arg: Option<&str>,
    baseline_override: Option<String>,
    latest_tag_fallback: impl FnOnce() -> Option<String>,
) -> Result<(String, String), ResolveVersionError> {
    let current = match baseline_override {
        Some(v) => {
            if v.starts_with('v') {
                v
            } else {
                format!("v{v}")
            }
        }
        None => {
            let previous = latest_version(root)
                .map_err(ResolveVersionError::Store)?
                .or_else(latest_tag_fallback);
            previous.as_deref().unwrap_or("v0.0.0").to_string()
        }
    };

    let next = match arg {
        Some(v) if looks_like_explicit(v) => {
            if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            }
        }
        Some(b) => {
            let bump: Bump = b.parse().map_err(ResolveVersionError::InvalidBump)?;
            crate::model::release::bump_version(&current, bump)
        }
        None => crate::model::release::bump_version(&current, Bump::Patch),
    };
    Ok((current, next))
}

/// Scan the configured files for the version they currently contain
/// (via `version_bump::detect_version`) and return that as the new
/// baseline for `joy release bump --adopt`. All files must agree; files
/// where detection fails are listed but tolerated as long as at least
/// one file produces a version. `Err` carries the ready-to-print
/// message (each surface wraps it as its invalid-input error).
pub fn adopt_baseline(
    root: &Path,
    version_files: &[version_bump::VersionFile],
) -> Result<String, String> {
    let mut by_version: std::collections::BTreeMap<String, Vec<std::path::PathBuf>> =
        std::collections::BTreeMap::new();
    let mut undetectable: Vec<std::path::PathBuf> = Vec::new();

    for vf in version_files {
        let pattern = root.join(&vf.path);
        let paths: Vec<_> = glob::glob(&pattern.to_string_lossy())
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();
        for path in paths {
            match version_bump::detect_version(&path) {
                Some(v) => by_version.entry(v).or_default().push(path),
                None => undetectable.push(path),
            }
        }
    }

    match by_version.len() {
        0 => {
            let mut msg =
                String::from("--adopt: could not detect a version in any configured file");
            for path in &undetectable {
                let rel = path.strip_prefix(root).unwrap_or(path);
                msg.push_str(&format!("\n  ! {}", rel.display()));
            }
            msg.push_str("\n  = help: pass an explicit X.Y.Z to bump instead");
            Err(msg)
        }
        1 => Ok(by_version.into_keys().next().expect("one entry")),
        _ => {
            let mut msg = String::from("--adopt: configured files disagree on the current version");
            for (v, files) in &by_version {
                for path in files {
                    let rel = path.strip_prefix(root).unwrap_or(path);
                    msg.push_str(&format!("\n  {} -> {}", rel.display(), v));
                }
            }
            msg.push_str("\n  = help: align the files manually or pass an explicit X.Y.Z");
            Err(msg)
        }
    }
}

/// The two halves of the "version mismatch" diagnostic emitted when a
/// release bump finds zero replacements, built by [`version_mismatch`].
/// The CLI prints `file_diagnostics` to stdout (framed by blank lines)
/// and fails with `summary`; surfaces without a stdout (desktop,
/// platform) raise [`VersionMismatch::merged`] as one error. Designed
/// for narrow terminals -- every line stays short.
#[derive(Debug)]
pub struct VersionMismatch {
    /// Per-file block, three lines per file (`! path` / `expected:` /
    /// `found:`), each line newline-terminated. Empty when `results`
    /// was empty.
    pub file_diagnostics: String,
    /// The error summary: the mismatch count and the two recovery
    /// commands (`bump --adopt`, `record <version>`). No trailing
    /// newline.
    pub summary: String,
}

impl VersionMismatch {
    /// Both halves as one message, for surfaces where the diagnostic
    /// cannot go to stdout: the per-file block, a blank line, then the
    /// summary.
    pub fn merged(&self) -> String {
        format!("{}\n{}", self.file_diagnostics, self.summary)
    }
}

/// Build the multi-line "version mismatch" diagnostic for a release
/// bump that found zero matches: per file what was expected versus what
/// `version_bump::detect_version` actually sees, plus a summary naming
/// the two recovery commands.
pub fn version_mismatch(
    root: &Path,
    results: &[version_bump::BumpResult],
    expected: &str,
) -> VersionMismatch {
    let mut detected_any: Option<String> = None;
    let mut file_diagnostics = String::new();
    for r in results {
        let rel = r.path.strip_prefix(root).unwrap_or(&r.path);
        let detected = version_bump::detect_version(&r.path);
        if detected_any.is_none() {
            detected_any = detected.clone();
        }
        file_diagnostics.push_str(&format!("  ! {}\n", rel.display()));
        file_diagnostics.push_str(&format!("      expected: {expected}\n"));
        match detected {
            Some(v) => file_diagnostics.push_str(&format!("      found:    {v}\n")),
            None => file_diagnostics.push_str("      found:    (no version detected)\n"),
        }
    }

    let n = results.len();
    let plural = if n == 1 { "" } else { "s" };
    let mut summary = format!("version mismatch ({n} of {n} file{plural})\n\nFix options:");
    summary.push_str("\n\n  joy release bump --adopt");
    summary.push_str("\n      adopt the file's detected version");
    if let Some(v) = detected_any {
        summary.push_str(&format!("\n\n  joy release record {v}"));
        summary.push_str("\n      skip bump, record at the detected version");
    } else {
        summary.push_str("\n\n  joy release record <X.Y.Z>");
        summary.push_str("\n      skip bump, record at an explicit version");
    }
    VersionMismatch {
        file_diagnostics,
        summary,
    }
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

    #[test]
    fn render_release_markdown_basic() {
        let release = Release {
            version: "v0.2.0".into(),
            title: Some("Second".into()),
            description: Some("Notes.".into()),
            date: NaiveDate::from_ymd_opt(2026, 3, 22).unwrap(),
            previous: Some("v0.1.0".into()),
            contributors: Vec::new(),
            items: ReleaseItems {
                bugs: vec![ReleaseItem {
                    id: "TP-0001".into(),
                    title: "fix crash".into(),
                }],
                ..Default::default()
            },
        };
        let md = render_release_markdown(&release);
        assert!(md.starts_with("# v0.2.0 - Second\n\n**Date:** 2026-03-22\n"));
        assert!(md.contains("**Previous:** v0.1.0\n"));
        assert!(md.contains("\n## Bugs\n\n- [TP-0001]"));
        assert!(md.ends_with("\n---\n*1 item*\n"));
    }

    #[test]
    fn resolve_version_from_ledger_bumps_and_accepts_explicit() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        let release = Release {
            version: "v0.3.0".into(),
            title: None,
            description: None,
            date: NaiveDate::from_ymd_opt(2026, 3, 22).unwrap(),
            previous: None,
            contributors: Vec::new(),
            items: ReleaseItems::default(),
        };
        save_release(dir.path(), "TP", &release).unwrap();

        // ledger present: the fallback must not run
        let (current, next) =
            resolve_version(dir.path(), None, None, || panic!("fallback consulted")).unwrap();
        assert_eq!((current.as_str(), next.as_str()), ("v0.3.0", "v0.3.1"));

        let (_, next) = resolve_version(dir.path(), Some("minor"), None, || None).unwrap();
        assert_eq!(next, "v0.4.0");

        let (_, next) = resolve_version(dir.path(), Some("1.2.3"), None, || None).unwrap();
        assert_eq!(next, "v1.2.3");

        let err = resolve_version(dir.path(), Some("bogus"), None, || None).unwrap_err();
        assert!(matches!(err, ResolveVersionError::InvalidBump(_)));
        assert!(err.to_string().contains("invalid bump: bogus"));
    }

    #[test]
    fn resolve_version_falls_back_to_tag_then_zero() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());

        let (current, _) =
            resolve_version(dir.path(), None, None, || Some("v0.9.0".into())).unwrap();
        assert_eq!(current, "v0.9.0");

        let (current, next) = resolve_version(dir.path(), None, None, || None).unwrap();
        assert_eq!((current.as_str(), next.as_str()), ("v0.0.0", "v0.0.1"));

        // --adopt: the baseline override wins and gets the v prefix
        let (current, next) =
            resolve_version(dir.path(), None, Some("2.0.0".into()), || None).unwrap();
        assert_eq!((current.as_str(), next.as_str()), ("v2.0.0", "v2.0.1"));
    }

    #[test]
    fn adopt_baseline_agrees_or_errors() {
        let dir = tempdir().unwrap();
        setup_project(dir.path());
        fs::write(
            dir.path().join("package.json"),
            "{\n  \"version\": \"1.5.0\"\n}\n",
        )
        .unwrap();
        let files = [version_bump::VersionFile {
            path: "package.json".into(),
        }];
        assert_eq!(adopt_baseline(dir.path(), &files).unwrap(), "1.5.0");

        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nversion = \"2.0.0\"\n",
        )
        .unwrap();
        let files = [
            version_bump::VersionFile {
                path: "package.json".into(),
            },
            version_bump::VersionFile {
                path: "Cargo.toml".into(),
            },
        ];
        let msg = adopt_baseline(dir.path(), &files).unwrap_err();
        assert!(msg.starts_with("--adopt: configured files disagree"));
        assert!(msg.contains("package.json -> 1.5.0"));
        assert!(msg.contains("Cargo.toml -> 2.0.0"));

        let files = [version_bump::VersionFile {
            path: "README.md".into(),
        }];
        let msg = adopt_baseline(dir.path(), &files).unwrap_err();
        assert!(msg.starts_with("--adopt: could not detect a version"));
    }

    #[test]
    fn version_mismatch_splits_diagnostic_and_summary() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            "{\n  \"version\": \"1.5.0\"\n}\n",
        )
        .unwrap();
        let results = [version_bump::BumpResult {
            path: dir.path().join("package.json"),
            replacements: 0,
        }];
        let vm = version_mismatch(dir.path(), &results, "1.4.0");
        assert_eq!(
            vm.file_diagnostics,
            "  ! package.json\n      expected: 1.4.0\n      found:    1.5.0\n"
        );
        assert!(vm
            .summary
            .starts_with("version mismatch (1 of 1 file)\n\nFix options:"));
        assert!(vm.summary.contains("joy release record 1.5.0"));
        assert_eq!(
            vm.merged(),
            format!("{}\n{}", vm.file_diagnostics, vm.summary)
        );
    }
}
