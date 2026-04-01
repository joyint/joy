// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Manifest-based version tracking for Joy-generated AI tool files.
//!
//! Stores file hashes in `.joy/ai/manifest.yaml` instead of embedding
//! version comments in generated files. This avoids breaking third-party
//! parsers that expect clean YAML frontmatter or specific file formats.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::JoyError;

const MANIFEST_REL_PATH: &str = "ai/manifest.yaml";

/// Manifest tracking Joy-generated AI tool files.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AiManifest {
    /// Joy version that last wrote these files.
    pub version: String,
    /// Per-tool file tracking.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, ToolManifest>,
}

/// Files generated for a single AI tool.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ToolManifest {
    /// Map of relative file path -> "sha256:<hex>"
    pub files: BTreeMap<String, String>,
}

impl AiManifest {
    /// Load manifest from `.joy/ai/manifest.yaml`, returning default if missing.
    pub fn load(root: &Path) -> Self {
        let path = crate::store::joy_dir(root).join(MANIFEST_REL_PATH);
        if !path.is_file() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_yaml_ng::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save manifest to `.joy/ai/manifest.yaml`.
    pub fn save(&self, root: &Path) -> Result<(), JoyError> {
        let dir = crate::store::joy_dir(root).join("ai");
        std::fs::create_dir_all(&dir)
            .map_err(|e| JoyError::Other(format!("create .joy/ai: {e}")))?;
        let path = dir.join("manifest.yaml");
        let content =
            serde_yaml_ng::to_string(self).map_err(|e| JoyError::Template(e.to_string()))?;
        std::fs::write(&path, content)
            .map_err(|e| JoyError::Other(format!("write manifest: {e}")))?;
        Ok(())
    }

    /// Record a file's hash for a tool.
    pub fn set_file(&mut self, tool: &str, rel_path: &str, hash: &str) {
        self.tools
            .entry(tool.to_string())
            .or_default()
            .files
            .insert(rel_path.to_string(), format!("sha256:{hash}"));
    }

    /// Check if a file's hash matches the recorded value.
    pub fn is_current(&self, tool: &str, rel_path: &str, hash: &str) -> bool {
        self.tools
            .get(tool)
            .and_then(|t| t.files.get(rel_path))
            .map(|stored| stored == &format!("sha256:{hash}"))
            .unwrap_or(false)
    }

    /// Remove all entries for a tool.
    pub fn remove_tool(&mut self, tool: &str) {
        self.tools.remove(tool);
    }

    /// Check which files are stale or missing.
    /// Returns a list of (tool, rel_path) pairs that need updating.
    pub fn stale_files(&self, current_version: &str, root: &Path) -> Vec<(String, String)> {
        let mut stale = Vec::new();

        // Version mismatch means everything is stale
        if self.version != current_version {
            for (tool, tm) in &self.tools {
                for rel_path in tm.files.keys() {
                    stale.push((tool.clone(), rel_path.clone()));
                }
            }
            return stale;
        }

        // Check each file's hash against on-disk content
        for (tool, tm) in &self.tools {
            for (rel_path, expected_hash) in &tm.files {
                let abs_path = root.join(rel_path);
                let current_hash = if abs_path.is_file() {
                    match std::fs::read_to_string(&abs_path) {
                        Ok(content) => {
                            let hash = hash_for_check(&content, rel_path);
                            format!("sha256:{hash}")
                        }
                        Err(_) => String::new(),
                    }
                } else {
                    String::new() // missing file
                };
                if &current_hash != expected_hash {
                    stale.push((tool.clone(), rel_path.clone()));
                }
            }
        }

        stale
    }
}

/// Files that use joy-block markers: hash only the block content.
/// Other files: hash the full content.
fn hash_for_check(content: &str, rel_path: &str) -> String {
    if is_joy_block_file(rel_path) {
        if let Some(block) = extract_joy_block(content) {
            return crate::ai_templates::content_hash(block);
        }
    }
    crate::ai_templates::content_hash(content)
}

/// Compute the hash to store in the manifest for a file.
/// For joy-block files, hashes only the managed block content.
pub fn manifest_hash(content: &str, rel_path: &str) -> String {
    hash_for_check(content, rel_path)
}

/// Files that embed a joy-block (user content outside markers must be ignored).
fn is_joy_block_file(rel_path: &str) -> bool {
    rel_path.ends_with("CLAUDE.md")
        || rel_path.ends_with("QWEN.md")
        || rel_path.ends_with("copilot-instructions.md")
}

/// Extract content between `<!-- joy:start -->` and `<!-- joy:end -->` markers.
fn extract_joy_block(content: &str) -> Option<&str> {
    let start_marker = "<!-- joy:start -->";
    let end_marker = "<!-- joy:end -->";
    let start = content.find(start_marker)?;
    let end = content.find(end_marker)?;
    if end > start {
        Some(&content[start..end + end_marker.len()])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create .joy directory
        std::fs::create_dir_all(root.join(".joy")).unwrap();

        let mut manifest = AiManifest {
            version: "0.8.0".to_string(),
            ..Default::default()
        };
        manifest.set_file("claude", ".claude/SKILL.md", "abc123");
        manifest.set_file("qwen", ".qwen/SKILL.md", "def456");

        manifest.save(root).unwrap();
        let loaded = AiManifest::load(root);
        assert_eq!(manifest, loaded);
    }

    #[test]
    fn manifest_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".joy")).unwrap();

        let manifest = AiManifest::load(root);
        assert_eq!(manifest.version, "");
        assert!(manifest.tools.is_empty());
    }

    #[test]
    fn manifest_set_and_check_file() {
        let mut manifest = AiManifest::default();
        manifest.set_file("claude", ".claude/SKILL.md", "abc123");

        assert!(manifest.is_current("claude", ".claude/SKILL.md", "abc123"));
        assert!(!manifest.is_current("claude", ".claude/SKILL.md", "wrong"));
        assert!(!manifest.is_current("qwen", ".claude/SKILL.md", "abc123"));
    }

    #[test]
    fn manifest_remove_tool() {
        let mut manifest = AiManifest::default();
        manifest.set_file("claude", ".claude/SKILL.md", "abc");
        manifest.set_file("qwen", ".qwen/SKILL.md", "def");

        manifest.remove_tool("claude");
        assert!(!manifest.tools.contains_key("claude"));
        assert!(manifest.tools.contains_key("qwen"));
    }

    #[test]
    fn staleness_detects_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let mut manifest = AiManifest {
            version: "0.7.0".to_string(),
            ..Default::default()
        };
        manifest.set_file("claude", ".claude/SKILL.md", "abc");

        let stale = manifest.stale_files("0.8.0", root);
        assert_eq!(stale.len(), 1);
        assert_eq!(
            stale[0],
            ("claude".to_string(), ".claude/SKILL.md".to_string())
        );
    }

    #[test]
    fn staleness_detects_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let mut manifest = AiManifest {
            version: "0.8.0".to_string(),
            ..Default::default()
        };
        manifest.set_file("claude", ".claude/SKILL.md", "abc");

        let stale = manifest.stale_files("0.8.0", root);
        assert_eq!(stale.len(), 1);
    }

    #[test]
    fn staleness_detects_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Write a file with different content than manifest expects
        let file_path = root.join(".claude/SKILL.md");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "modified content").unwrap();

        let hash = crate::ai_templates::content_hash("original content");
        let mut manifest = AiManifest {
            version: "0.8.0".to_string(),
            ..Default::default()
        };
        manifest.set_file("claude", ".claude/SKILL.md", &hash);

        let stale = manifest.stale_files("0.8.0", root);
        assert_eq!(stale.len(), 1);
    }

    #[test]
    fn joy_block_file_detection() {
        assert!(is_joy_block_file(".claude/CLAUDE.md"));
        assert!(is_joy_block_file(".qwen/QWEN.md"));
        assert!(is_joy_block_file(".github/copilot-instructions.md"));
        assert!(!is_joy_block_file(".claude/skills/joy/SKILL.md"));
        assert!(!is_joy_block_file(".claude/agents/implementer.md"));
    }

    #[test]
    fn extract_joy_block_content() {
        let content = "user stuff\n<!-- joy:start -->\nmanaged\n<!-- joy:end -->\nmore user stuff";
        let block = extract_joy_block(content).unwrap();
        assert!(block.contains("managed"));
        assert!(!block.contains("user stuff"));
    }

    #[test]
    fn joy_block_hash_ignores_user_content() {
        let content_v1 = "user v1\n<!-- joy:start -->\nmanaged\n<!-- joy:end -->\nuser v1";
        let content_v2 =
            "user v2 changed\n<!-- joy:start -->\nmanaged\n<!-- joy:end -->\nuser v2 changed";
        let hash_v1 = hash_for_check(content_v1, ".claude/CLAUDE.md");
        let hash_v2 = hash_for_check(content_v2, ".claude/CLAUDE.md");
        assert_eq!(
            hash_v1, hash_v2,
            "user content changes must not affect hash"
        );
    }
}
