// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy-forge-auth`: the connector's own credential, and the doors that
//! fill it (JOY-029B-B0, design `docs/design/forge-connection-ng.md`,
//! package J3).
//!
//! Until J3 the connector had no credential of its own: every verb took
//! a token from the environment or from a forge CLI it spawned, so a
//! machine with neither gh nor curl could not publish a release, and a
//! person with no forge CLI could not sign in at all. This module is
//! the other half:
//!
//! - [`store`] owns the entry (D2.6). The connector binary is the ONLY
//!   process that touches it, `Entry::new` is the only addressing, and
//!   a machine whose credential store cannot answer gets the 0600 file
//!   joy writes itself.
//! - [`lock`] is the refresh lock of D2.6a, taken on the one cross
//!   process primitive joy has (`joy_core::util::file_lock`). Two
//!   refreshes at once killed Codeberg tokens for an hour once; this is
//!   why they cannot happen again.
//! - [`oauth`] is the device grant (GitHub, GitLab) and the PKCE S256
//!   loopback door (the Gitea family), plus the refresh that rotation
//!   safe forges need (D2.7).
//! - [`choose`] is the login order of D4.1c: the device local pin, the
//!   transport memory, the only login, then one probe per remote.
//! - [`verbs`] is `token`, `token-store`, `login`, `logout` and
//!   `web-url` with the shapes of D2.4.
//!
//! Three rules hold in every file here:
//!
//! 1. **A secret never leaves this module in the clear.** It travels in
//!    a header or on stdin, never in an argument, never in a log line
//!    and never in an error text. The rule is held by the types and not
//!    by discipline: every type that carries a token prints its
//!    [`fingerprint`] instead (`Record`, [`Resolved`], `oauth::Grant`),
//!    so one `tracing::debug!(?resolved)` cannot leak one.
//! 2. **The connector never opens a browser** (D2.4). It says where the
//!    person must go; the host decides what to do with that.
//! 3. **A refusal is an answer, not a failure.** Every verb here exits
//!    0 with an object that names the state.

pub mod choose;
pub mod lock;
pub mod oauth;
pub mod pin;
pub mod store;
pub mod verbs;

/// What a sign in is FOR (`--for`, D2.4). It picks the scope set of
/// D2.7a, which is the whole reason the flag exists: a read only member
/// asks for less, and `joy forge login --for create` widens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Purpose {
    /// Read the repository and the account: groups A, B, C and E.
    Read,
    /// Read plus push over https: groups A to E. The default, because
    /// it is what a member of a project needs to work.
    #[default]
    Write,
    /// Create a repository as well: group F.
    Create,
    /// Publish a release: group G.
    Release,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Read => "read",
            Purpose::Write => "write",
            Purpose::Create => "create",
            Purpose::Release => "release",
        }
    }
}

impl std::str::FromStr for Purpose {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read" => Ok(Purpose::Read),
            "write" => Ok(Purpose::Write),
            "create" => Ok(Purpose::Create),
            "release" => Ok(Purpose::Release),
            other => Err(format!(
                "unknown access level '{other}'; expected read, write, create or release"
            )),
        }
    }
}

/// Which step of the login order of D4.1c chose the login this answer
/// names. Every answer that names a token carries it, because "one row
/// per host, each naming the login it holds" cannot be kept otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoseBy {
    /// The device local pin for this host in this project.
    Pin,
    /// The login the transport memory recorded for this remote.
    Memory,
    /// The only login the host holds, with no probe at all.
    Only,
    /// One REST call per candidate; the first that answered wins.
    Probe,
}

impl ChoseBy {
    pub fn as_str(self) -> &'static str {
        match self {
            ChoseBy::Pin => "pin",
            ChoseBy::Memory => "memory",
            ChoseBy::Only => "only",
            ChoseBy::Probe => "probe",
        }
    }
}

/// Where a token came from (`source` in D2.4). `keychain` and `file`
/// are the connector's own entry; the other three are read only for
/// joy, because joy never refreshes, writes or revokes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Keychain,
    File,
    Gh,
    Glab,
    Tea,
    Env,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Keychain => "keychain",
            Source::File => "file",
            Source::Gh => "gh",
            Source::Glab => "glab",
            Source::Tea => "tea",
            Source::Env => "env",
        }
    }

    /// Whether the connector owns this credential. It owns exactly the
    /// two it wrote: a foreign CLI's token is refreshed, rewritten and
    /// revoked by that CLI alone (D2.6).
    pub fn is_own(self) -> bool {
        matches!(self, Source::Keychain | Source::File)
    }
}

