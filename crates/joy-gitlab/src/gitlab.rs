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

// -- the store query (JP-013C-11) ---------------------------------------------
//
// A multi-account host (the platform) asks whether a repository holds a joy
// store instead of cloning it. One API call reads `.joy/project.yaml` raw
// from the default branch; only a 404 needs a second one on the project,
// because GitLab answers a missing file and a missing project alike. GitLab
// names an access level instead of a push flag, so a Developer's push
// permission also depends on the default branch's protection.

/// Where the store lives inside a repository.
const PROJECT_YAML: &str = ".joy/project.yaml";

/// One API answer: the HTTP status and the body.
struct ApiAnswer {
    status: u16,
    body: String,
}

/// "owner/repo" from a remote URL of any wire form, nested groups included.
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
    let mut child = Command::new("curl")
        .args([
            "--silent",
            "--max-time",
            "4",
            "--write-out",
            "\n%{http_code}",
            "-H",
            "Accept: application/json",
            "-H",
            "User-Agent: joy-gitlab",
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

/// GitLab's access levels that can push.
const DEVELOPER: i64 = 30;
const MAINTAINER: i64 = 40;

/// The STORE answer (docs/plugins.md `store`) for a remote. The instance
/// is the remote's own host. Without a token the instance is asked
/// anonymously, which only sees public projects.
pub fn store_answer(remote: &str, token_env: Option<&str>) -> serde_json::Value {
    let (Some(host), Some(path)) = (host_of(remote), repo_path_of(remote)) else {
        return serde_json::json!({ "state": "unknown" });
    };
    let project_url = format!(
        "https://{host}/api/v4/projects/{}",
        path.replace('/', "%2F")
    );
    let file = api_get(
        &format!(
            "{project_url}/repository/files/{}/raw?ref=HEAD",
            PROJECT_YAML.replace('/', "%2F")
        ),
        token_env,
    );
    store_verdict(
        file,
        || api_get(&project_url, token_env),
        || {
            api_get(
                &format!("{project_url}/protected_branches?per_page=100"),
                token_env,
            )
        },
    )
}

/// The decision over the answers, pure. Anything but a clear 2xx or 404
/// leaves the question unanswered.
fn store_verdict(
    file: Option<ApiAnswer>,
    project: impl FnOnce() -> Option<ApiAnswer>,
    protected: impl FnOnce() -> Option<ApiAnswer>,
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
    let Some(project) = project() else {
        return unknown;
    };
    match project.status {
        200..=299 => {}
        404 => return serde_json::json!({ "state": "gone" }),
        _ => return unknown,
    }
    let Ok(body) = serde_json::from_str::<serde_json::Value>(&project.body) else {
        return unknown;
    };
    let may_create = may_push(&body, protected);
    // an empty repository's first push goes to the branch the forge
    // names as its default, not to whatever a fresh clone guesses
    let default_branch = body.get("default_branch").and_then(|v| v.as_str());
    serde_json::json!({ "state": "missing", "may_create": may_create, "default_branch": default_branch })
}

/// Maintainers push to any branch; Developers only where the default
/// branch is not protected, and GitLab protects it by default. A
/// protection list GitLab would not show counts as protected.
fn may_push(project: &serde_json::Value, protected: impl FnOnce() -> Option<ApiAnswer>) -> bool {
    let level = ["project_access", "group_access"]
        .iter()
        .filter_map(|scope| {
            project
                .pointer(&format!("/permissions/{scope}/access_level"))
                .and_then(|v| v.as_i64())
        })
        .max()
        .unwrap_or(0);
    if level >= MAINTAINER {
        return true;
    }
    if level < DEVELOPER {
        return false;
    }
    let Some(branch) = project.get("default_branch").and_then(|v| v.as_str()) else {
        // an empty project has no branch to protect yet
        return true;
    };
    let Some(answer) = protected().filter(|a| (200..=299).contains(&a.status)) else {
        return false;
    };
    let Ok(rules) = serde_json::from_str::<Vec<serde_json::Value>>(&answer.body) else {
        return false;
    };
    !rules
        .iter()
        .filter_map(|rule| rule.get("name").and_then(|v| v.as_str()))
        .any(|rule| rule_matches(rule, branch))
}

/// A protection rule is a branch name or a pattern with `*`.
fn rule_matches(rule: &str, branch: &str) -> bool {
    match rule.split_once('*') {
        Some((head, tail)) => {
            branch.starts_with(head)
                && branch.ends_with(tail)
                && branch.len() >= head.len() + tail.len()
        }
        None => rule == branch,
    }
}

// -- the files query (JAPP-0293-A7) --------------------------------------------
//
// The setup of a new joy project points at the repository's documents, and
// the person picks them from the files the default branch carries. GitLab
// pages a recursive tree by 100; a bounded number of pages is read, and a
// tree that goes on beyond them is reported as cut off.

/// Entries per page (GitLab's maximum) and pages read for one listing.
const TREE_PAGE: usize = 100;
const TREE_PAGES: usize = 30;

/// The FILES answer (docs/plugins.md `files`) for a remote.
pub fn files_answer(remote: &str, token_env: Option<&str>) -> serde_json::Value {
    let (Some(host), Some(path)) = (host_of(remote), repo_path_of(remote)) else {
        return serde_json::json!({ "state": "unknown" });
    };
    let base = format!(
        "https://{host}/api/v4/projects/{}/repository/tree",
        path.replace('/', "%2F")
    );
    files_verdict(|page| {
        api_get(
            &format!("{base}?recursive=true&ref=HEAD&per_page={TREE_PAGE}&page={page}"),
            token_env,
        )
    })
}

/// The file paths over the pages, pure. A short page is the last one; an
/// empty project has no tree to list: no files.
fn files_verdict(mut page: impl FnMut(usize) -> Option<ApiAnswer>) -> serde_json::Value {
    let unknown = serde_json::json!({ "state": "unknown" });
    let mut paths: Vec<String> = Vec::new();
    for number in 1..=TREE_PAGES {
        let Some(answer) = page(number) else {
            return unknown;
        };
        match answer.status {
            200..=299 => {}
            404 if number == 1 => {
                return serde_json::json!({ "state": "files", "paths": [], "truncated": false })
            }
            _ => return unknown,
        }
        let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(&answer.body) else {
            return unknown;
        };
        let full = entries.len() >= TREE_PAGE;
        paths.extend(
            entries
                .iter()
                .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("blob"))
                .filter_map(|e| e.get("path").and_then(|p| p.as_str()))
                .map(String::from),
        );
        if !full {
            return serde_json::json!({ "state": "files", "paths": paths, "truncated": false });
        }
    }
    serde_json::json!({ "state": "files", "paths": paths, "truncated": true })
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

#[cfg(test)]
mod store_tests {
    use super::*;

    fn answer(status: u16, body: &str) -> Option<ApiAnswer> {
        Some(ApiAnswer {
            status,
            body: body.to_string(),
        })
    }

    fn project(level: i64, branch: Option<&str>) -> Option<ApiAnswer> {
        let body = serde_json::json!({
            "default_branch": branch,
            "permissions": {"project_access": {"access_level": level}, "group_access": null}
        });
        answer(200, &body.to_string())
    }

    #[test]
    fn a_readable_project_yaml_is_the_store() {
        assert_eq!(
            store_verdict(
                answer(200, "name: Demo\n"),
                || panic!("no second request"),
                || panic!("no third request")
            ),
            serde_json::json!({ "state": "store", "project_yaml": "name: Demo\n" })
        );
    }

    #[test]
    fn a_404_asks_the_project_whether_it_is_gone_or_only_storeless() {
        assert_eq!(
            store_verdict(answer(404, "{}"), || answer(404, "{}"), || None),
            serde_json::json!({ "state": "gone" })
        );
        assert_eq!(
            store_verdict(
                answer(404, "{}"),
                || project(40, Some("main")),
                || { panic!("a maintainer needs no protection list") }
            ),
            serde_json::json!({ "state": "missing", "may_create": true, "default_branch": "main" })
        );
    }

    #[test]
    fn a_developer_may_push_only_to_an_unprotected_default_branch() {
        let rules = || answer(200, r#"[{"name": "main"}, {"name": "release/*"}]"#);
        assert!(!may_push(
            &serde_json::from_str(&project(30, Some("main")).unwrap().body).unwrap(),
            rules
        ));
        assert!(may_push(
            &serde_json::from_str(&project(30, Some("trunk")).unwrap().body).unwrap(),
            rules
        ));
        // a list GitLab would not show counts as protected
        assert!(!may_push(
            &serde_json::from_str(&project(30, Some("trunk")).unwrap().body).unwrap(),
            || answer(403, "")
        ));
        // an empty project has nothing protected yet
        assert!(may_push(
            &serde_json::from_str(&project(30, None).unwrap().body).unwrap(),
            || None
        ));
        // reporters and guests never push
        assert!(!may_push(
            &serde_json::from_str(&project(20, Some("trunk")).unwrap().body).unwrap(),
            || answer(200, "[]")
        ));
    }

    #[test]
    fn anything_unclear_stays_unanswered() {
        let unknown = serde_json::json!({ "state": "unknown" });
        assert_eq!(store_verdict(None, || None, || None), unknown);
        assert_eq!(store_verdict(answer(401, ""), || None, || None), unknown);
        assert_eq!(
            store_verdict(answer(404, ""), || answer(500, ""), || None),
            unknown
        );
    }

    #[test]
    fn nested_groups_keep_their_path() {
        assert_eq!(
            repo_path_of("https://gitlab.com/grp/sub/repo.git").as_deref(),
            Some("grp/sub/repo")
        );
        assert!(rule_matches("release/*", "release/1.2"));
        assert!(!rule_matches("release/*", "releases/1"));
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;

    fn page(count: usize, name: &str) -> Option<ApiAnswer> {
        let entries: Vec<serde_json::Value> = (0..count)
            .map(|i| serde_json::json!({ "path": format!("{name}{i}.md"), "type": "blob" }))
            .collect();
        Some(ApiAnswer {
            status: 200,
            body: serde_json::Value::Array(entries).to_string(),
        })
    }

    #[test]
    fn a_short_page_ends_the_listing_and_the_bound_cuts_it_off() {
        let verdict = files_verdict(|n| {
            if n == 1 {
                page(TREE_PAGE, "a")
            } else {
                page(2, "b")
            }
        });
        assert_eq!(verdict["truncated"], serde_json::json!(false));
        assert_eq!(verdict["paths"].as_array().unwrap().len(), TREE_PAGE + 2);
        let endless = files_verdict(|_| page(TREE_PAGE, "x"));
        assert_eq!(endless["truncated"], serde_json::json!(true));
    }

    #[test]
    fn trees_are_skipped_an_empty_project_lists_nothing() {
        let body =
            r#"[{"path": "docs", "type": "tree"}, {"path": "docs/VISION.md", "type": "blob"}]"#;
        assert_eq!(
            files_verdict(|_| Some(ApiAnswer {
                status: 200,
                body: body.into()
            })),
            serde_json::json!({ "state": "files", "paths": ["docs/VISION.md"], "truncated": false })
        );
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
