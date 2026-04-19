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
    #[serde(default, skip_serializing_if = "Docs::is_empty")]
    pub docs: Docs,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub members: BTreeMap<String, Member>,
    pub created: DateTime<Utc>,
}

/// Configurable paths to the project's reference documentation, relative to
/// the project root. Used by `joy ai init` to support existing repos with
/// non-default doc layouts and read by AI tools via `joy project get docs.<key>`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Docs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributing: Option<String>,
}

impl Docs {
    pub const DEFAULT_ARCHITECTURE: &'static str = "docs/dev/architecture/README.md";
    pub const DEFAULT_VISION: &'static str = "docs/dev/vision/README.md";
    pub const DEFAULT_CONTRIBUTING: &'static str = "CONTRIBUTING.md";

    pub fn is_empty(&self) -> bool {
        self.architecture.is_none() && self.vision.is_none() && self.contributing.is_none()
    }

    /// Configured architecture path or the default if unset.
    pub fn architecture_or_default(&self) -> &str {
        self.architecture
            .as_deref()
            .unwrap_or(Self::DEFAULT_ARCHITECTURE)
    }

    /// Configured vision path or the default if unset.
    pub fn vision_or_default(&self) -> &str {
        self.vision.as_deref().unwrap_or(Self::DEFAULT_VISION)
    }

    /// Configured contributing path or the default if unset.
    pub fn contributing_or_default(&self) -> &str {
        self.contributing
            .as_deref()
            .unwrap_or(Self::DEFAULT_CONTRIBUTING)
    }
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai_delegations: BTreeMap<String, AiDelegationEntry>,
}

/// A stable per-(human, AI) delegation key (ADR-033). The matching private
/// key lives off-repo at
/// `~/.local/state/joy/delegations/<project>/<ai-member>.key`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiDelegationEntry {
    /// Public key of the stable delegation keypair (hex-encoded Ed25519).
    pub delegation_key: String,
    /// When this delegation was first issued.
    pub created: chrono::DateTime<chrono::Utc>,
    /// When this delegation was last rotated, if ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotated: Option<chrono::DateTime<chrono::Utc>>,
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

// ---------------------------------------------------------------------------
// Mode defaults (from project.defaults.yaml, overridable in project.yaml)
// ---------------------------------------------------------------------------

/// Interaction mode defaults: a global default plus optional per-capability overrides.
/// Deserializes from flat YAML like: `{ default: collaborative, implement: autonomous }`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModeDefaults {
    /// Fallback mode when no per-capability mode is set.
    #[serde(default)]
    pub default: InteractionLevel,
    /// Per-capability mode overrides (flattened into the same map).
    #[serde(flatten, default)]
    pub capabilities: BTreeMap<Capability, InteractionLevel>,
}

/// Default capabilities granted to AI members by joy ai init.
/// Loaded from `ai-defaults.capabilities` in project.defaults.yaml.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AiDefaults {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Capability>,
}

/// Source of a resolved interaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSource {
    /// From project.defaults.yaml (Joy's recommendation).
    Default,
    /// From project.yaml agents.defaults override.
    Project,
    /// From config.yaml personal preference.
    Personal,
    /// From item-level override (future).
    Item,
    /// Clamped by max-mode from project.yaml member config.
    ProjectMax,
}

impl std::fmt::Display for ModeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::Project => write!(f, "project"),
            Self::Personal => write!(f, "personal"),
            Self::Item => write!(f, "item"),
            Self::ProjectMax => write!(f, "project max"),
        }
    }
}

