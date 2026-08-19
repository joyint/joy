// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The Gitea knowledge: host matching, the alias address form, tea's
//! config, API access. Everything degrades silently to "unknown".
//!
//! Gitea (and its fork Forgejo) runs anywhere, so a URL alone identifies
//! only the well-known public instance, codeberg.org; every other
//! instance is reached through the project.yaml `forge:` override, the
//! same rule the GitLab twin uses for self-hosted GitLab.

use std::process::Command;

/// The public instance a URL can identify on its own.
const PUBLIC_INSTANCE: &str = "codeberg.org";

/// Does this remote URL belong to the public Gitea instance? Self-hosted
/// instances are not URL-recognizable; they are reached via the `forge:`
/// override.
pub fn claims_remote(url: &str) -> bool {
    host_of(url)
        .map(|h| h == PUBLIC_INSTANCE || h.ends_with(&format!(".{PUBLIC_INSTANCE}")))
        .unwrap_or(false)
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

/// A parsed Gitea noreply alias: `<username>@noreply.<instance host>`.
/// Gitea's "Keep Email Private" hands out exactly this form, with the
/// domain from the instance's NO_REPLY_ADDRESS setting; there is no
/// account id in it, unlike the GitHub and GitLab forms.
pub struct Alias {
    pub login: String,
}

/// Parse an address as a Gitea noreply alias, if it is one.
pub fn parse_alias(email: &str) -> Option<Alias> {
    let (local, domain) = email.trim().split_once('@')?;
    if local.is_empty() {
        return None;
    }
    let domain = domain.to_ascii_lowercase();
    // The GitHub and GitLab forms live under `users.noreply.<host>`; theirs
    // are their plugins' business, never this one's.
    let rest = domain.strip_prefix("noreply.")?;
    if rest.is_empty() || !rest.contains('.') {
        return None;
    }
    Some(Alias {
        login: local.to_string(),
    })
}

/// tea's configured login for an instance, offline: the `user:` of the
/// first login entry, plus its `url:` so the API calls know where to go.
pub struct TeaLogin {
    pub user: String,
    pub url: String,
}

pub fn tea_login() -> Option<TeaLogin> {
    let dir = match std::env::var("TEA_CONFIG_DIR") {
        Ok(d) if !d.trim().is_empty() => std::path::PathBuf::from(d),
        _ => std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config/tea"),
    };
    let text = std::fs::read_to_string(dir.join("config.yml")).ok()?;
    parse_config_yml(&text)
}

/// The first `logins:` entry's user and url. tea writes a list of maps;
/// the minimal line parse mirrors the gh and glab twins.
pub fn parse_config_yml(text: &str) -> Option<TeaLogin> {
    let mut in_logins = false;
    let mut user: Option<String> = None;
    let mut url: Option<String> = None;
    for line in text.lines() {
        let stripped = line.trim_start();
        if stripped.trim_end_matches(':').trim() == "logins" {
            in_logins = true;
            continue;
        }
        if !in_logins {
            continue;
        }
        // a new top-level key ends the logins block
        if !line.starts_with([' ', '\t', '-']) && line.trim_end().ends_with(':') {
            break;
        }
        let field = stripped.trim_start_matches("- ").trim();
        if let Some(value) = field.strip_prefix("user:") {
            let value = value.trim();
            if user.is_none() && !value.is_empty() {
                user = Some(value.to_string());
            }
        }
        if let Some(value) = field.strip_prefix("url:") {
            let value = value.trim();
            if url.is_none() && !value.is_empty() {
                url = Some(value.trim_end_matches('/').to_string());
            }
        }
        if user.is_some() && url.is_some() {
            break;
        }
    }
    Some(TeaLogin {
        user: user?,
        url: url.unwrap_or_else(|| format!("https://{PUBLIC_INSTANCE}")),
    })
}

fn run_stdout(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// The account's addresses, best effort. Gitea's API takes the token in
/// the `token` scheme; the base URL comes from tea's configured login,
/// else the public instance.
fn verified_emails(token_env: Option<&str>, base: &str) -> Vec<String> {
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
                &format!("Authorization: token {token}"),
                &format!("{base}/api/v1/user/emails"),
            ]))
        }
        None => run_stdout(Command::new("tea").args(["api", "get", "user/emails"])),
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

/// The ACTOR answer (docs/plugins.md `identity`): handed-in caller facts
/// win over tea's config.
pub fn identity_answer(
    login: Option<String>,
    user_id: Option<String>,
    token_env: Option<&str>,
) -> serde_json::Value {
    let configured = tea_login();
    let base = configured
        .as_ref()
        .map(|l| l.url.clone())
        .unwrap_or_else(|| format!("https://{PUBLIC_INSTANCE}"));
    let login = login.or_else(|| configured.map(|l| l.user));
    let Some(login) = login else {
        return serde_json::json!({ "known": false });
    };
    let emails = verified_emails(token_env, &base);
    serde_json::json!({
        "known": true,
        "login": login,
        "user_id": user_id,
        "emails": emails,
    })
}

/// The PURE address attribution (docs/plugins.md `resolve`): Gitea's
/// noreply alias carries the username, no account id. Never consults
/// ambient state, by contract.
pub fn resolve_answer(email: &str) -> serde_json::Value {
    match parse_alias(email) {
        Some(alias) => serde_json::json!({
            "known": true,
            "login": alias.login,
            "user_id": serde_json::Value::Null,
            "emails": [],
        }),
        None => serde_json::json!({ "known": false }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_instance_is_claimed_and_others_are_not() {
        assert!(claims_remote("git@codeberg.org:owner/repo.git"));
        assert!(claims_remote("https://codeberg.org/owner/repo.git"));
        assert!(!claims_remote("git@github.com:o/r.git"));
        assert!(!claims_remote("https://gitea.example.com/o/r.git"));
        assert!(!claims_remote("https://codeberg.org.evil.example/x.git"));
    }

    #[test]
    fn the_alias_form_parses_and_leaves_the_other_forges_alone() {
        assert_eq!(
            parse_alias("horst@noreply.codeberg.org").unwrap().login,
            "horst"
        );
        assert_eq!(
            parse_alias("a.dotted-name@noreply.gitea.example.com")
                .unwrap()
                .login,
            "a.dotted-name"
        );
        // the GitHub and GitLab forms belong to their own plugins
        assert!(parse_alias("7+login@users.noreply.github.com").is_none());
        assert!(parse_alias("7-login@users.noreply.gitlab.com").is_none());
        // a plain address is not an alias
        assert!(parse_alias("horst@example.com").is_none());
        assert!(parse_alias("@noreply.codeberg.org").is_none());
    }

    #[test]
    fn config_yml_yields_the_login_and_its_instance() {
        let text =
            "logins:\n- name: codeberg\n  url: https://codeberg.org/\n  token: x\n  user: horst\n";
        let login = parse_config_yml(text).unwrap();
        assert_eq!(login.user, "horst");
        assert_eq!(login.url, "https://codeberg.org");
        assert!(parse_config_yml("logins:\n- name: x\n  url: https://x/\n").is_none());
    }
}
