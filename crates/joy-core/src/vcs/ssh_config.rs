// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The person's ssh config, read by joy (design D1.4).
//!
//! libgit2 reads NONE of it. There is not one hit for `ssh_config`,
//! `IdentityFile` or `SSH_AUTH_SOCK` in its sources: it takes the host
//! and port from the URL, opens the socket itself and hands libssh2 an
//! already connected one. So everything a person wrote in
//! `~/.ssh/config` - the real host name behind an alias, a port, a
//! user, which key to offer, which agent to ask, which known-hosts
//! files count - is invisible to a joy contact unless joy reads it,
//! which is what this module does.
//!
//! Two directives are REFUSED rather than ignored: `ProxyCommand` and
//! `ProxyJump`. libssh2 never opens a socket (the string "proxy" does
//! not occur anywhere in its sources), and libgit2 opens a plain TCP
//! connection to the host in the URL, so a host that is only reachable
//! through a jump would fail with a DNS or connect error that names
//! the wrong cause. joy says the cause instead.

use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ssh2_config::{ParseRule, SshConfig};

/// What `StrictHostKeyChecking` says about a host joy has never seen
/// (design D1.4a). `ask` is ssh's own default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrictHostKeys {
    /// Refuse an unknown host without asking.
    Yes,
    /// Add an unknown host without asking. `no` and `off` land here
    /// too: joy never silently skips the check, it only skips the
    /// question.
    AcceptNew,
    /// Ask an interactive host, refuse every other one.
    #[default]
    Ask,
}

/// A rule that routes the connection through another process, which
/// joy cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyRule {
    Command(String),
    Jump(String),
}

impl ProxyRule {
    /// The directive's name, as written in the config.
    pub fn directive(&self) -> &'static str {
        match self {
            ProxyRule::Command(_) => "ProxyCommand",
            ProxyRule::Jump(_) => "ProxyJump",
        }
    }

    /// The sentence a person gets instead of a DNS error (D1.4).
    pub fn sentence(&self) -> String {
        format!(
            "This host uses {} in your ssh config. joy cannot run a proxy helper; \
             use the https remote for this host or remove the rule.",
            self.directive()
        )
    }
}

/// Everything joy takes from the ssh config for one host.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostSettings {
    /// `HostName`: the name to contact instead of the alias.
    pub host_name: Option<String>,
    /// `Port`.
    pub port: Option<u16>,
    /// `User`.
    pub user: Option<String>,
    /// `IdentityFile`, in the order the config lists them.
    pub identity_files: Vec<PathBuf>,
    /// `IdentityAgent`: the socket to ask instead of `SSH_AUTH_SOCK`.
    /// `None` when the config says nothing, `Some("none")` when it
    /// says `none`.
    pub identity_agent: Option<String>,
    /// `UserKnownHostsFile`, defaulted to ssh's own two files.
    pub user_known_hosts: Vec<PathBuf>,
    /// `GlobalKnownHostsFile`, defaulted to ssh's own two files.
    pub global_known_hosts: Vec<PathBuf>,
    /// `StrictHostKeyChecking`.
    pub strict_host_keys: StrictHostKeys,
    /// `HashKnownHosts`. Ubuntu ships `yes` in `/etc/ssh/ssh_config`,
    /// so a hashed entry is the normal case, not the exception.
    pub hash_known_hosts: bool,
    /// `ProxyCommand` or `ProxyJump`: the reason joy refuses the host.
    pub proxy: Option<ProxyRule>,
}

impl HostSettings {
    /// The host to contact: `HostName` when the config renames it.
    pub fn effective_host<'a>(&'a self, asked: &'a str) -> &'a str {
        self.host_name.as_deref().unwrap_or(asked)
    }

    /// The refusal sentence, when there is one.
    pub fn refusal(&self) -> Option<String> {
        self.proxy.as_ref().map(|p| p.sentence())
    }
}

/// A parsed ssh config (the person's file, then the machine's).
pub struct SshConfigFile {
    config: SshConfig,
}

