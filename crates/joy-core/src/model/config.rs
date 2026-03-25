// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use crate::fortune::Category;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncConfig>,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiConfig>,
    #[serde(default)]
    pub workflow: WorkflowConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowConfig {
    #[serde(rename = "auto-assign", default = "default_true")]
    pub auto_assign: bool,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self { auto_assign: true }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentsConfig {
    #[serde(default)]
    pub default: AgentRoleConfig,
    #[serde(flatten)]
    pub roles: BTreeMap<String, AgentRoleConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentRoleConfig {
    #[serde(default)]
    pub mode: InteractionLevel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InteractionLevel {
    Autonomous,
    Supervised,
    #[default]
    Collaborative,
    Interactive,
    Pairing,
}

impl std::fmt::Display for InteractionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Autonomous => write!(f, "autonomous"),
            Self::Supervised => write!(f, "supervised"),
            Self::Collaborative => write!(f, "collaborative"),
            Self::Interactive => write!(f, "interactive"),
            Self::Pairing => write!(f, "pairing"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncConfig {
    pub remote: String,
    pub auto: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputConfig {
    pub color: ColorMode,
    pub emoji: bool,
    #[serde(default)]
    pub short: bool,
    #[serde(default = "default_fortune")]
    pub fortune: bool,
    #[serde(
        rename = "fortune-category",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub fortune_category: Option<Category>,
}

fn default_fortune() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    pub tool: String,
    pub command: String,
    pub model: String,
    pub max_cost_per_job: f64,
    pub currency: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            sync: None,
            output: OutputConfig::default(),
            ai: None,
            workflow: WorkflowConfig::default(),
            agents: AgentsConfig::default(),
        }
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            color: ColorMode::Auto,
            emoji: true,
            short: false,
            fortune: true,
            fortune_category: None,
        }
    }
}

/// Return a human-readable hint for a config key, listing allowed values when
/// the field is an enum or constrained type. Derived from the Config struct
/// rather than a hand-maintained map.
pub fn field_hint(key: &str) -> Option<String> {
    let defaults = serde_json::to_value(Config::default()).ok()?;
    // Try navigating with the original key; if not found (e.g. optional fields
    // omitted by skip_serializing_if), fall back to probing directly.
    let current = navigate_json(&defaults, key);

    // Probe for enum variants regardless of whether the field is in defaults
    let candidates = probe_string_field(key);
    if !candidates.is_empty() {
        return Some(format!("allowed values: {}", candidates.join(", ")));
    }

    if let Some(current) = current {
        return match current {
            serde_json::Value::Bool(_) => Some("expected: true or false".to_string()),
            serde_json::Value::Number(_) => Some("expected: a number".to_string()),
            serde_json::Value::String(_) => Some("expected: a string".to_string()),
            _ => None,
        };
    }

    None
}

fn navigate_json<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for part in key.split('.') {
        // Try as-is first, then with hyphens/underscores swapped (YAML uses
        // hyphens, serde_json serializes Rust field names with underscores).
        current = current
            .get(part)
            .or_else(|| current.get(part.replace('-', "_")))
            .or_else(|| current.get(part.replace('_', "-")))?;
    }
    Some(current)
}

/// Try setting a config field to various string values to discover which ones
/// the schema accepts -- this reveals enum variants without hard-coding them.
/// Validates via YAML round-trip to correctly handle hyphen/underscore key
/// variants and optional fields.
fn probe_string_field(key: &str) -> Vec<String> {
    const PROBES: &[&str] = &[
        "auto",
        "always",
        "never",
        "none",
        "true",
        "false",
        "yes",
        "no",
        "on",
        "off",
        "list",
        "board",
        "calendar",
        "all",
        "tech",
        "science",
        "humor",
        "low",
        "medium",
        "high",
        "critical",
        "autonomous",
        "supervised",
        "collaborative",
        "interactive",
        "pairing",
    ];

    let mut accepted = Vec::new();
    for &candidate in PROBES {
        // Build a minimal YAML snippet with the candidate value and try
        // deserializing as Config. This uses the same path as load_config,
        // so hyphen/underscore handling matches real behavior.
        let yaml = build_yaml_for_key(key, candidate);
        let defaults_yaml = serde_yaml_ng::to_string(&Config::default()).unwrap_or_default();
        let Ok(mut base): Result<serde_json::Value, _> = serde_yaml_ng::from_str(&defaults_yaml)
        else {
            continue;
        };
        let Ok(overlay): Result<serde_json::Value, _> = serde_yaml_ng::from_str(&yaml) else {
            continue;
        };
        crate::store::deep_merge_value(&mut base, &overlay);
        if serde_json::from_value::<Config>(base).is_ok() {
            accepted.push(candidate.to_string());
        }
    }
    accepted
}

/// Build a nested YAML string from a dotted key and value.
/// e.g. "output.color" + "auto" -> "output:\n  color: auto\n"
fn build_yaml_for_key(key: &str, value: &str) -> String {
    let parts: Vec<&str> = key.split('.').collect();
    let mut yaml = String::new();
    for (i, part) in parts.iter().enumerate() {
        for _ in 0..i {
            yaml.push_str("  ");
        }
        if i == parts.len() - 1 {
            yaml.push_str(&format!("{part}: {value}\n"));
        } else {
            yaml.push_str(&format!("{part}:\n"));
        }
    }
    yaml
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrip() {
        let config = Config::default();
        let yaml = serde_yaml_ng::to_string(&config).unwrap();
        let parsed: Config = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn default_config_snapshot() {
        let config = Config::default();
        let yaml = serde_yaml_ng::to_string(&config).unwrap();
        insta::assert_snapshot!(yaml);
    }
}
