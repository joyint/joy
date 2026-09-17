// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE proxy decision of one contact (design D1.11, D1.13), and the
//! reading half of the trust store hatch of D1.12, whose one
//! `git2::opts` call lives in this crate's `lib.rs` because it is
//! process global and must run before the first contact.
//!
//! Until joy dropped the git binary, git read the proxy configuration
//! and joy never had to. `git2::ProxyOptions` derives `Default`, which
//! is `GIT_PROXY_NONE` (proxy_options.rs:9-16, libgit2-sys
//! lib.rs:1906-1909), so a joy that passes nothing behind a corporate
//! proxy does not fail with a sentence, it times out. Every
//! `FetchOptions`, every `PushOptions` and every `connect_auth` in
//! joy-core therefore carries the options [`options_for`] built, and
//! there is one such function.
//!
//! It decides between the three outcomes of D1.11:
//!
//! 1. **Bypassed** (`GIT_PROXY_NONE`). The host matches joy's own
//!    NO_PROXY evaluation, or the configuration turns the proxy off
//!    with an empty value. joy evaluates NO_PROXY itself and applies it
//!    to proxies from git config as well, because libgit2 applies it
//!    only to the environment branch (`http_proxy_config` never looks
//!    at no_proxy, remote.c:1085-1133) while git applies it always. The
//!    matcher follows libgit2's own grammar (net.c:1070-1117) and
//!    additionally TRIMS each entry, because libgit2 does not and
//!    `NO_PROXY="a.com, b.com"` therefore silently loses `b.com`.
//! 2. **Auto** (`GIT_PROXY_AUTO`). joy found no proxy of its own and
//!    lets libgit2 walk its order (remote.c:1085-1194). joy does not
//!    reimplement that walk to replace it; it repeats it only far
//!    enough to KNOW the proxy, which is what outcome 3 needs.
//! 3. **Specified** (`GIT_PROXY_SPECIFIED`). Whenever joy knows a
//!    proxy. That covers `ALL_PROXY`/`all_proxy`, which libgit2 never
//!    reads and git does, and it covers injected credentials. On
//!    Windows it is also the only correct mode: under `GIT_PROXY_AUTO`
//!    the 407 path passes a NULL URL into `acquire_credentials`
//!    (winhttp.c:1255-1270), so joy uses `GIT_PROXY_SPECIFIED` whenever
//!    a proxy is known.
//!
//! **Proxy credentials.** git2 0.21 hardwires `credentials: None` and
//! `certificate_check: None` in `ProxyOptions::raw`
//! (proxy_options.rs:44-53) and offers no setter, so joy can never
//! answer a 407 through a callback. It answers it the only way that
//! works: libgit2 presents a proxy URL's own userinfo before it
//! consults any callback (http.c:141-152), so joy resolves the proxy
//! credential through its own helper runner (`protocol=http`,
//! `host=<proxyhost>[:port]`, D1.3), builds the URL in memory and
//! passes it as `ProxyOptions::url`. The credential never touches the
//! person's git config, and it never appears in a log line or an error
//! text: [`Proxy`] prints its NAME, which is `host:port` and nothing
//! else, its `Debug` is written by hand for that reason, and the one
//! libgit2 message that could echo the URL back goes through
//! [`scrubbed`] on its way to a person.
//!
//! **Not supported, and said so instead of failing obscurely**: SOCKS
//! proxies. libgit2 parses any proxy URL as an HTTP proxy and always
//! speaks HTTP CONNECT (httpclient.c:686-700); a `socks5://host`
//! without an explicit port is rejected with libgit2's own "invalid
//! URL" because no default port exists for that scheme (net.c:110-123,
//! http.c:340-342). joy detects a non http/https proxy scheme BEFORE
//! the contact and refuses by name ([`Unsupported`]).
//!
//! Off Windows only Basic reaches a proxy at all, because libgit2-sys
//! defines neither `GIT_NTLM` nor `GIT_GSSAPI` (build.rs:256-269) and
//! both aliases resolve to `git_http_auth_dummy` (auth_ntlm.h:15,
//! auth_negotiate.h:15, auth.c:65-71). That asymmetry is not hidden: it
//! is the `proxy_auth` guidance of D1.8c, which
//! [`super::contact::verdict`] writes.

use std::cell::RefCell;
use std::path::Path;

