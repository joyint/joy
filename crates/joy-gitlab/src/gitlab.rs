// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The GitLab knowledge: host matching, the alias address form, glab's
//! config, the REST API. Everything a read query cannot answer degrades
//! to "unknown".
//!
//! Since JOY-0298-E4 (design D2.8) every API call is made in process
//! over the connector's own HTTP client, and it goes to the INSTANCE's
//! own base. The bug that fixes: `verified_emails` asked
//! `https://gitlab.com/api/v4/user/emails` even for a self hosted
//! instance, so a self hosted person's addresses came from a server
//! they had never signed in to, or from nowhere.

use joy_forge_net::auth::oauth::{Flow, OAuth};
use joy_forge_net::auth::Purpose;
use joy_forge_net::forge::{
    unknown, unknown_state, Account, Ctx, Listing, NewRepository, Reach, Target,
};
use joy_forge_net::http::Answer;
use joy_forge_net::scope::{self, Group};
use joy_forge_net::url::encode_segment;
use serde_json::{json, Value};

/// Where the store lives inside a repository.
const PROJECT_YAML: &str = ".joy/project.yaml";

/// GitLab's access levels that can push.
const DEVELOPER: i64 = 30;
const MAINTAINER: i64 = 40;

/// Entries per page (GitLab's maximum) and pages read for one listing.
const TREE_PAGE: usize = 100;
const TREE_PAGES: usize = 30;

/// Does this host belong to GitLab? The product's own domain, plus every
/// host glab is signed in to: that is how a self-hosted GitLab on any
/// domain becomes reachable without putting somebody's instance into
/// this code. `forges.yaml` is the other way (D2.5), and the dispatcher
/// consults it.
pub fn claims_host(host: &str, configured: &[String]) -> bool {
    host == "gitlab.com"
        || host.ends_with(".gitlab.com")
        || configured.iter().any(|known| known == host)
}

/// Every host block in glab's config.yml, lowercased.
pub fn configured_hosts() -> Vec<String> {
    glab_hosts().into_iter().map(|(host, _)| host).collect()
}

/// glab's `config.yml`, from the first of the per operating system
/// locations of D2.4 that exists.
pub fn glab_hosts() -> Vec<(String, String)> {
    match joy_forge_net::foreign::first_readable(&joy_forge_net::foreign::glab_config_files()) {
        Some((_, text)) => parse_hosts(&text),
        None => Vec::new(),
    }
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

/// The signed-in login from glab's config, offline: the host's own
/// block, else gitlab.com's, else whichever instance is configured.
pub fn glab_login(host: &str) -> Option<String> {
    let hosts = glab_hosts();
    hosts
        .iter()
        .find(|(known, _)| known == host)
        .or_else(|| hosts.iter().find(|(known, _)| known == "gitlab.com"))
        .or_else(|| hosts.first())
        .map(|(_, user)| user.clone())
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

/// The API root of the instance a host runs: what an operator
/// configured (D2.5), else the host's own `/api/v4`. Never gitlab.com's,
/// which is the bug of D2.8.
pub fn api_base(host: &str, ctx: &Ctx) -> String {
    if let Some(base) = ctx.instance(host).and_then(|i| i.api_base.clone()) {
        return base;
    }
    format!("https://{host}/api/v4")
}

/// One API GET. The token travels in a header, never in an argument.
fn api_get(ctx: &Ctx, host: &str, url: &str) -> Option<Answer> {
    let http = ctx.http(host).ok()?;
    let mut request = http.get(url).header("Accept", "application/json");
    if let Some(token) = ctx.token("gitlab", host) {
        request = request.bearer(&token);
    }
    match request.call() {
        Ok(answer) => Some(answer),
        Err(error) => {
            eprintln!("joy-forge gitlab: {error}");
            None
        }
    }
}

/// What a refusal means (D2.7c): a 403 whose WWW-Authenticate carries
/// `error="insufficient_scope"` is a scope problem and never `denied`.
pub fn classify(answer: &Answer) -> &'static str {
    match answer.status {
        403 => {
            let insufficient = answer
                .header("www-authenticate")
                .is_some_and(|value| value.contains("insufficient_scope"));
            if insufficient {
                "scope_missing"
            } else {
                "denied"
            }
        }
        401 => "needs_sign_in",
        429 => "rate_limited",
        _ => "denied",
    }
}

/// The granted scope set of the token in use: the personal access
/// token's own record first, then the OAuth token's info. `None` means
/// "not known", and an unknown set is never reported as a missing one.
pub fn granted_scopes(ctx: &Ctx, host: &str) -> Option<Vec<String>> {
    let base = api_base(host, ctx);
    if let Some(answer) = api_get(ctx, host, &format!("{base}/personal_access_tokens/self")) {
        if answer.ok() {
            if let Some(scopes) = string_list(&answer, "scopes") {
                return Some(scopes);
            }
        }
    }
    let info = base.strip_suffix("/api/v4").unwrap_or(&base).to_string();
    let answer = api_get(ctx, host, &format!("{info}/oauth/token/info"))?;
    answer.ok().then(|| string_list(&answer, "scope")).flatten()
}

fn string_list(answer: &Answer, key: &str) -> Option<Vec<String>> {
    let body = answer.json()?;
    let list = body.get(key)?;
    if let Some(array) = list.as_array() {
        return Some(
            array
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect(),
        );
    }
    list.as_str().map(scope::parse_granted)
}

/// The account's addresses, best effort, from the INSTANCE's own API.
/// Without a credential nothing is asked: an anonymous request cannot
/// name an account (decision 20).
fn verified_emails(ctx: &Ctx, host: &str) -> Vec<String> {
    if ctx.token("gitlab", host).is_none() {
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
    }
    serde_json::from_str::<Vec<Entry>>(&answer.body)
        .map(|entries| entries.into_iter().map(|e| e.email).collect())
        .unwrap_or_default()
}

/// The ACTOR answer (docs/plugins.md `identity`), same shape as the
/// GitHub twin: handed-in caller facts win over glab's config.
pub fn identity_answer(target: &Target, ctx: &Ctx) -> Value {
    let host = target.host().unwrap_or_else(|| "gitlab.com".to_string());
    let Some(login) = ctx.login.clone().or_else(|| glab_login(&host)) else {
        return unknown();
    };
    let emails = verified_emails(ctx, &host);
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
            "user_id": alias.user_id,
            "emails": [],
        }),
        None => unknown(),
    }
}

