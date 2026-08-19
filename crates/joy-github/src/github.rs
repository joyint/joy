// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The GitHub knowledge: host matching, alias address forms, gh config,
//! API access. Everything degrades silently to "unknown" — the caller
//! (joy-core's resolution fallback) treats every failure as no answer.

use std::process::Command;

/// Does this remote URL belong to GitHub? Handles the three wire forms
/// (`git@github.com:o/r.git`, `https://github.com/o/r.git`,
/// `ssh://git@github.com/o/r.git`); subdomains of github.com count
/// (GitHub Enterprise Cloud), lookalike hosts (`github.com.evil`) do not.
pub fn claims_remote(url: &str) -> bool {
    match host_of(url) {
        Some(host) => claims_host(&host, &configured_hosts()),
        None => false,
    }
}

/// The product's own domain, plus every host gh is signed in to: that is
/// how a GitHub Enterprise Server on any domain becomes reachable
/// without putting somebody's instance into this code. Pure, so the
/// tests need no ambient configuration.
fn claims_host(host: &str, configured: &[String]) -> bool {
    host == "github.com"
        || host.ends_with(".github.com")
        || configured.iter().any(|known| known == host)
}

/// Every host in gh's hosts.yml, lowercased.
fn configured_hosts() -> Vec<String> {
    let Some(path) = gh_config_dir().map(|d| d.join("hosts.yml")) else {
        return Vec::new();
    };
    match std::fs::read_to_string(path) {
        Ok(text) => parse_hosts(&text)
            .into_iter()
            .map(|(host, _)| host)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The host part of a git remote URL, lowercased.
fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    // scp-like: [user@]host:path — no scheme, exactly one ':' before '/'
    if !url.contains("://") {
        let (host_part, _path) = url.split_once(':')?;
        let host = host_part.rsplit('@').next()?;
        return Some(host.to_ascii_lowercase());
    }
    // scheme://[user@]host[/...]
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?; // strip a port
    Some(host.to_ascii_lowercase())
}

/// A parsed GitHub noreply alias: `<id>+<login>@users.noreply.github.com`
/// or the legacy `<login>@users.noreply.github.com`.
pub struct Alias {
    pub login: String,
    pub user_id: Option<String>,
}

/// Parse an address as a GitHub noreply alias, if it is one.
pub fn parse_alias(email: &str) -> Option<Alias> {
    let email = email.trim();
    // github.com and every Enterprise Server share the shape
    // `<id>+<login>@users.noreply.<host>`; the host is the instance's.
    let (local, domain) = email.split_once('@')?;
    if !domain.to_ascii_lowercase().starts_with("users.noreply.") || local.is_empty() {
        return None;
    }
    match local.split_once('+') {
        Some((id, login)) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => {
            (!login.is_empty()).then(|| Alias {
                login: login.to_string(),
                user_id: Some(id.to_string()),
            })
        }
        Some(_) => None,
        None => Some(Alias {
            login: local.to_string(),
            user_id: None,
        }),
    }
}

/// The signed-in login from gh's config (`hosts.yml`), offline. Minimal
/// line parse on purpose: the file is tiny, and a YAML dependency for two
/// lines would be the heavier contract.
pub fn gh_login() -> Option<String> {
    let path = gh_config_dir()?.join("hosts.yml");
    let text = std::fs::read_to_string(path).ok()?;
    parse_hosts_yml(&text)
}

fn gh_config_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("GH_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            return Some(std::path::PathBuf::from(dir));
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".config/gh"))
}

/// The `user:` under the `github.com:` block.
pub fn parse_hosts_yml(text: &str) -> Option<String> {
    let hosts = parse_hosts(text);
    // github.com first when it is there, else whichever instance is
    // configured (an Enterprise-only setup has no github.com block).
    hosts
        .iter()
        .find(|(host, _)| host == "github.com")
        .or_else(|| hosts.first())
        .map(|(_, user)| user.clone())
}

/// Every `<host>: { user: ... }` block of gh's hosts.yml, in file order.
pub fn parse_hosts(text: &str) -> Vec<(String, String)> {
    let mut hosts: Vec<(String, String)> = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with(' ') {
            current = Some(trimmed.trim_end_matches(':').trim().to_ascii_lowercase());
            continue;
        }
        if let Some(host) = &current {
            if let Some(user) = trimmed.trim_start().strip_prefix("user:") {
                let user = user.trim();
                if !user.is_empty() && !hosts.iter().any(|(h, _)| h == host) {
                    hosts.push((host.clone(), user.to_string()));
                }
            }
        }
    }
    hosts
}

/// The account's verified addresses, best effort. With `--token-env` the
/// API is called directly (curl, Bearer token from the named variable);
/// otherwise gh's own auth is used. Both need the user:email scope; an
/// answer without it is simply the empty list.
fn verified_emails(token_env: Option<&str>) -> Vec<String> {
    let raw = match token_env {
        Some(var) => {
            let Ok(token) = std::env::var(var) else {
                return Vec::new();
            };
            run_stdout(Command::new("curl").args([
                "--fail",
                "--silent",
                "--max-time",
                "4",
                "-H",
                &format!("Authorization: Bearer {token}"),
                "-H",
                "Accept: application/vnd.github+json",
                "https://api.github.com/user/emails",
            ]))
        }
        None => run_stdout(Command::new("gh").args(["api", "user/emails"])),
    };
    let Some(raw) = raw else { return Vec::new() };
    #[derive(serde::Deserialize)]
    struct Entry {
        email: String,
        #[serde(default)]
        verified: bool,
    }
    serde_json::from_str::<Vec<Entry>>(&raw)
        .map(|entries| {
            entries
                .into_iter()
                .filter(|e| e.verified)
                .map(|e| e.email)
                .collect()
        })
        .unwrap_or_default()
}

