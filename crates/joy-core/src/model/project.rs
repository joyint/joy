// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::config::InteractionLevel;
use super::item::Capability;

/// Serialize the member map, resolving opaque ids to their display value when
/// in presentation mode (`--json` output, ADR-042) and keeping the raw id for
/// on-disk persistence. The map key stays a raw id in memory; only output is
/// resolved, so an id never leaves Joy in `--json` either.
fn serialize_members<S>(
    members: &BTreeMap<String, Member>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let present = crate::member_ref::presentation_active();
    let mut map = serializer.serialize_map(Some(members.len()))?;
    for (k, v) in members {
        if present {
            map.serialize_entry(&crate::member_ref::resolve_str(k), v)?;
        } else {
            map.serialize_entry(k, v)?;
        }
    }
    map.end()
}

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
    /// Member-PII privacy mode (ADR-042). Absent means `Open` (today's
    /// behaviour: cleartext e-mail in the member entry). `Anonymous` moves
    /// member e-mail into an encrypted members.yaml and keys members by an
    /// opaque id plus an `email_match` verifier. Read via `privacy_mode()`;
    /// changed only by the dedicated mode-transition command, never by a
    /// bare field write (the switch is an atomic migration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<PrivacyMode>,
    #[serde(default, skip_serializing_if = "Docs::is_empty")]
    pub docs: Docs,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        serialize_with = "serialize_members"
    )]
    pub members: BTreeMap<String, Member>,
    /// Crypt zone registry. Empty / absent means encryption is not in
    /// use; `crypt_wraps` on members and `crypt_zone` on items only have
    /// meaning relative to the zones declared here. See ADR-038 and
    /// vision/guardianship/Crypt.md.
    #[serde(default, skip_serializing_if = "CryptConfig::is_empty")]
    pub crypt: CryptConfig,
    pub created: DateTime<Utc>,
}

/// Per-project member-PII privacy mode (ADR-042). Stored in project.yaml
/// (committed, project-wide); absent means `Open`. Inspected via
/// `joy project get privacy`; the switch to `Anonymous` is an atomic
/// migration owned by the mode-transition command, not a bare set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrivacyMode {
    /// Cleartext member e-mail in project.yaml (today's behaviour).
    #[default]
    Open,
    /// Member e-mail lives in an encrypted members.yaml; project.yaml
    /// carries an opaque member id and an `email_match` verifier.
    Anonymous,
}

impl std::fmt::Display for PrivacyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Anonymous => write!(f, "anonymous"),
        }
    }
}

/// Top-level Crypt configuration. Holds the zone registry; per-member
/// wraps live on `Member.crypt_wraps`, per-item zone references live on
/// `Item.crypt_zone`. The default zone uses the conventional name
/// `"default"` and is auto-created on first `joy crypt add`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CryptConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub zones: BTreeMap<String, CryptZone>,
}

impl CryptConfig {
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }
}

