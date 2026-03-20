// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrip() {
        let config = Config::default();
        let yaml = serde_yml::to_string(&config).unwrap();
        let parsed: Config = serde_yml::from_str(&yaml).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn default_config_snapshot() {
        let config = Config::default();
        let yaml = serde_yml::to_string(&config).unwrap();
        insta::assert_snapshot!(yaml);
    }
}