use super::remote_url::{RemoteUrl, Transport};
use crate::host::HostKind;

/// Which of the three outcomes of D1.11 this contact got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// `GIT_PROXY_NONE`: NO_PROXY matched, the configuration turns the
    /// proxy off, or the transport takes no proxy at all.
    Bypassed,
    /// `GIT_PROXY_AUTO`: joy knows no proxy and libgit2 may look.
    Auto,
    /// `GIT_PROXY_SPECIFIED`: joy knows the proxy and names it.
    Specified,
}

/// The proxy decision of one contact.
///
/// The URL it holds may carry a password, so this type has no `Display`
/// and its `Debug` prints the NAME only. Every log line, every error
/// text and every piece of evidence takes [`Proxy::name`].
#[derive(Clone, PartialEq, Eq)]
pub struct Proxy {
    outcome: Outcome,
    /// `host:port`, the only name a 407 may carry (D1.8c). `None` when
    /// joy knows no proxy.
    name: Option<String>,
    /// The URL handed to libgit2, userinfo included. Never logged.
    url: Option<String>,
    /// Whether the URL carries a credential, for the log line that says
    /// so without saying which.
    credentialed: bool,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy")
            .field("outcome", &self.outcome)
            .field("name", &self.name)
            .field("credentialed", &self.credentialed)
            .finish()
    }
}

impl Proxy {
    /// No proxy at all: what a local remote and an ssh remote get.
    fn bypassed() -> Proxy {
        Proxy {
            outcome: Outcome::Bypassed,
            name: None,
            url: None,
            credentialed: false,
        }
    }

    fn auto() -> Proxy {
        Proxy {
            outcome: Outcome::Auto,
            name: None,
            url: None,
            credentialed: false,
        }
    }

    /// The options this contact carries. Built fresh per call site,
    /// because `git2::ProxyOptions` is not `Clone` and each of
    /// `FetchOptions`, `PushOptions` and `connect_auth` wants its own.
    pub fn options(&self) -> git2::ProxyOptions<'static> {
        let mut options = git2::ProxyOptions::new();
        match (&self.outcome, &self.url) {
            (Outcome::Specified, Some(url)) => {
                options.url(url);
            }
            (Outcome::Auto, _) => {
                options.auto();
            }
            // Bypassed, and a Specified without a URL, which cannot
            // happen: the default IS `GIT_PROXY_NONE`.
            _ => {}
        }
        options
    }

    /// The proxy's own `host:port`, for the evidence of a 407 and for
    /// the log. Never carries a credential.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
}

/// A proxy joy will not use, with the sentence that says why.
///
/// It is raised BEFORE the contact, so no socket is opened and no
/// person is left reading libgit2's "invalid URL".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    pub sentence: String,
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.sentence)
    }
}

impl std::error::Error for Unsupported {}

/// What one contact's proxy left behind for the evidence: its name, and
/// whether joy put a credential into the URL it handed libgit2.
#[derive(Clone)]
struct Noted {
    name: String,
    credentialed: bool,
}

thread_local! {
    /// The proxy THIS thread's contact went through, for the evidence
    /// of D1.8c: a 407 names the proxy and never the forge. libgit2
    /// calls back on the contact's own thread, which is the same scope
    /// the certificate cell of [`super::certificates`] uses.
    static CURRENT: RefCell<Option<Noted>> = const { RefCell::new(None) };
}

/// The proxy of the contact running on this thread, if joy configured
/// one.
pub fn current() -> Option<String> {
    CURRENT.with(|cell| cell.borrow().as_ref().map(|noted| noted.name.clone()))
}

/// Whether the proxy URL this contact handed libgit2 carried a
/// credential, which is what decides whether libgit2's own words need
/// [`scrubbed`] before a person reads them.
pub fn carried_credential() -> bool {
    CURRENT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|noted| noted.credentialed)
    })
}

/// Forget what the last contact on this thread went through. Called by
/// the contact boundary before the work runs.
pub fn forget() {
    CURRENT.with(|cell| *cell.borrow_mut() = None);
}

pub(crate) fn note(name: Option<&str>, credentialed: bool) {
    CURRENT.with(|cell| {
        *cell.borrow_mut() = name.map(|name| Noted {
            name: name.to_string(),
            credentialed,
        })
    });
}

