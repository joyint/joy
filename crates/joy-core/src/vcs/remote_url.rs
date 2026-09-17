// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The remote URL, taken apart once for everyone who needs a piece of
//! it: the credential helper runner needs protocol, host, port and
//! path, the ssh chain needs user, host and port, and the credential
//! shape needs the host alone.
//!
//! joy parses this itself rather than reaching for a URL crate,
//! because the form git remotes use most is not a URL at all: the
//! scp-like `git@github.com:owner/repo.git` has no scheme and its
//! colon separates a PATH, not a port. Getting that wrong turns
//! `owner/repo.git` into a port number and the host into a nonsense
//! key for the throttle, the transport memory and the helper lookup.

/// How a remote is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Https,
    Http,
    Ssh,
    /// The unauthenticated git protocol (port 9418).
    Git,
    /// A path or a `file://` URL: no host, no credentials.
    Local,
}

impl Transport {
    /// The word the credential helper protocol uses (`protocol=`).
    pub fn protocol(self) -> &'static str {
        match self {
            Transport::Https => "https",
            Transport::Http => "http",
            Transport::Ssh => "ssh",
            Transport::Git => "git",
            Transport::Local => "file",
        }
    }

    /// Whether a credential helper answers for this transport at all.
    /// Helpers speak http and https; an ssh remote is served by the
    /// ssh chain (design D1.2).
    pub fn takes_helper(self) -> bool {
        matches!(self, Transport::Https | Transport::Http)
    }
}

/// A remote URL in pieces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUrl {
    pub transport: Transport,
    /// The user name in front of the host, if the URL carries one.
    pub user: Option<String>,
    /// The bare host, without brackets for an IPv6 literal and
    /// lowercased, because host names are case-insensitive and every
    /// key joy builds from them (throttle, memory, cache) must match.
    pub host: String,
    /// The port the URL names, if any. A scp-like remote never has
    /// one: its colon introduces the path.
    pub port: Option<u16>,
    /// The path, without its leading slash.
    pub path: String,
    /// Whether the host was written as an IP literal in brackets.
    pub bracketed: bool,
}

impl RemoteUrl {
    /// Take a remote URL apart; `None` for a local path, which has no
    /// host and needs no credential.
    pub fn parse(url: &str) -> Option<RemoteUrl> {
        let url = url.trim();
        if url.is_empty() {
            return None;
        }
        let (transport, rest) = match split_scheme(url) {
            Some(("https", rest)) => (Transport::Https, rest),
            Some(("http", rest)) => (Transport::Http, rest),
            Some(("ssh", rest)) => (Transport::Ssh, rest),
            Some(("git+ssh", rest)) => (Transport::Ssh, rest),
            Some(("git", rest)) => (Transport::Git, rest),
            Some(("file", rest)) => return local(rest),
            Some(_) => return None,
            // No scheme: either the scp-like ssh form or a plain path.
            None => match scp_like(url) {
                Some(parsed) => return Some(parsed),
                None => return None,
            },
        };
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => (rest, ""),
        };
        let (user, host_port) = match authority.rsplit_once('@') {
            Some((user, host)) => (non_empty(user), host),
            None => (None, authority),
        };
        let (host, port, bracketed) = split_host_port(host_port)?;
        Some(RemoteUrl {
            transport,
            user,
            host,
            port,
            path: path.to_string(),
            bracketed,
        })
    }

    /// The host the way the credential helper protocol wants it: with
    /// the port when the URL carried one, in brackets when it is an
    /// IPv6 literal. git2 leaves an IP literal host out entirely
    /// (cred.rs:214-220), which makes a helper answer for the wrong
    /// entry or for none.
    pub fn host_field(&self) -> String {
        let host = if self.bracketed {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        match self.port {
            Some(port) => format!("{host}:{port}"),
            None => host,
        }
    }

    /// `<protocol>://<host>[:<port>]`, the middle config key of
    /// design D1.3 and the key of the per-host credential cache.
    pub fn host_key(&self) -> String {
        format!("{}://{}", self.transport.protocol(), self.host_field())
    }
}

