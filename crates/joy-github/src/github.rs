// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The GitHub knowledge: host matching, alias address forms, gh's
//! config, the REST API. Everything a read query cannot answer degrades
//! to "unknown"; the one write verb (`release`) reports instead.
//!
//! Since JOY-0298-E4 (design D2.8) every API call is made in process
//! over the connector's own HTTP client. curl and gh are gone from the
//! API path: gh is still asked for a TOKEN (decision 19), which is a
//! different thing and the only device side credential source this wave
//! has.

use joy_forge_net::auth::oauth::{Flow, OAuth};
use joy_forge_net::auth::Purpose;
use joy_forge_net::forge::{
    unknown, unknown_state, Account, Ctx, Listing, NewRepository, Reach, ReleaseRequest, Target,
};
use joy_forge_net::http::Answer;
use joy_forge_net::scope::{self, Group};
use serde_json::{json, Value};

/// Where the store lives inside a repository.
const PROJECT_YAML: &str = ".joy/project.yaml";

/// The API version header GitHub asks every caller to send.
const ACCEPT_JSON: &str = "application/vnd.github+json";
const ACCEPT_RAW: &str = "application/vnd.github.raw+json";

/// Does this remote URL belong to GitHub? Handles the three wire forms;
/// subdomains of github.com count (GitHub Enterprise Cloud), lookalike
/// hosts (`github.com.evil`) do not.
pub fn claims_host(host: &str, configured: &[String]) -> bool {
    host == "github.com"
        || host.ends_with(".github.com")
        || configured.iter().any(|known| known == host)
}

