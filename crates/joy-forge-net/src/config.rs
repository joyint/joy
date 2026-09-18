// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `forges.yaml`: the instances a connector knows without a forge CLI
//! (D2.5).
//!
//! Until now a connector claimed a self hosted host only when gh, glab
//! or tea was already signed in to it, which makes the sign in door of
//! D2 circular for an enterprise: the person cannot sign in through joy
//! because joy does not claim the host, and joy does not claim the host
//! because nobody signed in. An operator ships this file with the
//! workstation image and the circle is cut.
//!
//! The file is a list. Both shapes are read, because both are what a
//! person writes:
//!
//! ```yaml
//! - host: git.acme.test
//!   kind: github
//!   api_base: https://git.acme.test/api/v3
//! ```
//!
//! ```yaml
//! forges:
//!   - host: git.acme.test
//!     kind: github
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One instance an operator configured.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct Instance {
    /// The host name as it appears in a remote URL, lowercased on read.
    pub host: String,
    /// Which forge software runs there: `github`, `gitlab` or `gitea`.
    pub kind: String,
    /// The API root, without a trailing slash. Defaults to the forge's
    /// own rule for that host when absent.
    #[serde(default)]
    pub api_base: Option<String>,
    /// Where a person reads the repository in a browser.
    #[serde(default)]
    pub web_base: Option<String>,
    /// The OAuth client registered on this instance (J3 uses it).
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub device_endpoint: Option<String>,
    #[serde(default)]
    pub auth_endpoint: Option<String>,
    #[serde(default)]
    pub token_endpoint: Option<String>,
    /// joy's one CA escape hatch, Linux only (D1.12, decision 25).
    #[serde(default)]
    pub ca_bundle: Option<PathBuf>,
    #[serde(default)]
    pub ca_dir: Option<PathBuf>,
    /// The scope set this instance's application is registered with.
    #[serde(default)]
    pub scopes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FileShape {
    Bare(Vec<Instance>),
    Keyed { forges: Vec<Instance> },
}

/// Every configured instance, in file order, first file first.
#[derive(Debug, Clone, Default)]
pub struct Instances {
    entries: Vec<Instance>,
}

impl Instances {
    /// Read every `forges.yaml` joy looks at. A file that does not
    /// parse is reported on stderr and skipped: a typo in an operator's
    /// file must not make the connector answer nothing at all.
    pub fn load() -> Self {
        let mut entries: Vec<Instance> = Vec::new();
        for path in files() {
            match std::fs::read_to_string(&path) {
                Ok(text) => match parse(&text) {
                    Ok(mut found) => entries.append(&mut found),
                    Err(error) => eprintln!("joy: {} is not readable: {error}", path.display()),
                },
                Err(_) => continue,
            }
        }
        Instances { entries }
    }

    /// Build one from text, for the tests and for a caller that has the
    /// file already.
    pub fn from_text(text: &str) -> Result<Self, String> {
        Ok(Instances {
            entries: parse(text)?,
        })
    }

    /// No instances at all.
    pub fn empty() -> Self {
        Instances {
            entries: Vec::new(),
        }
    }

    /// The instance configured for this host, if any. The first entry
    /// wins, so the file a person owns beats the one an image shipped.
    pub fn for_host(&self, host: &str) -> Option<&Instance> {
        let host = host.trim().to_ascii_lowercase();
        self.entries.iter().find(|entry| entry.host == host)
    }

    /// Every configured host of one forge kind.
    pub fn hosts_of_kind(&self, kind: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.host.as_str())
            .collect()
    }

    /// Whether this host is configured for this forge kind.
    pub fn claims(&self, kind: &str, host: &str) -> bool {
        self.for_host(host).is_some_and(|entry| entry.kind == kind)
    }
}

fn parse(text: &str) -> Result<Vec<Instance>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let shape: FileShape = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    let entries = match shape {
        FileShape::Bare(entries) => entries,
        FileShape::Keyed { forges } => forges,
    };
    Ok(entries
        .into_iter()
        .map(|mut entry| {
            entry.host = entry.host.trim().to_ascii_lowercase();
            entry.kind = entry.kind.trim().to_ascii_lowercase();
            entry.api_base = entry
                .api_base
                .map(|base| base.trim_end_matches('/').to_string());
            entry.web_base = entry
                .web_base
                .map(|base| base.trim_end_matches('/').to_string());
            entry
        })
        .filter(|entry| !entry.host.is_empty() && !entry.kind.is_empty())
        .collect())
}