/// THE proxy options of one contact (D1.11).
///
/// `url` is the remote URL the contact travels over and `repo` the
/// repository it belongs to, or `None` for a clone, which has none yet.
pub fn options_for(url: &str, repo: Option<&git2::Repository>) -> Result<Proxy, Unsupported> {
    let config = repo
        .and_then(|r| r.config().ok())
        .or_else(|| git2::Config::open_default().ok())
        .and_then(|mut c| c.snapshot().ok());
    let remote = repo.and_then(|r| remote_name_for(r, url));
    // The same config the decision reads answers for the proxy
    // credential: a person who configures a proxy for one repository
    // configures its login there too.
    let mut credential = |proxy: &ProxyUrl| helper_credential(proxy, config.as_ref());
    let proxy = decide(
        url,
        config.as_ref(),
        remote.as_deref(),
        &Environment::of_this_process(),
        &mut credential,
    )?;
    note(proxy.name(), proxy.credentialed);
    match (proxy.outcome, proxy.name()) {
        (Outcome::Specified, Some(name)) => tracing::debug!(
            proxy = name,
            credentialed = proxy.credentialed,
            "contact goes through a proxy"
        ),
        (Outcome::Bypassed, _) => tracing::trace!("contact takes no proxy"),
        _ => {}
    }
    Ok(proxy)
}

/// The name of the remote that carries `url`, for `remote.<name>.proxy`
/// (remote.c:1105-1111). libgit2 reads it off the remote object; joy
/// has the URL and asks the repository which of its remotes that is.
fn remote_name_for(repo: &git2::Repository, url: &str) -> Option<String> {
    let names = repo.remotes().ok()?;
    names
        .iter()
        .flatten()
        .flatten()
        .find(|name| {
            repo.find_remote(name)
                .ok()
                .and_then(|remote| remote.url().ok().map(|configured| configured == url))
                .unwrap_or(false)
        })
        .map(str::to_string)
}

// ---- the environment, read once and handed in --------------------------

/// The four environment variables a proxy decision reads, lower case
/// first and upper case second, exactly as libgit2 does
/// (remote.c:1137-1160).
///
/// They are a parameter and not a read inside the decision so that the
/// decision itself is a function of its inputs: the process environment
/// is shared by every thread of a desktop app, and a test that sets one
/// would decide what another test sees.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    pub https_proxy: Option<String>,
    pub http_proxy: Option<String>,
    /// `ALL_PROXY`/`all_proxy`, which libgit2 never reads and git does
    /// (git-config http.proxy).
    pub all_proxy: Option<String>,
    pub no_proxy: Option<String>,
}

impl Environment {
    pub fn of_this_process() -> Environment {
        Environment {
            https_proxy: either("https_proxy", "HTTPS_PROXY"),
            http_proxy: either("http_proxy", "HTTP_PROXY"),
            all_proxy: either("all_proxy", "ALL_PROXY"),
            no_proxy: either("no_proxy", "NO_PROXY"),
        }
    }
}

/// The lower case name first, then the upper case one: libgit2's own
/// order, and the one curl and git follow.
fn either(lower: &str, upper: &str) -> Option<String> {
    std::env::var(lower)
        .ok()
        .or_else(|| std::env::var(upper).ok())
        .filter(|value| !value.is_empty())
}

// ---- the decision ------------------------------------------------------

/// Where a proxy came from, for the log and for the failure text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Config,
    Environment,
}