/// Every host gh is signed in to, lowercased. That is how a GitHub
/// Enterprise Server on any domain becomes reachable without putting
/// somebody's instance into this code; `forges.yaml` is the other way
/// (D2.5), and the dispatcher consults it.
pub fn configured_hosts() -> Vec<String> {
    gh_hosts().into_iter().map(|(host, _)| host).collect()
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

/// gh's `hosts.yml`, from the first of the per operating system
/// locations of D2.4 that exists.
pub fn gh_hosts() -> Vec<(String, String)> {
    match joy_forge_net::foreign::first_readable(&joy_forge_net::foreign::gh_config_files()) {
        Some((_, text)) => parse_hosts(&text),
        None => Vec::new(),
    }
}

/// The signed-in login from gh's config, offline: the host's own block
/// when there is one, else github.com's, else whichever instance is
/// configured (an Enterprise-only setup has no github.com block).
pub fn gh_login(host: &str) -> Option<String> {
    let hosts = gh_hosts();
    hosts
        .iter()
        .find(|(known, _)| known == host)
        .or_else(|| hosts.iter().find(|(known, _)| known == "github.com"))
        .or_else(|| hosts.first())
        .map(|(_, user)| user.clone())
}

/// The `user:` under a host block of gh's hosts.yml. Minimal line parse
/// on purpose: the file is tiny, and a YAML dependency for two lines
/// would be the heavier contract.
pub fn parse_hosts_yml(text: &str) -> Option<String> {
    let hosts = parse_hosts(text);
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

/// The API root of the GitHub a host runs: what an operator configured
/// (D2.5), else github.com's own API host, else a GitHub Enterprise
/// Server's `/api/v3` ON ITS OWN DOMAIN.
///
/// The second bug D2.8 names lived here: every API call went to
/// api.github.com, so a GHES instance was asked about a person it had
/// never heard of.
pub fn api_base(host: &str, ctx: &Ctx) -> String {
    if let Some(base) = ctx.instance(host).and_then(|i| i.api_base.clone()) {
        return base;
    }
    if host == "github.com" || host.ends_with(".github.com") {
        return "https://api.github.com".to_string();
    }
    format!("https://{host}/api/v3")
}

/// One API GET. The token travels in a header, never in an argument.
fn api_get(ctx: &Ctx, host: &str, url: &str, accept: &str) -> Option<Answer> {
    let http = ctx.http(host).ok()?;
    let mut request = http.get(url).header("Accept", accept);
    if let Some(token) = ctx.token("github", host) {
        request = request.bearer(&token);
    }
    match request.call() {
        Ok(answer) => Some(answer),
        Err(error) => {
            eprintln!("joy-forge github: {error}");
            None
        }
    }
}

/// The granted scope set of the token in use, from the header GitHub
/// puts on every authenticated answer. `None` means "not known": a fine
/// grained token and `GITHUB_TOKEN` carry no such header, and an
/// unknown set must never be reported as a missing one (D2.7c).
pub fn granted_scopes(answer: &Answer) -> Option<Vec<String>> {
    answer.header("x-oauth-scopes").map(scope::parse_granted)
}

/// What a refusal means (D2.7c). Never `denied` for a scope problem.
pub fn classify(answer: &Answer) -> &'static str {
    match answer.status {
        403 => {
            if let (Some(accepted), Some(have)) = (
                answer.header("x-accepted-oauth-scopes"),
                answer.header("x-oauth-scopes"),
            ) {
                let have = scope::parse_granted(have);
                let accepted = scope::parse_granted(accepted);
                if !accepted.is_empty() && !accepted.iter().any(|want| have.contains(want)) {
                    return "scope_missing";
                }
            }
            if answer.header("x-github-sso").is_some() {
                return "needs_sso";
            }
            if answer.body.contains("OAuth App access restrictions") {
                return "needs_org_approval";
            }
            "denied"
        }
        401 => "needs_sign_in",
        429 => "rate_limited",
        _ => "denied",
    }
}

/// The account's verified addresses, best effort. With a token the
/// instance's OWN API is asked; without one there is nothing to ask,
/// and nothing is asked: an anonymous request cannot name an account,
/// and joy never spends a contact it knows the answer to (decision 20).
fn verified_emails(ctx: &Ctx, host: &str) -> Vec<String> {
    if ctx.token("github", host).is_none() {
        return Vec::new();
    }
    let Some(answer) = api_get(
        ctx,
        host,
        &format!("{}/user/emails", api_base(host, ctx)),
        ACCEPT_JSON,
    ) else {
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

/// The authenticated account (`GET /user`): the login and the public
/// profile address, where the person set one. Asked only where a
/// credential exists.
fn current_user(ctx: &Ctx, host: &str) -> Option<Value> {
    current_account(ctx, host).and_then(|(user, _)| user)
}

/// The same `GET /user`, with the granted scope set that rides on its
/// `X-OAuth-Scopes` header. One request carries both facts, and the
/// budget of D1.9 counts requests, so the two are never asked apart.
fn current_account(ctx: &Ctx, host: &str) -> Option<(Option<Value>, Option<Vec<String>>)> {
    // Nothing is asked without a credential: the endpoint is about the
    // account the token names (decision 20).
    ctx.token("github", host)?;
    let answer = api_get(
        ctx,
        host,
        &format!("{}/user", api_base(host, ctx)),
        ACCEPT_JSON,
    )?;
    if !answer.ok() {
        return None;
    }
    let scopes = granted_scopes(&answer);
    Some((answer.json(), scopes))
}

/// The ACTOR answer (docs/plugins.md `identity`): who acts on GitHub.
/// Handed-in caller facts (a multi-account host's session) win over
/// local discovery (gh's config). `known: false` when nobody is known.
pub fn identity_answer(target: &Target, ctx: &Ctx) -> Value {
    let host = target.host().unwrap_or_else(|| "github.com".to_string());
    let handed_in = ctx.login.is_some() || ctx.user_id.is_some();
    let Some(login) = ctx.login.clone().or_else(|| gh_login(&host)) else {
        return unknown();
    };
    // Addresses come from the account the credentials speak for.
    let mut emails = verified_emails(ctx, &host);
    if !handed_in && ctx.token_env.is_none() {
        if let Some(public) = current_user(ctx, &host)
            .and_then(|user| {
                user.get("email")
                    .and_then(|e| e.as_str())
                    .map(str::to_string)
            })
            .filter(|email| !email.trim().is_empty())
        {
            if !emails.contains(&public) {
                emails.push(public);
            }
        }
    }
    json!({
        "known": true,
        "login": login,
        "user_id": ctx.user_id,
        "emails": emails,
    })
}

/// The PURE address attribution (docs/plugins.md `resolve`): derived
/// from the address alone. Never consults ambient state, by contract.
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

/// The STORE answer for a remote. Without a token GitHub is asked
/// anonymously, which only sees public repositories.
pub fn store_answer(target: &Target, ctx: &Ctx) -> Value {
    let (Some(host), Some(path)) = (target.host(), target.repo_path()) else {
        return unknown_state();
    };
    let repo_url = format!("{}/repos/{path}", api_base(&host, ctx));
    let file = api_get(
        ctx,
        &host,
        &format!("{repo_url}/contents/{PROJECT_YAML}"),
        ACCEPT_RAW,
    );
    let authenticated = ctx.token("github", &host).is_some();
    store_verdict(file, authenticated, || {
        api_get(ctx, &host, &repo_url, ACCEPT_JSON)
    })
}

/// The decision over the two answers, pure. Anything but a clear 2xx or
/// 404 leaves the question unanswered.
///
/// The 404 rule is the one D2.7c corrects: GitHub answers 404 rather
/// than 403 for a private repository the caller may not see, so a 404
/// on a request that carried no token, or a token without `repo`, is
/// NOT the verdict "your repository is gone".
fn store_verdict(
    file: Option<Answer>,
    authenticated: bool,
    repo: impl FnOnce() -> Option<Answer>,
) -> Value {
    let Some(file) = file else {
        return unknown_state();
    };
    let store_body = match file.status {
        200..=299 => Some(file.body.clone()),
        404 => None,
        _ => return unknown_state(),
    };
    let Some(repo) = repo() else {
        // The repository call is what carries the size; without it a
        // readable store is still a store.
        return match store_body {
            Some(body) => json!({ "state": "store", "project_yaml": body }),
            None => unknown_state(),
        };
    };
    match repo.status {
        200..=299 => {}
        404 => {
            return if may_see_private(&repo, authenticated) {
                json!({ "state": "gone" })
            } else {
                // No token, or a token without `repo`: GitHub hides a
                // private repository behind the same 404, so this is
                // not a verdict about the repository at all.
                unknown_state()
            };
        }
        _ => return unknown_state(),
    }
    let Ok(body) = serde_json::from_str::<Value>(&repo.body) else {
        return match store_body {
            Some(body) => json!({ "state": "store", "project_yaml": body }),
            None => unknown_state(),
        };
    };
    // GitHub's repository `size` is KILOBYTES, and "Size is calculated
    // hourly. When a repository is initially created, the size is 0."
    // The unit is forge knowledge, so it is normalised here (D2.4).
    let size_bytes = body
        .get("size")
        .and_then(|v| v.as_u64())
        .map(|kilobytes| kilobytes * 1024);
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

/// Whether a 404 is a verdict: it is only when the request carried a
/// credential that could have seen a private repository.
fn may_see_private(answer: &Answer, authenticated: bool) -> bool {
    if !authenticated {
        return false;
    }
    match granted_scopes(answer) {
        // A classic token says what it may do.
        Some(scopes) => scope::missing("github", Group::RepositoryFacts, &scopes).is_empty(),
        // A fine grained token says nothing; it is still the person's
        // own credential, so its 404 is taken at face value.
        None => true,
    }
}

// -- the files query (JAPP-0293-A7) --------------------------------------------

/// The FILES answer for a remote.
pub fn files_answer(target: &Target, ctx: &Ctx) -> Value {
    let (Some(host), Some(path)) = (target.host(), target.repo_path()) else {
        return unknown_state();
    };
    let url = format!(
        "{}/repos/{path}/git/trees/HEAD?recursive=1",
        api_base(&host, ctx)
    );
    files_verdict(api_get(ctx, &host, &url, ACCEPT_JSON))
}

/// The file paths in a tree answer, pure. An empty repository has no
/// tree to list (GitHub answers 409 or 404 for it): no files.
fn files_verdict(answer: Option<Answer>) -> Value {
    let Some(answer) = answer else {
        return unknown_state();
    };
    match answer.status {
        200..=299 => {}
        404 | 409 => return json!({ "state": "files", "paths": [], "truncated": false }),
        _ => return unknown_state(),
    }
    let Ok(body) = serde_json::from_str::<Value>(&answer.body) else {
        return unknown_state();
    };
    let paths: Vec<&str> = body
        .get("tree")
        .and_then(|t| t.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("blob"))
                .filter_map(|e| e.get("path").and_then(|p| p.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let truncated = body
        .get("truncated")
        .and_then(|t| t.as_bool())
        .unwrap_or(false);
    json!({ "state": "files", "paths": paths, "truncated": truncated })
}

// -- the repository list (D2.4) ------------------------------------------------

/// Entries per page: GitHub's maximum, so a 200 repository answer costs
/// two requests and not eight.
const PAGE: usize = 100;

/// The REPOSITORIES answer: what this account can reach, paginated.
pub fn repositories_answer(target: &Target, listing: &Listing, ctx: &Ctx) -> Value {
    let Some(host) = target.host() else {
        return unknown_state();
    };
    if ctx.token("github", &host).is_none() {
        // `GET /user/repos` is the account's own list; anonymously
        // there is no account and therefore no answer.
        return json!({ "state": "needs_sign_in", "host": host });
    }
    let base = api_base(&host, ctx);
    let mut page: usize = listing
        .page
        .as_deref()
        .and_then(|p| p.parse().ok())
        .unwrap_or(1);
    let limit = listing.limit.max(1);
    // A cursor is a page number, so half a page has no number: the page
    // size is bounded by the caller's limit as well as by the forge's
    // maximum, and a page that would carry the answer past the limit is
    // left unread for `next` to point at. Cutting one in half here
    // would drop the rest of it out of every later answer too.
    let per_page = limit.clamp(1, PAGE);
    let mut repositories: Vec<Value> = Vec::new();
    let mut more = false;
    loop {
        let url = format!("{base}/user/repos?per_page={per_page}&page={page}&sort=updated");
        let Some(answer) = api_get(ctx, &host, &url, ACCEPT_JSON) else {
            return unknown_state();
        };
        if !answer.ok() {
            return json!({ "state": classify(&answer), "host": host });
        }
        let Ok(entries) = serde_json::from_str::<Vec<Value>>(&answer.body) else {
            return unknown_state();
        };
        let full_page = entries.len() >= per_page;
        let rows: Vec<Value> = entries
            .iter()
            .filter_map(|entry| repository_row(entry, listing.query.as_deref()))
            .collect();
        if !repositories.is_empty() && repositories.len() + rows.len() > limit {
            more = true;
            break;
        }
        repositories.extend(rows);
        page += 1;
        if !full_page {
            // the forge had nothing more to give
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

/// One repository as the protocol carries it, filtered by `--query`.
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

/// The CREATE-REPOSITORY answer. A project can only be brought to
/// joyint.com when it has a remote repository, so this is what makes
/// "picks or creates a repo" complete.
pub fn create_repository_answer(target: &Target, new: &NewRepository, ctx: &Ctx) -> Value {
    let Some(host) = target.host() else {
        return unknown_state();
    };
    if ctx.token("github", &host).is_none() {
        return json!({ "state": "needs_sign_in", "host": host });
    }
    // The local pre check of D2.7c, cheapest first: since J3 the set
    // the forge granted is stored beside the token, so a token that
    // cannot create is refused before a single request is sent.
    if let Some(refused) =
        joy_forge_net::auth::verbs::stored_create_gate(ctx, "github", &host, new.private)
    {
        return refused;
    }
    let base = api_base(&host, ctx);
    // One `GET /user` carries both facts this verb needs: who the token
    // speaks for, and what it may do (the `X-OAuth-Scopes` header).
    let (user, granted) = current_account(ctx, &host).unwrap_or((None, None));
    // The same pre check for a credential joy did not store: the header
    // is where a classic token says what it may do.
    if let Some(scopes) = granted {
        let missing = scope::missing_for_create("github", new.private, &scopes);
        if !missing.is_empty() {
            return scope::scope_missing(&host, "create-repository", &missing, &scopes);
        }
    }
    let own_login = user
        .as_ref()
        .and_then(|u| u.get("login").and_then(|l| l.as_str()))
        .map(str::to_string);
    let url = match new.owner.as_deref() {
        Some(owner) if Some(owner) != own_login.as_deref() => {
            format!("{base}/orgs/{owner}/repos")
        }
        _ => format!("{base}/user/repos"),
    };
    let Ok(http) = ctx.http(&host) else {
        return unknown_state();
    };
    let Some(token) = ctx.token("github", &host) else {
        return json!({ "state": "needs_sign_in", "host": host });
    };
    let body = json!({ "name": new.name, "private": new.private, "auto_init": false });
    let answer = match http
        .post(&url)
        .header("Accept", ACCEPT_JSON)
        .bearer(&token)
        .send_json(&body)
    {
        Ok(answer) => answer,
        Err(error) => {
            eprintln!("joy-forge github: {error}");
            return unknown_state();
        }
    };
    if !answer.ok() {
        let state = classify(&answer);
        if state == "scope_missing" {
            let have = granted_scopes(&answer).unwrap_or_default();
            let needed = scope::missing_for_create("github", new.private, &have);
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

/// GitHub's own error sentence, where it sent one.
fn message_of(answer: &Answer) -> String {
    answer
        .json()
        .and_then(|body| {
            body.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("GitHub answered {}", answer.status))
}

// -- the release capability (JOY-0256-64, moved to REST by D2.8) ---------------

/// Create (or complete) the release for `tag` on GitHub, over REST.
///
/// Idempotent the way publish needs it: a release may already exist
/// when this runs, because a tag-triggered forge workflow made it or an
/// earlier publish pushed and then failed. That pre-made release
/// carries only the installer section (JOY-0248-AE: v0.20.0 shipped
/// without a changelog that way), so the notes still have to land: they
/// are prepended unless a prior run already did.
pub fn release_answer(
    target: &Target,
    request: &ReleaseRequest,
    ctx: &Ctx,
) -> anyhow::Result<Value> {
    use anyhow::{anyhow, bail};
    let host = target.host().ok_or_else(|| {
        anyhow!("the release verb needs the repository's remote (--remote) or its host (--host)")
    })?;
    let path = target.repo_path().ok_or_else(|| {
        anyhow!(
            "'{}' does not name a GitHub repository (owner/name)",
            target.url().unwrap_or_default()
        )
    })?;
    let token = ctx.token("github", &host).ok_or_else(|| {
        // Name the variables this host reads, so the sentence is the
        // fix and not a category.
        let variables = joy_forge_net::forge::token_variables("github", &host).join(" or ");
        anyhow!(
            "no GitHub credential for {host}\n  \
             = help: set {variables}, hand one over with --token-env, or run `gh auth login`"
        )
    })?;
    // The local pre check of D2.7c: a token whose stored set cannot
    // carry a release is told so, instead of the forge's refusal being
    // reported as a failed publish.
    if let Some(refused) = joy_forge_net::auth::verbs::stored_scope_gate(
        ctx,
        "github",
        &host,
        "release",
        Group::Release,
    ) {
        return Ok(refused);
    }
    let http = ctx.http(&host)?;
    let base = api_base(&host, ctx);
    let releases = format!("{base}/repos/{path}/releases");

    let existing = http
        .get(&format!("{releases}/tags/{}", request.tag))
        .header("Accept", ACCEPT_JSON)
        .bearer(&token)
        .call()?;
    if existing.ok() {
        let body = existing.json().unwrap_or_default();
        let url = body
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let id = body.get("id").and_then(|v| v.as_i64());
        let notes = body.get("body").and_then(|v| v.as_str()).unwrap_or("");
        if !notes.contains(request.notes.trim()) {
            let combined = format!("{}\n\n{}", request.notes.trim_end(), notes);
            let id =
                id.ok_or_else(|| anyhow!("GitHub named no release id for tag {}", request.tag))?;
            let patched = http
                .patch(&format!("{releases}/{id}"))
                .header("Accept", ACCEPT_JSON)
                .bearer(&token)
                .send_json(&json!({ "body": combined }))?;
            if !patched.ok() {
                bail!(
                    "the release for {} could not be updated: {} ({})",
                    request.tag,
                    message_of(&patched),
                    classify(&patched)
                );
            }
        }
        return Ok(json!({ "url": url }));
    }
    if existing.status != 404 {
        bail!(
            "the release for {} could not be read: {} ({})",
            request.tag,
            message_of(&existing),
            classify(&existing)
        );
    }

    let created = http
        .post(&releases)
        .header("Accept", ACCEPT_JSON)
        .bearer(&token)
        .send_json(&json!({
            "tag_name": request.tag,
            "name": request.title,
            "body": request.notes,
        }))?;
    if !created.ok() {
        bail!(
            "the release for {} could not be created: {} ({})",
            request.tag,
            message_of(&created),
            classify(&created)
        );
    }
    let body = created.json().unwrap_or_default();
    Ok(json!({
        "url": body.get("html_url").and_then(|v| v.as_str()).unwrap_or_default(),
    }))
}

// No verb carries an asset yet. D2.8 names the asset upload on
// uploads.github.com as part of the release move, and the upload host
// is per release (GitHub puts it in the release's own `upload_url`,
// which is what would make it work on an Enterprise Server too), but
// `release` takes notes and nothing else in the verb catalogue of
// D2.4, and joy's own publish never uploaded one either. The code for
// it lands with the argument that carries it.

// -- the sign in half (D2.4, D2.7, package J3) --------------------------------

/// The scope set GitHub asks for (D2.7a). **One set covers A to G**:
/// `repo user:email`. There is no read only private scope on GitHub, so
/// the minimal scope demand cannot be met with an OAuth App; it is met
/// later by a GitHub App or a fine grained token with Contents read,
/// and the design says so instead of promising it now.
pub const SCOPES: &str = "repo user:email";

/// The sentence a read only member on GitHub hears (D2.7c). It is put
/// on stderr because the connector has no screen: the host renders it.
pub const READ_IS_WRITE: &str =
    "GitHub grants read and write in one scope, so this sign in asks for both. \
     joy never pushes without an explicit action.";

/// The OAuth application for a host (D2.7).
///
/// github.com signs in through joy's own public client; GitHub
/// Enterprise Server has instance local endpoints AND an instance local
/// client id, and the app must be registered on the instance, which is
/// what `forges.yaml` carries (D2.5). A GHES host with no configured
/// client has no door, and `login` says so rather than sending a
/// request nobody can answer.
pub fn oauth_for(host: &str, purpose: Purpose, ctx: &Ctx) -> Option<OAuth> {
    let instance = ctx.instance(host);
    let client_id = instance
        .and_then(|entry| entry.client_id.clone())
        .or_else(|| {
            (host == "github.com")
                .then(|| joy_forge_net::auth::oauth::clients::GITHUB_COM.to_string())
        })?;
    if purpose == Purpose::Read {
        eprintln!("joy: {READ_IS_WRITE}");
    }
    Some(OAuth {
        client_id,
        flow: Flow::Device,
        device_endpoint: instance
            .and_then(|entry| entry.device_endpoint.clone())
            .unwrap_or_else(|| format!("https://{host}/login/device/code")),
        auth_endpoint: instance
            .and_then(|entry| entry.auth_endpoint.clone())
            .unwrap_or_else(|| format!("https://{host}/login/oauth/authorize")),
        token_endpoint: instance
            .and_then(|entry| entry.token_endpoint.clone())
            .unwrap_or_else(|| format!("https://{host}/login/oauth/access_token")),
        scopes: instance
            .and_then(|entry| entry.scopes.clone())
            .unwrap_or_else(|| SCOPES.to_string()),
    })
}

/// One API GET with a NAMED token, for the calls that validate a token
/// the context does not hold yet (`token-store`, the probe of D4.1c).
fn api_get_as(ctx: &Ctx, host: &str, url: &str, token: &str) -> Option<Answer> {
    let http = ctx.http(host).ok()?;
    match http
        .get(url)
        .header("Accept", ACCEPT_JSON)
        .bearer(token)
        .call()
    {
        Ok(answer) => Some(answer),
        Err(error) => {
            eprintln!("joy-forge github: {error}");
            None
        }
    }
}

/// Who this token speaks for, asked of the instance's own API. This is
/// the `identity` validation `token-store` runs before it stores
/// anything, and what a finished `login` reports.
pub fn account_of(host: &str, token: &str, ctx: &Ctx) -> Option<Account> {
    let base = api_base(host, ctx);
    let answer = api_get_as(ctx, host, &format!("{base}/user"), token)?;
    if !answer.ok() {
        return None;
    }
    let body = answer.json()?;
    let login = body.get("login").and_then(|v| v.as_str())?.to_string();
    let mut emails: Vec<String> = Vec::new();
    if let Some(list) = api_get_as(ctx, host, &format!("{base}/user/emails"), token) {
        if list.ok() {
            #[derive(serde::Deserialize)]
            struct Entry {
                email: String,
                #[serde(default)]
                verified: bool,
            }
            if let Ok(entries) = serde_json::from_str::<Vec<Entry>>(&list.body) {
                emails = entries
                    .into_iter()
                    .filter(|entry| entry.verified)
                    .map(|entry| entry.email)
                    .collect();
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
        // The granted set rides on the answer's own header; a fine
        // grained token carries none, and an unknown set must never be
        // reported as a missing one (D2.7c).
        scopes: granted_scopes(&answer).map(|scopes| scopes.join(" ")),
    })
}

/// Whether this token reaches `owner/repo`, and whether it may push
/// (the probe of D4.1c). One request, per remote and never per contact.
pub fn reaches_repo(host: &str, repo_path: &str, token: &str, ctx: &Ctx) -> Option<Reach> {
    let url = format!("{}/repos/{repo_path}", api_base(host, ctx));
    let answer = api_get_as(ctx, host, &url, token)?;
    if !answer.ok() {
        // GitHub answers 404 rather than 403 for a private repository
        // the caller may not see, so both mean "this login is not the
        // one" and neither is an error.
        return Some(Reach::default());
    }
    let body = answer.json().unwrap_or_default();
    Some(Reach {
        read: true,
        push: body
            .pointer("/permissions/push")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

/// Revoke a token at GitHub: `DELETE /applications/{client_id}/token`,
/// and never `.../grant`. Deleting the GRANT deletes every token of
/// that app for the person, including the one another joy on another
/// machine is using (contradiction 13 of the design).
///
/// The endpoint authenticates with the application's own credentials.
/// joy registers a PUBLIC client, which has no secret, so a real
/// github.com refusal here is expected until the operator decides
/// otherwise; the entry is removed locally either way and the answer
/// says `"revoked": false` rather than pretending.
pub fn revoke_token(host: &str, record: &joy_forge_net::auth::store::Record, ctx: &Ctx) -> bool {
    let Some(client_id) = record.client_id.as_deref() else {
        return false;
    };
    let Ok(http) = ctx.http(host) else {
        return false;
    };
    let url = format!("{}/applications/{client_id}/token", api_base(host, ctx));
    match http
        .delete(&url)
        .header("Accept", ACCEPT_JSON)
        .basic(client_id, "")
        .send_json(&json!({ "access_token": record.token }))
    {
        // 204 is the documented success; anything else is a refusal
        // this connector reports honestly.
        Ok(answer) => answer.status == 204,
        Err(error) => {
            eprintln!("joy-forge github: {error}");
            false
        }
    }
}

/// Every login gh is signed in as on this host, the active one first
/// (D4.1c's probe candidate order).
///
/// gh keeps several accounts per host and documents the trap: "Without
/// the --user flag, the active account for the host is chosen." A
/// connector that asked `gh auth token --hostname H` and nothing else
/// would hand back whichever account the person last switched to.
pub fn gh_logins(host: &str) -> Vec<String> {
    match joy_forge_net::foreign::first_readable(&joy_forge_net::foreign::gh_config_files()) {
        Some((_, text)) => parse_logins(&text, host),
        None => Vec::new(),
    }
}

/// The `user:` and every key under `users:` of one host block of gh's
/// hosts.yml, the active one first and each name once.
pub fn parse_logins(text: &str, host: &str) -> Vec<String> {
    let host = host.trim().to_ascii_lowercase();
    let mut active: Option<String> = None;
    let mut users: Vec<String> = Vec::new();
    let mut in_host = false;
    let mut users_indent: Option<usize> = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            in_host = line
                .trim_end()
                .trim_end_matches(':')
                .trim()
                .to_ascii_lowercase()
                == host;
            users_indent = None;
            continue;
        }
        if !in_host {
            continue;
        }
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("user:") {
            let name = name.trim();
            if !name.is_empty() {
                active = Some(name.to_string());
            }
            continue;
        }
        if trimmed == "users:" {
            users_indent = Some(indent);
            continue;
        }
        match users_indent {
            // A key nested under `users:` is a login name; anything at
            // or above that indent ended the block.
            Some(block) if indent > block && trimmed.ends_with(':') => {
                let name = trimmed.trim_end_matches(':').trim();
                if !name.is_empty() && !users.iter().any(|known| known == name) {
                    users.push(name.to_string());
                }
            }
            Some(block) if indent <= block => users_indent = None,
            _ => {}
        }
    }
    let mut logins: Vec<String> = Vec::new();
    if let Some(active) = active {
        logins.push(active);
    }
    for user in users {
        if !logins.iter().any(|known| known == &user) {
            logins.push(user);
        }
    }
    logins
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx::bare(std::env::temp_dir())
    }

    #[test]
    fn github_hosts_are_claimed_and_lookalikes_are_not() {
        assert!(claims_host("github.com", &[]));
        assert!(claims_host("api.github.com", &[]));
        assert!(!claims_host("github.com.evil.example", &[]));
        assert!(!claims_host("gitlab.com", &[]));
    }

    /// GitHub Enterprise Server runs on the customer's own domain, so a
    /// remote is claimed when gh is signed in to that host. No instance
    /// belongs in this code.
    #[test]
    fn an_enterprise_host_is_claimed_once_gh_knows_it() {
        let configured = vec!["github.acme.test".to_string()];
        assert!(claims_host("github.acme.test", &configured));
        assert!(claims_host("github.com", &configured));
        assert!(!claims_host("github.acme.test.evil.example", &configured));
        assert!(!claims_host("github.acme.test", &[]));
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
    fn the_enterprise_alias_form_parses_too() {
        let a = parse_alias("77+horst@users.noreply.github.acme.test").unwrap();
        assert_eq!(a.login, "horst");
        assert_eq!(a.user_id.as_deref(), Some("77"));
    }

    #[test]
    fn hosts_yml_yields_the_login_of_github_com_or_the_configured_instance() {
        let text = "github.com:\n    user: joydev-horst\n    git_protocol: ssh\ngithub.acme.test:\n    user: nobody\n";
        assert_eq!(parse_hosts_yml(text).as_deref(), Some("joydev-horst"));
        assert_eq!(
            parse_hosts_yml("github.acme.test:\n    user: horst\n").as_deref(),
            Some("horst")
        );
        assert_eq!(parse_hosts_yml("").as_deref(), None);
    }

    /// The second hardcoded base of D2.8: a GHES host must ask its own
    /// `/api/v3`, never api.github.com.
    #[test]
    fn a_ghes_host_asks_its_own_api_v3() {
        let ctx = ctx();
        assert_eq!(api_base("github.com", &ctx), "https://api.github.com");
        assert_eq!(
            api_base("ghe.example.org", &ctx),
            "https://ghe.example.org/api/v3"
        );
        // and therefore the addresses of the person on THAT instance
        assert_eq!(
            format!("{}/user/emails", api_base("ghe.example.org", &ctx)),
            "https://ghe.example.org/api/v3/user/emails"
        );
    }

    /// And an operator's `forges.yaml` overrides even that (D2.5).
    #[test]
    fn a_configured_instance_names_its_own_api_base() {
        let ctx = ctx().with_instances(
            joy_forge_net::config::Instances::from_text(
                "- host: git.acme.test\n  kind: github\n  api_base: https://git.acme.test/api/v3\n",
            )
            .unwrap(),
        );
        assert_eq!(
            api_base("git.acme.test", &ctx),
            "https://git.acme.test/api/v3"
        );
    }

    #[test]
    fn resolve_is_pure_and_attributes_aliases_only() {
        let owner = resolve_answer("99+bob@users.noreply.github.com");
        assert_eq!(owner["known"], true);
        assert_eq!(owner["login"], "bob");
        assert_eq!(owner["user_id"], "99");
        assert_eq!(resolve_answer("bob@example.com")["known"], false);
    }

    fn answer(status: u16, body: &str) -> Option<Answer> {
        Some(Answer::new(status, body, Vec::new()))
    }

    fn with_scopes(status: u16, body: &str, scopes: &str) -> Option<Answer> {
        Some(Answer::new(
            status,
            body,
            vec![("x-oauth-scopes".into(), scopes.into())],
        ))
    }

    #[test]
    fn a_readable_project_yaml_is_the_store_and_carries_the_size_in_bytes() {
        let verdict = store_verdict(answer(200, "name: Demo\n"), true, || {
            answer(200, r#"{"size": 512, "default_branch": "main"}"#)
        });
        assert_eq!(verdict["state"], "store");
        assert_eq!(verdict["project_yaml"], "name: Demo\n");
        // GitHub counts kilobytes; the protocol carries bytes (D2.4)
        assert_eq!(verdict["size_bytes"], 512 * 1024);
    }

    #[test]
    fn a_store_stays_a_store_when_the_repository_call_fails() {
        let verdict = store_verdict(answer(200, "name: Demo\n"), true, || None);
        assert_eq!(
            verdict,
            json!({ "state": "store", "project_yaml": "name: Demo\n" })
        );
    }

    #[test]
    fn a_404_asks_the_repository_whether_it_is_gone_or_only_storeless() {
        assert_eq!(
            store_verdict(answer(404, "{}"), true, || answer(404, "{}"))["state"],
            "gone"
        );
        let missing = store_verdict(answer(404, "{}"), true, || {
            answer(200, r#"{"permissions": {"push": true}, "size": 4}"#)
        });
        assert_eq!(missing["state"], "missing");
        assert_eq!(missing["may_create"], true);
        assert_eq!(missing["size_bytes"], 4096);
        // an anonymous answer carries no permissions: nothing to create with
        let anonymous = store_verdict(answer(404, "{}"), false, || answer(200, "{}"));
        assert_eq!(anonymous["state"], "missing");
        assert_eq!(anonymous["may_create"], false);
    }

    /// D2.7c: GitHub answers 404 for a private repository the caller may
    /// not see, so an anonymous 404 must not tell a person their
    /// repository is gone.
    #[test]
    fn an_anonymous_404_is_not_the_verdict_gone() {
        assert_eq!(
            store_verdict(answer(404, "{}"), false, || answer(404, "{}")),
            json!({ "state": "unknown" })
        );
        // a token WITHOUT repo is the same case
        assert_eq!(
            store_verdict(answer(404, "{}"), true, || with_scopes(
                404,
                "{}",
                "public_repo,user:email"
            )),
            json!({ "state": "unknown" })
        );
        // with repo it is a verdict
        assert_eq!(
            store_verdict(answer(404, "{}"), true, || with_scopes(
                404,
                "{}",
                "repo,user:email"
            ))["state"],
            "gone"
        );
    }

    #[test]
    fn anything_unclear_stays_unanswered() {
        let unknown = json!({ "state": "unknown" });
        assert_eq!(store_verdict(None, true, || None), unknown);
        assert_eq!(store_verdict(answer(401, ""), true, || None), unknown);
        assert_eq!(store_verdict(answer(403, ""), true, || None), unknown);
        assert_eq!(store_verdict(answer(404, ""), true, || None), unknown);
        assert_eq!(
            store_verdict(answer(404, ""), true, || answer(500, "")),
            unknown
        );
    }

    #[test]
    fn a_tree_lists_its_files_and_says_when_it_was_cut_off() {
        let body = r#"{"tree": [
            {"path": "docs", "type": "tree"},
            {"path": "docs/VISION.md", "type": "blob"},
            {"path": "README.md", "type": "blob"}
        ], "truncated": true}"#;
        assert_eq!(
            files_verdict(answer(200, body)),
            json!({ "state": "files", "paths": ["docs/VISION.md", "README.md"], "truncated": true })
        );
        assert_eq!(
            files_verdict(answer(409, "")),
            json!({ "state": "files", "paths": [], "truncated": false })
        );
        assert_eq!(files_verdict(None), json!({ "state": "unknown" }));
        assert_eq!(
            files_verdict(answer(401, "")),
            json!({ "state": "unknown" })
        );
    }

    /// D2.7c, the classification rules that must never say `denied` for
    /// a scope problem.
    #[test]
    fn a_refusal_is_classified_by_its_headers_and_never_by_prose() {
        let scope_problem = Answer::new(
            403,
            "{}",
            vec![
                ("x-oauth-scopes".into(), "public_repo".into()),
                ("x-accepted-oauth-scopes".into(), "repo".into()),
            ],
        );
        assert_eq!(classify(&scope_problem), "scope_missing");
        let sso = Answer::new(
            403,
            "{}",
            vec![(
                "x-github-sso".into(),
                "required; url=https://github.com/orgs/acme/sso".into(),
            )],
        );
        assert_eq!(classify(&sso), "needs_sso");
        let org = Answer::new(
            403,
            r#"{"message":"Although you appear to have the correct authorization credentials, the `acme` organization has an IP allow list enabled, and OAuth App access restrictions"}"#,
            Vec::new(),
        );
        assert_eq!(classify(&org), "needs_org_approval");
        assert_eq!(classify(&Answer::new(403, "{}", Vec::new())), "denied");
        assert_eq!(
            classify(&Answer::new(401, "{}", Vec::new())),
            "needs_sign_in"
        );
    }

    /// D2.7: github.com signs in through joy's public client with the
    /// device grant; a GitHub Enterprise Server has instance local
    /// endpoints AND an instance local client id, and without one there
    /// is no door at all.
    #[test]
    fn the_device_grant_endpoints_are_the_instances_own() {
        let ctx = ctx();
        let public = oauth_for("github.com", Purpose::Write, &ctx).unwrap();
        assert_eq!(public.flow, Flow::Device);
        assert_eq!(
            public.device_endpoint,
            "https://github.com/login/device/code"
        );
        assert_eq!(
            public.token_endpoint,
            "https://github.com/login/oauth/access_token"
        );
        assert_eq!(public.scopes, SCOPES);
        assert!(
            joy_forge_net::auth::oauth::clients::is_placeholder(&public.client_id),
            "the public client id is a placeholder until it is registered"
        );
        // A GHES host with nothing configured has no client id, so
        // `login` says so instead of asking github.com about it.
        assert!(oauth_for("ghe.acme.test", Purpose::Write, &ctx).is_none());
        let configured = ctx.with_instances(
            joy_forge_net::config::Instances::from_text(
                "- host: ghe.acme.test\n  kind: github\n  client_id: Iv1.instance\n",
            )
            .unwrap(),
        );
        let instance = oauth_for("ghe.acme.test", Purpose::Write, &configured).unwrap();
        assert_eq!(instance.client_id, "Iv1.instance");
        assert_eq!(
            instance.device_endpoint,
            "https://ghe.acme.test/login/device/code"
        );
    }

    /// D2.7a: one set covers A to G on GitHub, so `--for` changes
    /// nothing, and D2.7c says the person is told why.
    #[test]
    fn github_asks_for_one_set_whatever_the_access_level_is() {
        assert_eq!(SCOPES, "repo user:email");
        assert!(READ_IS_WRITE.contains("never pushes without an explicit action"));
    }

    /// D4.1c: gh keeps several accounts per host, and the active one is
    /// the first candidate of the probe order.
    #[test]
    fn every_gh_account_of_a_host_is_a_probe_candidate_the_active_one_first() {
        let text = "github.com:\n    users:\n        work:\n            oauth_token: x\n        scotty:\n            oauth_token: y\n    user: scotty\n    git_protocol: ssh\ngithub.acme.test:\n    user: nobody\n";
        assert_eq!(parse_logins(text, "github.com"), vec!["scotty", "work"]);
        assert_eq!(parse_logins(text, "github.acme.test"), vec!["nobody"]);
        assert!(parse_logins(text, "gitlab.com").is_empty());
        // a host block with one account and no `users:` map still lists it
        assert_eq!(
            parse_logins("github.com:\n    user: solo\n", "github.com"),
            vec!["solo"]
        );
    }

    #[test]
    fn a_repository_row_is_filtered_by_the_query() {
        let entry = json!({
            "full_name": "joyint/app",
            "name": "app",
            "private": true,
            "clone_url": "https://github.com/joyint/app.git",
            "ssh_url": "git@github.com:joyint/app.git",
            "default_branch": "main",
            "html_url": "https://github.com/joyint/app"
        });
        let row = repository_row(&entry, Some("APP")).unwrap();
        assert_eq!(row["full_name"], "joyint/app");
        assert_eq!(row["private"], true);
        assert!(repository_row(&entry, Some("nothing")).is_none());
        assert!(repository_row(&entry, None).is_some());
    }
}