impl SshConfigFile {
    /// Parse config text. The parser is told to keep going on a
    /// directive it does not model, because joy reads several of those
    /// (`IdentityAgent`, the known-hosts settings) out of the leftovers
    /// itself, and because a person's config must never fail a fetch
    /// just for holding a directive this crate has not learned.
    pub fn parse(text: &str) -> SshConfigFile {
        let rules = ParseRule::ALLOW_UNKNOWN_FIELDS | ParseRule::ALLOW_UNSUPPORTED_FIELDS;
        let mut reader = BufReader::new(text.as_bytes());
        let config = SshConfig::default()
            .parse(&mut reader, rules)
            .unwrap_or_else(|e| {
                tracing::debug!(error = %e, "ssh config not parsed");
                SshConfig::default()
            });
        SshConfigFile { config }
    }

    /// Parse the person's file and then the machine's, in that order,
    /// which is the order ssh resolves them in: the first value found
    /// for a directive wins.
    fn parse_files(user: &Path, system: &Path) -> SshConfigFile {
        let mut text = std::fs::read_to_string(user).unwrap_or_default();
        if let Ok(system) = std::fs::read_to_string(system) {
            text.push('\n');
            text.push_str(&system);
        }
        SshConfigFile::parse(&text)
    }

    /// What the config says about `host`.
    pub fn settings(&self, host: &str) -> HostSettings {
        let params = self.config.query(host);
        let leftovers = Leftovers::of(&params);
        HostSettings {
            host_name: params.host_name.clone(),
            port: params.port,
            user: params.user.clone(),
            identity_files: params.identity_file.clone().unwrap_or_default(),
            identity_agent: leftovers.first("identityagent").map(expand_tilde_string),
            user_known_hosts: leftovers
                .paths("userknownhostsfile")
                .unwrap_or_else(|| default_paths(&[".ssh/known_hosts", ".ssh/known_hosts2"])),
            global_known_hosts: leftovers.paths("globalknownhostsfile").unwrap_or_else(|| {
                ["/etc/ssh/ssh_known_hosts", "/etc/ssh/ssh_known_hosts2"]
                    .iter()
                    .map(PathBuf::from)
                    .collect()
            }),
            strict_host_keys: match leftovers
                .first("stricthostkeychecking")
                .as_deref()
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("yes") => StrictHostKeys::Yes,
                // "no" and "off" mean "do not refuse"; joy still adds
                // the line, it just does not ask first.
                Some("accept-new") | Some("no") | Some("off") => StrictHostKeys::AcceptNew,
                _ => StrictHostKeys::Ask,
            },
            hash_known_hosts: leftovers
                .first("hashknownhosts")
                .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "yes")),
            proxy: proxy_rule(&params, &leftovers),
        }
    }
}

/// The directives ssh2-config parses but does not model. They arrive
/// keyed by the lowercased directive name with their arguments split
/// into words.
struct Leftovers<'p> {
    fields: &'p HashMap<String, Vec<String>>,
}

impl<'p> Leftovers<'p> {
    fn of(params: &'p ssh2_config::HostParams) -> Leftovers<'p> {
        Leftovers {
            fields: &params.unsupported_fields,
        }
    }

    fn first(&self, key: &str) -> Option<String> {
        self.fields
            .get(key)
            .and_then(|args| args.first())
            .map(|value| value.trim_matches('"').to_string())
            .filter(|value| !value.is_empty())
    }

    /// Every argument of a directive, as paths. `None` when the
    /// directive is absent, so the caller can put ssh's default in its
    /// place; an explicit `none` answers with an empty list.
    fn paths(&self, key: &str) -> Option<Vec<PathBuf>> {
        let args = self.fields.get(key)?;
        if args.len() == 1 && args[0].eq_ignore_ascii_case("none") {
            return Some(Vec::new());
        }
        Some(
            args.iter()
                .map(|value| {
                    PathBuf::from(expand_tilde_string(value.trim_matches('"').to_string()))
                })
                .collect(),
        )
    }
}

fn proxy_rule(params: &ssh2_config::HostParams, leftovers: &Leftovers<'_>) -> Option<ProxyRule> {
    if let Some(command) = leftovers.first("proxycommand") {
        let whole = leftovers
            .fields
            .get("proxycommand")
            .map(|args| args.join(" "))
            .unwrap_or(command);
        if !whole.eq_ignore_ascii_case("none") {
            return Some(ProxyRule::Command(whole));
        }
    }
    if let Some(jump) = params.proxy_jump.as_ref().and_then(|j| j.first()) {
        if !jump.eq_ignore_ascii_case("none") {
            return Some(ProxyRule::Jump(jump.clone()));
        }
    }
    None
}

fn default_paths(relative: &[&str]) -> Vec<PathBuf> {
    match home_dir() {
        Some(home) => relative.iter().map(|rel| home.join(rel)).collect(),
        None => Vec::new(),
    }
}

fn expand_tilde_string(value: String) -> String {
    let Some(rest) = value.strip_prefix('~') else {
        return value;
    };
    let Some(home) = home_dir() else {
        return value;
    };
    let rest = rest.trim_start_matches(['/', '\\']);
    home.join(rest).to_string_lossy().into_owned()
}

fn home_dir() -> Option<PathBuf> {
    // The same source ssh2-config itself uses, so a test that moves
    // HOME moves both.
    dirs_home()
}

#[cfg(unix)]
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(unix))]
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            let mut home = std::ffi::OsString::from(drive);
            home.push(path);
            Some(PathBuf::from(home))
        })
}