/// The whole decision, with the environment and the credential lookup
/// handed in so that both have a test.
fn decide(
    url: &str,
    config: Option<&git2::Config>,
    remote: Option<&str>,
    env: &Environment,
    credential: &mut dyn FnMut(&ProxyUrl) -> Option<(String, String)>,
) -> Result<Proxy, Unsupported> {
    let Some(target) = RemoteUrl::parse(url).filter(|t| t.transport.takes_helper()) else {
        // ssh, git:// and a path take no HTTP proxy. libgit2 would
        // ignore the options anyway; saying so here keeps the ssh
        // chain's failures free of a proxy that never applied.
        return Ok(Proxy::bypassed());
    };
    let port = target.port.unwrap_or(match target.transport {
        Transport::Https => 443,
        _ => 80,
    });
    // NO_PROXY first, and for EVERY source: this is the half libgit2
    // does not do (remote.c:1085-1133 never looks at no_proxy), and
    // doing it after the lookup would leave a config sourced proxy in
    // place for a host the person excluded.
    if let Some(list) = env.no_proxy.as_deref() {
        if no_proxy_matches(&target.host, port, list) {
            tracing::debug!(host = %target.host, "NO_PROXY excludes this host");
            return Ok(Proxy::bypassed());
        }
    }
    let found = match config.and_then(|config| config_proxy(config, remote, &target, port)) {
        Some(value) => Some((value, Source::Config)),
        None => env_proxy(&target, env).map(|value| (value, Source::Environment)),
    };
    let Some((value, source)) = found else {
        return Ok(Proxy::auto());
    };
    if value.trim().is_empty() {
        // git's own way of switching a proxy off for one repository or
        // one URL, and libgit2's too: an entry that is present and
        // empty ends the search (remote.c:1051-1068, http.c:336).
        tracing::debug!(?source, "the configuration turns the proxy off");
        return Ok(Proxy::bypassed());
    }
    let parsed = ProxyUrl::parse(value.trim())?;
    let (user, password) = match (&parsed.user, &parsed.password) {
        // What the person wrote wins, and is used as written.
        (Some(user), Some(password)) => (Some(user.clone()), Some(password.clone())),
        // A user name without a password is git's shape for "ask the
        // helper for the password of THIS user" (git-config http.proxy).
        _ => match credential(&parsed) {
            Some((user, password)) => (Some(user), Some(password)),
            None => (parsed.user.clone(), None),
        },
    };
    let credentialed = password.is_some();
    Ok(Proxy {
        outcome: Outcome::Specified,
        name: Some(parsed.name.clone()),
        url: Some(parsed.with_credentials(user.as_deref(), password.as_deref())),
        credentialed,
    })
}

/// The config half of libgit2's order, repeated so that joy KNOWS the
/// proxy (remote.c:1085-1133): `remote.<name>.proxy`, then
/// `http.<url>.proxy` from the full URL down the path to the bare host,
/// then `http.proxy`.
///
/// `Some("")` is an answer: an entry that exists and is empty turns the
/// proxy off and ends the search, which is how a repository switches a
/// global proxy off.
fn config_proxy(
    config: &git2::Config,
    remote: Option<&str>,
    target: &RemoteUrl,
    port: u16,
) -> Option<String> {
    if let Some(name) = remote.filter(|name| !name.is_empty()) {
        if let Ok(value) = config.get_string(&format!("remote.{name}.proxy")) {
            return Some(value);
        }
    }
    for key in url_config_keys(target, port) {
        if let Ok(value) = config.get_string(&format!("http.{key}.proxy")) {
            return Some(value);
        }
    }
    config.get_string("http.proxy").ok()
}

/// The `http.<url>.proxy` keys libgit2 tries, most specific first.
///
/// The key is `http.` plus `git_net_url_fmt` plus `.proxy`, and
/// `git_net_url_fmt` writes `scheme://[user[:password]@]host[:port]path`
/// with the port left out when it is the scheme's default
/// (net.c:1021-1055). `url_config_trim` then walks the path down: a
/// trailing slash goes first, otherwise the last segment does, and the
/// walk ends AFTER the key with the empty path, which is the bare host
/// (remote.c:1071-1083, :1112-1126). So `https://github.com/o/r.git`
/// asks five keys, ending at `https://github.com`.
fn url_config_keys(target: &RemoteUrl, port: u16) -> Vec<String> {
    let default_port = matches!(
        (target.transport, port),
        (Transport::Https, 443) | (Transport::Http, 80)
    );
    let host = if target.bracketed {
        format!("[{}]", target.host)
    } else {
        target.host.clone()
    };
    let user = match &target.user {
        Some(user) => format!("{user}@"),
        None => String::new(),
    };
    let authority = if default_port {
        format!("{}://{user}{host}", target.transport.protocol())
    } else {
        format!("{}://{user}{host}:{port}", target.transport.protocol())
    };
    let mut keys = Vec::new();
    let mut path = if target.path.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", target.path)
    };
    loop {
        keys.push(format!("{authority}{path}"));
        if path.is_empty() {
            break;
        }
        if path.ends_with('/') {
            path.pop();
        } else {
            while !path.is_empty() && !path.ends_with('/') {
                path.pop();
            }
        }
    }
    keys
}