// -- the store query (JP-013C-11) ---------------------------------------------
//
// GitLab names an access level instead of a push flag, so a Developer's
// push permission also depends on the default branch's protection.

/// The STORE answer for a remote. The instance is the remote's own host.
pub fn store_answer(target: &Target, ctx: &Ctx) -> Value {
    let (Some(host), Some(path)) = (target.host(), target.repo_path()) else {
        return unknown_state();
    };
    let project_url = format!(
        "{}/projects/{}",
        api_base(&host, ctx),
        encode_segment(&path)
    );
    let file = api_get(
        ctx,
        &host,
        &format!(
            "{project_url}/repository/files/{}/raw?ref=HEAD",
            encode_segment(PROJECT_YAML)
        ),
    );
    store_verdict(
        file,
        // `statistics=true` is what carries `repository_size`, and
        // GitLab answers it in BYTES for a caller with at least the
        // Reporter role; for everyone else the field is simply absent
        // and its absence is not an error (D2.4).
        || api_get(ctx, &host, &format!("{project_url}?statistics=true")),
        || {
            api_get(
                ctx,
                &host,
                &format!("{project_url}/protected_branches?per_page=100"),
            )
        },
        // Asked only where the 404 branch needs it, because it costs a
        // request of its own (D1.9).
        || may_see_private(ctx, &host),
    )
}

/// Whether a 404 is a verdict (D2.7c): "404 is `gone` only when the set
/// contains `read_api` or `api`".
///
/// GitLab answers 404, not 403, for a private project the caller may
/// not see, and `write_repository` "Uses Git-over-HTTP. Does not
/// support API authentication.", so a token with that scope alone meets
/// the API as an anonymous caller and sees the same 404. Reporting it
/// as "gone" would tell a read only member their repository is deleted.
/// A set the instance will not name stays "not known", and an unknown
/// set is never taken for a wide one.
fn may_see_private(ctx: &Ctx, host: &str) -> bool {
    if ctx.token("gitlab", host).is_none() {
        return false;
    }
    match granted_scopes(ctx, host) {
        Some(granted) => scope::missing("gitlab", Group::RepositoryFacts, &granted).is_empty(),
        None => false,
    }
}

