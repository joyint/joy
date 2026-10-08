// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::config::InteractionLevel;
use super::item::Capability;
use crate::error::JoyError;

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
    privacy: Option<PrivacyMode>,
    #[serde(default, skip_serializing_if = "Docs::is_empty")]
    pub docs: Docs,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        serialize_with = "serialize_members"
    )]
    members: BTreeMap<String, Member>,
    /// Crypt zone registry. Empty / absent means encryption is not in
    /// use; `crypt_wraps` on members and `crypt_zone` on items only have
    /// meaning relative to the zones declared here. See ADR-038 and
    /// vision/guardianship/Crypt.md.
    #[serde(default, skip_serializing_if = "CryptConfig::is_empty")]
    pub crypt: CryptConfig,
    pub created: DateTime<Utc>,
    /// How this project's members are kept on disk. Not part of the
    /// file: [`crate::store`] reads it off what it finds and writes back
    /// in the same shape.
    #[serde(skip)]
    layout: MemberLayout,
}

/// Where a project keeps its members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemberLayout {
    /// The whole member map inside project.yaml: every project written
    /// before the member files (JI-019D-46), until a person brings it
    /// over.
    #[default]
    InProject,
    /// One file per member under `.joy/members/`; project.yaml lists
    /// their ids.
    Files,
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
    /// An AI member's ONE interaction level: the most the project allows
    /// it to do on its own, part of what a manager signs for it
    /// (JI-019D-46, [`Granted`]). A person has none. In a project from
    /// before the member files this was the member's default level.
    #[serde(
        default,
        rename = "interaction-level",
        skip_serializing_if = "Option::is_none"
    )]
    pub interaction_level: Option<InteractionLevel>,
    /// The ACP adapter that runs an AI member. Since JOY-0231-74 the id is
    /// the tool name itself (claude | vibe | qwen | mock); first-generation
    /// pins (claude-code, mistral-vibe, qwen-code) are rewritten by the
    /// silent migration. Only meaningful on `ai:*` members. Set when the AI
    /// member is added; the rest of its key-bound ACP config (key, model,
    /// budget, guardrail) lives in the platform DB, not the repo (JI-0164 as
    /// revised by JI-0166-D8: no agent mode is stored anywhere). None on human
    /// members and on AI members with no adapter yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
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
    /// The model an AI member runs on for everybody, as its adapter
    /// names it, picked by a manager. None leaves the choice to each
    /// person (`AiDelegationEntry::model`). Member files only
    /// (JI-019D-46).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The signature a manager put under an AI member's capabilities and
    /// level: the project maximum. Member files only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted: Option<Granted>,
    /// Who brought a person into the project. Member files only; the
    /// attestation above is what older projects carry instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// The name of this member's file under `.joy/members/`, without the
    /// extension. Never written into a file: the file name says it.
    #[serde(skip)]
    pub file_id: Option<String>,
}

/// The signature under an AI member's project maximum (JI-019D-46): a
/// person with the manage capability signed the member's capabilities
/// and level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Granted {
    pub by: crate::member_ref::MemberRef,
    pub at: chrono::DateTime<chrono::Utc>,
    /// Hex-encoded Ed25519 signature over [`grant_text`].
    pub signature: String,
}

/// Who brought a person into the project and when (JI-019D-46). It is
/// made once, when the invitation is issued, and never touched again:
/// what the person may do is not part of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Origin {
    pub attester: crate::member_ref::MemberRef,
    pub signed_at: chrono::DateTime<chrono::Utc>,
    /// Hex-encoded Ed25519 signature over [`origin_text`]. Absent on a
    /// member that was in the project before the member files; `commit`
    /// then names where the attestation of that time can be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// The hash of the invitation's one-time password the signature
    /// covers. Kept here because the member's own copy is cleared once
    /// the invitation is redeemed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invitation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

/// What a person allows an AI member that acts for them: at most the
/// project maximum, signed with the person's own key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DelegationGrant {
    pub capabilities: Vec<Capability>,
    pub level: InteractionLevel,
    pub signed_at: chrono::DateTime<chrono::Utc>,
    /// Hex-encoded Ed25519 signature over [`grant_text`].
    pub signature: String,
}