/// The environment half: `https_proxy`/`http_proxy` per scheme as
/// libgit2 reads them (remote.c:1137-1160), then `ALL_PROXY`, which
/// libgit2 never reads and git does.
fn env_proxy(target: &RemoteUrl, env: &Environment) -> Option<String> {
    let per_scheme = match target.transport {
        Transport::Https => env.https_proxy.clone(),
        _ => env.http_proxy.clone(),
    };
    per_scheme.or_else(|| env.all_proxy.clone())
}

/// joy's own NO_PROXY evaluation (D1.11), which is ONE function for the
/// whole product (JOY-02A3-E4): `joy_forge_net::proxy::no_proxy_matches`
/// is this function, so a host the person excluded is excluded for a
/// git contact here and for a connector's REST call there, by the same
/// rule and with the same answer.
///
/// The implementation lives HERE and not in the shared network layer,
/// because that layer depends on this crate already: the refresh lock
/// of D2.6a takes `joy_core::util::file_lock`, which the design says
/// lands once (J4a, "no package after J4a carries a lock dependency of
/// its own for locking"). Two edges would be a cycle, and cargo says so
/// before rustc does. Nothing about the matcher needs libgit2, so this
/// direction costs a connector nothing it is forbidden to link (D2.1):
/// joy-core's default build carries no network transport at all, the
/// `forge-net` feature carries it.
///
/// The grammar is libgit2's (net.c:1070-1117): a comma separated list
/// of `*`, `*.domain`, `.domain`, `host` and `host:port`, with no CIDR
/// and no wildcard inside a name. The one difference is deliberate:
/// every entry is TRIMMED, because libgit2 compares the bytes as they
/// stand and `NO_PROXY="a.com, b.com"` therefore silently loses
/// `b.com`, which is the shape a person writes.
pub fn no_proxy_matches(host: &str, port: u16, list: &str) -> bool {
    list.split(',')
        .map(str::trim)
        .any(|pattern| pattern_matches(host, port, pattern))
}

