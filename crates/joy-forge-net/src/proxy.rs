// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Which proxy the connector's own HTTP client uses (D1.11, applied to
//! the connector by D2.8: "the client honours the same proxy sources").
//!
//! The engine can hand libgit2 `ProxyOptions::auto()` and let it walk
//! its own order. The connector has no libgit2, so the same order is
//! walked here, with the two corrections D1.11 names:
//!
//! - `NO_PROXY` is joy's own matcher and applies to a proxy from git
//!   config too, because libgit2 applies it to the environment branch
//!   only while git applies it always. Entries are trimmed, because
//!   libgit2 does not trim and `NO_PROXY="a.com, b.com"` silently loses
//!   `b.com` there. There is ONE such matcher for the whole product
//!   ([`no_proxy_matches`], JOY-02A3-E4): the engine's
//!   `joy_core::vcs::proxy::no_proxy_matches` is this function, so an
//!   excluded host is excluded for a git contact and for a REST call
//!   by the same rule. It sits here and not in the engine because a
//!   connector must not link libgit2 (D2.1).
//! - `ALL_PROXY` / `all_proxy` is read, because libgit2 never reads it
//!   and git does.
//!
//! A SOCKS proxy is refused by name instead of failing obscurely, with
//! the sentence the engine uses.
//!
//! One source of D1.11 is deliberately NOT here yet: the proxy password
//! that lives in the machine's credential helper. D1.11 resolves it
//! "through the helper runner of D1.3" and builds
//! `http://user:pass@proxy:port` in memory, and that runner is J4p's
//! work in the engine. Until it exists, the connector uses the userinfo
//! the proxy URL already carries, which is the same amount a person has
//! today; when J4p lands, the helper answer is fed in here in one place
//! ([`choose`]'s result) and nothing else changes.

use crate::gitconfig::GitConfig;

/// What the client does with a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyChoice {
    /// No proxy: nothing was configured, or `NO_PROXY` covers the host.
    Direct,
    /// This proxy URL, userinfo included where the source carried it.
    Proxy(String),
}

/// A proxy joy will not use, with the sentence a person reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRefusal {
    pub message: String,
}

impl std::fmt::Display for ProxyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProxyRefusal {}

/// Where the values come from. The tests hand in a map instead of the
/// process environment; nothing else differs.
pub trait EnvSource {
    fn var(&self, name: &str) -> Option<String>;
}

/// The process environment.
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.trim().is_empty())
    }
}

/// The proxy for `url`, decided from git config and the environment.
///
/// `url` is the full request URL; the host inside it is what `NO_PROXY`
/// is matched against and what `http.<url>.proxy` is looked up for.
pub fn choose(
    url: &str,
    config: &GitConfig,
    env: &dyn EnvSource,
) -> Result<ProxyChoice, ProxyRefusal> {
    let host = crate::url::host_of(url).unwrap_or_default();
    let port = port_of(url);
    if let Some(list) = env.var("NO_PROXY").or_else(|| env.var("no_proxy")) {
        if no_proxy_matches(&host, port, &list) {
            return Ok(ProxyChoice::Direct);
        }
    }
    let Some(candidate) = from_config(url, &host, config).or_else(|| from_env(url, env)) else {
        return Ok(ProxyChoice::Direct);
    };
    // git treats an empty value as "no proxy for this URL".
    if candidate.trim().is_empty() {
        return Ok(ProxyChoice::Direct);
    }
    check_scheme(&candidate)?;
    Ok(ProxyChoice::Proxy(candidate))
}

/// `http.<url>.proxy` from the full URL down the path to the bare host,
/// then `http.proxy`. The walk is libgit2's own (remote.c:1085-1194);
/// `remote.<name>.proxy` is not in it, because the connector asks an
/// API and has no remote name.
fn from_config(url: &str, host: &str, config: &GitConfig) -> Option<String> {
    for key in url_keys(url, host) {
        if let Some(value) = config.get_sub("http", &key, "proxy") {
            return Some(value.to_string());
        }
    }
    config.get("http", "proxy").map(str::to_string)
}

