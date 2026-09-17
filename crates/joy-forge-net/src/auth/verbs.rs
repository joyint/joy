// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `token`, `token-store`, `login`, `logout` and `web-url`, with the
//! shapes of D2.4 as amended by D4.1c.
//!
//! Every answer that names a token carries `login` and `chose_by`,
//! because D4.1's promise of "one row per host, each naming the login
//! it holds" cannot be kept otherwise, and because `logout`,
//! `joy forge status` and the Device host row all read them.
//!
//! The secret itself never leaves this file except in the one place it
//! belongs: the `token` answer, which is what the engine asked for. It
//! is never an argument, never a log line and never part of an error.

use serde_json::{json, Value};

use super::oauth::{self, Clock, Events, Flow, Poll};
use super::store::Record;
use super::{choose, lock, pin, ChoseBy, Purpose, Resolved, Source};
use crate::forge::{Ctx, Forge, HostKind, Target};

/// What the connector's own entry had to say.
pub enum Own {
    /// A usable token, refreshed if it needed to be.
    Found(Box<Resolved>),
    /// Nothing is stored for this host, or no login could be chosen
    /// without a probe.
    Nothing,
    /// Another process holds the refresh lock and the entry is past its
    /// lifetime: D2.6a says report busy, never refresh anyway.
    Busy,
}

/// The connector's own credential for this host, chosen by the steps of
/// D4.1c that spend no request (pin, memory, only login).
pub fn own_token(ctx: &Ctx, host: &str) -> Option<Resolved> {
    match own_token_full(ctx, host) {
        Own::Found(resolved) => Some(*resolved),
        _ => None,
    }
}

/// [`own_token`] with the third answer: `busy`.
pub fn own_token_full(ctx: &Ctx, host: &str) -> Own {
    let vault = ctx.vault();
    if vault.is_none() {
        return Own::Nothing;
    }
    let known = vault.logins(host);
    let chosen = choose::without_probe(
        ctx.login.as_deref(),
        pin::pinned(ctx.root(), host).as_deref(),
        ctx.remote
            .as_deref()
            .and_then(|remote| pin::remembered(ctx.state_dir(), remote))
            .as_deref(),
        &known,
    );
    let (login, chose_by) = match chosen {
        Some((login, chose_by)) => (Some(login), Some(chose_by)),
        // A host with no named login at all still has the `<host>` form
        // of D2.6's entry addressing; a host with several needs the
        // probe, which is the `token` verb's business and not this one's.
        None if known.is_empty() => (None, Some(ChoseBy::Only)),
        None => return Own::Nothing,
    };
    let Some((record, source)) = vault.get(host, login.as_deref()) else {
        return Own::Nothing;
    };
    match fresh(ctx, host, record, source) {
        Ok((record, source)) => Own::Found(Box::new(resolved_of(record, source, chose_by))),
        Err(()) => Own::Busy,
    }
}