/// A single Crypt zone: marked paths and project-wide properties. The
/// zone key itself is never stored in plaintext; it lives only as
/// per-member wraps under `Member.crypt_wraps[<zone-name>]` (humans) and
/// per-(operator, AI) wraps under `delegations[<ai-member>][<operator>]`
/// (AI Tool, ADR-041).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CryptZone {
    /// Path patterns (gitattributes-style globs) that belong to this
    /// zone. Empty list means item-only encryption (zone references
    /// come from items via `crypt_zone`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Per-(operator, AI) zone-key wraps for AI Tool delegations
    /// (ADR-041 §3-4). Outer key is the AI member id (e.g.
    /// `ai:claude@joy`); inner key is the operator email; value is the
    /// hex-encoded X25519 wrap of the zone key against the operator's
    /// stable delegation public key.
    ///
    /// One wrap per (operator, AI) pair, regardless of how many tokens
    /// the operator has issued. Token issuance writes nothing here; the
    /// embedded delegation private key in `--crypt` tokens is what the
    /// AI uses to unwrap (ADR-041 §5).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub delegations: BTreeMap<String, BTreeMap<String, String>>,
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
    pub const DEFAULT_ARCHITECTURE: &'static str = "ARCHITECTURE.md";
    pub const DEFAULT_VISION: &'static str = "VISION.md";
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
    pub verify_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdf_nonce: Option<String>,
    /// AES-256-GCM ciphertext of the member's identity seed, encrypted
    /// under a KEK derived from passphrase + kdf_nonce via Argon2id
    /// (ADR-039). Hex-encoded `nonce || ciphertext || tag`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_wrap_passphrase: Option<String>,
    /// AES-256-GCM ciphertext of the same seed, encrypted under a KEK
    /// derived from a recovery key via Argon2id (ADR-039). The recovery
    /// key itself is generated at `joy auth init`, displayed once, and
    /// stored externally by the user. Hex-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_wrap_recovery: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrollment_verifier: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai_delegations: BTreeMap<String, AiDelegationEntry>,
    /// Per-member Crypt zone-key wraps. Map from zone name to the
    /// hex-encoded `nonce || ciphertext || tag` produced by
    /// `joy_crypt::wrap::wrap` over the zone key. The KEK derives from
    /// the member's identity seed via HKDF-SHA256 with a fixed
    /// "crypt-member-kek" tag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub crypt_wraps: BTreeMap<String, String>,
    /// Non-reversible e-mail verifier (ADR-042 anonymous mode). Hex of
    /// HKDF-SHA256 over normalize(email) keyed by `kdf_nonce`. Present only in
    /// anonymous mode, where it replaces the cleartext e-mail map key. The
    /// platform compares this against verified account e-mails to decide
    /// membership without decrypting anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_match: Option<String>,
    /// Wrap of the members.yaml zone key for this member (ADR-042 anonymous
    /// mode). Pairwise X25519 wrap (`crypt::wrap_for_member`), unwrappable with
    /// the member's identity seed. Present only in anonymous mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members_wrap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<Attestation>,
}

/// Per-member attestation: a signature by a manage member over a stable
/// subset of the member's fields (email, capabilities, enrollment_verifier).
/// Verified locally against project.yaml by looking up the attester's
/// verify_key in the same file. The founder is the sole member allowed to
/// have no attestation in a fresh project; once any additional manage
/// member is added, that member implicitly reverse-attests the founder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attestation {
    /// The member who produced the signature (must be manage-capable at signing
    /// time). Resolves to name/e-mail on display and in `--json`; raw at rest.
    pub attester: crate::member_ref::MemberRef,
    /// The fields this signature covers. verify_key is intentionally
    /// excluded so that passphrase changes do not break existing
    /// attestations.
    pub signed_fields: AttestationSignedFields,
    /// When the attestation was produced.
    pub signed_at: chrono::DateTime<chrono::Utc>,
    /// Hex-encoded Ed25519 signature over the canonical serialization of
    /// `signed_fields`.
    pub signature: String,
}

/// The exact subset of a member's state covered by the attestation
/// signature. Changes to any of these fields invalidate the signature.
///
/// The serde key for `enrollment_verifier` is pinned to the historical
/// name `otp_hash` (per ADR-035) so signatures created before the field
/// rename remain bit-identically valid. Do not change the rename pin
/// without coordinating an attestation re-signing pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttestationSignedFields {
    pub email: String,
    pub capabilities: MemberCapabilities,
    #[serde(default, rename = "otp_hash", skip_serializing_if = "Option::is_none")]
    pub enrollment_verifier: Option<String>,
}

impl AttestationSignedFields {
    /// Produce a deterministic byte sequence for signing/verification.
    /// Stability relies on: (a) BTreeMap ordering in MemberCapabilities::Specific,
    /// (b) struct field declaration order via serde_json, (c) skip-empty rules
    /// being identical on write and read.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("AttestationSignedFields canonicalization")
    }
}

