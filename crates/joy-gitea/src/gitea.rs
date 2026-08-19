// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The Gitea knowledge: host matching, the alias address form, tea's
//! config, API access. Everything degrades silently to "unknown".
//!
//! Gitea (and its fork Forgejo) is SELF-HOSTED software with no
//! canonical host: any domain can run it, and no instance belongs in
//! this code. So the plugin claims a remote only when the person's own
//! tea configuration names that host as one of their Gitea instances,
//! and otherwise waits for the project.yaml `forge:` override, exactly
//! the road self-hosted GitLab takes.

use std::process::Command;

/// Does this remote URL belong to a Gitea instance THIS person is signed
/// in to (tea's own config)? An unknown host is not claimed: a URL alone
/// cannot tell Gitea from anything else, and guessing would steal the
/// remote from the plugin it really belongs to. Projects on an instance
/// nobody is signed in to use the project.yaml `forge:` override.
pub fn claims_remote(url: &str) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    configured_hosts()
        .iter()
        .any(|configured| configured == &host)
}

/// The hosts of every login in tea's config, lowercased.
fn configured_hosts() -> Vec<String> {
    tea_logins()
        .iter()
        .filter_map(|login| host_of(&login.url))
        .collect()
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

/// One login from tea's config: which instance, and who is signed in.
pub struct TeaLogin {
    pub user: String,
    pub url: String,
}

/// Every login tea has on file, offline. Empty when tea is not set up.
pub fn tea_logins() -> Vec<TeaLogin> {
    let dir = match std::env::var("TEA_CONFIG_DIR") {
        Ok(d) if !d.trim().is_empty() => std::path::PathBuf::from(d),
        _ => match std::env::var_os("HOME") {
            Some(home) => std::path::PathBuf::from(home).join(".config/tea"),
            None => return Vec::new(),
        },
    };
    match std::fs::read_to_string(dir.join("config.yml")) {
        Ok(text) => parse_config_yml(&text),
        Err(_) => Vec::new(),
    }
}

/// Every `logins:` entry with both a user and a url. tea writes a list
/// of maps; the minimal line parse mirrors the gh and glab twins.
pub fn parse_config_yml(text: &str) -> Vec<TeaLogin> {
    let mut logins = Vec::new();
    let mut in_logins = false;
    let mut user: Option<String> = None;
    let mut url: Option<String> = None;
    let flush = |user: &mut Option<String>, url: &mut Option<String>, out: &mut Vec<TeaLogin>| {
        if let (Some(u), Some(base)) = (user.take(), url.take()) {
            out.push(TeaLogin { user: u, url: base });
        }
    };
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
        // a new list item closes the entry before it
        if stripped.starts_with("- ") {
            flush(&mut user, &mut url, &mut logins);
        }
        let field = stripped.trim_start_matches("- ").trim();
        if let Some(value) = field.strip_prefix("user:") {
            let value = value.trim();
            if !value.is_empty() {
                user = Some(value.to_string());
            }
        }
        if let Some(value) = field.strip_prefix("url:") {
            let value = value.trim();
            if !value.is_empty() {
                url = Some(value.trim_end_matches('/').to_string());
            }
        }
    }
    flush(&mut user, &mut url, &mut logins);
    logins
}

fn run_stdout(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// The account's addresses, best effort. Gitea's API takes the token in
/// the `token` scheme; the base URL is the instance tea is signed in to.
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
    let configured = tea_logins();
    let first = configured.into_iter().next();
    let base = first.as_ref().map(|l| l.url.clone());
    let login = login.or_else(|| first.map(|l| l.user));
    let Some(login) = login else {
        return serde_json::json!({ "known": false });
    };
    // Without a known instance there is nowhere to ask; the login alone
    // is still a useful answer.
    let emails = match &base {
        Some(base) => verified_emails(token_env, base),
        None => Vec::new(),
    };
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

    /// Gitea and Forgejo have no canonical host, so no instance belongs
    /// in this code: a remote is claimed only when tea is signed in to
    /// that very host, whichever host that is.
    #[test]
    fn only_hosts_the_person_is_signed_in_to_are_claimed() {
        let dir = std::env::temp_dir().join(format!("joy-gitea-claims-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.yml"),
            "logins:\n- name: house\n  url: https://git.example.org/\n  user: alice\n",
        )
        .unwrap();
        std::env::set_var("TEA_CONFIG_DIR", &dir);

        assert!(claims_remote("git@git.example.org:owner/repo.git"));
        assert!(claims_remote("https://git.example.org/owner/repo.git"));
        // a host nobody is signed in to stays unclaimed, however
        // gitea-ish it looks; the project.yaml forge override is its road
        assert!(!claims_remote("https://gitea.example.com/o/r.git"));
        assert!(!claims_remote("git@github.com:o/r.git"));
        assert!(!claims_remote("https://git.example.org.evil.example/x.git"));

        std::env::remove_var("TEA_CONFIG_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_alias_form_parses_and_leaves_the_other_forges_alone() {
        assert_eq!(
            parse_alias("horst@noreply.git.example.org").unwrap().login,
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
        assert!(parse_alias("@noreply.git.example.org").is_none());
    }

    #[test]
    fn config_yml_yields_every_login_and_its_instance() {
        let text = "logins:\n\
             - name: house\n  url: https://git.example.org/\n  token: x\n  user: horst\n\
             - name: other\n  url: https://git.other.test/\n  user: alice\n";
        let logins = parse_config_yml(text);
        assert_eq!(logins.len(), 2);
        assert_eq!(logins[0].user, "horst");
        assert_eq!(logins[0].url, "https://git.example.org");
        assert_eq!(logins[1].user, "alice");
        // an entry without a user is no login
        assert!(parse_config_yml("logins:\n- name: x\n  url: https://x.test/\n").is_empty());
    }
}