/// Resolve the effective interaction mode for a given capability.
///
/// Resolution order (later wins):
/// 1. Effective defaults global mode (project.defaults.yaml merged with project.yaml)
/// 2. Effective defaults per-capability mode
/// 3. Personal config preference
///
/// All clamped by max-mode from the member's CapabilityConfig.
pub fn resolve_mode(
    capability: &Capability,
    raw_defaults: &ModeDefaults,
    effective_defaults: &ModeDefaults,
    personal_mode: Option<InteractionLevel>,
    member_cap_config: Option<&CapabilityConfig>,
) -> (InteractionLevel, ModeSource) {
    // 1. Global fallback from effective defaults
    let mut mode = effective_defaults.default;
    let mut source = if effective_defaults.default != raw_defaults.default {
        ModeSource::Project
    } else {
        ModeSource::Default
    };

    // 2. Per-capability default
    if let Some(&cap_mode) = effective_defaults.capabilities.get(capability) {
        mode = cap_mode;
        let from_raw = raw_defaults.capabilities.get(capability) == Some(&cap_mode);
        source = if from_raw {
            ModeSource::Default
        } else {
            ModeSource::Project
        };
    }

    // 3. Personal preference
    if let Some(personal) = personal_mode {
        mode = personal;
        source = ModeSource::Personal;
    }

    // 4. Clamp by max-mode (minimum interactivity required)
    if let Some(cap_config) = member_cap_config {
        if let Some(max) = cap_config.max_mode {
            if mode < max {
                mode = max;
                source = ModeSource::ProjectMax;
            }
        }
    }

    (mode, source)
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
            ai_delegations: BTreeMap::new(),
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
            docs: Docs::default(),
            members: BTreeMap::new(),
            created: Utc::now(),
        }
    }
}

