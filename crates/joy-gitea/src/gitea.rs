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
            run_stdout(joy_process::command("curl").args([
                "--fail",
                "--silent",
                "--max-time",
                "4",
                "-H",
                &format!("Authorization: token {token}"),
                &format!("{base}/api/v1/user/emails"),
            ]))
        }
        None => run_stdout(joy_process::command("tea").args(["api", "get", "user/emails"])),
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

// -- the store query (JP-013C-11) ---------------------------------------------
//
// A multi-account host (the platform) asks whether a repository holds a joy
// store instead of cloning it. One API call reads `.joy/project.yaml` raw
// from the default branch; only a 404 needs a second one on the repository,
// because Gitea answers a missing file and a missing repository alike.

/// Where the store lives inside a repository.
const PROJECT_YAML: &str = ".joy/project.yaml";

/// One API answer: the HTTP status and the body.
struct ApiAnswer {
    status: u16,
    body: String,
}

/// "owner/repo" from a remote URL of any wire form.
fn repo_path_of(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let path = path.trim_matches('/');
    path.contains('/').then(|| path.to_string())
}

/// GET through curl. The token comes from the named variable and reaches
/// curl on stdin as a header line, never on its command line. `None` when
/// the request did not complete (network, timeout, no curl).
fn api_get(url: &str, token_env: Option<&str>) -> Option<ApiAnswer> {
    use std::io::Write;
    use std::process::Stdio;
    let token = token_env
        .and_then(|var| std::env::var(var).ok())
        .filter(|t| !t.is_empty());
    let mut child = joy_process::command("curl")
        .args([
            "--silent",
            "--max-time",
            "4",
            "--write-out",
            "\n%{http_code}",
            "-H",
            "Accept: application/json",
            "-H",
            "User-Agent: joy-gitea",
            "--header",
            "@-",
            url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let mut stdin = child.stdin.take()?;
        if let Some(token) = token {
            writeln!(stdin, "Authorization: Bearer {token}").ok()?;
        }
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let (body, status) = text.rsplit_once('\n')?;
    Some(ApiAnswer {
        status: status.trim().parse().ok()?,
        body: body.to_string(),
    })
}

/// The STORE answer (docs/plugins.md `store`) for a remote. The instance
/// is the remote's own host. Without a token the instance is asked
/// anonymously, which only sees public repositories.
pub fn store_answer(remote: &str, token_env: Option<&str>) -> serde_json::Value {
    let (Some(host), Some(path)) = (host_of(remote), repo_path_of(remote)) else {
        return serde_json::json!({ "state": "unknown" });
    };
    let repo_url = format!("https://{host}/api/v1/repos/{path}");
    let file = api_get(&format!("{repo_url}/raw/{PROJECT_YAML}"), token_env);
    store_verdict(file, || api_get(&repo_url, token_env))
}

/// The decision over the two answers, pure. Anything but a clear 2xx or
/// 404 leaves the question unanswered.
fn store_verdict(
    file: Option<ApiAnswer>,
    repo: impl FnOnce() -> Option<ApiAnswer>,
) -> serde_json::Value {
    let unknown = serde_json::json!({ "state": "unknown" });
    let Some(file) = file else { return unknown };
    match file.status {
        200..=299 => {
            return serde_json::json!({ "state": "store", "project_yaml": file.body });
        }
        404 => {}
        _ => return unknown,
    }
    let Some(repo) = repo() else { return unknown };
    match repo.status {
        200..=299 => {}
        404 => return serde_json::json!({ "state": "gone" }),
        _ => return unknown,
    }
    let Ok(body) = serde_json::from_str::<serde_json::Value>(&repo.body) else {
        return unknown;
    };
    let may_create = body
        .pointer("/permissions/push")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // an empty repository's first push goes to the branch the forge
    // names as its default, not to whatever a fresh clone guesses
    let default_branch = body.get("default_branch").and_then(|v| v.as_str());
    serde_json::json!({ "state": "missing", "may_create": may_create, "default_branch": default_branch })
}

// -- the files query (JAPP-0293-A7) --------------------------------------------
//
// The setup of a new joy project points at the repository's documents, and
// the person picks them from the files the default branch carries. Gitea
// pages a recursive tree; a bounded number of pages is read, and a tree
// that goes on beyond them is reported as cut off.

/// Entries per page and pages read for one listing.
const TREE_PAGE: usize = 1000;
const TREE_PAGES: usize = 5;

/// The FILES answer (docs/plugins.md `files`) for a remote.
pub fn files_answer(remote: &str, token_env: Option<&str>) -> serde_json::Value {
    let (Some(host), Some(path)) = (host_of(remote), repo_path_of(remote)) else {
        return serde_json::json!({ "state": "unknown" });
    };
    let base = format!("https://{host}/api/v1/repos/{path}/git/trees/HEAD");
    files_verdict(|page| {
        api_get(
            &format!("{base}?recursive=true&per_page={TREE_PAGE}&page={page}"),
            token_env,
        )
    })
}

/// The file paths over the pages, pure. An empty repository has no tree to
/// list: no files.
fn files_verdict(mut page: impl FnMut(usize) -> Option<ApiAnswer>) -> serde_json::Value {
    let unknown = serde_json::json!({ "state": "unknown" });
    let mut paths: Vec<String> = Vec::new();
    for number in 1..=TREE_PAGES {
        let Some(answer) = page(number) else {
            return unknown;
        };
        match answer.status {
            200..=299 => {}
            404 | 409 if number == 1 => {
                return serde_json::json!({ "state": "files", "paths": [], "truncated": false })
            }
            _ => return unknown,
        }
        let Ok(body) = serde_json::from_str::<serde_json::Value>(&answer.body) else {
            return unknown;
        };
        if let Some(entries) = body.get("tree").and_then(|t| t.as_array()) {
            paths.extend(
                entries
                    .iter()
                    .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("blob"))
                    .filter_map(|e| e.get("path").and_then(|p| p.as_str()))
                    .map(String::from),
            );
        }
        // Gitea's `truncated` means "more pages follow"
        let more = body
            .get("truncated")
            .and_then(|t| t.as_bool())
            .unwrap_or(false);
        if !more {
            return serde_json::json!({ "state": "files", "paths": paths, "truncated": false });
        }
    }
    serde_json::json!({ "state": "files", "paths": paths, "truncated": true })
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

#[cfg(test)]
mod store_tests {
    use super::*;

    fn answer(status: u16, body: &str) -> Option<ApiAnswer> {
        Some(ApiAnswer {
            status,
            body: body.to_string(),
        })
    }

    #[test]
    fn a_readable_project_yaml_is_the_store() {
        assert_eq!(
            store_verdict(answer(200, "name: Demo\n"), || panic!("no second request")),
            serde_json::json!({ "state": "store", "project_yaml": "name: Demo\n" })
        );
    }

    #[test]
    fn a_404_asks_the_repository_whether_it_is_gone_or_only_storeless() {
        assert_eq!(
            store_verdict(answer(404, "{}"), || answer(404, "{}")),
            serde_json::json!({ "state": "gone" })
        );
        assert_eq!(
            store_verdict(answer(404, "{}"), || {
                answer(
                    200,
                    r#"{"permissions": {"admin": false, "push": true, "pull": true}}"#,
                )
            }),
            serde_json::json!({ "state": "missing", "may_create": true, "default_branch": null })
        );
    }

    #[test]
    fn anything_unclear_stays_unanswered() {
        let unknown = serde_json::json!({ "state": "unknown" });
        assert_eq!(store_verdict(None, || None), unknown);
        assert_eq!(store_verdict(answer(401, ""), || None), unknown);
        assert_eq!(store_verdict(answer(404, ""), || answer(502, "")), unknown);
    }

    #[test]
    fn the_path_comes_from_the_remote() {
        assert_eq!(
            repo_path_of("https://codeberg.org/joyint/demo.git").as_deref(),
            Some("joyint/demo")
        );
        assert_eq!(
            repo_path_of("git@codeberg.org:joyint/demo.git").as_deref(),
            Some("joyint/demo")
        );
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;

    fn page(entries: &[&str], more: bool) -> Option<ApiAnswer> {
        let tree: Vec<serde_json::Value> = entries
            .iter()
            .map(|p| serde_json::json!({ "path": p, "type": "blob" }))
            .collect();
        Some(ApiAnswer {
            status: 200,
            body: serde_json::json!({ "tree": tree, "truncated": more }).to_string(),
        })
    }

    #[test]
    fn pages_are_read_until_the_tree_ends_or_the_bound_is_reached() {
        let verdict = files_verdict(|n| match n {
            1 => page(&["VISION.md"], true),
            _ => page(&["docs/ARCHITECTURE.md"], false),
        });
        assert_eq!(
            verdict,
            serde_json::json!({ "state": "files", "paths": ["VISION.md", "docs/ARCHITECTURE.md"], "truncated": false })
        );
        let endless = files_verdict(|_| page(&["x.md"], true));
        assert_eq!(endless["truncated"], serde_json::json!(true));
        assert_eq!(endless["paths"].as_array().unwrap().len(), TREE_PAGES);
    }

    #[test]
    fn an_empty_repository_lists_nothing_and_a_failure_stays_unanswered() {
        assert_eq!(
            files_verdict(|_| Some(ApiAnswer {
                status: 404,
                body: String::new()
            })),
            serde_json::json!({ "state": "files", "paths": [], "truncated": false })
        );
        assert_eq!(
            files_verdict(|_| None),
            serde_json::json!({ "state": "unknown" })
        );
    }
}