/// The record, refreshed under the lock of D2.6a where it is past its
/// lifetime and the forge gave joy a refresh token.
///
/// The order matters and is the design's: gh, glab and tea are read
/// BEFORE the lock is taken and never while it is held, because flock
/// belongs to the open file description and a child that unlocks takes
/// the parent's lock with it. Nothing here spawns anything.
fn fresh(ctx: &Ctx, host: &str, record: Record, source: Source) -> Result<(Record, Source), ()> {
    if !record.is_expired() {
        return Ok((record, source));
    }
    if !record.can_refresh() {
        // An expired token with no way to renew it is still what this
        // machine has. The forge answers 401 and the classifier of
        // D1.8 turns that into `needs_sign_in`, which is the honest
        // end; refusing here would only lose the one attempt that
        // tells the person why.
        return Ok((record, source));
    }
    let login = record.login.clone();
    let before = record.fingerprint();
    let guard = match lock::take(ctx.state_dir(), host, login.as_deref()) {
        Ok(guard) => guard,
        Err(_) => {
            // Re read once: another process may have finished the
            // refresh while this one waited.
            let again = ctx.vault().get(host, login.as_deref());
            return match again {
                Some((record, source)) if !record.is_expired() => Ok((record, source)),
                _ => Err(()),
            };
        }
    };
    // Under the lock: re read, and refresh only if the entry is still
    // the one this process saw and is still expired.
    let (current, source) = ctx
        .vault()
        .get(host, login.as_deref())
        .unwrap_or((record, source));
    if current.fingerprint() != before || !current.is_expired() {
        drop(guard);
        return Ok((current, source));
    }
    let (Some(endpoint), Some(refresh_token), Some(client_id)) = (
        current.token_endpoint.clone(),
        current.refresh_token.clone(),
        current.client_id.clone(),
    ) else {
        drop(guard);
        return Ok((current, source));
    };
    let Ok(http) = ctx.http(host) else {
        drop(guard);
        return Ok((current, source));
    };
    let outcome = oauth::refresh(&http, &endpoint, &client_id, &refresh_token);
    let renewed = match outcome {
        Poll::Granted(grant) => grant,
        // A refresh that failed is not a reason to try again in a loop:
        // ten thousand attempts against one dead refresh token got a
        // whole OAuth app throttled once (D2.6a).
        _ => {
            drop(guard);
            return Ok((current, source));
        }
    };
    let mut next = current.clone();
    next.expires_at = renewed.expires_at();
    next.token = renewed.access_token;
    // Rotation safety: Forgejo issues a NEW refresh token on every use
    // and retires the old one, so the whole answer is written back and
    // never merged with what was there.
    next.refresh_token = renewed.refresh_token.or(next.refresh_token);
    if let Some(scope) = renewed.scope {
        next.scopes = crate::scope::parse_granted(&scope).join(" ");
    }
    let stored = ctx.vault().put(host, &next).unwrap_or(source);
    drop(guard);
    Ok((next, stored))
}

fn resolved_of(record: Record, source: Source, chose_by: Option<ChoseBy>) -> Resolved {
    Resolved {
        token: record.token,
        login: record.login,
        source,
        scopes: (!record.scopes.is_empty()).then_some(record.scopes),
        expires_at: record.expires_at,
        chose_by,
    }
}

// -- the token verb (D2.4, D4.1c) ---------------------------------------------

/// `token --remote <url> | --host <h> [--login <name>]`.
pub fn token(forge: &dyn Forge, target: &Target, ctx: &Ctx) -> Value {
    let Some(host) = target.host() else {
        return json!({ "known": false, "reason": "unsupported-host" });
    };
    match own_token_full(ctx, &host) {
        Own::Found(resolved) => return answer(forge, &host, &resolved),
        Own::Busy => return lock::busy_answer(),
        Own::Nothing => {}
    }
    // Several logins and nothing that names one: the probe of D4.1c,
    // one request per candidate, per remote and never per contact.
    if let Some(path) = target.repo_path() {
        match probe(forge, &host, &path, ctx) {
            Probed::Found(resolved) => {
                if let Some(remote) = ctx.remote.as_deref() {
                    if let Some(login) = resolved.login.as_deref() {
                        pin::remember(ctx.state_dir(), remote, login);
                    }
                }
                return answer(forge, &host, &resolved);
            }
            Probed::NoneReach(tried) => {
                // Whatever the memory said, it is wrong: no login this
                // machine holds reaches the repository (D4.1c).
                if let Some(remote) = ctx.remote.as_deref() {
                    pin::forget_remote(ctx.state_dir(), remote);
                }
                return choose::no_login_for_repo(forge.display(), &tried, &path);
            }
            Probed::NotAsked => {}
        }
    }
    // The rest of D2.4's source order: the forge's own variables, then
    // the forge CLI, spawned.
    match ctx.resolved_token(forge.id(), &host) {
        Some(resolved) => answer(forge, &host, &resolved),
        None if ctx.vault().is_none() => json!({ "known": false, "reason": "no-keychain" }),
        None => {
            let mut answer = json!({ "known": false, "reason": "no-login" });
            // A host that holds several logins and nothing that names
            // one has an answer a person can act on, and "no-login"
            // alone is not it (D4.1c).
            let known = ctx.vault().logins(&host);
            if known.len() > 1 {
                answer["message"] = json!(format!(
                    "This machine holds several {} logins ({}). \
                     Say which one with --login, or ask about a repository.",
                    forge.display(),
                    known.join(", ")
                ));
            }
            answer
        }
    }
}