/// The person's config file, `~/.ssh/config`.
pub fn user_config_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".ssh").join("config"))
}

/// The machine's config file. Ubuntu's ships `HashKnownHosts yes`,
/// which is why joy reads it at all.
pub fn system_config_path() -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
        base.join("ssh").join("ssh_config")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/ssh/ssh_config")
    }
}

/// Parsed configs, keyed by the file and its modification time: one
/// stat per contact is nothing beside the round trip, and a person who
/// edits the config while the desktop runs is not told to restart it.
/// The config file and the state it was in when joy read it.
type ConfigStamp = (PathBuf, Option<std::time::SystemTime>);

static PARSED: Mutex<Option<HashMap<ConfigStamp, Arc<SshConfigFile>>>> = Mutex::new(None);

fn parsed_config() -> Arc<SshConfigFile> {
    let user = user_config_path().unwrap_or_else(|| PathBuf::from(".ssh/config"));
    let stamp = std::fs::metadata(&user).and_then(|m| m.modified()).ok();
    let key = (user.clone(), stamp);
    let mut guard = PARSED.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(hit) = cache.get(&key) {
        return hit.clone();
    }
    let parsed = Arc::new(SshConfigFile::parse_files(&user, &system_config_path()));
    // One entry per config state; a config that changes often would
    // otherwise grow the map without bound.
    cache.clear();
    cache.insert(key, parsed.clone());
    parsed
}

/// What the machine's ssh config says about `host`.
pub fn for_host(host: &str) -> HostSettings {
    parsed_config().settings(host)
}

/// The sentence for a remote joy refuses to contact over ssh, `None`
/// when there is nothing in the way. Read before the socket opens, so
/// that a jump host fails by name instead of by DNS.
pub fn refusal_for_url(url: &str) -> Option<String> {
    let parsed = super::remote_url::RemoteUrl::parse(url)?;
    if parsed.transport != super::remote_url::Transport::Ssh {
        return None;
    }
    for_host(&parsed.host).refusal()
}

