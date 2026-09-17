// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The Gitea knowledge: host matching, the alias address form, tea's
//! config, the REST API. Everything a read query cannot answer degrades
//! to "unknown".
//!
//! Gitea (and its fork Forgejo) is SELF-HOSTED software with no
//! canonical host: any domain can run it, and no instance belongs in
//! this code. So the connector claims a host only when the person's own
//! tea configuration names it, when an operator's `forges.yaml` does
//! (D2.5), or when the project's own `forge:` override says so.
//!
//! Since JOY-0298-E4 (design D2.8) every API call is made in process
//! over the connector's own HTTP client.

use joy_forge_net::forge::{unknown, unknown_state, Ctx, Listing, NewRepository, Target};
use joy_forge_net::http::Answer;
use joy_forge_net::scope::{self, Group};
use serde_json::{json, Value};

/// Where the store lives inside a repository.
const PROJECT_YAML: &str = ".joy/project.yaml";

/// Entries per page and pages read for one listing.
const TREE_PAGE: usize = 1000;
const TREE_PAGES: usize = 5;

/// Does this host belong to a Gitea instance THIS person is signed in
/// to (tea's own config)? An unknown host is not claimed: a URL alone
/// cannot tell Gitea from anything else, and guessing would steal the
/// remote from the connector it really belongs to.
pub fn claims_host(host: &str, configured: &[String]) -> bool {
    configured.iter().any(|known| known == host)
}