/// The one object of D2.4, with the two fields D4.1c adds.
fn answer(forge: &dyn Forge, host: &str, resolved: &Resolved) -> Value {
    json!({
        "known": true,
        "host": host,
        "login": resolved.login,
        "token": resolved.token,
        "username": forge.https_username(),
        "source": resolved.source.as_str(),
        "scopes": resolved.scopes,
        "expires_at": resolved.expires_at,
        "chose_by": resolved.chose_by.map(ChoseBy::as_str),
    })
}

enum Probed {
    Found(Resolved),
    /// Every candidate was asked and none reached the repository.
    NoneReach(Vec<String>),
    /// There was nothing to probe with.
    NotAsked,
}

fn probe(forge: &dyn Forge, host: &str, repo_path: &str, ctx: &Ctx) -> Probed {
    let vault = ctx.vault();
    let own = vault.logins(host);
    let foreign = forge.foreign_logins(host);
    let order = choose::probe_order(&own, &foreign);
    if order.len() < 2 {
        // One candidate is step 3, not step 4, and zero is nothing.
        return Probed::NotAsked;
    }
    for login in &order {
        let candidate = match vault.get(host, Some(login)) {
            Some((record, source)) => match fresh(ctx, host, record, source) {
                Ok((record, source)) => resolved_of(record, source, Some(ChoseBy::Probe)),
                Err(()) => continue,
            },
            None => match foreign_token(forge, host, login) {
                Some(resolved) => resolved,
                None => continue,
            },
        };
        if let Some(reach) = forge.reaches(host, repo_path, &candidate.token, ctx) {
            if reach.read {
                return Probed::Found(candidate);
            }
        }
    }
    Probed::NoneReach(order)
}

/// A foreign CLI's token for one named login, by spawning it.
fn foreign_token(forge: &dyn Forge, host: &str, login: &str) -> Option<Resolved> {
    let (token, source) = match forge.foreign_cli() {
        "gh" => (crate::foreign::gh_token(host, Some(login))?, Source::Gh),
        "glab" => (crate::foreign::glab_token(host)?, Source::Glab),
        "tea" => (crate::foreign::tea_token(host)?, Source::Tea),
        _ => return None,
    };
    Some(Resolved {
        token,
        login: Some(login.to_string()),
        source,
        scopes: None,
        expires_at: None,
        chose_by: Some(ChoseBy::Probe),
    })
}

// -- the token-store verb (D2.4) ----------------------------------------------

/// `token-store --host <h> [--login <name>]`: one token from STDIN,
/// validated with `identity`, stored, and then the same answer `token`
/// gives.
///
/// The token is never an argument, in either direction. This is the
/// headless door: a Linux server, a CI runner, a Windows host with no
/// browser, and every Gitea family instance whose operator registered
/// no OAuth client.
pub fn token_store(forge: &dyn Forge, target: &Target, ctx: &Ctx) -> Value {
    let raw = match read_stdin() {
        Some(raw) => raw,
        None => {
            eprintln!("joy: no token arrived on stdin");
            return json!({ "known": false, "reason": "no-login" });
        }
    };
    store_token(forge, target, ctx, &raw)
}