/// The text a grant signature covers: the AI member's name, its
/// capabilities in their fixed order, the level, whose grant it is
/// (`project` for the maximum, else the delegating member's id) and the
/// project. A text, and not the YAML, so that no change to the file
/// format ever touches a signature.
pub fn grant_text(
    name: &str,
    capabilities: &[Capability],
    level: InteractionLevel,
    scope: &str,
    project_id: &str,
) -> String {
    let mut caps: Vec<Capability> = capabilities.to_vec();
    caps.sort();
    caps.dedup();
    let caps: Vec<String> = caps.iter().map(|c| c.to_string()).collect();
    format!("{name}|{}|{level}|{scope}|{project_id}", caps.join(","))
}

/// The text an origin signature covers: the project, the member and the
/// hash of the invitation's one-time password.
pub fn origin_text(project_id: &str, member: &str, otp_hash: &str) -> String {
    format!("{project_id}|{member}|{otp_hash}")
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
/// An entry without a `delegation_salt` cannot re-derive its key and counts
/// as NOT delegated everywhere ([`Member::delegation_usable`]); delegating
/// again (`joy ai rotate`, or the app's Delegate action) records the salt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiDelegationEntry {
    /// Public verifier of the stable delegation keypair (hex-encoded Ed25519).
    /// Used to verify the binding signature on delegation tokens.
    pub delegation_verifier: String,
    /// 32-byte hex salt feeding HKDF-SHA256 over the human's identity material
    /// (ADR-037). Every fresh delegation records it; an entry still without
    /// one (written before ADR-037 existed) counts as NOT delegated until a
    /// new delegation fills it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_salt: Option<String>,
    /// When this delegation was first issued.
    pub created: chrono::DateTime<chrono::Utc>,
    /// When this delegation was last rotated, if ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotated: Option<chrono::DateTime<chrono::Utc>>,
    /// What the delegating person allows this AI member (JI-019D-46).
    /// None means the project maximum applies as it stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<DelegationGrant>,
    /// The model this person runs the AI member on, where the project
    /// leaves the choice to them: the member itself names none. None is
    /// what the tool takes by itself. See `auth::grants::model_for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberCapabilities {
    All,
    Specific(BTreeMap<Capability, CapabilityConfig>),
}

impl MemberCapabilities {
    /// Every capability an AI member can hold: all of them but manage,
    /// which an AI member never holds.
    pub fn all_for_ai() -> Self {
        MemberCapabilities::Specific(
            Capability::ALL
                .iter()
                .filter(|cap| **cap != Capability::Manage)
                .map(|cap| (*cap, CapabilityConfig::default()))
                .collect(),
        )
    }
}

/// What hung on a single capability of a member in a project from before
/// the member files. Nothing hangs on a capability any more (JI-019D-46):
/// a member holds it or not. The two fields are read from such a project
/// and from nowhere else: the migration takes the most careful floor as
/// the AI member's one level and then empties them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CapabilityConfig {
    /// The per-capability default level of the older layout.
    #[serde(
        rename = "interaction-level",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub interaction_level: Option<InteractionLevel>,
    /// Floor on human oversight: the resolved level is clamped up to this
    /// (toward `Proposing`), never relaxed below it.
    #[serde(
        rename = "max-interaction-level",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_interaction_level: Option<InteractionLevel>,
}

// ---------------------------------------------------------------------------
// Interaction defaults (from project.defaults.yaml, overridable in project.yaml)
// ---------------------------------------------------------------------------

