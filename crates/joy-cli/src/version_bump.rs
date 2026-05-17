// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Pattern-based version bumping across configured files.
//!
//! Each file listed in `release.version-files` is processed with a
//! simple text substitution: every quoted occurrence of the current
//! version (`"X.Y.Z"`) is replaced by the new one. This captures both
//! the primary package version and any internal dependency pins that
//! happen to use the same version string, without parsing the file as
//! TOML/JSON/YAML. No language-specific knowledge is encoded here.
//!
//! Lockfiles, changelogs, and other derived state are not joy's
//! concern -- the project orchestrates those between `joy release
//! bump` and `joy release record` in its own release script.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// A version file entry from project.yaml. Only the path is needed;
/// the replacement is purely textual.
#[derive(Debug, Clone)]
pub struct VersionFile {
    pub path: String,
}

/// Result of a single file bump.
#[derive(Debug)]
pub struct BumpResult {
    pub path: PathBuf,
    pub replacements: usize,
}

/// Bump versions in all configured files. Returns one result per
/// matched file. Files with zero replacements are still returned so
/// the caller can warn about stale configuration.
pub fn bump_all(
    root: &Path,
    files: &[VersionFile],
    old_version: &str,
    new_version: &str,
) -> Result<Vec<BumpResult>> {
    let mut results = Vec::new();

    for file in files {
        let paths = expand_glob(root, &file.path)?;
        if paths.is_empty() {
            bail!(
                "no files matching '{}'\n  = help: check release.version-files in project.yaml",
                file.path
            );
        }
        for path in paths {
            results.push(bump_file(&path, old_version, new_version)?);
        }
    }

    Ok(results)
}

fn expand_glob(root: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let full_pattern = root.join(pattern).to_string_lossy().to_string();
    let paths: Vec<PathBuf> = glob::glob(&full_pattern)
        .with_context(|| format!("invalid glob pattern: {pattern}"))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(paths)
}

/// Replace every quoted occurrence of `old_version` with `new_version`
/// in the file. Matches `"X.Y.Z"` (double quotes) and `'X.Y.Z'`
/// (single quotes) so TOML, JSON, and YAML string literals all land.
/// Unquoted occurrences (comments, plain YAML scalars) are left alone
/// to avoid catching unrelated mentions of the version string.
pub fn bump_file(path: &Path, old_version: &str, new_version: &str) -> Result<BumpResult> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    let needles = [format!("\"{old_version}\""), format!("'{old_version}'")];
    let mut replacements = 0usize;
    let mut updated = content.clone();
    for needle in &needles {
        let replacement = if needle.starts_with('"') {
            format!("\"{new_version}\"")
        } else {
            format!("'{new_version}'")
        };
        let occurrences = updated.matches(needle).count();
        if occurrences > 0 {
            updated = updated.replace(needle, &replacement);
            replacements += occurrences;
        }
    }

    if updated != content {
        fs::write(path, updated).with_context(|| format!("failed to write {}", path.display()))?;
    }

    Ok(BumpResult {
        path: path.to_path_buf(),
        replacements,
    })
}

/// Best-effort detection of the version string currently written in
/// the file. Tries the line-anchored package-version patterns first
/// (TOML `version = "X.Y.Z"`, JSON `"version": "X.Y.Z"`), then falls
/// back to the first quoted SemVer-shaped literal anywhere in the
/// content. Returns `None` when nothing plausible is found; callers
/// can use this for diagnostic output without depending on accuracy.
pub fn detect_version(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;

    for line in content.lines() {
        if let Some(v) = parse_package_version_line(line) {
            return Some(v);
        }
    }
    first_quoted_semver_in(&content)
}

fn parse_package_version_line(line: &str) -> Option<String> {
    let l = line.trim();
    // TOML-style: version = "X.Y.Z"
    if let Some(rest) = l.strip_prefix("version") {
        let rest = rest.trim_start();
        if let Some(after_eq) = rest.strip_prefix('=') {
            return read_quoted_semver(after_eq.trim_start());
        }
    }
    // JSON-style: "version": "X.Y.Z"
    if let Some(rest) = l.strip_prefix("\"version\"") {
        let rest = rest.trim_start();
        if let Some(after_colon) = rest.strip_prefix(':') {
            return read_quoted_semver(after_colon.trim_start());
        }
    }
    None
}