/// One token, ready to be used or reported. Every field of D2.4's
/// `token` answer is here, so a caller cannot report half of it.
#[derive(Clone)]
pub struct Resolved {
    pub token: String,
    pub login: Option<String>,
    pub source: Source,
    /// The granted set, space separated, as the forge wrote it.
    pub scopes: Option<String>,
    pub expires_at: Option<String>,
    pub chose_by: Option<ChoseBy>,
}

/// Rule 1 of this module: the token prints as its fingerprint, which
/// identifies it without carrying it.
impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("token", &Redacted(&self.token))
            .field("login", &self.login)
            .field("source", &self.source)
            .field("scopes", &self.scopes)
            .field("expires_at", &self.expires_at)
            .field("chose_by", &self.chose_by)
            .finish()
    }
}

impl Resolved {
    /// The granted set as a list, for the `scope_missing` pre check.
    pub fn granted(&self) -> Vec<String> {
        self.scopes
            .as_deref()
            .map(crate::scope::parse_granted)
            .unwrap_or_default()
    }
}

/// The first twelve hex digits of a token's SHA-256: the shape the
/// platform already uses (platform/src/auth/mod.rs:571-577) and what
/// D2.6a compares under the refresh lock to decide whether the entry is
/// still the one this process read before it waited.
///
/// It is a fingerprint and never the token: twelve hex digits are
/// forty-eight bits of a hash, which identifies but does not reveal.
pub fn fingerprint(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)[..12].to_string()
}

/// A secret inside a `{:?}`: its [`fingerprint`], never itself.
///
/// This is how rule 1 of this module is enforced rather than asked for.
/// A type that carries a token writes its `Debug` by hand and puts the
/// token through this, so the worst a stray `?record` can print is
/// twelve hex digits of a hash.
pub struct Redacted<'a>(pub &'a str);

impl std::fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<secret {}>", fingerprint(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_purpose_parses_the_four_words_of_the_flag_and_nothing_else() {
        assert_eq!("create".parse::<Purpose>().unwrap(), Purpose::Create);
        assert_eq!("  Read ".parse::<Purpose>().unwrap(), Purpose::Read);
        assert_eq!(Purpose::default(), Purpose::Write);
        assert!("everything".parse::<Purpose>().is_err());
    }

    #[test]
    fn a_fingerprint_identifies_a_token_without_carrying_it() {
        let print = fingerprint("gho_the_secret");
        assert_eq!(print.len(), 12);
        assert!(print.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(print, fingerprint("gho_another_secret"));
        assert!(!print.contains("secret"));
    }

    /// Rule 1, proved on the three types that carry a token: a `{:?}`
    /// of any of them prints a fingerprint and never the secret.
    #[test]
    fn no_type_that_carries_a_token_can_print_one() {
        let record = store::Record {
            token: "gho_the_secret".to_string(),
            login: Some("scotty".to_string()),
            refresh_token: Some("rt_the_other_secret".to_string()),
            ..store::Record::default()
        };
        let printed = format!("{record:?}");
        assert!(!printed.contains("gho_the_secret"), "{printed}");
        assert!(!printed.contains("rt_the_other_secret"), "{printed}");
        assert!(
            printed.contains(&fingerprint("gho_the_secret")),
            "{printed}"
        );
        assert!(printed.contains("scotty"), "the login is not a secret");

        let resolved = Resolved {
            token: "gho_the_secret".to_string(),
            login: Some("scotty".to_string()),
            source: Source::Keychain,
            scopes: Some("repo".to_string()),
            expires_at: None,
            chose_by: Some(ChoseBy::Only),
        };
        let printed = format!("{resolved:?}");
        assert!(!printed.contains("gho_the_secret"), "{printed}");
        assert!(printed.contains("Keychain"), "the source is not a secret");

        let grant = oauth::Grant {
            access_token: "gho_the_secret".to_string(),
            refresh_token: Some("rt_the_other_secret".to_string()),
            expires_in: Some(3600),
            scope: Some("repo".to_string()),
        };
        let printed = format!("{grant:?}");
        assert!(!printed.contains("gho_the_secret"), "{printed}");
        assert!(!printed.contains("rt_the_other_secret"), "{printed}");
        // and the same holds one level up, where a Poll carries it
        let printed = format!("{:?}", oauth::Poll::Granted(grant));
        assert!(!printed.contains("gho_the_secret"), "{printed}");
    }

    #[test]
    fn only_the_two_entries_the_connector_wrote_belong_to_it() {
        assert!(Source::Keychain.is_own());
        assert!(Source::File.is_own());
        for foreign in [Source::Gh, Source::Glab, Source::Tea, Source::Env] {
            assert!(!foreign.is_own(), "{foreign:?}");
        }
    }
}
