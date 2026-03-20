// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acronym: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
    pub created: DateTime<Utc>,
}

fn default_language() -> String {
    "en".to_string()
}

impl Project {
    pub fn new(name: String, acronym: Option<String>) -> Self {
        Self {
            name,
            acronym,
            description: None,
            language: default_language(),
            created: Utc::now(),
        }
    }
}

/// Derive an acronym from a project name.
/// Takes the first letter of each word, uppercase, max 4 characters.
/// Single words use up to 3 uppercase characters.
pub fn derive_acronym(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() == 1 {
        words[0]
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(3)
            .collect::<String>()
            .to_uppercase()
    } else {
        words
            .iter()
            .filter_map(|w| w.chars().next())
            .filter(|c| c.is_alphanumeric())
            .take(4)
            .collect::<String>()
            .to_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_roundtrip() {
        let project = Project::new("Test Project".into(), Some("TP".into()));
        let yaml = serde_yml::to_string(&project).unwrap();
        let parsed: Project = serde_yml::from_str(&yaml).unwrap();
        assert_eq!(project, parsed);
    }

    #[test]
    fn derive_acronym_multi_word() {
        assert_eq!(derive_acronym("My Cool Project"), "MCP");
    }

    #[test]
    fn derive_acronym_single_word() {
        assert_eq!(derive_acronym("Joy"), "JOY");
    }

    #[test]
    fn derive_acronym_long_name() {
        assert_eq!(derive_acronym("A Very Long Project Name"), "AVLP");
    }

    #[test]
    fn derive_acronym_single_long_word() {
        assert_eq!(derive_acronym("Platform"), "PLA");
    }
}