/// The `http.<url>` keys git tries, longest first: the full URL, then
/// each parent path, then the bare host, each with and without the
/// trailing slash git writes in its examples.
fn url_keys(url: &str, host: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let trimmed = url.trim_end_matches('/');
    let mut rest = trimmed;
    loop {
        keys.push(rest.to_string());
        keys.push(format!("{rest}/"));
        match rest.rfind('/') {
            // stop above the scheme's own "//"
            Some(index) if !rest[..index].ends_with(':') && index > 0 => rest = &rest[..index],
            _ => break,
        }
    }
    keys.push(host.to_string());
    keys.push(format!("{host}/"));
    keys
}

/// The environment, in git's order: the scheme's own variable in both
/// cases, then `ALL_PROXY`, which libgit2 never reads.
fn from_env(url: &str, env: &dyn EnvSource) -> Option<String> {
    let secure = url
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("https://");
    let names: &[&str] = if secure {
        &["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"]
    } else {
        &["http_proxy", "HTTP_PROXY"]
    };
    for name in names {
        if let Some(value) = env.var(name) {
            return Some(value);
        }
    }
    env.var("ALL_PROXY").or_else(|| env.var("all_proxy"))
}

/// libgit2 parses every proxy URL as an HTTP proxy and always speaks
/// HTTP CONNECT (httpclient.c:686-700), so a SOCKS proxy is refused by
/// name rather than attempted. The connector says the same sentence the
/// engine says (D1.11, D2.8).
fn check_scheme(proxy: &str) -> Result<(), ProxyRefusal> {
    let scheme = proxy
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    match scheme.as_deref() {
        // git's own default for a bare `host:port` is an HTTP proxy
        None | Some("http") | Some("https") => Ok(()),
        Some(other) if other.starts_with("socks") => Err(ProxyRefusal {
            message: format!(
                "joy cannot use the SOCKS proxy {}; it supports HTTP and HTTPS proxies only.",
                redact(proxy)
            ),
        }),
        Some(_) => Err(ProxyRefusal {
            message: format!(
                "joy cannot use the proxy {}; it supports HTTP and HTTPS proxies only.",
                redact(proxy)
            ),
        }),
    }
}

/// A proxy URL as it may appear in a message: without its userinfo. A
/// proxy password never reaches a log line or an error text (D1.11).
pub fn redact(proxy: &str) -> String {
    let (scheme, rest) = match proxy.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), proxy),
    };
    match rest.split_once('@') {
        Some((_userinfo, host)) => format!("{scheme}{host}"),
        None => format!("{scheme}{rest}"),
    }
}

/// THE NO_PROXY matcher (D1.11), and there is one: the engine's
/// `joy_core::vcs::proxy::no_proxy_matches` is this function, so a host
/// the person excluded is excluded for a git contact and for a
/// connector's REST call by the same rule. It lives on this side of the
/// two because a connector must not link libgit2 (D2.1) while the
/// engine may depend on the shared network layer.
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