/// The transport a URL's SCHEME names, for a text [`RemoteUrl::parse`]
/// refused: `https://host:99999/o/r.git` carries no readable authority,
/// but it is still an https remote, and every caller that only needs
/// the branch (D1.8a) should hear that rather than "local". `None` for
/// a text with no scheme and for a scheme joy does not speak.
pub fn scheme_transport(url: &str) -> Option<Transport> {
    match split_scheme(url.trim())? {
        ("https", _) => Some(Transport::Https),
        ("http", _) => Some(Transport::Http),
        ("ssh", _) | ("git+ssh", _) => Some(Transport::Ssh),
        ("git", _) => Some(Transport::Git),
        ("file", _) => Some(Transport::Local),
        _ => None,
    }
}

/// A `file://` URL is local; joy keeps the path and stops there.
fn local(rest: &str) -> Option<RemoteUrl> {
    let path = rest.strip_prefix("//").unwrap_or(rest);
    Some(RemoteUrl {
        transport: Transport::Local,
        user: None,
        host: String::new(),
        port: None,
        path: path.to_string(),
        bracketed: false,
    })
}

/// `scheme://rest`, lowercased; `None` when there is no scheme.
fn split_scheme(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+')
    {
        return None;
    }
    // The scheme is compared lowercased; the slice itself is returned
    // as written, so the match below is on a lowered copy.
    let lowered: &'static str = match scheme.to_ascii_lowercase().as_str() {
        "https" => "https",
        "http" => "http",
        "ssh" => "ssh",
        "git+ssh" => "git+ssh",
        "git" => "git",
        "file" => "file",
        _ => return Some(("", rest)),
    };
    Some((lowered, rest))
}

/// The scp-like form `[user@]host:path`, which is ssh. A Windows drive
/// letter (`C:\repo`) and a plain path are not.
///
/// Two shapes carry a bracket, and libgit2 reads both
/// (`git_net_url_parse_scp`, net.c:661-806), so joy reads both:
/// `git@[::1]:owner/repo.git`, where the bracket holds an IPv6
/// address, and `[git@host:2222]:owner/repo.git`, the only scp-like
/// shape that carries a PORT, which design D1.5 names by name. Read as
/// a plain `host:path` the second one yields the host `host` and the
/// path `2222]:owner/repo.git`, and every key joy builds from a remote
/// (the throttle, the twin, the helper lookup) is then built from
/// nonsense.
fn scp_like(url: &str) -> Option<RemoteUrl> {
    if url.starts_with('/') || url.starts_with('.') || url.starts_with('~') {
        return None;
    }
    let (authority, path) = split_authority(url)?;
    if authority.is_empty() || path.starts_with('\\') {
        return None;
    }
    // `C:/src/repo` and `C:\src\repo`: a single letter before the
    // colon is a drive, not a host.
    if authority.len() == 1 && authority.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    // `[user@host:port]` wraps user, host and port together; a bracket
    // holding an IPv6 address and nothing else is the address itself,
    // which is how libgit2 tells the two apart (`is_ipv6`,
    // net.c:621-644).
    let (authority, ported) = match authority
        .strip_prefix('[')
        .and_then(|inside| inside.strip_suffix(']'))
    {
        Some(inside) if !is_ipv6_text(inside) => (inside, true),
        _ => (authority, false),
    };
    let (user, host) = match authority.rsplit_once('@') {
        Some((user, host)) => (non_empty(user), host),
        None => (None, authority),
    };
    if host.is_empty() || host.contains('/') {
        return None;
    }
    let (host, port, bracketed) = split_host_port(host)?;
    Some(RemoteUrl {
        transport: Transport::Ssh,
        user,
        host,
        // Without the brackets the scp-like form cannot carry a port at
        // all; ssh reads it from the config, and so does joy.
        port: port.filter(|_| ported),
        path: path.to_string(),
        bracketed,
    })
}

