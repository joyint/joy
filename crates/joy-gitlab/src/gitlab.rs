// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The GitLab knowledge: host matching, the alias address form, glab
//! config, API access. Everything degrades silently to "unknown".

use std::process::Command;

/// Does this remote URL belong to gitlab.com? Self-hosted instances are
/// not URL-recognizable; they are reached via the `forge:` override.
pub fn claims_remote(url: &str) -> bool {
    host_of(url)
        .map(|h| h == "gitlab.com" || h.ends_with(".gitlab.com"))
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

/// A parsed GitLab noreply alias: `<id>-<username>@users.noreply.gitlab.com`.
pub struct Alias {
    pub login: String,
    pub user_id: Option<String>,
}

/// Parse an address as a GitLab noreply alias, if it is one. The local
/// part is `<numeric id>-<username>`; usernames may contain `-`, so the
/// split is at the FIRST dash after the digits.
pub fn parse_alias(email: &str) -> Option<Alias> {
    let local = email
        .trim()
        .strip_suffix("@users.noreply.gitlab.com")
        .filter(|l| !l.is_empty())?;
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
    let dir = match std::env::var("GLAB_CONFIG_DIR") {
        Ok(d) if !d.trim().is_empty() => std::path::PathBuf::from(d),
        _ => std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config/glab-cli"),
    };
    let text = std::fs::read_to_string(dir.join("config.yml")).ok()?;
    parse_config_yml(&text)
}

/// The `user:` under the `gitlab.com:` block (nested under `hosts:`).
pub fn parse_config_yml(text: &str) -> Option<String> {
    let mut in_gitlab = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        let stripped = trimmed.trim_start();
        if stripped.trim_end_matches(':').trim() == "gitlab.com" {
            in_gitlab = true;
            continue;
        }
        if in_gitlab {
            if let Some(user) = stripped.strip_prefix("user:") {
                let user = user.trim();
                if !user.is_empty() {
                    return Some(user.to_string());
                }
            }
            // a new, less-indented host block ends the gitlab.com block
            if stripped.ends_with(':') && !stripped.contains(' ') && stripped != "gitlab.com:" {
                in_gitlab = false;
            }
        }
    }
    None
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
    fn config_yml_yields_the_gitlab_login() {
        let text = "hosts:\n    gitlab.com:\n        token: x\n        user: horst\n";
        assert_eq!(parse_config_yml(text).as_deref(), Some("horst"));
        assert_eq!(
            parse_config_yml("hosts:\n    other.host:\n        user: x\n"),
            None
        );
    }
}