/// The public profile address (`/user`.email), visible without extra
/// scope when the person set one. gh-authenticated only (local use).
fn public_email() -> Option<String> {
    let raw = run_stdout(Command::new("gh").args(["api", "user"]))?;
    #[derive(serde::Deserialize)]
    struct User {
        email: Option<String>,
    }
    serde_json::from_str::<User>(&raw)
        .ok()?
        .email
        .filter(|e| !e.trim().is_empty())
}

fn run_stdout(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// The ACTOR answer (docs/plugins.md `identity`): who acts on GitHub.
/// Handed-in caller facts (a multi-account host's session) win over
/// local discovery (gh's config). `known: false` when nobody is known.
pub fn identity_answer(
    login: Option<String>,
    user_id: Option<String>,
    token_env: Option<&str>,
) -> serde_json::Value {
    let handed_in = login.is_some() || user_id.is_some();
    let login = login.or_else(gh_login);
    let Some(login) = login else {
        return serde_json::json!({ "known": false });
    };
    // Addresses come from the account the credentials speak for: the
    // token rides the same session as handed-in facts; locally gh
    // answers for gh's own login.
    let mut emails = verified_emails(token_env);
    if !handed_in && token_env.is_none() {
        if let Some(public) = public_email() {
            if !emails.contains(&public) {
                emails.push(public);
            }
        }
    }
    serde_json::json!({
        "known": true,
        "login": login,
        "user_id": user_id,
        "emails": emails,
    })
}

/// The PURE address attribution (docs/plugins.md `resolve`): derived
/// from the address alone — GitHub's noreply alias encodes login and
/// account id. Never consults ambient state, by contract.
pub fn resolve_answer(email: &str) -> serde_json::Value {
    match parse_alias(email) {
        Some(alias) => serde_json::json!({
            "known": true,
            "login": alias.login,
            "user_id": alias.user_id,
            "emails": [],
        }),
        None => serde_json::json!({ "known": false }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_remotes_are_claimed_in_every_wire_form() {
        for url in [
            "git@github.com:joyint/app.git",
            "https://github.com/joyint/app.git",
            "ssh://git@github.com/joyint/app.git",
            "https://user@github.com:443/joyint/app",
        ] {
            assert!(claims_remote(url), "{url}");
        }
        for url in [
            "git@gitea.example.com:joyint/app.git",
            "https://gitlab.com/joyint/app.git",
            "https://github.com.evil.example/x.git",
            "/home/user/bare.git",
        ] {
            assert!(!claims_remote(url), "{url}");
        }
    }

    #[test]
    fn both_alias_forms_parse_and_strangers_do_not() {
        let a = parse_alias("12345+alice@users.noreply.github.com").unwrap();
        assert_eq!(a.login, "alice");
        assert_eq!(a.user_id.as_deref(), Some("12345"));
        let legacy = parse_alias("alice@users.noreply.github.com").unwrap();
        assert_eq!(legacy.login, "alice");
        assert_eq!(legacy.user_id, None);
        assert!(parse_alias("alice@example.com").is_none());
        assert!(parse_alias("x+alice@users.noreply.github.com").is_none());
        assert!(parse_alias("@users.noreply.github.com").is_none());
    }

    #[test]
    fn hosts_yml_yields_the_login_of_github_com_or_the_configured_instance() {
        let text = "github.com:\n    user: joydev-horst\n    git_protocol: ssh\ngithub.acme.test:\n    user: nobody\n";
        assert_eq!(parse_hosts_yml(text).as_deref(), Some("joydev-horst"));
        // an Enterprise-only setup has no github.com block; its login counts
        assert_eq!(
            parse_hosts_yml("github.acme.test:\n    user: horst\n").as_deref(),
            Some("horst")
        );
        assert_eq!(parse_hosts_yml("").as_deref(), None);
    }

    /// GitHub Enterprise Server runs on the customer's own domain, so a
    /// remote is claimed when gh is signed in to that host. No instance
    /// belongs in this code.
    #[test]
    fn an_enterprise_host_is_claimed_once_gh_knows_it() {
        let configured = vec!["github.acme.test".to_string()];
        assert!(claims_host("github.acme.test", &configured));
        assert!(claims_host("github.com", &configured));
        assert!(!claims_host("gitlab.com", &configured));
        assert!(!claims_host("github.acme.test.evil.example", &configured));
        // without a signed-in enterprise host only the product domain counts
        assert!(!claims_host("github.acme.test", &[]));
    }

    #[test]
    fn the_enterprise_alias_form_parses_too() {
        let a = parse_alias("77+horst@users.noreply.github.acme.test").unwrap();
        assert_eq!(a.login, "horst");
        assert_eq!(a.user_id.as_deref(), Some("77"));
    }

    #[test]
    fn resolve_is_pure_and_attributes_aliases_only() {
        // hermetic on purpose: even with a signed-in gh, an address the
        // plugin cannot attribute stays unknown (the contract's purity)
        std::env::set_var("GH_CONFIG_DIR", "/nonexistent-joy-github-test");
        let owner = resolve_answer("99+bob@users.noreply.github.com");
        assert_eq!(owner["known"], true);
        assert_eq!(owner["login"], "bob");
        assert_eq!(owner["user_id"], "99");
        assert_eq!(resolve_answer("bob@example.com")["known"], false);
    }
}
