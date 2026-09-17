// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Forge abstraction: create releases on hosting platforms.
//!
//! Since JOY-0256-64 the forge knowledge lives in the forge PLUGINS
//! (docs/plugins.md): joy-core's registry names them, `claims` decides
//! whose remote a project is, and the plugin's `release` verb does the
//! actual work (joy-github shells gh; a forge without a release backend
//! answers `unsupported` and publish keeps its tag-only path). Nothing
//! in here parses a forge URL or shells a forge CLI any more.

use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Result};

use joy_core::forge_plugins::{self, CallContext, ForgePluginSpec, Target};
use joy_core::vcs;

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

/// A forge with a plugin: the release rides the plugin contract.
struct PluginForge {
    spec: &'static ForgePluginSpec,
}

impl ForgeRelease for PluginForge {
    fn create_release(
        &self,
        root: &Path,
        tag: &str,
        title: &str,
        notes: &str,
    ) -> Result<Option<String>> {
        // The notes travel as a file: they are multi-line, may be long,
        // and must reach the plugin byte-exact.
        let dir = tempfile::tempdir()?;
        let notes_file = dir.path().join("notes.md");
        std::fs::write(&notes_file, notes)?;
        // The connector's own stderr is captured now (D2.3), so its
        // message travels INSIDE the error instead of on a terminal
        // that may not exist. The state name comes along for the same
        // reason: missing, outdated, refused and timed out are four
        // different things to do next.
        let ctx = CallContext::in_project(root);
        // Which repository the release belongs to. gh used to read this
        // out of the working directory; the connector's own REST call
        // has to be told (D2.8), and the remote is what tells it.
        let target = default_remote_url(root).map(Target::remote);
        let outcome = forge_plugins::release(self.spec, target.as_ref(), tag, title, &notes_file, &ctx)
            .map_err(|e| {
                anyhow!(
                    "{e}\n  = note: state {}\n  \
                     = help: install the connector or set `forge: none` to publish without a forge release",
                    e.state()
                )
            })?;
        if outcome.unsupported {
            // the plugin's forge has no release backend yet: same
            // downgrade the lenient project.yaml path always offered
            eprintln!(
                "warning: forge '{}' does not support releases yet",
                self.spec.id
            );
            eprintln!("  = note: falling back to git tag only (no forge release)");
            return Ok(None);
        }
        Ok(outcome.url)
    }
}

/// The URL of the remote a release is published to: the project's
/// default remote, when it has one. A project with no remote has no
/// repository on a forge either, and the connector says so.
fn default_remote_url(root: &Path) -> Option<String> {
    let git = vcs::default_vcs();
    let remote = git.default_remote(root).ok()?;
    git.remote_url(root, &remote).ok()
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

/// The forge values `--forge` and `forge:` accept: the plugin registry's
/// ids. Whether a forge can RELEASE is the plugin's answer at release
/// time, not a second list here. Thin alias over joy-core's registry so
/// the CLI and joy-core's own error help render the same list.
pub fn supported_forges() -> Vec<&'static str> {
    forge_plugins::supported_ids()
}

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
/// 3. Auto-detect: the registry plugins' `claims` over the git remotes
///
/// "none" at any layer means skip the forge release step. When several
/// plugins claim remotes and stdin is a TTY, the user is prompted; on a
/// non-TTY this is a hard error so CI fails loudly instead of guessing.
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
/// project.yaml. A stale or forward-looking value that names no
/// registered plugin prints a warning and falls back to NoForge so
/// `joy release publish` still pushes the tag instead of hard-failing.
/// The strict path (`from_value`) is used for CLI inputs where
/// rejecting typos early is the right behaviour.
fn from_project_value(value: &str) -> Box<dyn ForgeRelease> {
    if value == "none" {
        return Box::new(NoForge);
    }
    match forge_plugins::by_id(value) {
        Some(spec) => Box::new(PluginForge { spec }),
        None => {
            eprintln!("warning: forge '{value}' from project.yaml names no registered plugin");
            eprintln!("  = note: falling back to git tag only (no forge release)");
            Box::new(NoForge)
        }
    }
}

/// Build a backend from an explicit config string. `source` names where
/// the value came from so validation errors are actionable.
fn from_value(value: &str, source: &str) -> Result<Box<dyn ForgeRelease>> {
    if value == "none" {
        return Ok(Box::new(NoForge));
    }
    match forge_plugins::by_id(value) {
        Some(spec) => Ok(Box::new(PluginForge { spec })),
        None => bail!(
            "unsupported forge '{value}' from {source}\n  \
             = help: supported values are: {}, none",
            supported_forges().join(", ")
        ),
    }
}

fn auto_detect(root: &Path) -> Result<Resolution> {
    let remotes = vcs::default_vcs().all_remotes(root).unwrap_or_default();

    // One claims round per registry plugin: a plugin that claims any
    // remote is a candidate. The plugin decides what is "its" URL —
    // joy never parses a forge URL itself (JOY-0256-64).
    let ctx = CallContext::in_project(root);
    let mut claimed: Vec<(String, &'static ForgePluginSpec)> = Vec::new();
    for spec in forge_plugins::FORGE_PLUGINS {
        if let Some((name, _)) = remotes
            .iter()
            .find(|(_, url)| forge_plugins::claims(spec, &Target::remote(url.as_str()), &ctx))
        {
            claimed.push((name.clone(), spec));
        }
    }

    match claimed.len() {
        0 => {
            let remote_summary = if remotes.is_empty() {
                "no git remotes are configured".to_string()
            } else {
                let list = remotes
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
                supported_forges().join(", ")
            )
        }
        1 => {
            let (remote, spec) = &claimed[0];
            Ok(Resolution {
                forge: Box::new(PluginForge { spec }),
                note: Some(format!(
                    "auto-detected forge '{}' from remote '{remote}'",
                    spec.id
                )),
            })
        }
        _ => pick_interactive(&claimed),
    }
}

fn pick_interactive(choices: &[(String, &'static ForgePluginSpec)]) -> Result<Resolution> {
    if !std::io::stdin().is_terminal() {
        let labels: Vec<String> = choices
            .iter()
            .map(|(remote, spec)| format!("{} (remote '{remote}')", spec.id))
            .collect();
        bail!(
            "multiple supported forges detected: {}\n  \
             = help: pass --forge <value> or set `forge:` in project.yaml to disambiguate",
            labels.join(", ")
        );
    }

    println!("Multiple supported forges detected. Pick one:");
    for (i, (remote, spec)) in choices.iter().enumerate() {
        println!("  [{}] {} (remote '{remote}')", i + 1, spec.id);
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

    let (remote, spec) = &choices[idx - 1];
    Ok(Resolution {
        forge: Box::new(PluginForge { spec }),
        note: Some(format!("using forge '{}' from remote '{remote}'", spec.id)),
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
    fn from_value_accepts_every_registered_plugin() {
        // Builds a backend without invoking it. create_release would
        // shell out to the plugin; we only check construction succeeds.
        for id in supported_forges() {
            let _forge = from_value(id, "test").unwrap();
        }
    }

    #[test]
    fn from_value_rejects_typo() {
        match from_value("guthub", "--forge") {
            Ok(_) => panic!("expected an error for typo"),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("--forge"));
                assert!(msg.contains("supported values"));
            }
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
        let forge = from_project_value("sourcehut");
        let out = forge
            .create_release(Path::new("."), "v0.1.0", "t", "n")
            .unwrap();
        assert!(out.is_none());
    }
}
