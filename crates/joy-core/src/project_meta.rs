// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The get/set core of `joy project`: the key catalogue, the read
//! snapshot, and the per-key write rules for project metadata.
//!
//! Extracted from joy-cli's project command so the desktop app and the
//! platform server, which link joy-core as a library, apply the exact
//! same validation and normalization as the CLI when they edit project
//! metadata. Kept OUT of `model::project` on purpose: the model stays a
//! pure data shape, while this module owns the command-facing policy
//! (which keys exist, how values normalize, when on-disk YAML needs
//! pruning). The CLI keeps everything interactive on its side: clap
//! parsing, editor flows, printing, and JSON payload shapes.

use anyhow::{bail, Result};

use crate::model::project::{validate_acronym, PrivacyMode};
use crate::model::Project;

/// The keys `joy project get`/`set` know, in display order. Also the
/// completion source for the CLI's key argument.
pub const PROJECT_KEYS: &[&str] = &[
    "name",
    "acronym",
    "description",
    "language",
    "forge",
    "privacy",
    "created",
    "docs.architecture",
    "docs.vision",
    "docs.contributing",
    "release.version-files",
];

/// Keys whose value is a list rather than a scalar. List keys accept
/// `--add` / `--rm` flags on `joy project set` plus CSV form for whole-
/// list replacement; their `get` output is one entry per line (or a
/// JSON array under --json). Scalar keys reject `--add`/`--rm`.
pub const LIST_KEYS: &[&str] = &["release.version-files"];

/// Whether `key` is one of the [`LIST_KEYS`].
pub fn is_list_key(key: &str) -> bool {
    LIST_KEYS.contains(&key)
}

/// Strip a trailing `.*` (or bare `*`) from `key` and return the
/// prefix that remains. `None` when the key has no wildcard.
pub fn wildcard_prefix(key: &str) -> Option<&str> {
    if key == "*" {
        Some("")
    } else {
        key.strip_suffix(".*")
    }
}

/// Snapshot the project's read-exposed metadata into a nested JSON
/// tree so `flatten_under` and `describe_value` can walk it the same
/// way `joy config get` does. The shape matches PROJECT_KEYS: top-level
/// scalars plus a `docs` object holding the three resolved doc paths.
/// Unset optional fields (acronym, description) are represented as
/// `null` so the existing JSON payload shape on those keys is
/// preserved.
pub fn project_value_tree(root: &std::path::Path, project: &Project) -> serde_json::Value {
    let version_files: serde_json::Value = match crate::version_files::version_files_get(root) {
        Ok(v) if !v.is_empty() => {
            serde_json::Value::Array(v.into_iter().map(serde_json::Value::String).collect())
        }
        _ => serde_json::Value::Null,
    };
    serde_json::json!({
        "name": project.name,
        "acronym": project.acronym,
        "description": project.description,
        "language": project.language,
        "forge": project.forge,
        "privacy": project.privacy().map(|p| p.to_string()).unwrap_or_else(|| "none".to_string()),
        "created": project.created.format("%Y-%m-%d %H:%M").to_string(),
        "docs": {
            "architecture": project.docs.architecture_or_default(),
            "vision": project.docs.vision_or_default(),
            "contributing": project.docs.contributing_or_default(),
        },
        "release": {
            "version-files": version_files,
        }
    })
}

/// Apply one scalar `joy project set` write to the in-memory project,
/// running the per-key validation/normalization. The caller persists
/// (and prunes) afterwards; splitting mutation from persistence keeps
/// this reusable for hosts that batch several writes.
pub fn set_value(project: &mut Project, key: &str, value: &str) -> Result<()> {
    match key {
        "name" => project.name = value.to_string(),
        "description" => {
            project.description = if value.is_empty() || value == "none" {
                None
            } else {
                Some(value.to_string())
            };
        }
        "language" => project.language = value.to_string(),
        "forge" => project.forge = normalize_forge_value(value)?,
        "privacy" => match value.trim() {
            "none" => project.set_privacy_non_anonymous(None)?,
            "open" => project.set_privacy_non_anonymous(Some(PrivacyMode::Open))?,
            "anonymous" => anyhow::bail!(
                "privacy: anonymous is not yet implemented; it arrives with the mode-transition task JOY-01BF-2E"
            ),
            other => {
                anyhow::bail!("invalid privacy mode '{other}'; expected: none, open, or anonymous")
            }
        },
        "docs.architecture" => project.docs.architecture = normalize_docs_value(value),
        "docs.vision" => project.docs.vision = normalize_docs_value(value),
        "docs.contributing" => project.docs.contributing = normalize_docs_value(value),
        "acronym" => {
            let normalized = validate_acronym(value).map_err(|e| anyhow::anyhow!(e))?;
            project.acronym = Some(normalized);
        }
        "created" => {
            anyhow::bail!("'created' is read-only");
        }
        _ => anyhow::bail!(
            "unknown key: {key}\nknown keys: {}",
            PROJECT_KEYS.join(", ")
        ),
    }
    Ok(())
}