/// [`token_store`] with the token already in hand, which is what the
/// tests drive: stdin is a process wide resource and a test must not
/// need one.
pub fn store_token(forge: &dyn Forge, target: &Target, ctx: &Ctx, raw: &str) -> Value {
    let Some(host) = target.host() else {
        return json!({ "known": false, "reason": "unsupported-host" });
    };
    let token = raw.trim();
    if token.is_empty() {
        eprintln!("joy: no token arrived on stdin");
        return json!({ "known": false, "reason": "no-login" });
    }
    // The validation D2.4 asks for: the token names an account on THIS
    // instance, or it is not stored at all.
    let Some(account) = forge.account(&host, token, ctx) else {
        eprintln!("joy: {} did not accept this token", forge.display());
        return json!({ "known": false, "reason": "no-login" });
    };
    let login = ctx
        .login
        .clone()
        .filter(|login| !login.trim().is_empty())
        .unwrap_or_else(|| account.login.clone());
    let record = Record {
        token: token.to_string(),
        login: Some(login.clone()),
        user_id: account.user_id.clone(),
        scopes: account.scopes.clone().unwrap_or_default(),
        ..Record::default()
    };
    let guard = lock::take(ctx.state_dir(), &host, Some(&login));
    if guard.is_err() {
        return lock::busy_answer();
    }
    let stored = match ctx.vault().put(&host, &record) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("joy: this token could not be stored: {message}");
            return json!({ "known": false, "reason": "no-keychain" });
        }
    };
    drop(guard);
    let resolved = resolved_of(record, stored, chose_by_now(ctx, &host, &login));
    answer(forge, &host, &resolved)
}

/// Which step of D4.1c would choose this login now that it is stored.
fn chose_by_now(ctx: &Ctx, host: &str, login: &str) -> Option<ChoseBy> {
    let known = ctx.vault().logins(host);
    match choose::without_probe(
        ctx.login.as_deref(),
        pin::pinned(ctx.root(), host).as_deref(),
        ctx.remote
            .as_deref()
            .and_then(|remote| pin::remembered(ctx.state_dir(), remote))
            .as_deref(),
        &known,
    ) {
        Some((chosen, chose_by)) if chosen == login => Some(chose_by),
        // Several logins, and nothing pins this one: the next `token`
        // call will have to probe for it.
        _ => Some(ChoseBy::Probe),
    }
}

fn read_stdin() -> Option<String> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).ok()?;
    Some(raw)
}

// -- the login verb (D2.4, D2.7, D3.11) ---------------------------------------

/// The sentence of D3.11 for a host that has no person at it.
pub const NO_PERSON_HERE: &str =
    "joy forge login needs a person at this machine; this process runs under a delegation \
     session. Sign in on the machine that owns the session, or store a token there with \
     joy forge login --token-stdin";

/// `login --remote <url> | --host <h> [--for ...] [--login <name>]`,
/// newline delimited JSON on stdout, one object per line, each flushed.
///
/// The connector never opens a browser and never prints the token. The
/// host decides: the desktop opens the URL, the CLI prints it, a
/// delegated session never opens anything and the verb is refused
/// before it starts.
pub fn login(
    forge: &dyn Forge,
    target: &Target,
    purpose: Purpose,
    ctx: &Ctx,
    events: &mut dyn Events,
    clock: &dyn Clock,
) -> i32 {
    // Layer 3 of D3.11: the agent image builds joy-cli from source, so
    // a delegated agent has this verb on its PATH. The refusal is
    // instant, not a fifteen minute wait.
    if matches!(ctx.host_kind, HostKind::Background | HostKind::Delegated) {
        events.emit(oauth::error_event("unsupported", NO_PERSON_HERE));
        return 0;
    }
    let Some(host) = target.host() else {
        events.emit(oauth::error_event(
            "unsupported",
            "this call named no host to sign in to",
        ));
        return 0;
    };
    let Some(config) = forge.oauth(&host, purpose, ctx) else {
        events.emit(oauth::error_event(
            "unsupported",
            &oauth::unregistered_sentence(&host),
        ));
        return 0;
    };
    if oauth::clients::is_placeholder(&config.client_id) {
        events.emit(oauth::error_event(
            "unsupported",
            &oauth::unregistered_sentence(&host),
        ));
        return 0;
    }
    let Ok(http) = ctx.http(&host) else {
        events.emit(oauth::error_event(
            "network",
            "joy could not build an HTTP client for this host",
        ));
        return 0;
    };
    let grant = match config.flow {
        Flow::Device => device_login(&http, &config, &host, events, clock),
        Flow::Pkce => pkce_login(&http, &config, &host, events),
    };
    let grant = match grant {
        Ok(grant) => grant,
        Err(Poll::Failed { code, message }) => {
            events.emit(oauth::error_event(&code, &message));
            return 0;
        }
        Err(_) => {
            events.emit(oauth::error_event(
                "unsupported",
                "the forge answered an OAuth message joy could not read",
            ));
            return 0;
        }
    };
    finish(forge, &host, &config, grant, ctx, events)
}

