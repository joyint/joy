// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Forge abstraction: create releases on hosting platforms.
//! See ADR-017 and docs/dev/vision/ForgeSync.md.

use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Result};

use joy_core::vcs::{self, Forge};

/// Trait for hosting platform operations.
pub trait ForgeRelease {
    /// Create a release on the forge. Returns the release URL if available.
    fn create_release(
        &self,
        root: &Path,
        tag: &str,
        title: &str,
        notes: &str,
    ) -> Result<Option<String>>;
}

/// GitHub implementation using the gh CLI.
struct GitHubForge;

impl ForgeRelease for GitHubForge {
    fn create_release(
        &self,
        root: &Path,
        tag: &str,
        title: &str,
        notes: &str,
    ) -> Result<Option<String>> {
        // Check gh is available
        let gh_ver = vcs::gh_version().map_err(|_| {
            anyhow!(
                "forge 'github' requires gh CLI\n  \
                 = help: install gh (https://cli.github.com) or run `joy forge setup` to reconfigure"
            )
        })?;

        // Check gh is authenticated
        let auth_status = std::process::Command::new("gh")
            .args(["auth", "status"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if !auth_status.is_ok_and(|s| s.success()) {
            bail!(
                "gh is installed ({gh_ver}) but not authenticated\n  \
                 = help: run `gh auth login` or `joy forge setup`"
            );
        }

        // Idempotent: a release may already exist if an earlier
        // `just publish` made it and the subsequent push failed, or
        // if the forge workflow created it from a tag push. The
        // release is then already there, but the forge workflow only
        // writes its installer section (JOY-0248-AE: v0.20.0 shipped
        // without a changelog that way), so the notes still have to
        // land: prepend them unless a prior run already did.
        let existing = std::process::Command::new("gh")
            .args(["release", "view", tag, "--json", "url,body"])
            .current_dir(root)
            .output();
        if let Ok(output) = existing {
            if output.status.success() {
                let parsed: serde_json::Value =
                    serde_json::from_slice(&output.stdout).unwrap_or_default();
                let url = parsed["url"].as_str().unwrap_or("").to_string();
                if !url.is_empty() {
                    let body = parsed["body"].as_str().unwrap_or("");
                    if !body.contains(notes.trim()) {
                        let combined = format!("{}\n\n{}", notes.trim_end(), body);
                        vcs::gh_edit_release_notes(root, tag, &combined)?;
                    }
                    return Ok(Some(url));
                }
            }
        }

        let url = vcs::gh_create_release(root, tag, title, notes)?;
        Ok(Some(url))
    }
}

/// No-op forge for `forge: none` or explicit skip.
struct NoForge;

impl ForgeRelease for NoForge {
    fn create_release(
        &self,
        _root: &Path,
        _tag: &str,
        _title: &str,
        _notes: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

/// Forge values that have a working ForgeRelease backend today.
/// Used both for auto-detection filtering and for validating
/// user-supplied values from `--forge` and `forge:`.
pub const SUPPORTED_FORGES: &[&str] = &["github"];

/// Outcome of [`resolve`]: the forge to talk to plus a human-readable
/// note about how the choice was made (auto-detected, override, etc.).
/// The note is empty when the source is unambiguous (explicit `forge:`
/// or `--forge` flag).
pub struct Resolution {
    pub forge: Box<dyn ForgeRelease>,
    pub note: Option<String>,
}

/// Resolve which forge to use for a release.
///
/// Precedence:
/// 1. `flag_value` (CLI `--forge`)
/// 2. `project_forge` (project.yaml `forge:`)
/// 3. Auto-detect from configured git remotes
///
/// "none" at any layer means skip the forge release step. When
/// auto-detection finds multiple supported forges and stdin is a TTY,
/// the user is prompted; on a non-TTY this is a hard error so CI fails
/// loudly instead of guessing.
pub fn resolve(
    root: &Path,
    project_forge: Option<&str>,
    flag_value: Option<&str>,
) -> Result<Resolution> {
    if let Some(value) = flag_value {
        return Ok(Resolution {
            forge: from_value(value, "--forge")?,
            note: None,
        });
    }
    if let Some(value) = project_forge {
        return Ok(Resolution {
            forge: from_project_value(value),
            note: None,
        });
    }
    auto_detect(root)
}

/// Lenient counterpart to [`from_value`] for values read from
/// project.yaml. A stale or forward-looking value (e.g. `gitea`
/// before the backend lands) prints a warning and falls back to
/// NoForge so `joy release publish` still pushes the tag instead
/// of hard-failing. The strict path (`from_value`) is used for
/// CLI inputs where rejecting typos early is the right behaviour.
fn from_project_value(value: &str) -> Box<dyn ForgeRelease> {
    match value {
        "none" => Box::new(NoForge),
        "github" => Box::new(GitHubForge),
        other => {
            eprintln!(
                "warning: forge '{other}' from project.yaml is not yet supported for releases"
            );
            eprintln!("  = note: falling back to git tag only (no forge release)");
            Box::new(NoForge)
        }
    }
}

/// Build a backend from an explicit config string. `source` names where
/// the value came from so validation errors are actionable.
fn from_value(value: &str, source: &str) -> Result<Box<dyn ForgeRelease>> {
    match value {
        "none" => Ok(Box::new(NoForge)),
        "github" => Ok(Box::new(GitHubForge)),
        other => bail!(
            "unsupported forge '{other}' from {source}\n  \
             = help: supported values are: {}, none",
            SUPPORTED_FORGES.join(", ")
        ),
    }
}

fn auto_detect(root: &Path) -> Result<Resolution> {
    let detected = vcs::default_vcs().detect_forges(root);

    // Dedupe by canonical config string while preserving discovery order.
    let mut unique: Vec<(String, Forge)> = Vec::new();
    for (name, forge) in detected.iter() {
        let canonical = forge.as_config_str();
        if canonical.is_none() {
            continue;
        }
        if !SUPPORTED_FORGES.contains(&canonical.unwrap()) {
            continue;
        }
        if unique.iter().any(|(_, f)| f == forge) {
            continue;
        }
        unique.push((name.clone(), forge.clone()));
    }

    match unique.len() {
        0 => {
            let remote_summary = if detected.is_empty() {
                "no git remotes are configured".to_string()
            } else {
                let list = detected
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("configured remotes: {list}")
            };
            bail!(
                "no supported forge detected from git remotes ({remote_summary})\n  \
                 = help: add a remote on a supported host ({}), pass --forge <value>, \
                 or set `forge: none` with `joy project set forge none` to publish without a forge release",
                SUPPORTED_FORGES.join(", ")
            )
        }
        1 => {
            let (remote, forge) = &unique[0];
            let label = forge.as_config_str().unwrap_or("unknown");
            Ok(Resolution {
                forge: from_value(label, "auto-detect")?,
                note: Some(format!(
                    "auto-detected forge '{label}' from remote '{remote}'"
                )),
            })
        }
        _ => pick_interactive(&unique),
    }
}

fn pick_interactive(choices: &[(String, Forge)]) -> Result<Resolution> {
    if !std::io::stdin().is_terminal() {
        let labels: Vec<String> = choices
            .iter()
            .map(|(remote, forge)| {
                format!(
                    "{} (remote '{remote}')",
                    forge.as_config_str().unwrap_or("unknown")
                )
            })
            .collect();
        bail!(
            "multiple supported forges detected: {}\n  \
             = help: pass --forge <value> or set `forge:` in project.yaml to disambiguate",
            labels.join(", ")
        );
    }

    println!("Multiple supported forges detected. Pick one:");
    for (i, (remote, forge)) in choices.iter().enumerate() {
        println!(
            "  [{}] {} (remote '{remote}')",
            i + 1,
            forge.as_config_str().unwrap_or("unknown")
        );
    }
    print!("Choose [1-{}]: ", choices.len());
    std::io::stdout().flush().ok();

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let idx: usize = input
        .trim()
        .parse()
        .map_err(|_| anyhow!("invalid selection: {}", input.trim()))?;
    if idx == 0 || idx > choices.len() {
        bail!("selection {idx} out of range");
    }

    let (remote, forge) = &choices[idx - 1];
    let label = forge.as_config_str().unwrap_or("unknown");
    Ok(Resolution {
        forge: from_value(label, "interactive selection")?,
        note: Some(format!("using forge '{label}' from remote '{remote}'")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_value_accepts_none() {
        let forge = from_value("none", "test").unwrap();
        let result = forge
            .create_release(Path::new("."), "v0.1.0", "test", "notes")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn from_value_accepts_github() {
        // Builds a backend without invoking it. create_release would
        // shell out to gh; we only check construction succeeds.
        let _forge = from_value("github", "test").unwrap();
    }

    #[test]
    fn from_value_rejects_unsupported() {
        match from_value("gitlab", "project.yaml") {
            Ok(_) => panic!("expected an error for unsupported forge"),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("unsupported forge 'gitlab'"));
                assert!(msg.contains("project.yaml"));
                assert!(msg.contains("supported values"));
            }
        }
    }

    #[test]
    fn from_value_rejects_typo() {
        match from_value("guthub", "--forge") {
            Ok(_) => panic!("expected an error for typo"),
            Err(e) => assert!(e.to_string().contains("--forge")),
        }
    }

    #[test]
    fn resolve_flag_beats_project_and_detection() {
        // Pass an explicit "none" via flag; project value and any
        // remote configuration must be ignored.
        let res = resolve(Path::new("."), Some("github"), Some("none")).unwrap();
        assert!(res.note.is_none());
        let out = res
            .forge
            .create_release(Path::new("."), "v0.1.0", "t", "n")
            .unwrap();
        assert!(out.is_none(), "explicit --forge none must short-circuit");
    }

    #[test]
    fn resolve_project_value_used_when_no_flag() {
        let res = resolve(Path::new("."), Some("none"), None).unwrap();
        assert!(res.note.is_none());
        let out = res
            .forge
            .create_release(Path::new("."), "v0.1.0", "t", "n")
            .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn from_project_value_is_lenient_about_unknown() {
        // Legacy / forward-looking values in project.yaml must not
        // hard-fail joy release publish; they downgrade to NoForge.
        let forge = from_project_value("gitea");
        let out = forge
            .create_release(Path::new("."), "v0.1.0", "t", "n")
            .unwrap();
        assert!(out.is_none());
    }
}