fn pattern_matches(host: &str, port: u16, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    if pattern == "*" {
        return true;
    }
    let (wildcard, rest) = if let Some(rest) = pattern.strip_prefix("*.") {
        (true, rest)
    } else if let Some(rest) = pattern.strip_prefix('.') {
        (true, rest)
    } else {
        (false, pattern)
    };
    // An IPv6 pattern is written in brackets, and so is the host in a
    // URL; joy compares the bare addresses.
    let rest = rest.trim_start_matches('[');
    let (domain, wanted_port) = match rest.rsplit_once(':') {
        // `[::1]:8080` splits at the LAST colon, which is the port
        // separator; a bare IPv6 address has no port and its colons
        // belong to the address.
        Some((domain, tail)) if tail.chars().all(|c| c.is_ascii_digit()) && !tail.is_empty() => {
            match tail.parse::<u16>() {
                Ok(port) => (domain, Some(port)),
                // A port no contact can have: libgit2 compares the port
                // TEXT (net.c:1100-1103), so `acme.example:99999` matches
                // nothing there. It must not become "this pattern names
                // no port", which would bypass the proxy for the host on
                // every port.
                Err(_) => return false,
            }
        }
        _ => (rest, None),
    };
    let domain = domain.trim_end_matches(']');
    if domain.is_empty() {
        return false;
    }
    // A pattern's port MUST match when it names one (net.c:1100-1103).
    if let Some(wanted) = wanted_port {
        if wanted != port {
            return false;
        }
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if !wildcard {
        return host.eq_ignore_ascii_case(domain);
    }
    if host.len() < domain.len() {
        return false;
    }
    let suffix = &host[host.len() - domain.len()..];
    if !suffix.eq_ignore_ascii_case(domain) {
        return false;
    }
    // `*.domain` matches `domain` itself and `foo.domain`, and nothing
    // that merely ends in those letters (net.c:1109-1116).
    host.len() == domain.len() || host.as_bytes()[host.len() - domain.len() - 1] == b'.'
}

// ---- the proxy URL -----------------------------------------------------

/// A proxy URL joy will use, taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyUrl {
    pub scheme: String,
    /// `host:port`, the name a 407 carries.
    pub name: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

impl ProxyUrl {
    /// Read a proxy URL, or refuse it by name.
    ///
    /// A value with no scheme is http, as `git_net_url_parse_http`
    /// treats it (net.c:466-486). A scheme that is not http or https is
    /// refused here, before a socket is opened: libgit2 would speak
    /// HTTP CONNECT to it anyway (httpclient.c:686-700).
    pub fn parse(value: &str) -> Result<ProxyUrl, Unsupported> {
        let (scheme, rest) = match value.split_once("://") {
            Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
            None => ("http".to_string(), value),
        };
        if scheme != "http" && scheme != "https" {
            return Err(Unsupported {
                sentence: unsupported_sentence(&scheme, value),
            });
        }
        let with_scheme = format!("{scheme}://{rest}");
        let parsed = RemoteUrl::parse(&with_scheme).ok_or_else(|| Unsupported {
            sentence: format!(
                "joy cannot read the proxy {}; it expects http://host:port.",
                redacted(value)
            ),
        })?;
        if parsed.host.is_empty() {
            return Err(Unsupported {
                sentence: format!(
                    "joy cannot read the proxy {}; it names no host.",
                    redacted(value)
                ),
            });
        }
        let port = parsed
            .port
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        let host = if parsed.bracketed {
            format!("[{}]", parsed.host)
        } else {
            parsed.host.clone()
        };
        // The userinfo is split by joy, because `RemoteUrl` keeps the
        // whole `user:password` in one field: it parses REMOTES, where
        // a password in the URL is a shape joy never writes.
        let (user, password) = match parsed.user.as_deref() {
            Some(userinfo) => match userinfo.split_once(':') {
                Some((user, password)) => {
                    (Some(percent_decode(user)), Some(percent_decode(password)))
                }
                None => (Some(percent_decode(userinfo)), None),
            },
            None => (None, None),
        };
        Ok(ProxyUrl {
            scheme,
            name: format!("{host}:{port}"),
            user,
            password,
        })
    }

    /// The URL libgit2 gets, with the credential percent encoded:
    /// libgit2 percent DECODES the userinfo it parses (net.c:401-409),
    /// so a password with a `@` or a `:` in it has to arrive encoded or
    /// it is read as part of the host.
    fn with_credentials(&self, user: Option<&str>, password: Option<&str>) -> String {
        match (user, password) {
            (Some(user), Some(password)) => format!(
                "{}://{}:{}@{}",
                self.scheme,
                percent_encode(user),
                percent_encode(password),
                self.name
            ),
            (Some(user), None) => {
                format!("{}://{}@{}", self.scheme, percent_encode(user), self.name)
            }
            _ => format!("{}://{}", self.scheme, self.name),
        }
    }

    /// What joy asks its credential helper runner about: `protocol=http`
    /// and `host=<proxyhost>[:port]` (D1.11), whatever the proxy's own
    /// scheme is.
    ///
    /// The protocol is `http` for an `https://` proxy too, and that is
    /// the design's word: it is one login, to one machine in the middle,
    /// and a person who stored it once should not have to store it
    /// again because the proxy URL gained a `s`. libgit2 presents the
    /// userinfo the same way either way (http.c:141-152).
    fn credential_url(&self) -> String {
        format!("http://{}", self.name)
    }
}

/// The refusal of D1.11, which names the proxy and never its password.
fn unsupported_sentence(scheme: &str, value: &str) -> String {
    if scheme.starts_with("socks") {
        format!(
            "joy cannot use the SOCKS proxy {}; it supports HTTP and HTTPS proxies only.",
            redacted(value)
        )
    } else {
        format!(
            "joy cannot use the {scheme} proxy {}; it supports HTTP and HTTPS proxies only.",
            redacted(value)
        )
    }
}

/// A proxy URL with whatever userinfo it carried replaced, for a text a
/// person reads. Every sentence and every log line that names a proxy
/// goes through here or through [`Proxy::name`].
pub fn redacted(value: &str) -> String {
    let (scheme, rest) = match value.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), value),
    };
    match rest.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}<credential>@{host}"),
        None => format!("{scheme}{rest}"),
    }
}