/// Where joy looks: the XDG configuration directory, and the one the
/// platform uses on this operating system (D2.5).
fn files() -> Vec<PathBuf> {
    joy_config_dirs()
        .into_iter()
        .map(|dir| dir.join("forges.yaml"))
        .collect()
}

/// joy's own configuration directories, most specific first.
pub fn joy_config_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = env_dir("XDG_CONFIG_HOME") {
        dirs.push(dir.join("joy"));
    }
    if let Some(home) = env_dir("HOME") {
        dirs.push(home.join(".config/joy"));
        if cfg!(target_os = "macos") {
            dirs.push(home.join("Library/Application Support/joy"));
        }
    }
    if cfg!(windows) {
        if let Some(dir) = env_dir("APPDATA") {
            dirs.push(dir.join("joy"));
        }
    }
    dedup(dirs)
}

pub(crate) fn env_dir(name: &str) -> Option<PathBuf> {
    match std::env::var_os(name) {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => None,
    }
}

pub(crate) fn dedup(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if !seen.iter().any(|known| known == &dir) {
            seen.push(dir);
        }
    }
    seen
}

/// The `forge:` override of the project the call runs in (D2.5: "the
/// existing project level `forge:` override keeps working and wins for
/// that project"). Read as a line, because the connector needs one key
/// out of a file joy owns and must not grow a project loader for it.
pub fn project_forge(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".joy/project.yaml")).ok()?;
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        if let Some(value) = line.strip_prefix("forge:") {
            let value = value.split('#').next().unwrap_or("").trim();
            let value = value.trim_matches(['"', '\''].as_ref()).trim();
            if !value.is_empty() {
                return Some(value.to_ascii_lowercase());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"
- host: Git.ACME.test
  kind: GitHub
  api_base: https://git.acme.test/api/v3/
  web_base: https://git.acme.test/
  ca_bundle: /etc/pki/corp.pem
- host: gitlab.acme.test
  kind: gitlab
"#;

    #[test]
    fn a_bare_list_parses_and_host_and_kind_are_lowercased() {
        let instances = Instances::from_text(FILE).unwrap();
        let entry = instances.for_host("git.acme.test").unwrap();
        assert_eq!(entry.kind, "github");
        assert_eq!(
            entry.api_base.as_deref(),
            Some("https://git.acme.test/api/v3")
        );
        assert_eq!(entry.web_base.as_deref(), Some("https://git.acme.test"));
        assert_eq!(
            entry.ca_bundle.as_deref(),
            Some(Path::new("/etc/pki/corp.pem"))
        );
        assert!(instances.claims("github", "GIT.acme.test"));
        assert!(!instances.claims("gitlab", "git.acme.test"));
        assert_eq!(instances.hosts_of_kind("gitlab"), vec!["gitlab.acme.test"]);
    }

    #[test]
    fn the_keyed_shape_parses_too() {
        let instances =
            Instances::from_text("forges:\n  - host: g.example\n    kind: gitea\n").unwrap();
        assert!(instances.claims("gitea", "g.example"));
    }

    #[test]
    fn an_empty_or_broken_file_is_not_a_crash() {
        assert_eq!(
            Instances::from_text("")
                .unwrap()
                .hosts_of_kind("github")
                .len(),
            0
        );
        assert!(Instances::from_text("not: [a, yaml, list").is_err());
    }

    #[test]
    fn the_project_override_is_read_from_the_projects_own_file() {
        let dir = std::env::temp_dir().join(format!("joy-forges-yaml-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".joy")).unwrap();
        std::fs::write(
            dir.join(".joy/project.yaml"),
            "name: Demo\nforge: gitea   # the operator's word\nmembers:\n  forge: nonsense\n",
        )
        .unwrap();
        assert_eq!(project_forge(&dir).as_deref(), Some("gitea"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