fn device_login(
    http: &crate::http::Http,
    config: &oauth::OAuth,
    host: &str,
    events: &mut dyn Events,
    clock: &dyn Clock,
) -> Result<oauth::Grant, Poll> {
    let start = oauth::start_device(http, config)?;
    events.emit(oauth::verification_event(
        host,
        &start.verification_uri,
        start.verification_uri_complete.as_deref(),
        Some(&start.user_code),
        start.expires_in,
        start.interval,
    ));
    let mut interval = start.interval.max(1);
    let mut left = start.expires_in;
    while left > 0 {
        clock.sleep(std::time::Duration::from_secs(interval as u64));
        left -= interval;
        match oauth::poll_device(http, config, &start.device_code) {
            Poll::Granted(grant) => return Ok(grant),
            Poll::Pending => events.emit(json!({
                "event": "waiting",
                "seconds_left": left.max(0),
            })),
            Poll::SlowDown => {
                // D2.7: plus five seconds, and the new interval is
                // announced so the host can say what it is waiting for.
                interval += 5;
                events.emit(json!({ "event": "slow_down", "interval": interval }));
            }
            failed => return Err(failed),
        }
    }
    Err(Poll::Failed {
        code: "expired_token".to_string(),
        message: "the verification code expired before the sign in finished".to_string(),
    })
}

fn pkce_login(
    http: &crate::http::Http,
    config: &oauth::OAuth,
    host: &str,
    events: &mut dyn Events,
) -> Result<oauth::Grant, Poll> {
    let pkce = oauth::Pkce::start().map_err(|e| Poll::Failed {
        code: "network".to_string(),
        message: format!("joy could not open a loopback listener: {e}"),
    })?;
    let url = pkce.authorize_url(config);
    // The Gitea family has no device grant in any released version, so
    // there is no user code to type: the whole request is in the URL.
    events.emit(oauth::verification_event(
        host,
        &url,
        None,
        None,
        PKCE_WAIT.as_secs() as i64,
        1,
    ));
    let mut ticked: Vec<Value> = Vec::new();
    let code = pkce.wait(PKCE_WAIT, |seconds_left| {
        ticked.push(json!({ "event": "waiting", "seconds_left": seconds_left }));
    });
    for event in ticked {
        events.emit(event);
    }
    let code = code?;
    match oauth::exchange_code(http, config, &code, pkce.verifier(), &pkce.redirect_uri()) {
        Poll::Granted(grant) => Ok(grant),
        other => Err(other),
    }
}

/// The cap on a loopback sign in: the same fifteen minutes D2.3 gives
/// the whole `login` call.
const PKCE_WAIT: std::time::Duration = std::time::Duration::from_secs(900);