/// The port a contact to `url` really opens, which is what a `host:port`
/// entry of NO_PROXY is compared against: the one the URL names, or the
/// scheme's own. The engine derives it the same way (D1.11), so
/// `NO_PROXY="api.acme.example:443"` excludes the host on both sides.
fn port_of(url: &str) -> u16 {
    let secure = url
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("https://");
    let default = if secure { 443 } else { 80 };
    let Some(rest) = url.split_once("://").map(|(_, rest)| rest) else {
        return default;
    };
    let Some(authority) = rest.split(['/', '?']).next() else {
        return default;
    };
    let Some(authority) = authority.rsplit('@').next() else {
        return default;
    };
    authority
        .rsplit_once(':')
        .and_then(|(_, tail)| tail.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Map(HashMap<&'static str, &'static str>);

    impl Map {
        fn new(pairs: &[(&'static str, &'static str)]) -> Self {
            Map(pairs.iter().copied().collect())
        }
    }

    impl EnvSource for Map {
        fn var(&self, name: &str) -> Option<String> {
            self.0
                .get(name)
                .map(|v| v.to_string())
                .filter(|v| !v.trim().is_empty())
        }
    }

    fn empty() -> GitConfig {
        GitConfig::from_text("")
    }

    #[test]
    fn nothing_configured_is_a_direct_contact() {
        assert_eq!(
            choose("https://api.github.com/user", &empty(), &Map::new(&[])).unwrap(),
            ProxyChoice::Direct
        );
    }

    #[test]
    fn the_environment_is_read_in_gits_order_including_all_proxy() {
        let env = Map::new(&[("ALL_PROXY", "http://all.example:3128")]);
        assert_eq!(
            choose("https://api.github.com/user", &empty(), &env).unwrap(),
            ProxyChoice::Proxy("http://all.example:3128".into())
        );
        let env = Map::new(&[
            ("ALL_PROXY", "http://all.example:3128"),
            ("https_proxy", "http://secure.example:3128"),
        ]);
        assert_eq!(
            choose("https://api.github.com/user", &empty(), &env).unwrap(),
            ProxyChoice::Proxy("http://secure.example:3128".into())
        );
    }

    /// D1.11's own example: libgit2 loses `b.com` because it does not
    /// trim. joy trims, so a contact to b.com bypasses the proxy.
    #[test]
    fn no_proxy_entries_are_trimmed() {
        let env = Map::new(&[
            ("https_proxy", "http://proxy.example:3128"),
            ("NO_PROXY", "a.com, b.com"),
        ]);
        assert_eq!(
            choose("https://b.com/api", &empty(), &env).unwrap(),
            ProxyChoice::Direct
        );
        assert_eq!(
            choose("https://c.com/api", &empty(), &env).unwrap(),
            ProxyChoice::Proxy("http://proxy.example:3128".into())
        );
    }

    #[test]
    fn no_proxy_knows_the_wildcard_the_suffix_and_the_port() {
        assert!(no_proxy_matches("anything.example", 443, "*"));
        assert!(no_proxy_matches("git.example.com", 443, "*.example.com"));
        // `*.domain` covers the domain itself (net.c:1109-1116), the
        // reading the engine has always had
        assert!(no_proxy_matches("example.com", 443, "*.example.com"));
        assert!(no_proxy_matches("git.example.com", 443, ".example.com"));
        assert!(no_proxy_matches("example.com", 443, ".example.com"));
        assert!(!no_proxy_matches("git.example.com", 443, "example.com"));
        assert!(!no_proxy_matches("notexample.com", 443, "*.example.com"));
        assert!(no_proxy_matches("GIT.example.com", 443, "git.EXAMPLE.com"));
        assert!(no_proxy_matches(
            "git.example.com",
            8443,
            "git.example.com:8443"
        ));
        assert!(!no_proxy_matches(
            "git.example.com",
            443,
            "git.example.com:8443"
        ));
        // no CIDR, here as in the engine
        assert!(!no_proxy_matches("10.0.0.7", 443, "10.0.0.0/8"));
        assert!(no_proxy_matches("10.0.0.7", 443, "10.0.0.7"));
        assert!(!no_proxy_matches("acme.example", 443, ""));
        assert!(!no_proxy_matches("acme.example", 443, ",,"));
    }

    /// The bug JOY-02A3-E4 names: an entry whose port is no port at all
    /// used to parse as `None`, which read as "this entry names no
    /// port" and bypassed the proxy for that host on EVERY port. It
    /// matches nothing now, and the rest of the list still counts.
    #[test]
    fn a_port_that_is_not_a_port_never_bypasses_the_proxy() {
        assert!(!no_proxy_matches("acme.example", 443, "acme.example:99999"));
        assert!(!no_proxy_matches("acme.example", 99, "acme.example:99999"));
        assert!(no_proxy_matches("b.com", 443, "acme.example:99999, b.com"));

        let env = Map::new(&[
            ("https_proxy", "http://proxy.example:3128"),
            ("NO_PROXY", "acme.example:99999"),
        ]);
        assert_eq!(
            choose("https://acme.example/api", &empty(), &env).unwrap(),
            ProxyChoice::Proxy("http://proxy.example:3128".into()),
            "a port no contact can have must not turn the proxy off"
        );
    }

    /// A `host:port` entry is compared against the port the contact
    /// really opens, and an https URL without a port opens 443. The
    /// engine derives the port the same way, so both sides answer the
    /// same for the same NO_PROXY.
    #[test]
    fn the_schemes_own_port_is_what_an_entrys_port_is_compared_against() {
        let env = Map::new(&[
            ("https_proxy", "http://proxy.example:3128"),
            ("NO_PROXY", "api.acme.example:443"),
        ]);
        assert_eq!(
            choose("https://api.acme.example/v1/user", &empty(), &env).unwrap(),
            ProxyChoice::Direct
        );
        assert_eq!(
            choose("https://api.acme.example:8443/v1/user", &empty(), &env).unwrap(),
            ProxyChoice::Proxy("http://proxy.example:3128".into())
        );
    }

    /// A proxy from git config is subject to NO_PROXY too, which is the
    /// correction of D1.11 against libgit2.
    #[test]
    fn a_config_proxy_also_bows_to_no_proxy() {
        let config = GitConfig::from_text("[http]\n proxy = http://proxy.example:3128\n");
        let env = Map::new(&[("NO_PROXY", "internal.example")]);
        assert_eq!(
            choose("https://internal.example/api", &config, &env).unwrap(),
            ProxyChoice::Direct
        );
    }

    #[test]
    fn the_most_specific_url_key_of_git_config_wins() {
        let config = GitConfig::from_text(
            "[http]\n proxy = http://general.example:3128\n[http \"https://git.acme.test/\"]\n proxy = http://special.example:8080\n",
        );
        assert_eq!(
            choose("https://git.acme.test/api/v3/user", &config, &Map::new(&[])).unwrap(),
            ProxyChoice::Proxy("http://special.example:8080".into())
        );
        assert_eq!(
            choose("https://other.test/x", &config, &Map::new(&[])).unwrap(),
            ProxyChoice::Proxy("http://general.example:3128".into())
        );
    }

    #[test]
    fn git_config_wins_over_the_environment() {
        let config = GitConfig::from_text("[http]\n proxy = http://config.example:3128\n");
        let env = Map::new(&[("https_proxy", "http://env.example:3128")]);
        assert_eq!(
            choose("https://api.github.com/user", &config, &env).unwrap(),
            ProxyChoice::Proxy("http://config.example:3128".into())
        );
    }

    #[test]
    fn a_socks_proxy_is_refused_by_name_and_never_attempted() {
        let env = Map::new(&[("ALL_PROXY", "socks5://socks.example:1080")]);
        let refusal = choose("https://api.github.com/user", &empty(), &env).unwrap_err();
        assert_eq!(
            refusal.message,
            "joy cannot use the SOCKS proxy socks5://socks.example:1080; it supports HTTP and HTTPS proxies only."
        );
    }

    #[test]
    fn a_proxy_password_never_appears_in_a_message() {
        assert_eq!(
            redact("http://user:s3cr3t@proxy.example:3128"),
            "http://proxy.example:3128"
        );
        let env = Map::new(&[("ALL_PROXY", "socks5://user:s3cr3t@socks.example:1080")]);
        let refusal = choose("https://api.github.com/user", &empty(), &env).unwrap_err();
        assert!(!refusal.message.contains("s3cr3t"), "{}", refusal.message);
    }

    #[test]
    fn an_empty_value_disables_the_proxy_for_that_url() {
        let config = GitConfig::from_text(
            "[http]\n proxy = http://general.example:3128\n[http \"https://git.acme.test/\"]\n proxy =\n",
        );
        assert_eq!(
            choose("https://git.acme.test/api", &config, &Map::new(&[])).unwrap(),
            ProxyChoice::Direct
        );
    }
}