fn read_quoted_semver(s: &str) -> Option<String> {
    let (quote, rest) = if let Some(r) = s.strip_prefix('"') {
        ('"', r)
    } else if let Some(r) = s.strip_prefix('\'') {
        ('\'', r)
    } else {
        return None;
    };
    let end = rest.find(quote)?;
    let candidate = &rest[..end];
    if looks_like_semver(candidate) {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn first_quoted_semver_in(content: &str) -> Option<String> {
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != c && bytes[j] != b'\n' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == c {
                if let Ok(s) = std::str::from_utf8(&bytes[i + 1..j]) {
                    if looks_like_semver(s) {
                        return Some(s.to_string());
                    }
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    None
}

/// Accepts `MAJOR.MINOR.PATCH` with optional `-prerelease` / `+build`
/// suffix. Each numeric segment must be ASCII digits and non-empty.
fn looks_like_semver(s: &str) -> bool {
    let dot1 = match s.find('.') {
        Some(p) => p,
        None => return false,
    };
    let rest_after_major = &s[dot1 + 1..];
    let dot2 = match rest_after_major.find('.') {
        Some(p) => p,
        None => return false,
    };
    let major = &s[..dot1];
    let minor = &rest_after_major[..dot2];
    let patch_and_suffix = &rest_after_major[dot2 + 1..];

    if major.is_empty() || !major.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if minor.is_empty() || !minor.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let patch_digits: usize = patch_and_suffix
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if patch_digits == 0 {
        return false;
    }
    let after_patch = &patch_and_suffix[patch_digits..];
    after_patch.is_empty() || after_patch.starts_with('-') || after_patch.starts_with('+')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bump_toml_package_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").unwrap();

        let result = bump_file(&path, "0.1.0", "0.2.0").unwrap();
        assert_eq!(result.replacements, 1);

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("version = \"0.2.0\""));
        assert!(content.contains("name = \"test\""));
    }

    #[test]
    fn bump_toml_catches_internal_dep_pin() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(
            &path,
            "[package]\nname = \"joy-cli\"\nversion = \"0.8.4\"\n\n\
             [dependencies]\n\
             joy-core = { version = \"0.8.4\", path = \"../joy-core\" }\n\
             joy-ai = { version = \"0.8.4\", path = \"../joy-ai\" }\n",
        )
        .unwrap();

        let result = bump_file(&path, "0.8.4", "0.9.0").unwrap();
        assert_eq!(result.replacements, 3, "package + 2 deps");

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("joy-core = { version = \"0.9.0\""));
        assert!(content.contains("joy-ai = { version = \"0.9.0\""));
        assert!(content.contains("version = \"0.9.0\"\n\n[dependencies]"));
        assert!(!content.contains("0.8.4"));
    }

    #[test]
    fn bump_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("package.json");
        fs::write(
            &path,
            "{\n  \"name\": \"app\",\n  \"version\": \"1.0.0\"\n}\n",
        )
        .unwrap();

        let result = bump_file(&path, "1.0.0", "1.1.0").unwrap();
        assert_eq!(result.replacements, 1);

        assert!(fs::read_to_string(&path).unwrap().contains("\"1.1.0\""));
    }

    #[test]
    fn bump_yaml_quoted_string() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pubspec.yaml");
        fs::write(&path, "name: app\nversion: '0.1.0'\n").unwrap();

        let result = bump_file(&path, "0.1.0", "0.2.0").unwrap();
        assert_eq!(result.replacements, 1);

        assert!(fs::read_to_string(&path).unwrap().contains("'0.2.0'"));
    }

    #[test]
    fn bump_does_not_touch_unrelated_versions() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(
            &path,
            "[package]\nversion = \"0.8.4\"\n\n\
             [dependencies]\nserde = \"1.0.0\"\ntokio = { version = \"1.2.3\" }\n",
        )
        .unwrap();

        let result = bump_file(&path, "0.8.4", "0.9.0").unwrap();
        assert_eq!(result.replacements, 1);

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("serde = \"1.0.0\""));
        assert!(content.contains("tokio = { version = \"1.2.3\" }"));
    }

    #[test]
    fn bump_file_with_no_match_returns_zero() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("other.toml");
        fs::write(&path, "[settings]\nname = \"x\"\n").unwrap();

        let result = bump_file(&path, "0.1.0", "0.2.0").unwrap();
        assert_eq!(result.replacements, 0);
    }

    #[test]
    fn detect_version_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, "[package]\nname = \"x\"\nversion = \"0.1.0\"\n").unwrap();
        assert_eq!(detect_version(&path).as_deref(), Some("0.1.0"));
    }

    #[test]
    fn detect_version_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("package.json");
        fs::write(
            &path,
            "{\n  \"name\": \"app\",\n  \"version\": \"1.2.3\"\n}\n",
        )
        .unwrap();
        assert_eq!(detect_version(&path).as_deref(), Some("1.2.3"));
    }

    #[test]
    fn detect_version_prefers_package_field_over_deps() {
        // Workspace deps appear before the package version: the line-
        // anchored pass picks the package field rather than the first
        // matching quoted string anywhere in the file.
        let dir = tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(
            &path,
            "[workspace.dependencies]\n\
             serde = \"1.0.0\"\n\n\
             [package]\nname = \"x\"\nversion = \"0.5.0\"\n",
        )
        .unwrap();
        assert_eq!(detect_version(&path).as_deref(), Some("0.5.0"));
    }

    #[test]
    fn detect_version_falls_back_to_first_quoted_semver() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("oddball.txt");
        fs::write(&path, "release: \"2.3.4\"\nfoo: 1\n").unwrap();
        assert_eq!(detect_version(&path).as_deref(), Some("2.3.4"));
    }

    #[test]
    fn detect_version_handles_prerelease_suffix() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, "version = \"1.0.0-beta.1\"\n").unwrap();
        assert_eq!(detect_version(&path).as_deref(), Some("1.0.0-beta.1"));
    }

    #[test]
    fn detect_version_returns_none_when_nothing_matches() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plain.txt");
        fs::write(&path, "no version here\n").unwrap();
        assert!(detect_version(&path).is_none());
    }

    #[test]
    fn detect_version_ignores_two_digit_strings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.toml");
        fs::write(&path, "[deps]\nfoo = \"1.0\"\n").unwrap();
        assert!(detect_version(&path).is_none());
    }

    #[test]
    fn bump_all_with_glob() {
        let dir = tempdir().unwrap();
        let crates = dir.path().join("crates");
        fs::create_dir_all(crates.join("a")).unwrap();
        fs::create_dir_all(crates.join("b")).unwrap();
        fs::write(
            crates.join("a/Cargo.toml"),
            "[package]\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            crates.join("b/Cargo.toml"),
            "[package]\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let files = vec![VersionFile {
            path: "crates/*/Cargo.toml".into(),
        }];
        let results = bump_all(dir.path(), &files, "0.1.0", "0.2.0").unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.replacements == 1));
    }
}