/// Point this process at the agent the config names for this host.
///
/// `SSH_AUTH_SOCK` in joy's own environment is the ONLY way to reach
/// 1Password, Secretive or KeePassXC: libgit2 passes libssh2 no agent
/// path, and libssh2 reads the variable itself. Process-wide by
/// nature, so it is set only when the config actually names an agent
/// and only when it differs from what is already there.
pub fn apply_identity_agent(settings: &HostSettings) -> Option<String> {
    let agent = settings.identity_agent.as_deref()?;
    if agent.eq_ignore_ascii_case("none") {
        return None;
    }
    // The documented way to say "the variable you already have".
    if agent == "SSH_AUTH_SOCK" {
        return std::env::var("SSH_AUTH_SOCK").ok();
    }
    if std::env::var("SSH_AUTH_SOCK").as_deref() == Ok(agent) {
        return Some(agent.to_string());
    }
    std::env::set_var("SSH_AUTH_SOCK", agent);
    tracing::debug!(agent, "ssh agent socket taken from IdentityAgent");
    Some(agent.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
Host short
    HostName git.example.com
    Port 2222
    User deploy
    IdentityFile /keys/deploy_ed25519
    IdentityAgent /run/user/1000/1password/agent.sock

Host jump.example.com
    ProxyCommand /usr/bin/corkscrew proxy 8080 %h %p

Host hop.example.com
    ProxyJump bastion.example.com

Host strict.example.com
    StrictHostKeyChecking yes
    UserKnownHostsFile /etc/joy/known_hosts /etc/joy/known_hosts.more
    HashKnownHosts yes

Host *
    GlobalKnownHostsFile /etc/ssh/ssh_known_hosts
"#;

    #[test]
    fn an_alias_brings_its_host_port_user_key_and_agent() {
        let config = SshConfigFile::parse(FIXTURE);
        let settings = config.settings("short");
        assert_eq!(settings.host_name.as_deref(), Some("git.example.com"));
        assert_eq!(settings.effective_host("short"), "git.example.com");
        assert_eq!(settings.port, Some(2222));
        assert_eq!(settings.user.as_deref(), Some("deploy"));
        assert_eq!(
            settings.identity_files,
            vec![PathBuf::from("/keys/deploy_ed25519")]
        );
        assert_eq!(
            settings.identity_agent.as_deref(),
            Some("/run/user/1000/1password/agent.sock")
        );
        assert!(settings.proxy.is_none());
    }

    #[test]
    fn a_proxy_command_is_refused_by_name() {
        let config = SshConfigFile::parse(FIXTURE);
        let settings = config.settings("jump.example.com");
        let sentence = settings.refusal().expect("a refusal");
        assert_eq!(
            sentence,
            "This host uses ProxyCommand in your ssh config. joy cannot run a proxy helper; \
             use the https remote for this host or remove the rule."
        );
        assert!(!sentence.to_lowercase().contains("dns"));
    }

    #[test]
    fn a_proxy_jump_is_refused_by_its_own_name() {
        let config = SshConfigFile::parse(FIXTURE);
        let settings = config.settings("hop.example.com");
        let sentence = settings.refusal().expect("a refusal");
        assert!(
            sentence.starts_with("This host uses ProxyJump in your ssh config."),
            "{sentence}"
        );
    }

    #[test]
    fn the_known_hosts_settings_are_read_and_defaulted() {
        let config = SshConfigFile::parse(FIXTURE);
        let strict = config.settings("strict.example.com");
        assert_eq!(strict.strict_host_keys, StrictHostKeys::Yes);
        assert!(strict.hash_known_hosts);
        assert_eq!(
            strict.user_known_hosts,
            vec![
                PathBuf::from("/etc/joy/known_hosts"),
                PathBuf::from("/etc/joy/known_hosts.more")
            ]
        );
        assert_eq!(
            strict.global_known_hosts,
            vec![PathBuf::from("/etc/ssh/ssh_known_hosts")]
        );
        let plain = config.settings("nothing.example.com");
        assert_eq!(plain.strict_host_keys, StrictHostKeys::Ask);
        assert!(!plain.hash_known_hosts);
        assert!(plain
            .user_known_hosts
            .iter()
            .any(|p| p.ends_with("known_hosts")));
    }

    #[test]
    fn off_and_no_mean_add_without_asking_never_skip_the_check() {
        for value in ["no", "off", "accept-new"] {
            let config =
                SshConfigFile::parse(&format!("Host h\n  StrictHostKeyChecking {value}\n"));
            assert_eq!(
                config.settings("h").strict_host_keys,
                StrictHostKeys::AcceptNew,
                "for {value}"
            );
        }
    }

    #[test]
    fn a_config_joy_cannot_parse_is_not_a_failed_fetch() {
        let config = SshConfigFile::parse("Host h\n  ThisDirectiveDoesNotExist yes\n  Port 44\n");
        assert_eq!(config.settings("h").port, Some(44));
    }

    #[test]
    fn proxy_command_none_is_not_a_proxy() {
        let config = SshConfigFile::parse("Host h\n  ProxyCommand none\n");
        assert!(config.settings("h").proxy.is_none());
    }

    #[test]
    fn a_wildcard_pattern_applies_to_the_hosts_it_matches() {
        let config = SshConfigFile::parse("Host *.example.com\n  User deploy\n  Port 2200\n");
        let inside = config.settings("git.example.com");
        assert_eq!(inside.user.as_deref(), Some("deploy"));
        assert_eq!(inside.port, Some(2200));
        assert_eq!(config.settings("github.com").user, None);
    }
}