fn finish(
    forge: &dyn Forge,
    host: &str,
    config: &oauth::OAuth,
    grant: oauth::Grant,
    ctx: &Ctx,
    events: &mut dyn Events,
) -> i32 {
    let Some(account) = forge.account(host, &grant.access_token, ctx) else {
        events.emit(oauth::error_event(
            "unsupported",
            "the forge granted a token it then did not accept",
        ));
        return 0;
    };
    // Gitea's AccessTokenResponse has no scope field, so for the Gitea
    // family the set stored is the set requested (D2.7c). Whichever
    // source names it, it is stored SPACE separated: GitHub writes its
    // sets with commas and D2.7c asks for one spelling.
    let scopes = grant
        .scope
        .clone()
        .or_else(|| account.scopes.clone())
        .unwrap_or_else(|| config.scopes.clone());
    let scopes = crate::scope::parse_granted(&scopes).join(" ");
    let record = Record {
        token: grant.access_token.clone(),
        login: Some(account.login.clone()),
        user_id: account.user_id.clone(),
        scopes: scopes.clone(),
        expires_at: grant.expires_at(),
        refresh_token: grant.refresh_token.clone(),
        token_endpoint: Some(config.token_endpoint.clone()),
        client_id: Some(config.client_id.clone()),
    };
    let guard = lock::take(ctx.state_dir(), host, Some(&account.login));
    if guard.is_err() {
        events.emit(oauth::error_event(
            "unsupported",
            "another joy process is writing this credential; try again in a moment",
        ));
        return 0;
    }
    let stored = match ctx.vault().put(host, &record) {
        Ok(source) => source,
        Err(message) => {
            events.emit(oauth::error_event(
                "unsupported",
                &format!("the token could not be stored: {message}"),
            ));
            return 0;
        }
    };
    drop(guard);
    events.emit(json!({
        "event": "result",
        "known": true,
        "login": account.login,
        "user_id": account.user_id,
        "emails": account.emails,
        "scopes": scopes,
        "stored": stored.as_str(),
        "expires_at": record.expires_at,
    }));
    0
}

// -- the logout verb (D2.4) ---------------------------------------------------

/// `logout --host <h> [--login <name>]`.
///
/// Where the credential came from a foreign CLI the connector removes
/// nothing and names the foreign command: joy never refreshes, writes
/// or revokes what gh, glab and tea own (D2.6).
pub fn logout(forge: &dyn Forge, target: &Target, ctx: &Ctx) -> Value {
    let Some(host) = target.host() else {
        return json!({ "removed": false, "revoked": false, "source": null });
    };
    let vault = ctx.vault();
    let known = vault.logins(&host);
    let login = ctx
        .login
        .clone()
        .filter(|login| !login.trim().is_empty())
        .or_else(|| {
            choose::without_probe(
                None,
                pin::pinned(ctx.root(), &host).as_deref(),
                None,
                &known,
            )
            .map(|(login, _)| login)
        });
    if let Some((record, source)) = vault.get(&host, login.as_deref()) {
        let revoked = forge.revoke(&host, &record, ctx);
        let removed = vault.remove(&host, record.login.as_deref()).is_some();
        if let Some(login) = record.login.as_deref() {
            pin::forget_login(ctx.state_dir(), &host, login);
        }
        return json!({
            "removed": removed,
            "revoked": revoked,
            "source": source.as_str(),
            "login": record.login,
        });
    }
    // Nothing of joy's own. If a forge CLI holds one, say whose it is
    // and which command removes it.
    if foreign_token(forge, &host, login.as_deref().unwrap_or_default()).is_some()
        || !forge.foreign_logins(&host).is_empty()
    {
        return json!({
            "removed": false,
            "revoked": false,
            "source": foreign_source(forge),
            "command": forge.foreign_logout_command(&host),
        });
    }
    json!({ "removed": false, "revoked": false, "source": null })
}

fn foreign_source(forge: &dyn Forge) -> &'static str {
    match forge.foreign_cli() {
        "gh" => Source::Gh.as_str(),
        "glab" => Source::Glab.as_str(),
        "tea" => Source::Tea.as_str(),
        other => other,
    }
}

// -- the web-url verb (D1.5, D2.4) --------------------------------------------

/// `web-url --remote <url>`: the https twin of a remote, which only the
/// forge can compute for a self hosted instance.
pub fn web_url(forge: &dyn Forge, target: &Target, ctx: &Ctx) -> Value {
    forge.web_url(target, ctx)
}

/// The https twin every forge of these three families computes the same
/// way: the configured web base, or the host itself, plus the
/// repository path and `.git`.
pub fn https_twin(target: &Target, ctx: &Ctx) -> Value {
    let (Some(host), Some(path)) = (target.host(), target.repo_path()) else {
        return json!({ "known": false });
    };
    let base = ctx
        .instance(&host)
        .and_then(|entry| entry.web_base.clone())
        .unwrap_or_else(|| format!("https://{host}"));
    json!({ "known": true, "https_url": format!("{}/{path}.git", base.trim_end_matches('/')) })
}