/// Any URL in a text libgit2 wrote, with its userinfo taken out.
///
/// There is one libgit2 message that echoes the proxy URL joy built,
/// userinfo included: `git_error_set(GIT_ERROR_HTTP, "invalid URL:
/// '%s'", proxy)` (http.c:340-342), reached when `git_net_url_parse_http`
/// succeeds on that URL and `git_net_url_valid` then refuses it. joy
/// validates the host and the port itself and percent encodes the
/// userinfo, so this is a narrow door, but D1.11's promise is absolute
/// ("never appears in a log line or an error text") and the defence is
/// cheap. [`redacted`] does the same for one URL joy holds as a whole;
/// this does it for a sentence with a URL somewhere inside it.
pub fn scrubbed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, after) = rest.split_at(at + 3);
        out.push_str(head);
        // The authority runs to the path, the query, the fragment or
        // whatever punctuation the message wrapped the URL in.
        let end = after
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '/' | '?' | '#' | '\'' | '"' | '`' | '<' | '>')
            })
            .unwrap_or(after.len());
        let (authority, tail) = after.split_at(end);
        match authority.rsplit_once('@') {
            // The LAST `@`, because a percent decoded password may hold
            // one of its own.
            Some((_, host)) => {
                out.push_str("<credential>@");
                out.push_str(host);
            }
            None => out.push_str(authority),
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Everything but the unreserved set of RFC 3986 is encoded: joy has no
/// URL crate, and the whole point is that a `:` or an `@` in a password
/// must not be read as a delimiter.
fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The inverse, for the userinfo a person wrote into the configuration.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap_or_default();
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The proxy credential from joy's own helper runner (D1.3).
///
/// The host kind is [`HostKind::Background`] whatever the contact's own
/// kind is, on purpose: this lookup runs BEFORE the contact, where
/// nobody has been told why they are being asked, and a helper that
/// draws a window here would draw it on every open proxy too. The way
/// in for a person is the `proxy_auth` state's own next step (D1.8c),
/// which stores the answer under this same host.
fn helper_credential(proxy: &ProxyUrl, config: Option<&git2::Config>) -> Option<(String, String)> {
    let opened;
    let config = match config {
        Some(config) => config,
        None => {
            opened = git2::Config::open_default().ok()?;
            &opened
        }
    };
    let answer = super::credential_helper::get(
        config,
        &proxy.credential_url(),
        proxy.user.as_deref(),
        HostKind::Background,
    );
    match answer {
        Ok(Some(credential)) => Some((credential.username, credential.password)),
        Ok(None) => None,
        Err(e) => {
            // The helper's own sentence, with no credential in it.
            tracing::debug!(proxy = %proxy.name, detail = %e.detail, "no proxy credential");
            None
        }
    }
}

// ---- the Linux only CA escape hatch, read here, applied in lib.rs ------

/// One CA location a person or an operator configured, with the place
/// it came from so that a refusal can name it (D1.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaEntry {
    /// `ca_bundle`, `ca_dir`, `http.sslCAInfo` or `http.sslCAPath`.
    pub key: String,
    /// `~/.config/joy/forges.yaml` or `git config`.
    pub source: String,
    pub value: String,
    pub kind: CaKind,
}

/// A bundle is one file of concatenated certificates, a directory holds
/// one per file. They are two different OpenSSL settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaKind {
    Bundle,
    Directory,
}

/// The `ca_bundle` and `ca_dir` entries of `forges.yaml` (D2.5).
///
/// The setting is process global, so it cannot be per host: the FIRST
/// entry that carries one wins, and the log says which host it came
/// from. joy reads only these two keys here; the rest of the file
/// belongs to the connector (package J2).
pub fn forges_yaml_ca(file: &Path) -> Vec<CaEntry> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let entries: Vec<ForgeEntry> = match serde_yaml_ng::from_str(&text) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(file = %file.display(), error = %e, "forges.yaml could not be read");
            return Vec::new();
        }
    };
    let source = file.display().to_string();
    let mut found = Vec::new();
    for entry in entries {
        if let Some(value) = entry.ca_bundle.filter(|v| !v.trim().is_empty()) {
            if !found.iter().any(|e: &CaEntry| e.kind == CaKind::Bundle) {
                found.push(CaEntry {
                    key: "ca_bundle".to_string(),
                    source: source.clone(),
                    value,
                    kind: CaKind::Bundle,
                });
            }
        }
        if let Some(value) = entry.ca_dir.filter(|v| !v.trim().is_empty()) {
            if !found.iter().any(|e: &CaEntry| e.kind == CaKind::Directory) {
                found.push(CaEntry {
                    key: "ca_dir".to_string(),
                    source: source.clone(),
                    value,
                    kind: CaKind::Directory,
                });
            }
        }
    }
    found
}

