// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The GitLab knowledge: host matching, the alias address form, glab
//! config, API access. Everything degrades silently to "unknown".

use std::process::Command;

/// Does this remote URL belong to GitLab? The product's own domain, plus
/// every host glab is signed in to: that is how a self-hosted GitLab on
/// any domain becomes reachable without putting somebody's instance into
/// this code. An instance nobody is signed in to still has the
/// project.yaml `forge:` override.
pub fn claims_remote(url: &str) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    host == "gitlab.com"
        || host.ends_with(".gitlab.com")
        || configured_hosts().iter().any(|known| known == &host)
}

/// Every host block in glab's config.yml, lowercased.
fn configured_hosts() -> Vec<String> {
    let Some(path) = glab_config_dir().map(|d| d.join("config.yml")) else {
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
    if !url.contains("://") {
        let (host_part, _path) = url.split_once(':')?;
        let host = host_part.rsplit('@').next()?;
        return Some(host.to_ascii_lowercase());
    }
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    Some(host.to_ascii_lowercase())
}

/// A parsed GitLab noreply alias: `<id>-<username>@users.noreply.gitlab.com`.
pub struct Alias {
    pub login: String,
    pub user_id: Option<String>,
}

/// Parse an address as a GitLab noreply alias, if it is one. The local
/// part is `<numeric id>-<username>`; usernames may contain `-`, so the
/// split is at the FIRST dash after the digits. gitlab.com and every
/// self-hosted instance share the shape, the domain is the instance's.
pub fn parse_alias(email: &str) -> Option<Alias> {
    let (local, domain) = email.trim().split_once('@')?;
    if !domain.to_ascii_lowercase().starts_with("users.noreply.") || local.is_empty() {
        return None;
    }
    let digits: String = local.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &local[digits.len()..];
    let login = rest.strip_prefix('-').filter(|l| !l.is_empty())?;
    Some(Alias {
        login: login.to_string(),
        user_id: Some(digits),
    })
}

/// The signed-in login from glab's config, offline. Same minimal line
/// parse as the gh twin: the `user:` under the `gitlab.com:` block.
pub fn glab_login() -> Option<String> {
    let text = std::fs::read_to_string(glab_config_dir()?.join("config.yml")).ok()?;
    parse_config_yml(&text)
}

fn glab_config_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("GLAB_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            return Some(std::path::PathBuf::from(dir));
        }
    }
    Some(std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config/glab-cli"))
}

/// The signed-in login: gitlab.com's when configured, else whichever
/// instance is (a self-hosted-only setup has no gitlab.com block).
pub fn parse_config_yml(text: &str) -> Option<String> {
    let hosts = parse_hosts(text);
    hosts
        .iter()
        .find(|(host, _)| host == "gitlab.com")
        .or_else(|| hosts.first())
        .map(|(_, user)| user.clone())
}

/// Every `<host>: { user: ... }` block under `hosts:`, in file order.
pub fn parse_hosts(text: &str) -> Vec<(String, String)> {
    let mut hosts: Vec<(String, String)> = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let stripped = line.trim_end().trim_start();
        if stripped.is_empty() || stripped == "hosts:" {
            continue;
        }
        // a host block header: a bare `<something>:` with no value
        if stripped.ends_with(':') && !stripped.contains(' ') {
            current = Some(stripped.trim_end_matches(':').to_ascii_lowercase());
            continue;
        }
        if let (Some(host), Some(user)) = (&current, stripped.strip_prefix("user:")) {
            let user = user.trim();
            if !user.is_empty() && !hosts.iter().any(|(h, _)| h == host) {
                hosts.push((host.clone(), user.to_string()));
            }
        }
    }
    hosts
}

fn run_stdout(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// The account's addresses, best effort: with `--token-env` directly
/// against the API (Bearer), else through glab's own auth.
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
                "https://gitlab.com/api/v4/user/emails",
            ]))
        }
        None => run_stdout(Command::new("glab").args(["api", "user/emails"])),
    };
    let Some(raw) = raw else { return Vec::new() };
    #[derive(serde::Deserialize)]
    struct Entry {
        email: String,
    }
    serde_json::from_str::<Vec<Entry>>(&raw)
        .map(|entries| entries.into_iter().map(|e| e.email).collect())
        .unwrap_or_default()
}

/// The ACTOR answer (docs/plugins.md `identity`), same shape as the gh
/// twin: handed-in caller facts win over glab's config.
pub fn identity_answer(
    login: Option<String>,
    user_id: Option<String>,
    token_env: Option<&str>,
) -> serde_json::Value {
    let login = login.or_else(glab_login);
    let Some(login) = login else {
        return serde_json::json!({ "known": false });
    };
    let emails = verified_emails(token_env);
    serde_json::json!({
        "known": true,
        "login": login,
        "user_id": user_id,
        "emails": emails,
    })
}

/// The PURE address attribution (docs/plugins.md `resolve`): GitLab's
/// noreply alias encodes account id and username. Never consults
/// ambient state, by contract.
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
    fn gitlab_remotes_are_claimed_and_others_are_not() {
        assert!(claims_remote("git@gitlab.com:group/proj.git"));
        assert!(claims_remote("https://gitlab.com/group/proj.git"));
        assert!(!claims_remote("git@github.com:o/r.git"));
        assert!(!claims_remote("https://gitlab.example.com/g/p.git"));
        assert!(!claims_remote("https://gitlab.com.evil.example/x.git"));
    }

    #[test]
    fn the_alias_form_parses_with_dashed_usernames() {
        let a = parse_alias("1234567-a-dashed-name@users.noreply.gitlab.com").unwrap();
        assert_eq!(a.login, "a-dashed-name");
        assert_eq!(a.user_id.as_deref(), Some("1234567"));
        assert!(parse_alias("nodigits@users.noreply.gitlab.com").is_none());
        assert!(parse_alias("123@users.noreply.gitlab.com").is_none());
        assert!(parse_alias("a@example.com").is_none());
    }

    #[test]
    fn config_yml_yields_the_login_of_gitlab_com_or_the_configured_instance() {
        let text = "hosts:\n    gitlab.com:\n        token: x\n        user: horst\n";
        assert_eq!(parse_config_yml(text).as_deref(), Some("horst"));
        // a self-hosted-only setup has no gitlab.com block; its login counts
        assert_eq!(
            parse_config_yml("hosts:\n    gitlab.acme.test:\n        user: alice\n").as_deref(),
            Some("alice")
        );
        assert_eq!(parse_config_yml(""), None);
    }

    /// A self-hosted GitLab lives on the customer's own domain, so a
    /// remote is claimed when glab is signed in to that host. No
    /// instance belongs in this code.
    #[test]
    fn a_self_hosted_host_is_claimed_once_glab_knows_it() {
        let dir = std::env::temp_dir().join(format!("joy-gitlab-claims-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.yml"),
            "hosts:\n    gitlab.acme.test:\n        user: alice\n",
        )
        .unwrap();
        std::env::set_var("GLAB_CONFIG_DIR", &dir);

        assert!(claims_remote("git@gitlab.acme.test:group/proj.git"));
        assert!(claims_remote("https://gitlab.com/group/proj.git"));
        assert!(!claims_remote("https://github.com/o/r.git"));
        assert!(!claims_remote(
            "https://gitlab.acme.test.evil.example/x.git"
        ));

        std::env::remove_var("GLAB_CONFIG_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_self_hosted_alias_form_parses_too() {
        let a = parse_alias("42-alice@users.noreply.gitlab.acme.test").unwrap();
        assert_eq!(a.login, "alice");
        assert_eq!(a.user_id.as_deref(), Some("42"));
    }
}