/// A stable per-(human, AI) delegation key.
///
/// Under ADR-037 the delegation seed is deterministically derived from the
/// human's Argon2id-derived identity material (`derive_key(passphrase, kdf_nonce)`)
/// plus the per-(human, AI) `delegation_salt` recorded here. Identical inputs
/// on any of the human's machines yield the same Ed25519 keypair, so the same
/// delegation is reachable from anywhere without per-machine state in
/// `project.yaml`. The matching private seed is cached at
/// `~/.local/state/joy/delegations/<project>/<ai-member>.key` (0600); a missing
/// cache is regenerated transparently from passphrase + salt at next use.
///
/// Legacy entries (created under ADR-033 §1, before ADR-037) carry no
/// `delegation_salt`. They keep working on the machine whose local cache holds
/// the matching random seed; rotating under the new code populates the salt
/// and unblocks every other machine going forward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiDelegationEntry {
    /// Public verifier of the stable delegation keypair (hex-encoded Ed25519).
    /// Used to verify the binding signature on delegation tokens.
    pub delegation_verifier: String,
    /// 32-byte hex salt feeding HKDF-SHA256 over the human's identity material
    /// (ADR-037). `None` for legacy entries created under ADR-033 §1; populated
    /// by `joy ai rotate` and by every fresh delegation issued under ADR-037.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_salt: Option<String>,
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
            verify_key: None,
            kdf_nonce: None,
            seed_wrap_passphrase: None,
            seed_wrap_recovery: None,
            enrollment_verifier: None,
            ai_delegations: BTreeMap::new(),
            crypt_wraps: BTreeMap::new(),
            email_match: None,
            members_wrap: None,
            attestation: None,
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