/// The two keys of `forges.yaml` this module reads. Every other key of
/// D2.5 belongs to the connector, and serde ignores what it does not
/// name.
#[derive(Debug, serde::Deserialize)]
struct ForgeEntry {
    #[serde(default)]
    ca_bundle: Option<String>,
    #[serde(default)]
    ca_dir: Option<String>,
}

/// `http.sslCAInfo` and `http.sslCAPath` from git config: the ONE named
/// exception of D1.12, because that is the setting a corporate
/// workstation image already carries. libgit2 itself reads neither (no
/// hit for either key in the whole 1.9.6 tree).
pub fn git_config_ca(config: &git2::Config) -> Vec<CaEntry> {
    let mut found = Vec::new();
    if let Ok(value) = config.get_string("http.sslCAInfo") {
        if !value.trim().is_empty() {
            found.push(CaEntry {
                key: "http.sslCAInfo".to_string(),
                source: "git config".to_string(),
                value,
                kind: CaKind::Bundle,
            });
        }
    }
    if let Ok(value) = config.get_string("http.sslCAPath") {
        if !value.trim().is_empty() {
            found.push(CaEntry {
                key: "http.sslCAPath".to_string(),
                source: "git config".to_string(),
                value,
                kind: CaKind::Directory,
            });
        }
    }
    found
}

/// Which certificate store this build talks to. The escape hatch exists
/// for exactly one of them, because `GIT_OPT_SET_SSL_CERT_LOCATIONS` is
/// compiled only for OpenSSL and mbedTLS (settings.c:207-223).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustStore {
    /// Linux and every other OpenSSL build.
    OpenSsl,
    /// macOS: Secure Transport, the system anchors and the keychain
    /// trust settings.
    Keychain,
    /// Windows: WinHTTP and the Windows certificate store.
    WindowsStore,
}

impl TrustStore {
    /// The store THIS build talks to (libgit2-sys build.rs:257-269).
    pub fn of_this_build() -> TrustStore {
        if cfg!(target_os = "macos") {
            TrustStore::Keychain
        } else if cfg!(windows) {
            TrustStore::WindowsStore
        } else {
            TrustStore::OpenSsl
        }
    }
}

/// What to do with the configured CA locations on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaDecision {
    /// Nothing was configured, and the system store decides alone.
    Nothing,
    /// Apply these, once, before the first contact.
    Apply(Vec<CaEntry>),
    /// Refused by name, with one sentence per entry.
    Refused(Vec<String>),
}

/// The decision of D1.12 over the entries that were found.
///
/// `forges.yaml` wins over git config for the same kind: it is joy's
/// own file and the operator ships it, while `http.sslCAInfo` is
/// whatever the workstation image happened to carry.
pub fn ca_decision(entries: Vec<CaEntry>, store: TrustStore) -> CaDecision {
    if entries.is_empty() {
        return CaDecision::Nothing;
    }
    match store {
        TrustStore::OpenSsl => {
            let mut applied: Vec<CaEntry> = Vec::new();
            for entry in entries {
                if !applied.iter().any(|kept| kept.kind == entry.kind) {
                    applied.push(entry);
                }
            }
            CaDecision::Apply(applied)
        }
        TrustStore::Keychain | TrustStore::WindowsStore => {
            CaDecision::Refused(entries.iter().map(|e| ca_refusal(e, store)).collect())
        }
    }
}

/// The sentence a refused entry gets, naming the entry and the store
/// that decides instead (D1.12).
fn ca_refusal(entry: &CaEntry, store: TrustStore) -> String {
    let store_sentence = match store {
        TrustStore::Keychain => {
            "add your organisation's CA to the login or System keychain and mark it trusted"
        }
        TrustStore::WindowsStore => {
            "your administrator must install the CA in the Windows certificate store"
        }
        TrustStore::OpenSsl => "install the CA with update-ca-certificates",
    };
    format!(
        "joy ignores {} from {}: it does not apply here, because this system checks certificates \
         against its own store. To trust an internal CA, {store_sentence}.",
        entry.key, entry.source
    )
}

#[cfg(test)]
mod tests;