/// The decision over the answers, pure. Anything but a clear 2xx or 404
/// leaves the question unanswered.
fn store_verdict(
    file: Option<Answer>,
    project: impl FnOnce() -> Option<Answer>,
    protected: impl FnOnce() -> Option<Answer>,
    may_see_private: impl FnOnce() -> bool,
) -> Value {
    let Some(file) = file else {
        return unknown_state();
    };
    let store_body = match file.status {
        200..=299 => Some(file.body.clone()),
        404 => None,
        _ => return unknown_state(),
    };
    let Some(project) = project() else {
        return match store_body {
            Some(body) => json!({ "state": "store", "project_yaml": body }),
            None => unknown_state(),
        };
    };
    match project.status {
        200..=299 => {}
        404 => {
            return match store_body {
                Some(body) => json!({ "state": "store", "project_yaml": body }),
                // Not a verdict about the project unless the caller's
                // set could have seen a private one.
                None if may_see_private() => json!({ "state": "gone" }),
                None => unknown_state(),
            };
        }
        _ => return unknown_state(),
    }
    let Ok(body) = serde_json::from_str::<Value>(&project.body) else {
        return match store_body {
            Some(body) => json!({ "state": "store", "project_yaml": body }),
            None => unknown_state(),
        };
    };
    // GitLab returns `statistics.repository_size` in BYTES already.
    let size_bytes = body
        .pointer("/statistics/repository_size")
        .and_then(|v| v.as_u64());
    if let Some(project_yaml) = store_body {
        return json!({ "state": "store", "project_yaml": project_yaml, "size_bytes": size_bytes });
    }
    let may_create = may_push(&body, protected);
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

/// Maintainers push to any branch; Developers only where the default
/// branch is not protected, and GitLab protects it by default. A
/// protection list GitLab would not show counts as protected.
fn may_push(project: &Value, protected: impl FnOnce() -> Option<Answer>) -> bool {
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
    let Some(answer) = protected().filter(|a| a.ok()) else {
        return false;
    };
    let Ok(rules) = serde_json::from_str::<Vec<Value>>(&answer.body) else {
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

/// The FILES answer for a remote.
pub fn files_answer(target: &Target, ctx: &Ctx) -> Value {
    let (Some(host), Some(path)) = (target.host(), target.repo_path()) else {
        return unknown_state();
    };
    let base = format!(
        "{}/projects/{}/repository/tree",
        api_base(&host, ctx),
        encode_segment(&path)
    );
    files_verdict(|page| {
        api_get(
            ctx,
            &host,
            &format!("{base}?recursive=true&ref=HEAD&per_page={TREE_PAGE}&page={page}"),
        )
    })
}

/// The file paths over the pages, pure. A short page is the last one; an
/// empty project has no tree to list: no files.
fn files_verdict(mut page: impl FnMut(usize) -> Option<Answer>) -> Value {
    let mut paths: Vec<String> = Vec::new();
    for number in 1..=TREE_PAGES {
        let Some(answer) = page(number) else {
            return unknown_state();
        };
        match answer.status {
            200..=299 => {}
            404 if number == 1 => {
                return json!({ "state": "files", "paths": [], "truncated": false })
            }
            _ => return unknown_state(),
        }
        let Ok(entries) = serde_json::from_str::<Vec<Value>>(&answer.body) else {
            return unknown_state();
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
            return json!({ "state": "files", "paths": paths, "truncated": false });
        }
    }
    json!({ "state": "files", "paths": paths, "truncated": true })
}

// -- the repository list (D2.4) ------------------------------------------------

/// The REPOSITORIES answer: the projects this account is a member of.
pub fn repositories_answer(target: &Target, listing: &Listing, ctx: &Ctx) -> Value {
    let Some(host) = target.host() else {
        return unknown_state();
    };
    if ctx.token("gitlab", &host).is_none() {
        return json!({ "state": "needs_sign_in", "host": host });
    }
    let base = api_base(&host, ctx);
    let mut page: usize = listing
        .page
        .as_deref()
        .and_then(|p| p.parse().ok())
        .unwrap_or(1);
    let limit = listing.limit.max(1);
    // The page size is bounded by the caller's limit too, so no page has
    // to be cut in half: a cursor is a page number, and the rest of a
    // half read page would be lost to every later answer (D2.4).
    let per_page = limit.clamp(1, TREE_PAGE);
    let mut repositories: Vec<Value> = Vec::new();
    let mut more = false;
    loop {
        let search = listing
            .query
            .as_deref()
            .map(|q| format!("&search={}", encode_segment(q)))
            .unwrap_or_default();
        let url = format!(
            "{base}/projects?membership=true&order_by=updated_at&per_page={per_page}&page={page}{search}"
        );
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
        let rows: Vec<Value> = entries.iter().map(repository_row).collect();
        if !repositories.is_empty() && repositories.len() + rows.len() > limit {
            more = true;
            break;
        }
        repositories.extend(rows);
        page += 1;
        if !full_page {
            // the instance had nothing more to give
            break;
        }
        if repositories.len() >= limit {
            more = true;
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

fn repository_row(entry: &Value) -> Value {
    json!({
        "full_name": entry.get("path_with_namespace").and_then(|v| v.as_str()),
        "name": entry.get("path").and_then(|v| v.as_str()),
        "private": entry.get("visibility").and_then(|v| v.as_str()) != Some("public"),
        "clone_url": entry.get("http_url_to_repo").and_then(|v| v.as_str()),
        "ssh_url": entry.get("ssh_url_to_repo").and_then(|v| v.as_str()),
        "default_branch": entry.get("default_branch").and_then(|v| v.as_str()),
        "web_url": entry.get("web_url").and_then(|v| v.as_str()),
    })
}

// -- creating a repository (D2.4, decision 17) ---------------------------------

/// The CREATE-REPOSITORY answer.
///
/// This is where D2.7a's resolution shows: `write_repository` "Uses
/// Git-over-HTTP. Does not support API authentication.", so a read
/// write member is answered locally with `scope_missing` naming `api`
/// instead of spending a request that GitLab would refuse.
pub fn create_repository_answer(target: &Target, new: &NewRepository, ctx: &Ctx) -> Value {
    let Some(host) = target.host() else {
        return unknown_state();
    };
    if ctx.token("gitlab", &host).is_none() {
        return json!({ "state": "needs_sign_in", "host": host });
    }
    // The local pre check of D2.7c, cheapest first: since J3 the set
    // the forge granted is stored beside the token, so the two requests
    // `granted_scopes` costs are spent only for a credential joy did
    // not write itself.
    if let Some(refused) =
        joy_forge_net::auth::verbs::stored_create_gate(ctx, "gitlab", &host, new.private)
    {
        return refused;
    }
    if ctx.granted_scopes("gitlab", &host).is_none() {
        if let Some(granted) = granted_scopes(ctx, &host) {
            let missing = scope::missing("gitlab", Group::CreateRepository, &granted);
            if !missing.is_empty() {
                return scope::scope_missing(&host, "create-repository", &missing, &granted);
            }
        }
    }
    let base = api_base(&host, ctx);
    let Ok(http) = ctx.http(&host) else {
        return unknown_state();
    };
    let Some(token) = ctx.token("gitlab", &host) else {
        return json!({ "state": "needs_sign_in", "host": host });
    };
    let mut body = json!({
        "name": new.name,
        "path": new.name,
        "visibility": if new.private { "private" } else { "public" },
    });
    if let Some(owner) = new.owner.as_deref() {
        // A namespace is named by id; the group's own record carries it.
        if let Some(id) = namespace_id(ctx, &host, owner) {
            body["namespace_id"] = json!(id);
        } else {
            return json!({
                "state": "denied",
                "host": host,
                "message": format!("{host} shows no group '{owner}' this account may create in"),
            });
        }
    }
    let answer = match http
        .post(&format!("{base}/projects"))
        .header("Accept", "application/json")
        .bearer(&token)
        .send_json(&body)
    {
        Ok(answer) => answer,
        Err(error) => {
            eprintln!("joy-forge gitlab: {error}");
            return unknown_state();
        }
    };
    if !answer.ok() {
        let state = classify(&answer);
        if state == "scope_missing" {
            let have = granted_scopes(ctx, &host).unwrap_or_default();
            let needed = scope::missing("gitlab", Group::CreateRepository, &have);
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
        "clone_url": created.get("http_url_to_repo").and_then(|v| v.as_str()),
        "ssh_url": created.get("ssh_url_to_repo").and_then(|v| v.as_str()),
        "default_branch": created.get("default_branch").and_then(|v| v.as_str()),
        "web_url": created.get("web_url").and_then(|v| v.as_str()),
    })
}

fn namespace_id(ctx: &Ctx, host: &str, owner: &str) -> Option<i64> {
    let url = format!(
        "{}/namespaces/{}",
        api_base(host, ctx),
        encode_segment(owner)
    );
    let answer = api_get(ctx, host, &url)?;
    answer
        .ok()
        .then(|| answer.json())
        .flatten()
        .and_then(|body| body.get("id").and_then(|v| v.as_i64()))
}

/// GitLab's own error sentence, where it sent one.
fn message_of(answer: &Answer) -> String {
    answer
        .json()
        .and_then(|body| {
            body.get("message")
                .or_else(|| body.get("error"))
                .map(|m| m.to_string())
        })
        .unwrap_or_else(|| format!("GitLab answered {}", answer.status))
}

// -- the sign in half (D2.4, D2.7, package J3) --------------------------------

/// The three scope sets of D2.7a. v2's single set was wrong in both
/// directions and this is the replacement:
///
/// | Set | Scopes | Covers |
/// | --- | --- | --- |
/// | read only member | `read_api read_repository` | A, B, C, E |
/// | read write member | `read_api write_repository` | A to E |
/// | full | `api write_repository` | A to G |
///
/// `write_repository` "Uses Git-over-HTTP. Does not support API
/// authentication.", so `create-repository` (POST /projects) and the
/// Releases API need `api`. The registered application carries the
/// union `api write_repository`, because since the fix for issue 543138
/// a device request may narrow but never widen.
pub fn scopes_for(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Read => "read_api read_repository",
        Purpose::Write => "read_api write_repository",
        Purpose::Create | Purpose::Release => "api write_repository",
    }
}

/// The OAuth application for a host (D2.7). GitLab has the device grant
/// from 17.3; gitlab.com's OIDC discovery does not advertise the device
/// endpoint, so the path is written down rather than discovered.
pub fn oauth_for(host: &str, purpose: Purpose, ctx: &Ctx) -> Option<OAuth> {
    let instance = ctx.instance(host);
    let client_id = instance
        .and_then(|entry| entry.client_id.clone())
        .or_else(|| {
            (host == "gitlab.com")
                .then(|| joy_forge_net::auth::oauth::clients::GITLAB_COM.to_string())
        })?;
    // The API base may sit under a relative URL root, and the OAuth
    // endpoints sit beside it and not under `/api/v4`.
    let base = instance_root(host, ctx);
    Some(OAuth {
        client_id,
        flow: Flow::Device,
        device_endpoint: instance
            .and_then(|entry| entry.device_endpoint.clone())
            .unwrap_or_else(|| format!("{base}/oauth/authorize_device")),
        auth_endpoint: instance
            .and_then(|entry| entry.auth_endpoint.clone())
            .unwrap_or_else(|| format!("{base}/oauth/authorize")),
        token_endpoint: instance
            .and_then(|entry| entry.token_endpoint.clone())
            .unwrap_or_else(|| format!("{base}/oauth/token")),
        scopes: instance
            .and_then(|entry| entry.scopes.clone())
            .unwrap_or_else(|| scopes_for(purpose).to_string()),
    })
}

/// The instance root the OAuth endpoints hang off: the configured API
/// base without its `/api/v4` tail (a relative URL install keeps its sub
/// path that way), else the host itself.
fn instance_root(host: &str, ctx: &Ctx) -> String {
    let base = api_base(host, ctx);
    base.strip_suffix("/api/v4")
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://{host}"))
}

/// One API GET with a NAMED token, for the calls that validate a token
/// the context does not hold yet.
fn api_get_as(ctx: &Ctx, host: &str, url: &str, token: &str) -> Option<Answer> {
    let http = ctx.http(host).ok()?;
    match http
        .get(url)
        .header("Accept", "application/json")
        .bearer(token)
        .call()
    {
        Ok(answer) => Some(answer),
        Err(error) => {
            eprintln!("joy-forge gitlab: {error}");
            None
        }
    }
}

/// Who this token speaks for, asked of the instance's own API.
pub fn account_of(host: &str, token: &str, ctx: &Ctx) -> Option<Account> {
    let base = api_base(host, ctx);
    let answer = api_get_as(ctx, host, &format!("{base}/user"), token)?;
    if !answer.ok() {
        return None;
    }
    let body = answer.json()?;
    let login = body.get("username").and_then(|v| v.as_str())?.to_string();
    let mut emails: Vec<String> = Vec::new();
    if let Some(list) = api_get_as(ctx, host, &format!("{base}/user/emails"), token) {
        if list.ok() {
            #[derive(serde::Deserialize)]
            struct Entry {
                email: String,
            }
            if let Ok(entries) = serde_json::from_str::<Vec<Entry>>(&list.body) {
                emails = entries.into_iter().map(|entry| entry.email).collect();
            }
        }
    }
    Some(Account {
        login,
        user_id: body
            .get("id")
            .and_then(|v| v.as_i64())
            .map(|id| id.to_string()),
        emails,
        scopes: scopes_of(ctx, host, token).map(|scopes| scopes.join(" ")),
    })
}

/// The granted set of a NAMED token: a personal access token's own
/// record first, then the OAuth token's info. `None` means "not known",
/// and an unknown set is never reported as a missing one (D2.7c).
fn scopes_of(ctx: &Ctx, host: &str, token: &str) -> Option<Vec<String>> {
    let base = api_base(host, ctx);
    if let Some(answer) = api_get_as(
        ctx,
        host,
        &format!("{base}/personal_access_tokens/self"),
        token,
    ) {
        if answer.ok() {
            if let Some(scopes) = string_list(&answer, "scopes") {
                return Some(scopes);
            }
        }
    }
    let answer = api_get_as(
        ctx,
        host,
        &format!("{}/oauth/token/info", instance_root(host, ctx)),
        token,
    )?;
    answer.ok().then(|| string_list(&answer, "scope")).flatten()
}

/// Whether this token reaches `owner/repo`, and whether it may push
/// (the probe of D4.1c).
pub fn reaches_repo(host: &str, repo_path: &str, token: &str, ctx: &Ctx) -> Option<Reach> {
    let url = format!(
        "{}/projects/{}",
        api_base(host, ctx),
        encode_segment(repo_path)
    );
    let answer = api_get_as(ctx, host, &url, token)?;
    if !answer.ok() {
        // GitLab answers 404, not 403, for a private project the caller
        // may not see: both mean "this login is not the one".
        return Some(Reach::default());
    }
    let body = answer.json().unwrap_or_default();
    let level = [
        "/permissions/project_access/access_level",
        "/permissions/group_access/access_level",
    ]
    .iter()
    .filter_map(|pointer| body.pointer(pointer).and_then(|v| v.as_i64()))
    .max()
    .unwrap_or(0);
    Some(Reach {
        read: true,
        push: level >= DEVELOPER,
    })
}

/// Revoke a token at GitLab: the OAuth revocation endpoint, which a
/// public client may call with its client id alone (RFC 7009).
pub fn revoke_token(host: &str, record: &joy_forge_net::auth::store::Record, ctx: &Ctx) -> bool {
    let Some(client_id) = record.client_id.as_deref() else {
        return false;
    };
    let Ok(http) = ctx.http(host) else {
        return false;
    };
    let body = joy_forge_net::auth::oauth::form(&[
        ("client_id", client_id),
        ("token", &record.token),
        ("token_type_hint", "access_token"),
    ]);
    match http
        .post(&format!("{}/oauth/revoke", instance_root(host, ctx)))
        .header("Accept", "application/json")
        .send_bytes("application/x-www-form-urlencoded", body.into_bytes())
    {
        Ok(answer) => answer.ok(),
        Err(error) => {
            eprintln!("joy-forge gitlab: {error}");
            false
        }
    }
}

/// The login glab is signed in as on this host. glab holds ONE token
/// per host block, so D4.1c's order collapses to step 3 here, and the
/// list has at most one entry.
pub fn glab_logins(host: &str) -> Vec<String> {
    glab_login(host).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitlab_hosts_are_claimed_and_others_are_not() {
        assert!(claims_host("gitlab.com", &[]));
        assert!(!claims_host("github.com", &[]));
        assert!(!claims_host("gitlab.example.com", &[]));
        assert!(!claims_host("gitlab.com.evil.example", &[]));
    }

    /// A self-hosted GitLab lives on the customer's own domain, so a
    /// host is claimed when glab is signed in to it. No instance
    /// belongs in this code.
    #[test]
    fn a_self_hosted_host_is_claimed_once_glab_knows_it() {
        let configured = vec!["gitlab.acme.test".to_string()];
        assert!(claims_host("gitlab.acme.test", &configured));
        assert!(claims_host("gitlab.com", &configured));
        assert!(!claims_host("gitlab.acme.test.evil.example", &configured));
        assert!(!claims_host("gitlab.acme.test", &[]));
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
    fn the_self_hosted_alias_form_parses_too() {
        let a = parse_alias("42-alice@users.noreply.gitlab.acme.test").unwrap();
        assert_eq!(a.login, "alice");
        assert_eq!(a.user_id.as_deref(), Some("42"));
    }

    #[test]
    fn config_yml_yields_the_login_of_gitlab_com_or_the_configured_instance() {
        let text = "hosts:\n    gitlab.com:\n        token: x\n        user: horst\n";
        assert_eq!(parse_config_yml(text).as_deref(), Some("horst"));
        assert_eq!(
            parse_config_yml("hosts:\n    gitlab.acme.test:\n        user: alice\n").as_deref(),
            Some("alice")
        );
        assert_eq!(parse_config_yml(""), None);
    }

    /// The first hardcoded base of D2.8: a self hosted instance must be
    /// asked about its own people, not gitlab.com.
    #[test]
    fn a_self_hosted_instance_is_asked_its_own_api_v4() {
        let ctx = Ctx::bare(std::env::temp_dir());
        assert_eq!(api_base("gitlab.com", &ctx), "https://gitlab.com/api/v4");
        assert_eq!(
            api_base("gitlab.acme.test", &ctx),
            "https://gitlab.acme.test/api/v4"
        );
        let configured = Ctx::bare(std::env::temp_dir()).with_instances(
            joy_forge_net::config::Instances::from_text(
                "- host: gitlab.acme.test\n  kind: gitlab\n  api_base: https://gitlab.acme.test/gl/api/v4\n",
            )
            .unwrap(),
        );
        assert_eq!(
            api_base("gitlab.acme.test", &configured),
            "https://gitlab.acme.test/gl/api/v4"
        );
    }

    #[test]
    fn a_refusal_is_classified_by_its_header_and_never_by_prose() {
        let insufficient = Answer::new(
            403,
            "{}",
            vec![(
                "www-authenticate".into(),
                r#"Bearer realm="GitLab", error="insufficient_scope""#.into(),
            )],
        );
        assert_eq!(classify(&insufficient), "scope_missing");
        assert_eq!(classify(&Answer::new(403, "{}", Vec::new())), "denied");
        assert_eq!(
            classify(&Answer::new(401, "{}", Vec::new())),
            "needs_sign_in"
        );
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

    fn answer(status: u16, body: &str) -> Option<Answer> {
        Some(Answer::new(status, body, Vec::new()))
    }

    fn project(level: i64, branch: Option<&str>) -> Option<Answer> {
        let body = json!({
            "default_branch": branch,
            "permissions": {"project_access": {"access_level": level}, "group_access": null}
        });
        answer(200, &body.to_string())
    }

    #[test]
    fn a_readable_project_yaml_is_the_store_and_carries_the_size_in_bytes() {
        let verdict = store_verdict(
            answer(200, "name: Demo\n"),
            || answer(200, r#"{"statistics": {"repository_size": 4096}}"#),
            || panic!("no third request when the store is there"),
            || panic!("a store that reads is never a question of scopes"),
        );
        assert_eq!(verdict["state"], "store");
        assert_eq!(verdict["project_yaml"], "name: Demo\n");
        // GitLab counts bytes already
        assert_eq!(verdict["size_bytes"], 4096);
    }

    #[test]
    fn a_store_stays_a_store_when_the_project_call_is_refused() {
        // a Reporter role is needed for statistics; without it the
        // field is absent and that is not an error (D2.4)
        let verdict = store_verdict(
            answer(200, "name: Demo\n"),
            || answer(200, "{}"),
            || None,
            || false,
        );
        assert_eq!(verdict["state"], "store");
        assert_eq!(verdict["size_bytes"], Value::Null);
        let no_project = store_verdict(answer(200, "name: Demo\n"), || None, || None, || false);
        assert_eq!(
            no_project,
            json!({ "state": "store", "project_yaml": "name: Demo\n" })
        );
    }

    #[test]
    fn a_404_asks_the_project_whether_it_is_gone_or_only_storeless() {
        // D2.7c: the same pair is "gone" only for a caller whose set
        // could have seen a private project, and "not known" for
        // everybody else, an anonymous caller included.
        assert_eq!(
            store_verdict(answer(404, "{}"), || answer(404, "{}"), || None, || true),
            json!({ "state": "gone" })
        );
        assert_eq!(
            store_verdict(answer(404, "{}"), || answer(404, "{}"), || None, || false),
            json!({ "state": "unknown" })
        );
        let missing = store_verdict(
            answer(404, "{}"),
            || project(40, Some("main")),
            || panic!("a maintainer needs no protection list"),
            || true,
        );
        assert_eq!(missing["state"], "missing");
        assert_eq!(missing["may_create"], true);
        assert_eq!(missing["default_branch"], "main");
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
        assert!(!may_push(
            &serde_json::from_str(&project(30, Some("trunk")).unwrap().body).unwrap(),
            || answer(403, "")
        ));
        assert!(may_push(
            &serde_json::from_str(&project(30, None).unwrap().body).unwrap(),
            || None
        ));
        assert!(!may_push(
            &serde_json::from_str(&project(20, Some("trunk")).unwrap().body).unwrap(),
            || answer(200, "[]")
        ));
    }

    #[test]
    fn anything_unclear_stays_unanswered() {
        let unknown = json!({ "state": "unknown" });
        assert_eq!(store_verdict(None, || None, || None, || true), unknown);
        assert_eq!(
            store_verdict(answer(401, ""), || None, || None, || true),
            unknown
        );
        assert_eq!(
            store_verdict(answer(404, ""), || answer(500, ""), || None, || true),
            unknown
        );
    }

    #[test]
    fn nested_groups_keep_their_path_and_a_rule_may_be_a_pattern() {
        assert_eq!(
            joy_forge_net::url::repo_path_of("https://gitlab.com/grp/sub/repo.git").as_deref(),
            Some("grp/sub/repo")
        );
        assert!(rule_matches("release/*", "release/1.2"));
        assert!(!rule_matches("release/*", "releases/1"));
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;

    fn page(count: usize, name: &str) -> Option<Answer> {
        let entries: Vec<Value> = (0..count)
            .map(|i| json!({ "path": format!("{name}{i}.md"), "type": "blob" }))
            .collect();
        Some(Answer::new(
            200,
            Value::Array(entries).to_string(),
            Vec::new(),
        ))
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
        assert_eq!(verdict["truncated"], json!(false));
        assert_eq!(verdict["paths"].as_array().unwrap().len(), TREE_PAGE + 2);
        let endless = files_verdict(|_| page(TREE_PAGE, "x"));
        assert_eq!(endless["truncated"], json!(true));
    }

    #[test]
    fn trees_are_skipped_an_empty_project_lists_nothing() {
        let body =
            r#"[{"path": "docs", "type": "tree"}, {"path": "docs/VISION.md", "type": "blob"}]"#;
        assert_eq!(
            files_verdict(|_| Some(Answer::new(200, body, Vec::new()))),
            json!({ "state": "files", "paths": ["docs/VISION.md"], "truncated": false })
        );
        assert_eq!(
            files_verdict(|_| Some(Answer::new(404, "", Vec::new()))),
            json!({ "state": "files", "paths": [], "truncated": false })
        );
        assert_eq!(files_verdict(|_| None), json!({ "state": "unknown" }));
    }
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx::bare(std::env::temp_dir())
    }

    /// D2.7a's three sets, and the one contradiction they resolve:
    /// `write_repository` "Does not support API authentication", so
    /// creating a repository and publishing a release need `api`.
    #[test]
    fn the_three_scope_sets_are_the_ones_the_design_tabulates() {
        assert_eq!(scopes_for(Purpose::Read), "read_api read_repository");
        assert_eq!(scopes_for(Purpose::Write), "read_api write_repository");
        assert_eq!(scopes_for(Purpose::Create), "api write_repository");
        assert_eq!(scopes_for(Purpose::Release), "api write_repository");
    }

    /// D2.7: GitLab has the device grant from 17.3, and gitlab.com's
    /// OIDC discovery does not advertise the endpoint, so the path is
    /// written down.
    #[test]
    fn the_device_endpoint_is_the_instances_own_oauth_path() {
        let ctx = ctx();
        let public = oauth_for("gitlab.com", Purpose::Write, &ctx).unwrap();
        assert_eq!(public.flow, Flow::Device);
        assert_eq!(
            public.device_endpoint,
            "https://gitlab.com/oauth/authorize_device"
        );
        assert_eq!(public.token_endpoint, "https://gitlab.com/oauth/token");
        assert_eq!(public.scopes, "read_api write_repository");
        assert!(
            !joy_forge_net::auth::oauth::clients::is_placeholder(&public.client_id),
            "gitlab.com carries the registered application id"
        );
        assert!(oauth_for("gitlab.acme.test", Purpose::Write, &ctx).is_none());
    }

    /// A relative URL install keeps its sub path: the OAuth endpoints
    /// sit beside `/api/v4` and not under it.
    #[test]
    fn a_relative_url_install_keeps_its_sub_path_on_the_oauth_endpoints() {
        let ctx = ctx().with_instances(
            joy_forge_net::config::Instances::from_text(
                "- host: git.acme.test\n  kind: gitlab\n  client_id: cid\n  api_base: https://git.acme.test/gitlab/api/v4\n",
            )
            .unwrap(),
        );
        let oauth = oauth_for("git.acme.test", Purpose::Create, &ctx).unwrap();
        assert_eq!(
            oauth.device_endpoint,
            "https://git.acme.test/gitlab/oauth/authorize_device"
        );
        assert_eq!(
            oauth.token_endpoint,
            "https://git.acme.test/gitlab/oauth/token"
        );
        assert_eq!(oauth.scopes, "api write_repository");
    }
}