/// Interaction-level defaults: a global default plus optional per-capability overrides.
/// Deserializes from flat YAML like: `{ default: proposing, implement: confirmed }`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InteractionLevelDefaults {
    /// Fallback interaction level when no per-capability override is set.
    #[serde(default)]
    pub default: InteractionLevel,
    /// Per-capability interaction-level overrides (flattened into the same map).
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
    /// Whether this member's delegation to `ai` is USABLE: the entry can
    /// re-derive its delegation key from the member's passphrase (it
    /// carries the ADR-037 salt). An entry without one is a broken
    /// artifact — `joy ai init` never writes such a thing — and counts
    /// as NOT delegated (operator rule 2026-08-02): the UI greys the
    /// member and offers Delegate, which rewrites the entry properly.
    /// THE one predicate for every surface; hosts never re-derive it.
    pub fn delegation_usable(&self, ai: &str) -> bool {
        self.delegation_to(ai)
            .is_some_and(|entry| entry.delegation_salt.is_some())
    }

    /// This member's delegation to the AI member `ai`, whichever way the
    /// AI member's id is written (its name, or the older
    /// `ai:<name>@joy`): a token issued before a project was brought
    /// over names it the old way.
    pub fn delegation_to(&self, ai: &str) -> Option<&AiDelegationEntry> {
        self.ai_delegations.get(ai).or_else(|| {
            let name = ai_member_name(ai);
            self.ai_delegations
                .iter()
                .find(|(key, _)| ai_member_name(key) == name)
                .map(|(_, entry)| entry)
        })
    }

    /// [`Member::delegation_to`], to change the entry.
    pub fn delegation_to_mut(&mut self, ai: &str) -> Option<&mut AiDelegationEntry> {
        let key = self.delegation_key(ai)?;
        self.ai_delegations.get_mut(&key)
    }

    /// Record this member's delegation to `ai`, in place of the one they
    /// had, under whichever spelling of the AI member's name it stood.
    ///
    /// The model the person picked stays with them across a new
    /// delegation: it hangs on no key, so delegating again is no reason
    /// to lose it.
    pub fn put_delegation(
        &mut self,
        ai: impl Into<String>,
        mut entry: AiDelegationEntry,
    ) -> Option<AiDelegationEntry> {
        let ai = ai.into();
        let key = self.delegation_key(&ai).unwrap_or(ai);
        if entry.model.is_none() {
            entry.model = self.ai_delegations.get(&key).and_then(|d| d.model.clone());
        }
        self.ai_delegations.insert(key, entry)
    }

    /// Take this member's delegation to `ai` away.
    pub fn drop_delegation(&mut self, ai: &str) -> Option<AiDelegationEntry> {
        let key = self.delegation_key(ai)?;
        self.ai_delegations.remove(&key)
    }

    /// The key this member's delegation to `ai` stands under, in either
    /// spelling of the AI member's name.
    fn delegation_key(&self, ai: &str) -> Option<String> {
        if self.ai_delegations.contains_key(ai) {
            return Some(ai.to_string());
        }
        let name = ai_member_name(ai);
        self.ai_delegations
            .keys()
            .find(|key| ai_member_name(key) == name)
            .cloned()
    }

    /// Create a member with the given capabilities and no auth fields.
    pub fn new(capabilities: MemberCapabilities) -> Self {
        Self {
            capabilities,
            interaction_level: None,
            adapter: None,
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
            model: None,
            granted: None,
            origin: None,
            file_id: None,
        }
    }

    /// Check whether this member has a specific capability.
    pub fn has_capability(&self, cap: &Capability) -> bool {
        match &self.capabilities {
            MemberCapabilities::All => true,
            MemberCapabilities::Specific(map) => map.contains_key(cap),
        }
    }

    /// Replace the whole capability set. Capabilities present in both the
    /// old and new set keep their existing [`CapabilityConfig`]
    /// (interaction-level default, max-interaction-level / max-cost floors);
    /// newly granted capabilities start from the default (no floors).
    /// Switching to or from [`MemberCapabilities::All`]
    /// replaces the set wholesale.
    ///
    /// Editing capabilities invalidates any stored attestation; callers
    /// must re-sign (see `joy project member edit`).
    pub fn set_capabilities(&mut self, caps: MemberCapabilities) {
        self.capabilities = match (&self.capabilities, caps) {
            (MemberCapabilities::Specific(old), MemberCapabilities::Specific(mut next)) => {
                for (cap, cfg) in next.iter_mut() {
                    if let Some(prev) = old.get(cap) {
                        *cfg = prev.clone();
                    }
                }
                MemberCapabilities::Specific(next)
            }
            (_, other) => other,
        };
    }
}