/// One-line description for a `joy project get` key. Returned by
/// `--describe` so the CLI is the single source of truth for what
/// each project field means. Mirrors `crate::model::config::describe_value`
/// for the config tree.
pub fn describe_value(key: &str, _value: &serde_json::Value) -> Option<String> {
    let text = match key {
        "name" => "human-readable project name",
        "acronym" => "short prefix used in item IDs",
        "description" => "one-paragraph project description",
        "language" => "project language for written artifacts (titles, comments, commits)",
        "forge" => {
            "release forge override (e.g. github, none); unset = auto-detect from git remotes"
        }
        "privacy" => {
            "member-PII privacy mode: none (default, behaves as open), open, or anonymous (e-mail in an encrypted members.yaml, opaque ids in project.yaml)"
        }
        "release.version-files" => {
            "paths whose version strings `joy release bump` rewrites; managed with `joy project set release.version-files --add/--rm/<csv>`"
        }
        "created" => "ISO timestamp when the project was initialized",
        "docs.architecture" => "path to the technical architecture document",
        "docs.vision" => "path to the product-vision document",
        "docs.contributing" => "path to the contributing guide",
        _ => return None,
    };
    Some(text.to_string())
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
            privacy: None,
            docs: Docs::default(),
            members: BTreeMap::new(),
            crypt: CryptConfig::default(),
            created: Utc::now(),
        }
    }

    /// The effective privacy mode: `Open` when unset (ADR-042).
    pub fn privacy_mode(&self) -> PrivacyMode {
        self.privacy.unwrap_or_default()
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
    fn privacy_mode_defaults_to_open() {
        let project = Project::new("T".into(), Some("T".into()));
        assert_eq!(project.privacy, None);
        assert_eq!(project.privacy_mode(), PrivacyMode::Open);
    }

    #[test]
    fn privacy_absent_from_yaml_by_default() {
        let project = Project::new("T".into(), Some("T".into()));
        let yaml = serde_yaml_ng::to_string(&project).unwrap();
        assert!(
            !yaml.contains("privacy"),
            "none (absent) is the default; got:\n{yaml}"
        );
    }

    #[test]
    fn privacy_open_serializes_explicitly() {
        let mut project = Project::new("T".into(), Some("T".into()));
        project.privacy = Some(PrivacyMode::Open);
        let yaml = serde_yaml_ng::to_string(&project).unwrap();
        assert!(yaml.contains("privacy: open"), "got:\n{yaml}");
        let parsed: Project = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(parsed.privacy, Some(PrivacyMode::Open));
    }

    #[test]
    fn privacy_mode_accessor_maps_none_and_open_to_open() {
        let mut project = Project::new("T".into(), Some("T".into()));
        assert_eq!(project.privacy_mode(), PrivacyMode::Open);
        project.privacy = Some(PrivacyMode::Open);
        assert_eq!(project.privacy_mode(), PrivacyMode::Open);
        project.privacy = Some(PrivacyMode::Anonymous);
        assert_eq!(project.privacy_mode(), PrivacyMode::Anonymous);
    }

    #[test]
    fn privacy_anonymous_roundtrips() {
        let mut project = Project::new("T".into(), Some("T".into()));
        project.privacy = Some(PrivacyMode::Anonymous);
        let yaml = serde_yaml_ng::to_string(&project).unwrap();
        assert!(yaml.contains("privacy: anonymous"), "got:\n{yaml}");
        let parsed: Project = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(parsed.privacy, Some(PrivacyMode::Anonymous));
        assert_eq!(parsed.privacy_mode(), PrivacyMode::Anonymous);
    }

    #[test]
    fn privacy_mode_display() {
        assert_eq!(PrivacyMode::Open.to_string(), "open");
        assert_eq!(PrivacyMode::Anonymous.to_string(), "anonymous");
    }

    #[test]
    fn project_roundtrip() {
        let project = Project::new("Test Project".into(), Some("TP".into()));
        let yaml = serde_yaml_ng::to_string(&project).unwrap();
        let parsed: Project = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(project, parsed);
    }

    #[test]
    fn describe_value_covers_documented_keys() {
        let dummy = serde_json::Value::Null;
        for key in &[
            "name",
            "acronym",
            "description",
            "language",
            "forge",
            "release.version-files",
            "created",
            "docs.architecture",
            "docs.vision",
            "docs.contributing",
        ] {
            assert!(
                describe_value(key, &dummy).is_some(),
                "missing description for project key {key}"
            );
        }
        assert!(describe_value("unknown", &dummy).is_none());
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
        m.verify_key = Some("aa".repeat(32));
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        let parsed: Member = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(m, parsed);
    }

    #[test]
    fn ai_delegations_yaml_roundtrip() {
        let mut m = Member::new(MemberCapabilities::All);
        m.verify_key = Some("aa".repeat(32));
        m.kdf_nonce = Some("bb".repeat(32));
        m.ai_delegations.insert(
            "ai:claude@joy".into(),
            AiDelegationEntry {
                delegation_verifier: "cc".repeat(32),
                delegation_salt: None,
                created: chrono::DateTime::parse_from_rfc3339("2026-04-15T10:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                rotated: None,
            },
        );
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        assert!(yaml.contains("ai_delegations:"));
        assert!(yaml.contains("ai:claude@joy:"));
        assert!(yaml.contains("delegation_verifier:"));
        assert!(
            !yaml.contains("delegation_salt:"),
            "unset delegation_salt should be skipped (legacy entry)"
        );
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
                delegation_verifier: "dd".repeat(32),
                delegation_salt: None,
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

    // -----------------------------------------------------------------------
    // attestation (JOY-00FA-A5) tests
    // -----------------------------------------------------------------------

    #[test]
    fn attestation_omitted_when_none() {
        let m = Member::new(MemberCapabilities::All);
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        assert!(!yaml.contains("attestation:"));
    }

    #[test]
    fn attestation_yaml_roundtrips() {
        let mut m = Member::new(MemberCapabilities::All);
        m.attestation = Some(Attestation {
            attester: "horst@example.com".into(),
            signed_fields: AttestationSignedFields {
                email: "alice@example.com".into(),
                capabilities: MemberCapabilities::All,
                enrollment_verifier: Some("ff".repeat(32)),
            },
            signed_at: chrono::DateTime::parse_from_rfc3339("2026-04-20T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            signature: "aa".repeat(32),
        });
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        assert!(yaml.contains("attestation:"));
        assert!(yaml.contains("attester: horst@example.com"));
        let parsed: Member = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn attestation_signed_fields_canonical_is_deterministic() {
        let a = AttestationSignedFields {
            email: "alice@example.com".into(),
            capabilities: MemberCapabilities::All,
            enrollment_verifier: Some("abc".into()),
        };
        let b = a.clone();
        assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn attestation_signed_fields_differ_on_capability_change() {
        let a = AttestationSignedFields {
            email: "alice@example.com".into(),
            capabilities: MemberCapabilities::All,
            enrollment_verifier: None,
        };
        let mut caps = BTreeMap::new();
        caps.insert(Capability::Implement, CapabilityConfig::default());
        let b = AttestationSignedFields {
            email: "alice@example.com".into(),
            capabilities: MemberCapabilities::Specific(caps),
            enrollment_verifier: None,
        };
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
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
    delegation_verifier: newkey
    created: "2026-04-15T10:00:00Z"
"#;
        let parsed: Member = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            parsed.ai_delegations["ai:claude@joy"].delegation_verifier,
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
