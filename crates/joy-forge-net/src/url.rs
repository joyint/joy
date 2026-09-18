// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The two readings of a remote URL every connector needs: which host
//! is this, and which repository path is this.
//!
//! They lived three times over, once per forge crate, and drifted:
//! this is the one copy (JOY-0298-E4). It stays deliberately small.
//! joy-core has the full remote URL parser (JOY-02A2-27); a connector
//! only ever sees the string a caller hands it and answers two
//! questions about it.

/// The host part of a git remote URL, lowercased, port stripped, and
/// without the brackets an IPv6 literal is written in.
///
/// Handles the three wire forms: `git@host:path`, `https://host/path`
/// and `ssh://git@host/path`.
///
/// The bare address is what the engine answers too
/// (`joy_core::vcs::remote_url::RemoteUrl::host`), and NO_PROXY is
/// compared against this answer on both sides (D1.11): read naively,
/// `https://[::1]:8443/x` splits at the FIRST colon and yields the host
/// `[`, and `NO_PROXY="[::1]"` then excluded the host for a git contact
/// and not for a REST call (JOY-02A3-E4).
pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    // scp-like: [user@]host:path - the ':' before the path is the first
    // one OUTSIDE every bracket, because a colon inside one belongs to
    // an IPv6 address or to the port of `[user@host:port]:path`.
    if !url.contains("://") {
        let (host_part, _path) = split_at_path_colon(url)?;
        let host = host_part.rsplit('@').next()?;
        return non_empty(bare_host(host).to_ascii_lowercase());
    }
    // scheme://[user@]host[:port][/...]
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?']).next()?;
    let host = authority.rsplit('@').next()?;
    non_empty(bare_host(host).to_ascii_lowercase())
}

/// The host of `host`, `host:port`, `[::1]` or `[::1]:8443`.
fn bare_host(authority: &str) -> &str {
    match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => authority.split(':').next().unwrap_or(authority),
    }
}

/// The authority and the path of an scp-like remote, split at the first
/// colon outside every bracket (libgit2 counts brackets the same way,
/// net.c:661-806).
fn split_at_path_colon(url: &str) -> Option<(&str, &str)> {
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

/// The `owner/repo` path of a remote URL of any wire form, nested
/// groups included (GitLab), without a `.git` suffix.
pub fn repo_path_of(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let path = path.trim_matches('/');
    path.contains('/').then(|| path.to_string())
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// Percent encode one path segment for an API URL. Only the characters
/// a repository path or a file path can carry are escaped; this is not
/// a general purpose encoder.
pub fn encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wire_form_yields_its_host() {
        for (url, host) in [
            ("git@github.com:joyint/app.git", "github.com"),
            ("https://github.com/joyint/app.git", "github.com"),
            ("ssh://git@github.com/joyint/app.git", "github.com"),
            ("https://user@GitHub.com:443/joyint/app", "github.com"),
            // an IPv6 literal, bare like the engine reads it
            ("https://[::1]:8443/joyint/app.git", "::1"),
            ("https://[2001:db8::1]/joyint/app", "2001:db8::1"),
            ("ssh://git@[::1]:2222/joyint/app.git", "::1"),
            ("git@[::1]:joyint/app.git", "::1"),
        ] {
            assert_eq!(host_of(url).as_deref(), Some(host), "{url}");
        }
        assert_eq!(host_of("/home/user/bare.git"), None);
        assert_eq!(host_of(""), None);
    }

    #[test]
    fn the_repository_path_survives_nesting_and_the_git_suffix() {
        assert_eq!(
            repo_path_of("https://gitlab.com/group/sub/app.git").as_deref(),
            Some("group/sub/app")
        );
        assert_eq!(
            repo_path_of("git@github.com:joyint/app.git").as_deref(),
            Some("joyint/app")
        );
        assert_eq!(repo_path_of("https://github.com/joyint"), None);
    }

    #[test]
    fn a_segment_is_encoded_for_an_api_path() {
        assert_eq!(encode_segment("group/sub/app"), "group%2Fsub%2Fapp");
        assert_eq!(encode_segment(".joy/project.yaml"), ".joy%2Fproject.yaml");
    }
}