pub use joy_model::{ai_member_name, is_ai_member};

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
            "forge override: which forge plugin answers for this project and where releases go (github, gitlab, gitea, none); unset = auto-detect from git remotes. Set it for an instance nobody is signed in to locally, e.g. a self-hosted GitLab, a GitHub Enterprise Server, or any Gitea/Forgejo"
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
            layout: MemberLayout::default(),
        }
    }

    /// How the members are kept on disk.
    pub fn member_layout(&self) -> MemberLayout {
        self.layout
    }

    /// Privileged: only the store (reading what is on disk) and the
    /// migration that moves a project over say how members are kept.
    pub(crate) fn set_member_layout(&mut self, layout: MemberLayout) {
        self.layout = layout;
    }

    /// The effective privacy mode: `Open` when unset (ADR-042).
    pub fn privacy_mode(&self) -> PrivacyMode {
        self.privacy.unwrap_or_default()
    }

    /// The raw privacy field. `None` means unset (which is `Open`). Prefer
    /// [`Self::privacy_mode`] unless you must distinguish an explicit `open`
    /// from an unset field, e.g. deciding whether to prune the key on disk.
    pub fn privacy(&self) -> Option<PrivacyMode> {
        self.privacy
    }

    /// Set the privacy field to a non-anonymous value (`open` or unset). This is
    /// the only public privacy writer: it refuses to set `Anonymous` and refuses
    /// to write when the project is already anonymous, because both directions of
    /// the anonymous boundary are atomic migrations
    /// ([`crate::privacy::switch_to_anonymous`] / [`switch_to_open`]), never a
    /// bare field write. Used by `joy project set privacy open|none` on an
    /// already-open project.
    pub fn set_privacy_non_anonymous(&mut self, mode: Option<PrivacyMode>) -> Result<(), JoyError> {
        if matches!(mode, Some(PrivacyMode::Anonymous)) {
            return Err(JoyError::Other(
                "switching to anonymous is a migration; use switch_to_anonymous".into(),
            ));
        }
        if self.privacy_mode() == PrivacyMode::Anonymous {
            return Err(JoyError::Other(
                "leaving anonymous is a migration; use switch_to_open, not a field write".into(),
            ));
        }
        self.privacy = mode;
        Ok(())
    }

    // --- Member access (ADR-042) -------------------------------------------
    //
    // The member map is keyed by a value that depends on the privacy mode: the
    // cleartext e-mail in `open` mode, an opaque id in `anonymous` mode. The map
    // itself is private so no call site can index it by a raw e-mail (which only
    // works in `open` mode and silently fails in `anonymous` mode). All access
    // goes through the methods below, which state in their name which key space
    // the caller holds. This is the lookup-direction counterpart to `MemberRef`
    // guarding the display direction: the privacy guarantee is bound to the
    // type, not to each call site remembering to resolve.

    /// Resolve the member-map key for a git e-mail, honoring the privacy mode.
    /// `open`: the key is the e-mail itself; `anonymous`: the opaque id whose
    /// stored `email_match` verifies against the e-mail. `None` when the e-mail
    /// is not a member.
    pub fn member_key_for_email(&self, email: &str) -> Option<String> {
        if self.privacy_mode() != PrivacyMode::Anonymous {
            return self.members.contains_key(email).then(|| email.to_string());
        }
        for (id, member) in &self.members {
            if let (Some(verifier), Some(nonce)) = (&member.email_match, &member.kdf_nonce) {
                if crate::member_id::email_match(email, nonce).ok().as_deref()
                    == Some(verifier.as_str())
                {
                    return Some(id.clone());
                }
            }
        }
        None
    }

    /// Look up a member by their git e-mail, honoring the privacy mode.
    pub fn member_by_email(&self, email: &str) -> Option<&Member> {
        let key = self.member_key_for_email(email)?;
        self.members.get(&key)
    }

    /// Mutable lookup by git e-mail, honoring the privacy mode.
    pub fn member_by_email_mut(&mut self, email: &str) -> Option<&mut Member> {
        let key = self.member_key_for_email(email)?;
        self.members.get_mut(&key)
    }

    /// The id an AI member called `name` has in this project: the name
    /// itself with member files (JI-019D-46), `ai:<name>@joy` in a
    /// project from before. Either spelling may be handed in.
    pub fn ai_member_id(&self, name: &str) -> String {
        let name = ai_member_name(name);
        match self.layout {
            MemberLayout::Files => name.to_string(),
            MemberLayout::InProject => format!("ai:{name}@joy"),
        }
    }

    /// The member work is assigned to when a person names `member`, in
    /// the spelling this project keeps them under, or in words why work
    /// cannot be assigned to them. A person is named by an address (or,
    /// in an anonymous project, by their id) and need not be a member
    /// yet; an AI member is named by its name, in either spelling, and
    /// has to be one of this project's.
    pub fn assignee(&self, member: &str) -> Result<String, String> {
        if !is_ai_member(member) {
            if member.contains('@') || crate::member_id::is_opaque_member_id(member) {
                return Ok(member.to_string());
            }
            return Err(format!(
                "{member} is neither an address nor the name of an AI member"
            ));
        }
        self.member_key(member).ok_or_else(|| {
            format!(
                "this project has no AI member named {}",
                ai_member_name(member)
            )
        })
    }

    /// The assignee an assignment is taken away from: the member as the
    /// project keeps them, and as they were named where the project does
    /// not know them any more. Taking away asks nothing.
    pub fn former_assignee(&self, member: &str) -> String {
        self.member_key(member)
            .unwrap_or_else(|| member.to_string())
    }

    /// The key a member is stored under, for a key as anybody may write
    /// it. A person's key is taken as it is. An AI member is found under
    /// its name and under `ai:<name>@joy` alike, whichever of the two
    /// this project uses: a command typed the old way, a token issued
    /// before the project was brought over and a chat written back then
    /// all keep meaning the same member.
    pub fn member_key(&self, key: &str) -> Option<String> {
        if self.members.contains_key(key) {
            return Some(key.to_string());
        }
        if !is_ai_member(key) {
            return None;
        }
        let name = ai_member_name(key);
        [name.to_string(), format!("ai:{name}@joy")]
            .into_iter()
            .find(|candidate| self.members.contains_key(candidate))
    }

    /// Look up a member by their at-rest map key (an AI member's name, or
    /// an already resolved key). Use [`Self::member_by_email`] when you
    /// only have an e-mail.
    pub fn member_by_key(&self, key: &str) -> Option<&Member> {
        let key = self.member_key(key)?;
        self.members.get(&key)
    }

    /// Mutable lookup by at-rest map key.
    pub fn member_by_key_mut(&mut self, key: &str) -> Option<&mut Member> {
        let key = self.member_key(key)?;
        self.members.get_mut(&key)
    }

    /// Whether a member with this at-rest map key exists.
    pub fn has_member_key(&self, key: &str) -> bool {
        self.member_key(key).is_some()
    }

    /// Iterate `(key, member)` pairs. Keys are raw at-rest ids; wrap them in a
    /// [`crate::member_ref::MemberRef`] before any output.
    pub fn members(&self) -> impl Iterator<Item = (&String, &Member)> {
        self.members.iter()
    }

    /// Iterate the at-rest member keys.
    pub fn member_keys(&self) -> impl Iterator<Item = &String> {
        self.members.keys()
    }

    /// Iterate the member entries.
    pub fn member_values(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    /// Number of registered members.
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Whether the project has any members.
    pub fn has_members(&self) -> bool {
        !self.members.is_empty()
    }

    /// Register a brand-new member, placing it correctly for the privacy mode.
    ///
    /// `id` is the caller-facing identity: an `ai:` synthetic id (no PII,
    /// identical in both modes) or a cleartext e-mail for a human. AI members
    /// are inserted under their synthetic id in either mode; human members under
    /// their e-mail in `open` mode. Adding a *human* to an `anonymous` project is
    /// refused: the opaque id derives from a verify_key the new member does not
    /// have yet, and a naive e-mail insert would leak cleartext PII into the
    /// committed project.yaml. That onboarding flow is tracked separately
    /// (JOY-01C3-A7). The caller owns any duplicate-key policy.
    pub fn register_member(&mut self, id: &str, member: Member) -> Result<(), JoyError> {
        if !is_ai_member(id) && self.privacy_mode() == PrivacyMode::Anonymous {
            return Err(JoyError::Other(format!(
                "cannot register human member {id} in an anonymous project: \
                 anonymous human onboarding is not yet supported (JOY-01C3-A7)"
            )));
        }
        // An AI member never holds manage, whichever host adds it and
        // however its capabilities were named (`all` included): the guard
        // refuses it every manage action anyway, and a capability that
        // can never be used must not stand in the project.
        if is_ai_member(id) && member.has_capability(&Capability::Manage) {
            return Err(JoyError::Other(
                "an AI member never holds the manage capability: name what it may do".into(),
            ));
        }
        let mut member = member;
        if self.layout == MemberLayout::Files && member.file_id.is_none() {
            member.file_id = Some(crate::member_id::new_member_file_id());
        }
        // An AI member is stored under the spelling this project uses,
        // however the caller wrote it.
        let key = if is_ai_member(id) {
            self.ai_member_id(id)
        } else {
            id.to_string()
        };
        self.members.insert(key, member);
        Ok(())
    }

    /// Put a member into the project as a file read from disk would:
    /// no rule of [`Project::register_member`] is asked. For tests of
    /// what joy does with a project written before a rule existed (an AI
    /// member that holds manage, say).
    #[cfg(test)]
    pub(crate) fn insert_as_read(&mut self, id: &str, member: Member) {
        self.members.insert(id.to_string(), member);
    }

    /// Remove a member by at-rest map key, returning the removed entry.
    pub fn remove_member(&mut self, key: &str) -> Option<Member> {
        let key = self.member_key(key)?;
        self.members.remove(&key)
    }

    /// Privileged: take the whole member map out, leaving it empty. Only the
    /// privacy-mode migration ([`crate::privacy`]) rekeys the map wholesale;
    /// every other caller uses the typed accessors above.
    pub(crate) fn take_members(&mut self) -> BTreeMap<String, Member> {
        std::mem::take(&mut self.members)
    }

    /// Privileged: the member map as it is, for the store that writes it.
    pub(crate) fn member_map(&self) -> &BTreeMap<String, Member> {
        &self.members
    }

    /// Privileged: replace the whole member map. See [`Self::take_members`].
    pub(crate) fn replace_members(&mut self, members: BTreeMap<String, Member>) {
        self.members = members;
    }

    /// Privileged: set the privacy mode field directly. Only the migration in
    /// [`crate::privacy`] may flip this; it is an atomic part of rekeying the
    /// member map, never a bare field write elsewhere.
    pub(crate) fn set_privacy_mode(&mut self, mode: Option<PrivacyMode>) {
        self.privacy = mode;
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

/// Validate and normalize a project language: two ASCII letters, the
/// ISO 639-1 shape (`en`, `de`). Trimmed and lowercased on success.
pub fn validate_language(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() == 2 && normalized.chars().all(|c| c.is_ascii_lowercase()) {
        Ok(normalized)
    } else {
        Err(format!(
            "language must be two letters like en or de, got '{}'",
            value.trim()
        ))
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
    fn a_delegation_without_its_salt_counts_as_not_delegated() {
        // Operator rule 2026-08-02: an entry whose key cannot re-derive
        // is a broken artifact, not a delegation — every surface greys it
        // and offers Delegate. THE one predicate lives here.
        let mut member = Member::new(MemberCapabilities::All);
        assert!(!member.delegation_usable("ai:claude@joy"));
        member.ai_delegations.insert(
            "ai:claude@joy".into(),
            AiDelegationEntry {
                delegation_verifier: "ab".repeat(32),
                delegation_salt: None,
                created: chrono::Utc::now(),
                rotated: None,
                grant: None,
                model: None,
            },
        );
        assert!(!member.delegation_usable("ai:claude@joy"));
        member
            .ai_delegations
            .get_mut("ai:claude@joy")
            .expect("entry")
            .delegation_salt = Some("cd".repeat(32));
        assert!(member.delegation_usable("ai:claude@joy"));
    }

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
    // ai_delegations tests
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
                grant: None,
                model: None,
            },
        );
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        assert!(yaml.contains("ai_delegations:"));
        assert!(yaml.contains("ai:claude@joy:"));
        assert!(yaml.contains("delegation_verifier:"));
        assert!(
            !yaml.contains("delegation_salt:"),
            "an unset delegation_salt is not serialized"
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
                grant: None,
                model: None,
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
    fn validate_language_takes_two_letters() {
        assert_eq!(validate_language(" DE "), Ok("de".to_string()));
        assert!(validate_language("eng").is_err());
        assert!(validate_language("e1").is_err());
        assert!(validate_language("").is_err());
    }

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
    // InteractionLevelDefaults deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn level_defaults_flat_yaml_roundtrip() {
        let yaml = r#"
default: proposing
implement: confirmed
test: autonomous
"#;
        let parsed: InteractionLevelDefaults = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.default, InteractionLevel::Proposing);
        assert_eq!(
            parsed.capabilities[&Capability::Implement],
            InteractionLevel::Confirmed
        );
        assert_eq!(
            parsed.capabilities[&Capability::Test],
            InteractionLevel::Autonomous
        );
    }

    #[test]
    fn level_defaults_empty_yaml() {
        let yaml = "{}";
        let parsed: InteractionLevelDefaults = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.default, InteractionLevel::Proposing);
        assert!(parsed.capabilities.is_empty());
    }

    #[test]
    fn level_defaults_only_default() {
        let yaml = "default: confirmed";
        let parsed: InteractionLevelDefaults = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed.default, InteractionLevel::Confirmed);
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

    #[test]
    fn member_interaction_level_yaml_roundtrip() {
        let mut m = Member::new(MemberCapabilities::All);
        m.interaction_level = Some(InteractionLevel::Confirmed);
        let yaml = serde_yaml_ng::to_string(&m).unwrap();
        assert!(yaml.contains("interaction-level: confirmed"));
        let parsed: Member = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(parsed.interaction_level, Some(InteractionLevel::Confirmed));

        // Absent when unset.
        let bare = Member::new(MemberCapabilities::All);
        let yaml = serde_yaml_ng::to_string(&bare).unwrap();
        assert!(!yaml.contains("interaction-level"));
    }

    #[test]
    fn set_capabilities_carries_over_surviving_configs() {
        let mut m = Member::new(MemberCapabilities::Specific(
            [
                (
                    Capability::Implement,
                    CapabilityConfig {
                        max_interaction_level: Some(InteractionLevel::Confirmed),
                        ..Default::default()
                    },
                ),
                (Capability::Review, CapabilityConfig::default()),
            ]
            .into(),
        ));
        // Replace with plan+implement: implement keeps its floor, plan is new
        // (no floor), review is dropped.
        m.set_capabilities(MemberCapabilities::Specific(
            [
                (Capability::Plan, CapabilityConfig::default()),
                (Capability::Implement, CapabilityConfig::default()),
            ]
            .into(),
        ));
        match &m.capabilities {
            MemberCapabilities::Specific(map) => {
                assert!(map.contains_key(&Capability::Plan));
                assert!(map.contains_key(&Capability::Implement));
                assert!(!map.contains_key(&Capability::Review));
                assert_eq!(
                    map[&Capability::Implement].max_interaction_level,
                    Some(InteractionLevel::Confirmed)
                );
                assert_eq!(map[&Capability::Plan].max_interaction_level, None);
            }
            _ => panic!("expected specific capabilities"),
        }
    }

    #[test]
    fn set_capabilities_to_all_replaces_wholesale() {
        let mut m = Member::new(MemberCapabilities::Specific(
            [(Capability::Implement, CapabilityConfig::default())].into(),
        ));
        m.set_capabilities(MemberCapabilities::All);
        assert!(matches!(m.capabilities, MemberCapabilities::All));
    }

    // -----------------------------------------------------------------------
    // Item interaction-level serialization
    // -----------------------------------------------------------------------

    #[test]
    fn item_interaction_level_field_roundtrip() {
        use crate::model::item::{Item, ItemType, Priority};

        let mut item = Item::new(
            "TST-0001".into(),
            "Test".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        item.interaction_level = Some(InteractionLevel::Proposing);

        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        assert!(
            yaml.contains("interaction-level: proposing"),
            "interaction-level field not serialized"
        );

        let parsed: Item = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(parsed.interaction_level, Some(InteractionLevel::Proposing));
    }

    #[test]
    fn item_interaction_level_field_absent_when_none() {
        use crate::model::item::{Item, ItemType, Priority};

        let item = Item::new(
            "TST-0002".into(),
            "Test".into(),
            ItemType::Task,
            Priority::Medium,
            vec![],
        );
        assert_eq!(item.interaction_level, None);

        let yaml = serde_yaml_ng::to_string(&item).unwrap();
        assert!(
            !yaml.contains("interaction-level:"),
            "interaction-level field should not appear when None"
        );
    }

    #[test]
    fn item_interaction_level_deserialized_from_existing_yaml() {
        let yaml = r#"
id: TST-0003
title: Test
type: task
status: new
priority: medium
interaction-level: confirmed
created: "2026-01-01T00:00:00+00:00"
updated: "2026-01-01T00:00:00+00:00"
"#;
        let item: crate::model::item::Item = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(item.interaction_level, Some(InteractionLevel::Confirmed));
    }

    // -----------------------------------------------------------------------
    // Full resolution scenario
    // -----------------------------------------------------------------------
}
