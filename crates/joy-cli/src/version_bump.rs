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
