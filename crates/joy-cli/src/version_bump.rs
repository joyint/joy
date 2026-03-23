// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Structured version bumping: parse TOML/JSON/YAML files and update a specific key.
#![allow(dead_code)] // Functions are used by the --full release flow (JOY-0043)

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// A version file entry from project.yaml.
#[derive(Debug, Clone)]
pub struct VersionFile {
    pub path: String,
    pub key: String,
}

/// Result of a single file bump.
#[derive(Debug)]
pub struct BumpResult {
    pub path: PathBuf,
    pub old_version: String,
    pub new_version: String,
}

/// Bump version in all configured files. Returns list of changed files.
pub fn bump_all(root: &Path, files: &[VersionFile], new_version: &str) -> Result<Vec<BumpResult>> {
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
            let result = bump_file(&path, &file.key, new_version)?;
            results.push(result);
        }
    }

    Ok(results)
}

/// Expand a glob pattern relative to root.
fn expand_glob(root: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let full_pattern = root.join(pattern).to_string_lossy().to_string();
    let paths: Vec<PathBuf> = glob::glob(&full_pattern)
        .with_context(|| format!("invalid glob pattern: {pattern}"))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(paths)
}

/// Bump a single file by updating the given key to new_version.
fn bump_file(path: &Path, key: &str, new_version: &str) -> Result<BumpResult> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "toml" => bump_toml(path, key, new_version),
        "json" => bump_json(path, key, new_version),
        "yaml" | "yml" => bump_yaml(path, key, new_version),
        _ => bail!(
            "unsupported file format: {}\n  = help: supported formats: .toml, .json, .yaml",
            path.display()
        ),
    }
}

fn bump_toml(path: &Path, key: &str, new_version: &str) -> Result<BumpResult> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("failed to parse TOML: {}", path.display()))?;

    // Navigate dotted key: "package.version" → doc["package"]["version"]
    let parts: Vec<&str> = key.split('.').collect();
    let old_version = get_toml_value(&doc, &parts)
        .with_context(|| format!("key '{}' not found in {}", key, path.display()))?;

    set_toml_value(&mut doc, &parts, new_version)
        .with_context(|| format!("failed to set '{}' in {}", key, path.display()))?;

    fs::write(path, doc.to_string())
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(BumpResult {
        path: path.to_path_buf(),
        old_version,
        new_version: new_version.to_string(),
    })
}

fn get_toml_value(doc: &toml_edit::DocumentMut, parts: &[&str]) -> Result<String> {
    let mut current = doc.as_item();
    for part in parts {
        current = current
            .get(part)
            .with_context(|| format!("missing key '{part}'"))?;
    }
    current
        .as_str()
        .map(|s| s.to_string())
        .with_context(|| "value is not a string".to_string())
}

fn set_toml_value(doc: &mut toml_edit::DocumentMut, parts: &[&str], value: &str) -> Result<()> {
    let mut current = doc.as_item_mut();
    for part in &parts[..parts.len() - 1] {
        current = current
            .get_mut(part)
            .with_context(|| format!("missing key '{part}'"))?;
    }
    let last = parts.last().unwrap();
    current[last] = toml_edit::value(value);
    Ok(())
}

fn bump_json(path: &Path, key: &str, new_version: &str) -> Result<BumpResult> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    let mut doc: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse JSON: {}", path.display()))?;

    let parts: Vec<&str> = key.split('.').collect();
    let old_version = get_json_value(&doc, &parts)
        .with_context(|| format!("key '{}' not found in {}", key, path.display()))?;

    set_json_value(&mut doc, &parts, new_version)
        .with_context(|| format!("failed to set '{}' in {}", key, path.display()))?;

    // Preserve formatting: re-serialize with 2-space indent
    let output = serde_json::to_string_pretty(&doc)
        .with_context(|| format!("failed to serialize JSON: {}", path.display()))?;
    fs::write(path, format!("{output}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(BumpResult {
        path: path.to_path_buf(),
        old_version,
        new_version: new_version.to_string(),
    })
}

