// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::config::InteractionLevel;
use super::item::Capability;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acronym: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forge: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub members: BTreeMap<String, Member>,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub capabilities: MemberCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otp_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberCapabilities {
    All,
    Specific(BTreeMap<Capability, CapabilityConfig>),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CapabilityConfig {
    #[serde(rename = "max-mode", default, skip_serializing_if = "Option::is_none")]
    pub max_mode: Option<InteractionLevel>,
    #[serde(
        rename = "max-cost-per-job",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_cost_per_job: Option<f64>,
}

// Custom serde for MemberCapabilities: "all" string or map of capabilities
impl Serialize for MemberCapabilities {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            MemberCapabilities::All => serializer.serialize_str("all"),
            MemberCapabilities::Specific(map) => map.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for MemberCapabilities {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_yaml_ng::Value::deserialize(deserializer)?;
        match &value {
            serde_yaml_ng::Value::String(s) if s == "all" => Ok(MemberCapabilities::All),
            serde_yaml_ng::Value::Mapping(_) => {
                let map: BTreeMap<Capability, CapabilityConfig> =
                    serde_yaml_ng::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(MemberCapabilities::Specific(map))
            }
            _ => Err(serde::de::Error::custom(
                "expected \"all\" or a map of capabilities",
            )),
        }
    }
}

impl Member {
    /// Create a member with the given capabilities and no auth fields.
    pub fn new(capabilities: MemberCapabilities) -> Self {
        Self {
            capabilities,
            public_key: None,
            salt: None,
            otp_hash: None,
        }
    }

    /// Check whether this member has a specific capability.
    pub fn has_capability(&self, cap: &Capability) -> bool {
        match &self.capabilities {
            MemberCapabilities::All => true,
            MemberCapabilities::Specific(map) => map.contains_key(cap),
        }
    }
}

/// Check whether a member ID represents an AI member.
pub fn is_ai_member(id: &str) -> bool {
    id.starts_with("ai:")
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
            forge: None,
            members: BTreeMap::new(),
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
        let yaml = serde_yaml_ng::to_string(&project).unwrap();
        let parsed: Project = serde_yaml_ng::from_str(&yaml).unwrap();
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