/// Validate and normalize a `forge:` value. Empty input clears the
/// field (auto-detection at publish time applies). `"none"` is an
/// explicit opt-out and is stored verbatim so the intent is visible
/// in project.yaml. Any other value must name a registered forge
/// plugin (joy-core's registry, JOY-0256-64); this rejects typos at
/// write time, which is the right moment for strictness (read-time
/// stays lenient so legacy values don't hard-fail publish).
pub fn normalize_forge_value(value: &str) -> Result<Option<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed == "none" {
        return Ok(Some("none".to_string()));
    }
    if crate::forge_plugins::by_id(trimmed).is_some() {
        return Ok(Some(trimmed.to_string()));
    }
    bail!(
        "unsupported forge '{trimmed}'\n  = help: supported values are: {}, none (pass an empty value to clear)",
        crate::forge_plugins::supported_ids().join(", ")
    );
}

/// Empty / "none" / "default" reset a docs path to its built-in default.
pub fn normalize_docs_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("default")
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Remove a top-level key from the on-disk project YAML. Used after
/// `write_yaml_preserve` to clear optional Option<String> fields that
/// the preserve step would otherwise re-add from the original file.
pub fn prune_yaml_key(path: &std::path::Path, key: &str) -> Result<()> {
    use serde_yaml_ng::Value;
    let raw = std::fs::read_to_string(path)?;
    let mut value: Value = serde_yaml_ng::from_str(&raw)?;
    if let Some(map) = value.as_mapping_mut() {
        map.remove(Value::String(key.to_string()));
    }
    let yaml = serde_yaml_ng::to_string(&value)?;
    std::fs::write(path, yaml)?;
    Ok(())
}

/// Rewrite the project YAML so the `docs:` block exactly reflects the desired
/// state. Removes the block entirely when no overrides are set; otherwise
/// replaces it with only the configured fields. Needed because
/// `write_yaml_preserve` keeps unknown top-level keys (which would otherwise
/// re-introduce a stale `docs:` block when an override is cleared).
pub fn prune_docs_yaml(path: &std::path::Path, docs: &crate::model::Docs) -> Result<()> {
    use serde_yaml_ng::Value;

    let raw = std::fs::read_to_string(path)?;
    let mut value: Value = serde_yaml_ng::from_str(&raw)?;
    let map = match value.as_mapping_mut() {
        Some(m) => m,
        None => return Ok(()),
    };
    let docs_key = Value::String("docs".to_string());
    if docs.is_empty() {
        map.remove(&docs_key);
    } else {
        let docs_value = serde_yaml_ng::to_value(docs)?;
        map.insert(docs_key, docs_value);
    }
    let yaml = serde_yaml_ng::to_string(&value)?;
    std::fs::write(path, yaml)?;
    Ok(())
}

/// Read the current scalar value for `key` as plain text. Returns
/// empty string for unset Option fields and for `docs.*` overrides
/// at the built-in default (callers can tell the user this is the
/// default by inspecting the value, but for editor pre-population
/// the empty case is fine).
pub fn current_scalar_value(project: &Project, key: &str) -> String {
    match key {
        "name" => project.name.clone(),
        "acronym" => project.acronym.clone().unwrap_or_default(),
        "description" => project.description.clone().unwrap_or_default(),
        "language" => project.language.clone(),
        "forge" => project.forge.clone().unwrap_or_default(),
        "privacy" => project
            .privacy()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "none".to_string()),
        "docs.architecture" => project.docs.architecture.clone().unwrap_or_default(),
        "docs.vision" => project.docs.vision.clone().unwrap_or_default(),
        "docs.contributing" => project.docs.contributing.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Some project fields are `Option<String>` (acronym, description).
/// In the existing JSON contract those return `null` when unset, not
/// the string `"null"`. Preserve that.
pub fn value_as_optional_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Render a JSON leaf for terminal display: strings verbatim (no
/// quotes), null as the empty string, everything else via Display.
pub fn scalar_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}