/// The hosts of every login in tea's config, lowercased.
pub fn configured_hosts() -> Vec<String> {
    tea_logins()
        .iter()
        .filter_map(|login| joy_forge_net::url::host_of(&login.url))
        .collect()
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
    // The GitHub and GitLab forms live under `users.noreply.<host>`;
    // theirs are their connectors' business, never this one's.
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

/// Every login tea has on file, offline, from the first of the per
/// operating system locations of D2.4 that exists. Empty when tea is
/// not set up.
pub fn tea_logins() -> Vec<TeaLogin> {
    match joy_forge_net::foreign::first_readable(&joy_forge_net::foreign::tea_config_files()) {
        Some((_, text)) => parse_config_yml(&text),
        None => Vec::new(),
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

/// The API root of the instance a host runs: what an operator
/// configured (D2.5), else the host's own `/api/v1`.
pub fn api_base(host: &str, ctx: &Ctx) -> String {
    if let Some(base) = ctx.instance(host).and_then(|i| i.api_base.clone()) {
        return base;
    }
    format!("https://{host}/api/v1")
}

/// One API GET. Gitea's own scheme is `Authorization: token <t>`, and
/// the token travels in that header, never in an argument.
fn api_get(ctx: &Ctx, host: &str, url: &str) -> Option<Answer> {
    let http = ctx.http(host).ok()?;
    let mut request = http.get(url).header("Accept", "application/json");
    if let Some(token) = ctx.token("gitea", host) {
        request = request.token_header(&token);
    }
    match request.call() {
        Ok(answer) => Some(answer),
        Err(error) => {
            eprintln!("joy-forge gitea: {error}");
            None
        }
    }
}

/// What a refusal means (D2.7c). Gitea says which scopes it wanted, in
/// prose it writes itself, and the `required=` list is parsed out of it.
pub fn classify(answer: &Answer) -> &'static str {
    match answer.status {
        403 if required_scopes(&answer.body).is_some() => "scope_missing",
        403 => "denied",
        401 => "needs_sign_in",
        429 => "rate_limited",
        _ => "denied",
    }
}

/// The `required=...` list out of Gitea's own refusal:
/// "token does not have at least one of required scope(s), required=[read:repository], token scope=read:user"
pub fn required_scopes(body: &str) -> Option<Vec<String>> {
    let rest = body.split("required=").nth(1)?;
    let list = rest
        .trim_start()
        .trim_start_matches('[')
        .split(']')
        .next()
        .unwrap_or("")
        .split(&[',', '"'][..])
        .map(str::trim)
        .filter(|scope| !scope.is_empty() && !scope.contains(' '))
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!list.is_empty()).then_some(list)
}

/// The account's addresses, best effort, from the instance's own API.
/// Without a credential nothing is asked: an anonymous request cannot
/// name an account (decision 20).
fn verified_emails(ctx: &Ctx, host: &str) -> Vec<String> {
    if ctx.token("gitea", host).is_none() {
        return Vec::new();
    }
    let Some(answer) = api_get(ctx, host, &format!("{}/user/emails", api_base(host, ctx))) else {
        return Vec::new();
    };
    if !answer.ok() {
        return Vec::new();
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        email: String,
        #[serde(default)]
        verified: bool,
    }
    serde_json::from_str::<Vec<Entry>>(&answer.body)
        .map(|entries| {
            entries
                .into_iter()
                .filter(|e| e.verified)
                .map(|e| e.email)
                .collect()
        })
        .unwrap_or_default()
}

/// The ACTOR answer (docs/plugins.md `identity`): handed-in caller
/// facts win over tea's config.
pub fn identity_answer(target: &Target, ctx: &Ctx) -> Value {
    let configured = tea_logins();
    let host = target.host().or_else(|| {
        configured
            .first()
            .and_then(|login| joy_forge_net::url::host_of(&login.url))
    });
    let local = host.as_deref().and_then(|host| {
        configured
            .iter()
            .find(|login| joy_forge_net::url::host_of(&login.url).as_deref() == Some(host))
            .or_else(|| configured.first())
            .map(|login| login.user.clone())
    });
    let Some(login) = ctx.login.clone().or(local) else {
        return unknown();
    };
    // Without a known instance there is nowhere to ask; the login alone
    // is still a useful answer.
    let emails = match host.as_deref() {
        Some(host) => verified_emails(ctx, host),
        None => Vec::new(),
    };
    json!({
        "known": true,
        "login": login,
        "user_id": ctx.user_id,
        "emails": emails,
    })
}

/// The PURE address attribution (docs/plugins.md `resolve`).
pub fn resolve_answer(email: &str) -> Value {
    match parse_alias(email) {
        Some(alias) => json!({
            "known": true,
            "login": alias.login,
            "user_id": Value::Null,
            "emails": [],
        }),
        None => unknown(),
    }
}

// -- the store query (JP-013C-11) ---------------------------------------------

/// The STORE answer for a remote. The instance is the remote's own host.
pub fn store_answer(target: &Target, ctx: &Ctx) -> Value {
    let (Some(host), Some(path)) = (target.host(), target.repo_path()) else {
        return unknown_state();
    };
    let repo_url = format!("{}/repos/{path}", api_base(&host, ctx));
    let file = api_get(ctx, &host, &format!("{repo_url}/raw/{PROJECT_YAML}"));
    store_verdict(file, || api_get(ctx, &host, &repo_url))
}

/// The decision over the two answers, pure.
fn store_verdict(file: Option<Answer>, repo: impl FnOnce() -> Option<Answer>) -> Value {
    let Some(file) = file else {
        return unknown_state();
    };
    let store_body = match file.status {
        200..=299 => Some(file.body.clone()),
        404 => None,
        _ => return unknown_state(),
    };
    let Some(repo) = repo() else {
        return match store_body {
            Some(body) => json!({ "state": "store", "project_yaml": body }),
            None => unknown_state(),
        };
    };
    match repo.status {
        200..=299 => {}
        404 => {
            return match store_body {
                Some(body) => json!({ "state": "store", "project_yaml": body }),
                None => json!({ "state": "gone" }),
            }
        }
        _ => return unknown_state(),
    }
    let Ok(body) = serde_json::from_str::<Value>(&repo.body) else {
        return match store_body {
            Some(body) => json!({ "state": "store", "project_yaml": body }),
            None => unknown_state(),
        };
    };
    // Gitea's API `size` is KiB (services/convert/repository.go:205);
    // the protocol carries bytes (D2.4).
    let size_bytes = body
        .get("size")
        .and_then(|v| v.as_u64())
        .map(|kib| kib * 1024);
    if let Some(project_yaml) = store_body {
        return json!({ "state": "store", "project_yaml": project_yaml, "size_bytes": size_bytes });
    }
    let may_create = body
        .pointer("/permissions/push")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // an empty repository's first push goes to the branch the forge
    // names as its default, not to whatever a fresh clone guesses
    let default_branch = body.get("default_branch").and_then(|v| v.as_str());
    json!({
        "state": "missing",
        "may_create": may_create,
        "default_branch": default_branch,
        "size_bytes": size_bytes,
    })
}

// -- the files query (JAPP-0293-A7) --------------------------------------------

/// The FILES answer for a remote.
pub fn files_answer(target: &Target, ctx: &Ctx) -> Value {
    let (Some(host), Some(path)) = (target.host(), target.repo_path()) else {
        return unknown_state();
    };
    let base = format!("{}/repos/{path}/git/trees/HEAD", api_base(&host, ctx));
    files_verdict(|page| {
        api_get(
            ctx,
            &host,
            &format!("{base}?recursive=true&per_page={TREE_PAGE}&page={page}"),
        )
    })
}

/// The file paths over the pages, pure. An empty repository has no tree
/// to list: no files.
fn files_verdict(mut page: impl FnMut(usize) -> Option<Answer>) -> Value {
    let mut paths: Vec<String> = Vec::new();
    for number in 1..=TREE_PAGES {
        let Some(answer) = page(number) else {
            return unknown_state();
        };
        match answer.status {
            200..=299 => {}
            404 | 409 if number == 1 => {
                return json!({ "state": "files", "paths": [], "truncated": false })
            }
            _ => return unknown_state(),
        }
        let Ok(body) = serde_json::from_str::<Value>(&answer.body) else {
            return unknown_state();
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
            return json!({ "state": "files", "paths": paths, "truncated": false });
        }
    }
    json!({ "state": "files", "paths": paths, "truncated": true })
}

// -- the repository list (D2.4) ------------------------------------------------

/// The REPOSITORIES answer: the repositories this account can reach.
pub fn repositories_answer(target: &Target, listing: &Listing, ctx: &Ctx) -> Value {
    let Some(host) = target.host() else {
        return unknown_state();
    };
    if ctx.token("gitea", &host).is_none() {
        return json!({ "state": "needs_sign_in", "host": host });
    }
    let base = api_base(&host, ctx);
    let mut page: usize = listing
        .page
        .as_deref()
        .and_then(|p| p.parse().ok())
        .unwrap_or(1);
    let limit = listing.limit.max(1);
    let per_page = limit.clamp(1, 50);
    let mut repositories: Vec<Value> = Vec::new();
    let mut more = false;
    loop {
        let url = format!("{base}/user/repos?page={page}&limit={per_page}");
        let Some(answer) = api_get(ctx, &host, &url) else {
            return unknown_state();
        };
        if !answer.ok() {
            return json!({ "state": classify(&answer), "host": host });
        }
        let Ok(entries) = serde_json::from_str::<Vec<Value>>(&answer.body) else {
            return unknown_state();
        };
        let full_page = entries.len() >= per_page;
        for entry in &entries {
            if let Some(row) = repository_row(entry, listing.query.as_deref()) {
                repositories.push(row);
            }
        }
        page += 1;
        if repositories.len() >= limit {
            repositories.truncate(limit);
            more = true;
            break;
        }
        if !full_page {
            break;
        }
    }
    json!({
        "state": "repositories",
        "repositories": repositories,
        "truncated": more,
        "next": more.then(|| page.to_string()),
    })
}

fn repository_row(entry: &Value, query: Option<&str>) -> Option<Value> {
    let full_name = entry.get("full_name").and_then(|v| v.as_str())?;
    if let Some(query) = query {
        let query = query.trim().to_ascii_lowercase();
        if !query.is_empty() && !full_name.to_ascii_lowercase().contains(&query) {
            return None;
        }
    }
    Some(json!({
        "full_name": full_name,
        "name": entry.get("name").and_then(|v| v.as_str()),
        "private": entry.get("private").and_then(|v| v.as_bool()).unwrap_or(false),
        "clone_url": entry.get("clone_url").and_then(|v| v.as_str()),
        "ssh_url": entry.get("ssh_url").and_then(|v| v.as_str()),
        "default_branch": entry.get("default_branch").and_then(|v| v.as_str()),
        "web_url": entry.get("html_url").and_then(|v| v.as_str()),
    }))
}

// -- creating a repository (D2.4, decision 17) ---------------------------------

/// The CREATE-REPOSITORY answer. `POST /user/repos` is checked twice by
/// Gitea, by the /user group and by the route, which is why the scope
/// set for it is `write:user write:repository` (D2.7a).
pub fn create_repository_answer(target: &Target, new: &NewRepository, ctx: &Ctx) -> Value {
    let Some(host) = target.host() else {
        return unknown_state();
    };
    let Some(token) = ctx.token("gitea", &host) else {
        return json!({ "state": "needs_sign_in", "host": host });
    };
    let base = api_base(&host, ctx);
    let url = match new.owner.as_deref() {
        Some(owner) => format!("{base}/orgs/{owner}/repos"),
        None => format!("{base}/user/repos"),
    };
    let Ok(http) = ctx.http(&host) else {
        return unknown_state();
    };
    let answer = match http
        .post(&url)
        .header("Accept", "application/json")
        .token_header(&token)
        .send_json(&json!({ "name": new.name, "private": new.private, "auto_init": false }))
    {
        Ok(answer) => answer,
        Err(error) => {
            eprintln!("joy-forge gitea: {error}");
            return unknown_state();
        }
    };
    if !answer.ok() {
        let state = classify(&answer);
        if state == "scope_missing" {
            // Gitea names the scopes it wanted; joy's own set is what a
            // person is asked to sign in with.
            let have = required_scopes(&answer.body).unwrap_or_default();
            let needed = scope::missing("gitea", Group::CreateRepository, &[]);
            return scope::scope_missing(&host, "create-repository", &needed, &have);
        }
        return json!({
            "state": state,
            "host": host,
            "message": message_of(&answer),
        });
    }
    let created = answer.json().unwrap_or_default();
    json!({
        "created": true,
        "clone_url": created.get("clone_url").and_then(|v| v.as_str()),
        "ssh_url": created.get("ssh_url").and_then(|v| v.as_str()),
        "default_branch": created.get("default_branch").and_then(|v| v.as_str()),
        "web_url": created.get("html_url").and_then(|v| v.as_str()),
    })
}

/// Gitea's own error sentence, where it sent one.
fn message_of(answer: &Answer) -> String {
    answer
        .json()
        .and_then(|body| {
            body.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("the instance answered {}", answer.status))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gitea and Forgejo have no canonical host, so no instance belongs
    /// in this code: a host is claimed only when the person is signed in
    /// to that very host, whichever host that is.
    #[test]
    fn only_hosts_the_person_is_signed_in_to_are_claimed() {
        let configured = vec!["git.example.org".to_string()];
        assert!(claims_host("git.example.org", &configured));
        assert!(!claims_host("gitea.example.com", &configured));
        assert!(!claims_host("github.com", &configured));
        assert!(!claims_host("git.example.org.evil.example", &configured));
        assert!(!claims_host("codeberg.org", &[]));
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
        assert!(parse_alias("7+login@users.noreply.github.com").is_none());
        assert!(parse_alias("7-login@users.noreply.gitlab.com").is_none());
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
        assert!(parse_config_yml("logins:\n- name: x\n  url: https://x.test/\n").is_empty());
    }

    #[test]
    fn the_instance_is_asked_its_own_api_v1() {
        let ctx = Ctx::bare(std::env::temp_dir());
        assert_eq!(
            api_base("codeberg.org", &ctx),
            "https://codeberg.org/api/v1"
        );
        let configured = Ctx::bare(std::env::temp_dir()).with_instances(
            joy_forge_net::config::Instances::from_text(
                "- host: git.acme.test\n  kind: gitea\n  api_base: https://git.acme.test/api/v1\n",
            )
            .unwrap(),
        );
        assert_eq!(
            api_base("git.acme.test", &configured),
            "https://git.acme.test/api/v1"
        );
    }

    /// D2.7c: Gitea writes the scopes it wanted into its refusal, and
    /// that list is parsed instead of being reported as `denied`.
    #[test]
    fn a_scope_refusal_is_read_out_of_giteas_own_sentence() {
        let body = r#"{"message":"token does not have at least one of required scope(s), required=[read:repository], token scope=read:user"}"#;
        assert_eq!(
            required_scopes(body),
            Some(vec!["read:repository".to_string()])
        );
        let answer = Answer::new(403, body, Vec::new());
        assert_eq!(classify(&answer), "scope_missing");
        assert_eq!(classify(&Answer::new(403, "{}", Vec::new())), "denied");
        assert_eq!(required_scopes("{}"), None);
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

    fn answer(status: u16, body: &str) -> Option<Answer> {
        Some(Answer::new(status, body, Vec::new()))
    }

    #[test]
    fn a_readable_project_yaml_is_the_store_and_carries_the_size_in_bytes() {
        let verdict = store_verdict(answer(200, "name: Demo\n"), || {
            answer(200, r#"{"size": 3}"#)
        });
        assert_eq!(verdict["state"], "store");
        assert_eq!(verdict["project_yaml"], "name: Demo\n");
        // Gitea counts KiB; the protocol carries bytes (D2.4)
        assert_eq!(verdict["size_bytes"], 3 * 1024);
    }

    #[test]
    fn a_404_asks_the_repository_whether_it_is_gone_or_only_storeless() {
        assert_eq!(
            store_verdict(answer(404, "{}"), || answer(404, "{}")),
            json!({ "state": "gone" })
        );
        let missing = store_verdict(answer(404, "{}"), || {
            answer(
                200,
                r#"{"permissions": {"admin": false, "push": true, "pull": true}}"#,
            )
        });
        assert_eq!(missing["state"], "missing");
        assert_eq!(missing["may_create"], true);
    }

    #[test]
    fn anything_unclear_stays_unanswered() {
        let unknown = json!({ "state": "unknown" });
        assert_eq!(store_verdict(None, || None), unknown);
        assert_eq!(store_verdict(answer(401, ""), || None), unknown);
        assert_eq!(store_verdict(answer(404, ""), || answer(502, "")), unknown);
    }

    #[test]
    fn the_path_comes_from_the_remote() {
        assert_eq!(
            joy_forge_net::url::repo_path_of("https://codeberg.org/joyint/demo.git").as_deref(),
            Some("joyint/demo")
        );
        assert_eq!(
            joy_forge_net::url::repo_path_of("git@codeberg.org:joyint/demo.git").as_deref(),
            Some("joyint/demo")
        );
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;

    fn page(entries: &[&str], more: bool) -> Option<Answer> {
        let tree: Vec<Value> = entries
            .iter()
            .map(|p| json!({ "path": p, "type": "blob" }))
            .collect();
        Some(Answer::new(
            200,
            json!({ "tree": tree, "truncated": more }).to_string(),
            Vec::new(),
        ))
    }

    #[test]
    fn pages_are_read_until_the_tree_ends_or_the_bound_is_reached() {
        let verdict = files_verdict(|n| match n {
            1 => page(&["VISION.md"], true),
            _ => page(&["docs/ARCHITECTURE.md"], false),
        });
        assert_eq!(
            verdict,
            json!({ "state": "files", "paths": ["VISION.md", "docs/ARCHITECTURE.md"], "truncated": false })
        );
        let endless = files_verdict(|_| page(&["x.md"], true));
        assert_eq!(endless["truncated"], json!(true));
        assert_eq!(endless["paths"].as_array().unwrap().len(), TREE_PAGES);
    }

    #[test]
    fn an_empty_repository_lists_nothing_and_a_failure_stays_unanswered() {
        assert_eq!(
            files_verdict(|_| Some(Answer::new(404, "", Vec::new()))),
            json!({ "state": "files", "paths": [], "truncated": false })
        );
        assert_eq!(files_verdict(|_| None), json!({ "state": "unknown" }));
    }
}