fn get_json_value(doc: &serde_json::Value, parts: &[&str]) -> Result<String> {
    let mut current = doc;
    for part in parts {
        current = current
            .get(part)
            .with_context(|| format!("missing key '{part}'"))?;
    }
    current
        .as_str()
        .map(|s| s.to_string())
        .with_context(|| "value is not a string".to_string())
}

fn set_json_value(doc: &mut serde_json::Value, parts: &[&str], value: &str) -> Result<()> {
    let mut current = doc;
    for part in &parts[..parts.len() - 1] {
        current = current
            .get_mut(part)
            .with_context(|| format!("missing key '{part}'"))?;
    }
    let last = parts.last().unwrap();
    current[last] = serde_json::Value::String(value.to_string());
    Ok(())
}

fn bump_yaml(path: &Path, key: &str, new_version: &str) -> Result<BumpResult> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    let mut doc: serde_json::Value = serde_yaml_ng::from_str(&content)
        .with_context(|| format!("failed to parse YAML: {}", path.display()))?;

    let parts: Vec<&str> = key.split('.').collect();
    let old_version = get_json_value(&doc, &parts)
        .with_context(|| format!("key '{}' not found in {}", key, path.display()))?;

    set_json_value(&mut doc, &parts, new_version)
        .with_context(|| format!("failed to set '{}' in {}", key, path.display()))?;

    let output = serde_yaml_ng::to_string(&doc)
        .with_context(|| format!("failed to serialize YAML: {}", path.display()))?;
    fs::write(path, output).with_context(|| format!("failed to write {}", path.display()))?;

    Ok(BumpResult {
        path: path.to_path_buf(),
        old_version,
        new_version: new_version.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bump_toml_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").unwrap();

        let result = bump_toml(&path, "package.version", "0.2.0").unwrap();
        assert_eq!(result.old_version, "0.1.0");
        assert_eq!(result.new_version, "0.2.0");

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("version = \"0.2.0\""));
        assert!(content.contains("name = \"test\""));
    }

    #[test]
    fn bump_json_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("package.json");
        fs::write(
            &path,
            "{\n  \"name\": \"test\",\n  \"version\": \"1.0.0\"\n}\n",
        )
        .unwrap();

        let result = bump_json(&path, "version", "1.1.0").unwrap();
        assert_eq!(result.old_version, "1.0.0");
        assert_eq!(result.new_version, "1.1.0");

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("\"version\": \"1.1.0\""));
    }

    #[test]
    fn bump_yaml_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        fs::write(&path, "name: test\nversion: 0.1.0\n").unwrap();

        let result = bump_yaml(&path, "version", "0.2.0").unwrap();
        assert_eq!(result.old_version, "0.1.0");
        assert_eq!(result.new_version, "0.2.0");

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("0.2.0"));
    }

    #[test]
    fn bump_toml_nested_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Move.toml");
        fs::write(
            &path,
            "[package]\nname = \"contract\"\nversion = \"0.0.1\"\n",
        )
        .unwrap();

        let result = bump_toml(&path, "package.version", "0.0.2").unwrap();
        assert_eq!(result.old_version, "0.0.1");
    }

    #[test]
    fn bump_missing_key_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, "[package]\nname = \"test\"\n").unwrap();

        let result = bump_toml(&path, "package.version", "0.2.0");
        assert!(result.is_err());
    }

    #[test]
    fn bump_unsupported_format_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.txt");
        fs::write(&path, "version=0.1.0\n").unwrap();

        let result = bump_file(&path, "version", "0.2.0");
        assert!(result.is_err());
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
            key: "package.version".into(),
        }];
        let results = bump_all(dir.path(), &files, "0.2.0").unwrap();
        assert_eq!(results.len(), 2);

        for r in &results {
            assert_eq!(r.old_version, "0.1.0");
            assert_eq!(r.new_version, "0.2.0");
        }
    }
}