/// The local `scope_missing` pre check of D2.7c, against the set the
/// connector stored beside the token.
///
/// This is the cheap half of D2.7c: the set was written down when the
/// token was granted, so a verb the set cannot carry is refused without
/// spending a request, and the forge's own refusal is never reported as
/// `denied`. An UNKNOWN set (a fine grained token, a variable somebody
/// exported) is never reported as a missing one: this answers `None`
/// and the verb is attempted.
pub fn stored_scope_gate(
    ctx: &Ctx,
    forge_id: &str,
    host: &str,
    verb: &str,
    group: crate::scope::Group,
) -> Option<Value> {
    let granted = ctx.granted_scopes(forge_id, host)?;
    let missing = crate::scope::missing(forge_id, group, &granted);
    (!missing.is_empty()).then(|| crate::scope::scope_missing(host, verb, &missing, &granted))
}

/// [`stored_scope_gate`] for `create-repository`, where GitHub's answer
/// depends on the repository's visibility: "public_repo or repo scope
/// to create a public repository, and repo scope to create a private
/// repository" (D2.7a).
pub fn stored_create_gate(ctx: &Ctx, forge_id: &str, host: &str, private: bool) -> Option<Value> {
    let granted = ctx.granted_scopes(forge_id, host)?;
    let missing = crate::scope::missing_for_create(forge_id, private, &granted);
    (!missing.is_empty())
        .then(|| crate::scope::scope_missing(host, "create-repository", &missing, &granted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_https_twin_uses_the_configured_web_base_of_a_self_hosted_instance() {
        let ctx = Ctx::bare(std::env::temp_dir()).with_instances(
            crate::config::Instances::from_text(
                "- host: git.acme.test\n  kind: gitlab\n  web_base: https://git.acme.test/code/\n",
            )
            .unwrap(),
        );
        let answer = https_twin(
            &Target::Remote("git@git.acme.test:team/sub/repo.git".into()),
            &ctx,
        );
        assert_eq!(answer["known"], true);
        assert_eq!(
            answer["https_url"],
            "https://git.acme.test/code/team/sub/repo.git"
        );
        let plain = https_twin(
            &Target::Remote("git@github.com:joyint/app.git".into()),
            &ctx,
        );
        assert_eq!(plain["https_url"], "https://github.com/joyint/app.git");
        assert_eq!(
            https_twin(&Target::Host("github.com".into()), &ctx)["known"],
            false
        );
    }

    /// D2.7c: the pre check answers locally from the set stored beside
    /// the token, and an UNKNOWN set is never reported as a missing one.
    #[test]
    fn the_scope_gate_answers_from_the_stored_set_and_stays_quiet_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::bare(dir.path())
            .with_vault(crate::auth::store::Vault::file_at(dir.path()))
            .with_state_dir(dir.path());
        ctx.vault()
            .put(
                "gitlab.com",
                &Record {
                    token: "glpat".into(),
                    login: Some("scotty".into()),
                    scopes: "read_api write_repository".into(),
                    ..Record::default()
                },
            )
            .unwrap();
        let refused = stored_create_gate(&ctx, "gitlab", "gitlab.com", false).unwrap();
        assert_eq!(refused["state"], "scope_missing");
        assert_eq!(refused["needed"], json!(["api"]));
        assert_eq!(refused["have"], json!(["read_api", "write_repository"]));
        assert!(
            stored_scope_gate(
                &ctx,
                "gitlab",
                "gitlab.com",
                "store",
                crate::scope::Group::GitRead
            )
            .is_none(),
            "the set covers reading, so nothing is refused"
        );
        // A credential with no set recorded beside it: not known, so
        // the verb is attempted and the forge decides.
        let quiet = Ctx::bare(dir.path()).with_state_dir(dir.path());
        assert!(stored_create_gate(&quiet, "gitlab", "gitlab.com", false).is_none());
    }

    #[test]
    fn the_delegation_refusal_names_the_headless_door() {
        assert!(NO_PERSON_HERE.contains("--token-stdin"));
        assert!(NO_PERSON_HERE.contains("delegation session"));
    }
}