/// Validate and normalize a project acronym.
///
/// Acronyms drive item ID prefixes (`ACRONYM-XXXX`) and must therefore be
/// ASCII, filesystem-safe, and short. Rules: ASCII uppercase letters (A-Z) or
/// digits (0-9), length 2-8 after trimming. Input is trimmed and uppercased;
/// the normalized form is returned on success so callers can store it as-is.
pub fn validate_acronym(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_uppercase();
    if normalized.len() < 2 || normalized.len() > 8 {
        return Err(format!(
            "acronym must be 2-8 characters, got {} ('{}')",
            normalized.len(),
            normalized
        ));
    }
    for (i, c) in normalized.chars().enumerate() {
        if !(c.is_ascii_uppercase() || c.is_ascii_digit()) {
            return Err(format!(
                "acronym character '{c}' at position {i} is not A-Z or 0-9"
            ));
        }
    }
    Ok(normalized)
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

    // -----------------------------------------------------------------------
    // ai_delegations (ADR-033) tests
    // -----------------------------------------------------------------------

    #[test]
    fn ai_delegations_omitted_when_empty() {
        let mut m = Member::new(MemberCapabilities::All);
        assert!(m.ai_delegations.is_empty());
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        assert!(
            !yaml.contains("ai_delegations"),
            "empty ai_delegations should be skipped, got: {yaml}"
        );
        // sanity: round-trips empty
        m.public_key = Some("aa".repeat(32));
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        let parsed: Member = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(m, parsed);
    }

    #[test]
    fn ai_delegations_yaml_roundtrip() {
        let mut m = Member::new(MemberCapabilities::All);
        m.public_key = Some("aa".repeat(32));
        m.salt = Some("bb".repeat(32));
        m.ai_delegations.insert(
            "ai:claude@joy".into(),
            AiDelegationEntry {
                delegation_key: "cc".repeat(32),
                created: chrono::DateTime::parse_from_rfc3339("2026-04-15T10:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                rotated: None,
            },
        );
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        assert!(yaml.contains("ai_delegations:"));
        assert!(yaml.contains("ai:claude@joy:"));
        assert!(yaml.contains("delegation_key:"));
        assert!(
            !yaml.contains("rotated:"),
            "unset rotated should be skipped"
        );

        let parsed: Member = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(m, parsed);
    }

    #[test]
    fn ai_delegations_with_rotated_roundtrips() {
        let mut m = Member::new(MemberCapabilities::All);
        let created = chrono::DateTime::parse_from_rfc3339("2026-04-01T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let rotated = chrono::DateTime::parse_from_rfc3339("2026-04-15T12:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        m.ai_delegations.insert(
            "ai:claude@joy".into(),
            AiDelegationEntry {
                delegation_key: "dd".repeat(32),
                created,
                rotated: Some(rotated),
            },
        );
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        assert!(yaml.contains("rotated:"));
        let parsed: Member = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(m.ai_delegations["ai:claude@joy"].rotated, Some(rotated));
        assert_eq!(parsed, m);
    }

    #[test]
    fn unknown_fields_from_legacy_yaml_are_ignored() {
        // project.yaml files written by older Joy versions may still carry
        // ai_tokens entries. They are silently discarded by serde default
        // behaviour and do not block deserialisation.
        let yaml = r#"
capabilities: all
public_key: aa
salt: bb
ai_tokens:
  ai:claude@joy:
    token_key: oldkey
    created: "2026-03-28T22:00:00Z"
ai_delegations:
  ai:claude@joy:
    delegation_key: newkey
    created: "2026-04-15T10:00:00Z"
"#;
        let parsed: Member = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            parsed.ai_delegations["ai:claude@joy"].delegation_key,
            "newkey"
        );
    }

    // -----------------------------------------------------------------------
    // Docs tests
    // -----------------------------------------------------------------------

    #[test]
    fn docs_defaults_when_unset() {
        let docs = Docs::default();
        assert_eq!(docs.architecture_or_default(), Docs::DEFAULT_ARCHITECTURE);
        assert_eq!(docs.vision_or_default(), Docs::DEFAULT_VISION);
        assert_eq!(docs.contributing_or_default(), Docs::DEFAULT_CONTRIBUTING);
    }

    #[test]
    fn docs_returns_configured_value() {
        let docs = Docs {
            architecture: Some("ARCHITECTURE.md".into()),
            vision: Some("docs/product/vision.md".into()),
            contributing: None,
        };
        assert_eq!(docs.architecture_or_default(), "ARCHITECTURE.md");
        assert_eq!(docs.vision_or_default(), "docs/product/vision.md");
        assert_eq!(docs.contributing_or_default(), Docs::DEFAULT_CONTRIBUTING);
    }

    #[test]
    fn docs_omitted_from_yaml_when_empty() {
        let project = Project::new("X".into(), None);
        let yaml = serde_yaml_ng::to_string(&project).unwrap();
        assert!(
            !yaml.contains("docs:"),
            "empty docs should be skipped, got: {yaml}"
        );
    }

    #[test]
    fn docs_present_in_yaml_when_set() {
        let mut project = Project::new("X".into(), None);
        project.docs.architecture = Some("ARCHITECTURE.md".into());
        let yaml = serde_yaml_ng::to_string(&project).unwrap();
        assert!(yaml.contains("docs:"), "docs block expected: {yaml}");
        assert!(yaml.contains("architecture: ARCHITECTURE.md"));
        assert!(!yaml.contains("vision:"), "unset fields should be skipped");
    }

    #[test]
    fn docs_yaml_roundtrip_with_overrides() {
        let yaml = r#"
name: Existing
language: en
docs:
  architecture: ARCHITECTURE.md
  contributing: docs/CONTRIBUTING.md
created: 2026-01-01T00:00:00Z
"#;
        let parsed: Project = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.docs.architecture.as_deref(), Some("ARCHITECTURE.md"));
        assert_eq!(parsed.docs.vision, None);
        assert_eq!(
            parsed.docs.contributing.as_deref(),
            Some("docs/CONTRIBUTING.md")
        );
        assert_eq!(parsed.docs.vision_or_default(), Docs::DEFAULT_VISION);
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

    // -----------------------------------------------------------------------
    // validate_acronym tests
    // -----------------------------------------------------------------------

    #[test]
    fn validate_acronym_accepts_real_project_acronyms() {
        for a in ["JI", "JOT", "JOY", "JON", "JP", "JAPP", "JOYC", "JISITE"] {
            assert_eq!(validate_acronym(a).unwrap(), a, "rejected real acronym {a}");
        }
    }

    #[test]
    fn validate_acronym_accepts_alphanumeric() {
        assert_eq!(validate_acronym("V2").unwrap(), "V2");
        assert_eq!(validate_acronym("A1B2").unwrap(), "A1B2");
    }

    #[test]
    fn validate_acronym_normalizes_case_and_whitespace() {
        assert_eq!(validate_acronym("jyn").unwrap(), "JYN");
        assert_eq!(validate_acronym("Jyn").unwrap(), "JYN");
        assert_eq!(validate_acronym("  jyn  ").unwrap(), "JYN");
    }

    #[test]
    fn validate_acronym_rejects_too_short() {
        assert!(validate_acronym("").is_err());
        assert!(validate_acronym("J").is_err());
        assert!(validate_acronym(" J ").is_err());
    }

    #[test]
    fn validate_acronym_rejects_too_long() {
        assert!(validate_acronym("ABCDEFGHI").is_err());
    }

    #[test]
    fn validate_acronym_rejects_non_alnum() {
        assert!(validate_acronym("JY-N").is_err());
        assert!(validate_acronym("JY N").is_err());
        assert!(validate_acronym("JY_N").is_err());
        assert!(validate_acronym("JY.N").is_err());
    }

    #[test]
    fn validate_acronym_rejects_non_ascii() {
        assert!(validate_acronym("AEBC").is_ok());
        assert!(validate_acronym("ABC").is_ok());
        assert!(validate_acronym("\u{00c4}BC").is_err());
    }

    // -----------------------------------------------------------------------
    // ModeDefaults deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn mode_defaults_flat_yaml_roundtrip() {
        let yaml = r#"
default: interactive
implement: collaborative
review: pairing
"#;
        let parsed: ModeDefaults = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.default, InteractionLevel::Interactive);
        assert_eq!(
            parsed.capabilities[&Capability::Implement],
            InteractionLevel::Collaborative
        );
        assert_eq!(
            parsed.capabilities[&Capability::Review],
            InteractionLevel::Pairing
        );
    }

    #[test]
    fn mode_defaults_empty_yaml() {
        let yaml = "{}";
        let parsed: ModeDefaults = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.default, InteractionLevel::Collaborative);
        assert!(parsed.capabilities.is_empty());
    }

    #[test]
    fn mode_defaults_only_default() {
        let yaml = "default: pairing";
        let parsed: ModeDefaults = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.default, InteractionLevel::Pairing);
        assert!(parsed.capabilities.is_empty());
    }

    #[test]
    fn ai_defaults_yaml_roundtrip() {
        let yaml = r#"
capabilities:
  - implement
  - review
"#;
        let parsed: AiDefaults = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.capabilities.len(), 2);
        assert_eq!(parsed.capabilities[0], Capability::Implement);
    }

    // -----------------------------------------------------------------------
    // resolve_mode tests
    // -----------------------------------------------------------------------

    fn defaults_with_mode(mode: InteractionLevel) -> ModeDefaults {
        ModeDefaults {
            default: mode,
            ..Default::default()
        }
    }

    fn defaults_with_cap_mode(cap: Capability, mode: InteractionLevel) -> ModeDefaults {
        let mut d = ModeDefaults::default();
        d.capabilities.insert(cap, mode);
        d
    }

    #[test]
    fn resolve_mode_uses_global_default() {
        let raw = defaults_with_mode(InteractionLevel::Collaborative);
        let effective = raw.clone();
        let (mode, source) = resolve_mode(&Capability::Implement, &raw, &effective, None, None);
        assert_eq!(mode, InteractionLevel::Collaborative);
        assert_eq!(source, ModeSource::Default);
    }

    #[test]
    fn resolve_mode_uses_per_capability_default() {
        let raw = defaults_with_cap_mode(Capability::Review, InteractionLevel::Interactive);
        let effective = raw.clone();
        let (mode, source) = resolve_mode(&Capability::Review, &raw, &effective, None, None);
        assert_eq!(mode, InteractionLevel::Interactive);
        assert_eq!(source, ModeSource::Default);
    }

    #[test]
    fn resolve_mode_project_override_detected() {
        let raw = defaults_with_cap_mode(Capability::Implement, InteractionLevel::Collaborative);
        let effective =
            defaults_with_cap_mode(Capability::Implement, InteractionLevel::Interactive);
        let (mode, source) = resolve_mode(&Capability::Implement, &raw, &effective, None, None);
        assert_eq!(mode, InteractionLevel::Interactive);
        assert_eq!(source, ModeSource::Project);
    }

    #[test]
    fn resolve_mode_personal_overrides_default() {
        let raw = defaults_with_mode(InteractionLevel::Collaborative);
        let effective = raw.clone();
        let (mode, source) = resolve_mode(
            &Capability::Implement,
            &raw,
            &effective,
            Some(InteractionLevel::Pairing),
            None,
        );
        assert_eq!(mode, InteractionLevel::Pairing);
        assert_eq!(source, ModeSource::Personal);
    }

    #[test]
    fn resolve_mode_max_mode_clamps_upward() {
        let raw = defaults_with_mode(InteractionLevel::Autonomous);
        let effective = raw.clone();
        let cap_config = CapabilityConfig {
            max_mode: Some(InteractionLevel::Supervised),
            ..Default::default()
        };
        let (mode, source) = resolve_mode(
            &Capability::Implement,
            &raw,
            &effective,
            None,
            Some(&cap_config),
        );
        assert_eq!(mode, InteractionLevel::Supervised);
        assert_eq!(source, ModeSource::ProjectMax);
    }

    #[test]
    fn resolve_mode_max_mode_does_not_lower() {
        let raw = defaults_with_mode(InteractionLevel::Pairing);
        let effective = raw.clone();
        let cap_config = CapabilityConfig {
            max_mode: Some(InteractionLevel::Supervised),
            ..Default::default()
        };
        let (mode, source) = resolve_mode(
            &Capability::Implement,
            &raw,
            &effective,
            None,
            Some(&cap_config),
        );
        // Pairing > Supervised, so no clamping
        assert_eq!(mode, InteractionLevel::Pairing);
        assert_eq!(source, ModeSource::Default);
    }

    #[test]
    fn resolve_mode_personal_clamped_by_max() {
        let raw = defaults_with_mode(InteractionLevel::Collaborative);
        let effective = raw.clone();
        let cap_config = CapabilityConfig {
            max_mode: Some(InteractionLevel::Interactive),
            ..Default::default()
        };
        let (mode, source) = resolve_mode(
            &Capability::Implement,
            &raw,
            &effective,
            Some(InteractionLevel::Autonomous),
            Some(&cap_config),
        );
        // Personal is Autonomous but max is Interactive, clamp up
        assert_eq!(mode, InteractionLevel::Interactive);
        assert_eq!(source, ModeSource::ProjectMax);
    }

    // -----------------------------------------------------------------------
    // Item mode serialization
    // -----------------------------------------------------------------------

    #[test]
    fn item_mode_field_roundtrip() {
        use crate::model::item::{Item, ItemType, Priority};

        let mut item = Item::new(
            "TST-0001".into(),
            "Test".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        item.mode = Some(InteractionLevel::Pairing);

        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        assert!(yaml.contains("mode: pairing"), "mode field not serialized");

        let parsed: Item = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(parsed.mode, Some(InteractionLevel::Pairing));
    }

    #[test]
    fn item_mode_field_absent_when_none() {
        use crate::model::item::{Item, ItemType, Priority};

        let item = Item::new(
            "TST-0002".into(),
            "Test".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        assert_eq!(item.mode, None);

        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        assert!(
            !yaml.contains("mode:"),
            "mode field should not appear when None"
        );
    }

    #[test]
    fn item_mode_deserialized_from_existing_yaml() {
        let yaml = r#"
id: TST-0003
title: Test
type: task
status: new
priority: medium
mode: interactive
created: "2026-01-01T00:00:00+00:00"
updated: "2026-01-01T00:00:00+00:00"
"#;
        let item: crate::model::item::Item = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(item.mode, Some(InteractionLevel::Interactive));
    }

    // -----------------------------------------------------------------------
    // Full four-layer resolution scenario
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_mode_full_scenario() {
        // Joy default: implement = collaborative
        let raw = defaults_with_cap_mode(Capability::Implement, InteractionLevel::Collaborative);
        // Project override: implement = interactive
        let effective =
            defaults_with_cap_mode(Capability::Implement, InteractionLevel::Interactive);
        // Personal preference: autonomous
        let personal = Some(InteractionLevel::Autonomous);
        // Project max-mode: supervised (minimum interactivity)
        let cap_config = CapabilityConfig {
            max_mode: Some(InteractionLevel::Supervised),
            ..Default::default()
        };

        let (mode, source) = resolve_mode(
            &Capability::Implement,
            &raw,
            &effective,
            personal,
            Some(&cap_config),
        );

        // Personal (autonomous) < max (supervised), so clamped up to supervised
        assert_eq!(mode, InteractionLevel::Supervised);
        assert_eq!(source, ModeSource::ProjectMax);
    }

    #[test]
    fn resolve_mode_all_layers_no_clamping() {
        // Joy default: implement = collaborative
        let raw = defaults_with_cap_mode(Capability::Implement, InteractionLevel::Collaborative);
        // Project override: implement = interactive
        let effective =
            defaults_with_cap_mode(Capability::Implement, InteractionLevel::Interactive);
        // Personal preference: pairing (more interactive than project)
        let personal = Some(InteractionLevel::Pairing);
        // No max-mode
        let cap_config = CapabilityConfig::default();

        let (mode, source) = resolve_mode(
            &Capability::Implement,
            &raw,
            &effective,
            personal,
            Some(&cap_config),
        );

        // Personal wins, no clamping
        assert_eq!(mode, InteractionLevel::Pairing);
        assert_eq!(source, ModeSource::Personal);
    }
}
