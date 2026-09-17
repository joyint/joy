// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The handful of git config keys the connector's own HTTP client has
//! to honour, read from the files themselves.
//!
//! Why not git2: the connector is a small binary that must stay small,
//! and it needs four keys, not a configuration engine. Why not the git
//! binary: joy never spawns one (D3.2). So the three files git reads
//! are read here, in git's own precedence (system, global, local, last
//! value wins), for these keys only:
//!
//! - `http.proxy` and `http.<url>.proxy` (D1.11);
//! - `http.sslCAInfo` and `http.sslCAPath`, honoured on Linux only
//!   (D1.12, decision 25).
//!
//! Everything else git offers is deliberately not read, and D1.12 lists
//! it so nobody expects it.

use std::path::{Path, PathBuf};

/// One parsed entry: the section, its optional subsection (git keeps
/// the subsection case sensitive), the key (lowercased) and the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub section: String,
    pub subsection: Option<String>,
    pub key: String,
    pub value: String,
}

/// The git configuration as the connector reads it: the entries of every
/// file that exists, in the order git applies them.
#[derive(Debug, Default, Clone)]
pub struct GitConfig {
    entries: Vec<Entry>,
}

impl GitConfig {
    /// Read the system, global and (when a repository root is known)
    /// local file, in that order.
    pub fn load(repo_root: Option<&Path>) -> Self {
        let mut entries = Vec::new();
        for path in files(repo_root) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                entries.extend(parse(&text));
            }
        }
        GitConfig { entries }
    }

    /// Build one directly, for the tests and for a caller that already
    /// has the text.
    pub fn from_text(text: &str) -> Self {
        GitConfig {
            entries: parse(text),
        }
    }

    /// The last value of `<section>.<key>` without a subsection.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.section == section && e.subsection.is_none() && e.key == key)
            .map(|e| e.value.as_str())
    }

    /// The last value of `<section>.<subsection>.<key>`.
    pub fn get_sub(&self, section: &str, subsection: &str, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|e| {
                e.section == section && e.subsection.as_deref() == Some(subsection) && e.key == key
            })
            .map(|e| e.value.as_str())
    }

    /// Every subsection of `<section>` that carries `<key>`.
    pub fn subsections(&self, section: &str, key: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| e.section == section && e.key == key)
            .filter_map(|e| e.subsection.as_deref())
            .collect()
    }
}

/// The files git reads, in the order it applies them. `GIT_CONFIG_NOSYSTEM`
/// drops the system file, exactly as git does, because the test harness
/// sets it and a CI image with a system file must not change a verdict.
fn files(repo_root: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if std::env::var_os("GIT_CONFIG_NOSYSTEM").is_none() {
        paths.push(PathBuf::from("/etc/gitconfig"));
    }
    match std::env::var_os("GIT_CONFIG_GLOBAL") {
        Some(path) => paths.push(PathBuf::from(path)),
        None => {
            if let Some(xdg) = config_home() {
                paths.push(xdg.join("git/config"));
            }
            if let Some(home) = std::env::var_os("HOME") {
                paths.push(PathBuf::from(home).join(".gitconfig"));
            }
        }
    }
    if let Some(root) = repo_root {
        paths.push(root.join(".git/config"));
    }
    paths
}

fn config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")),
    }
}

/// Parse one git config file. Enough of the grammar for the four keys
/// above: sections with and without a subsection, `key = value`,
/// `key` alone (true), `#` and `;` comments, and a quoted value.
fn parse(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut section = String::new();
    let mut subsection: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(head) = line.strip_prefix('[') {
            let Some(head) = head.split(']').next() else {
                continue;
            };
            match head.split_once('"') {
                Some((name, rest)) => {
                    section = name.trim().to_ascii_lowercase();
                    subsection = Some(rest.trim_end_matches('"').to_string());
                }
                None => {
                    // `[http "x"]` is the quoted form; `[a.b]` is the
                    // legacy one, where the part after the dot is the
                    // subsection and stays case sensitive.
                    match head.trim().split_once('.') {
                        Some((name, sub)) => {
                            section = name.trim().to_ascii_lowercase();
                            subsection = Some(sub.trim().to_string());
                        }
                        None => {
                            section = head.trim().to_ascii_lowercase();
                            subsection = None;
                        }
                    }
                }
            }
            continue;
        }
        if section.is_empty() {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((key, value)) => (key.trim(), unquote(strip_comment(value.trim()))),
            None => (line, "true".to_string()),
        };
        if key.is_empty() {
            continue;
        }
        entries.push(Entry {
            section: section.clone(),
            subsection: subsection.clone(),
            key: key.to_ascii_lowercase(),
            value,
        });
    }
    entries
}

/// A trailing comment, but only outside a quoted value.
fn strip_comment(value: &str) -> &str {
    let mut quoted = false;
    for (index, ch) in value.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            '#' | ';' if !quoted => return value[..index].trim_end(),
            _ => {}
        }
    }
    value
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1].to_string();
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# a comment
[http]
    proxy = http://proxy.example:3128
    sslCAInfo = /etc/pki/tls/certs/corp.pem
[http "https://internal.example.com/"]
    proxy = http://inside.example:8080
[HTTP]
    proxy = http://last-wins.example:3128
"#;

    #[test]
    fn the_last_value_of_a_key_wins_and_sections_are_case_insensitive() {
        let config = GitConfig::from_text(SAMPLE);
        assert_eq!(
            config.get("http", "proxy"),
            Some("http://last-wins.example:3128")
        );
        assert_eq!(
            config.get("http", "sslcainfo"),
            Some("/etc/pki/tls/certs/corp.pem")
        );
    }

    #[test]
    fn a_url_subsection_is_kept_verbatim() {
        let config = GitConfig::from_text(SAMPLE);
        assert_eq!(
            config.get_sub("http", "https://internal.example.com/", "proxy"),
            Some("http://inside.example:8080")
        );
        assert_eq!(
            config.subsections("http", "proxy"),
            vec!["https://internal.example.com/"]
        );
    }

    #[test]
    fn a_trailing_comment_and_quotes_are_not_part_of_the_value() {
        let config = GitConfig::from_text("[http]\n proxy = \"http://a.example:8080\" # work\n");
        assert_eq!(config.get("http", "proxy"), Some("http://a.example:8080"));
        let bare = GitConfig::from_text("[http]\n proxy = http://a.example:8080 ; work\n");
        assert_eq!(bare.get("http", "proxy"), Some("http://a.example:8080"));
    }

    #[test]
    fn the_legacy_dotted_section_form_parses_too() {
        let config = GitConfig::from_text("[http.https://x.example]\n proxy = http://p:1\n");
        assert_eq!(
            config.get_sub("http", "https://x.example", "proxy"),
            Some("http://p:1")
        );
    }
}