/// The authority and the path of an scp-like remote, split at the
/// colon that introduces the path: the first one OUTSIDE every
/// bracket. A colon inside a bracket belongs to an IPv6 address or to
/// the port of the bracketed form, and libgit2 skips those the same
/// way, with a bracket counter (net.c:661-806).
fn split_authority(url: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (at, byte) in url.bytes().enumerate() {
        match byte {
            b'[' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b':' if depth == 0 => return Some((&url[..at], &url[at + 1..])),
            _ => {}
        }
    }
    None
}

/// Whether the text between brackets is an IPv6 address: hex digits
/// and colons, more than one colon. libgit2's own test (net.c:621-644).
fn is_ipv6_text(inside: &str) -> bool {
    inside.chars().filter(|c| *c == ':').count() > 1
        && inside.chars().all(|c| c == ':' || c.is_ascii_hexdigit())
}

/// `host`, `host:port`, `[v6]` or `[v6]:port`.
fn split_host_port(text: &str) -> Option<(String, Option<u16>, bool)> {
    if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port.parse().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        return Some((host.to_ascii_lowercase(), port, true));
    }
    match text.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            Some((host.to_ascii_lowercase(), Some(port.parse().ok()?), false))
        }
        _ if text.is_empty() => None,
        _ => Some((text.to_ascii_lowercase(), None, false)),
    }
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scp_like_form_is_ssh_and_its_colon_is_a_path() {
        let parsed = RemoteUrl::parse("git@github.com:owner/repo.git").unwrap();
        assert_eq!(parsed.transport, Transport::Ssh);
        assert_eq!(parsed.user.as_deref(), Some("git"));
        assert_eq!(parsed.host, "github.com");
        assert_eq!(parsed.port, None);
        assert_eq!(parsed.path, "owner/repo.git");
    }

    /// The bracketed form design D1.5 names by name. Read as a plain
    /// `host:path` it yields the host `git.example.com` with the path
    /// `2222]:owner/repo.git`, and every key joy builds from a remote
    /// is then built from nonsense.
    #[test]
    fn the_bracketed_scp_form_carries_a_port_and_the_path_after_it() {
        let parsed = RemoteUrl::parse("[git@git.example.com:2222]:owner/repo.git").unwrap();
        assert_eq!(parsed.transport, Transport::Ssh);
        assert_eq!(parsed.user.as_deref(), Some("git"));
        assert_eq!(parsed.host, "git.example.com");
        assert_eq!(parsed.port, Some(2222));
        assert_eq!(parsed.path, "owner/repo.git");
        assert_eq!(parsed.host_key(), "ssh://git.example.com:2222");
        // Without a user, and with an IPv6 address inside.
        let bare = RemoteUrl::parse("[git.example.com:2222]:o/r.git").unwrap();
        assert_eq!(bare.host, "git.example.com");
        assert_eq!(bare.port, Some(2222));
        assert_eq!(bare.user, None);
        let v6 = RemoteUrl::parse("[git@[2001:db8::1]:2222]:o/r.git").unwrap();
        assert_eq!(v6.host, "2001:db8::1");
        assert_eq!(v6.port, Some(2222));
        assert!(v6.bracketed);
        assert_eq!(v6.host_field(), "[2001:db8::1]:2222");
    }

    /// A bracket that holds an address and nothing else is the
    /// address, and the colon after it is the path separator, which is
    /// how libgit2 tells the two bracketed shapes apart (`is_ipv6`,
    /// net.c:621-644).
    #[test]
    fn an_ipv6_address_in_the_scp_form_is_an_address_and_not_a_port() {
        let parsed = RemoteUrl::parse("[2001:db8::1]:owner/repo.git").unwrap();
        assert_eq!(parsed.transport, Transport::Ssh);
        assert_eq!(parsed.host, "2001:db8::1");
        assert_eq!(parsed.port, None);
        assert_eq!(parsed.path, "owner/repo.git");
        assert!(parsed.bracketed);
        let with_user = RemoteUrl::parse("git@[2001:db8::1]:owner/repo.git").unwrap();
        assert_eq!(with_user.host, "2001:db8::1");
        assert_eq!(with_user.user.as_deref(), Some("git"));
        assert_eq!(with_user.port, None);
        assert_eq!(with_user.path, "owner/repo.git");
    }

    #[test]
    fn an_ssh_url_carries_its_port() {
        let parsed = RemoteUrl::parse("ssh://git@git.example.com:2222/joyint/joy.git").unwrap();
        assert_eq!(parsed.transport, Transport::Ssh);
        assert_eq!(parsed.port, Some(2222));
        assert_eq!(parsed.host, "git.example.com");
        assert_eq!(parsed.path, "joyint/joy.git");
    }

    #[test]
    fn the_host_field_carries_the_port_and_the_brackets() {
        let parsed = RemoteUrl::parse("https://gitea.example.com:8443/o/r.git").unwrap();
        assert_eq!(parsed.host_field(), "gitea.example.com:8443");
        assert_eq!(parsed.host_key(), "https://gitea.example.com:8443");
        let plain = RemoteUrl::parse("https://github.com/o/r.git").unwrap();
        assert_eq!(plain.host_field(), "github.com");
        assert_eq!(plain.host_key(), "https://github.com");
        let v6 = RemoteUrl::parse("https://[2001:db8::1]:8443/o/r.git").unwrap();
        assert_eq!(v6.host, "2001:db8::1");
        assert_eq!(v6.host_field(), "[2001:db8::1]:8443");
    }

    #[test]
    fn an_ip_literal_host_is_a_host_like_any_other() {
        let parsed = RemoteUrl::parse("https://10.0.0.7/o/r.git").unwrap();
        assert_eq!(parsed.host, "10.0.0.7");
        assert_eq!(parsed.host_field(), "10.0.0.7");
    }

    #[test]
    fn the_host_is_lowercased_so_every_key_matches() {
        let parsed = RemoteUrl::parse("https://GitHub.COM/o/r.git").unwrap();
        assert_eq!(parsed.host, "github.com");
    }

    #[test]
    fn a_local_path_is_local_and_a_drive_letter_is_not_a_host() {
        assert_eq!(RemoteUrl::parse("/srv/git/repo.git"), None);
        assert_eq!(RemoteUrl::parse("../sibling.git"), None);
        assert_eq!(RemoteUrl::parse("C:\\src\\repo"), None);
        assert_eq!(RemoteUrl::parse("C:/src/repo"), None);
        let file = RemoteUrl::parse("file:///srv/git/repo.git").unwrap();
        assert_eq!(file.transport, Transport::Local);
        assert_eq!(file.host, "");
    }

    #[test]
    fn only_http_transports_take_a_helper() {
        assert!(Transport::Https.takes_helper());
        assert!(Transport::Http.takes_helper());
        assert!(!Transport::Ssh.takes_helper());
        assert!(!Transport::Local.takes_helper());
    }

    #[test]
    fn a_user_in_the_url_survives() {
        let parsed = RemoteUrl::parse("https://someone@gitlab.com/o/r.git").unwrap();
        assert_eq!(parsed.user.as_deref(), Some("someone"));
        assert_eq!(parsed.host, "gitlab.com");
    }

    /// The scheme alone still names a transport for a URL the full
    /// parser refuses, so a broken remote keeps the branch it belongs
    /// to instead of falling to "local".
    #[test]
    fn the_scheme_names_a_transport_even_when_the_rest_is_unreadable() {
        assert_eq!(
            scheme_transport("https://host.example.com:99999/o/r.git"),
            Some(Transport::Https)
        );
        assert_eq!(
            scheme_transport("HTTP://host.example.com/o/r"),
            Some(Transport::Http)
        );
        assert_eq!(
            scheme_transport("git+ssh://host.example.com/o/r"),
            Some(Transport::Ssh)
        );
        assert_eq!(
            scheme_transport("git://host.example.com/o/r"),
            Some(Transport::Git)
        );
        assert_eq!(
            scheme_transport("file:///srv/repo.git"),
            Some(Transport::Local)
        );
        // A scheme joy does not speak, and a text with no scheme.
        assert_eq!(scheme_transport("ftps://host.example.com/o/r"), None);
        assert_eq!(scheme_transport("git@github.com:o/r.git"), None);
        assert_eq!(scheme_transport("/srv/repo.git"), None);
    }
}
