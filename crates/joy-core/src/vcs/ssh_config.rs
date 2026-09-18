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
        let scoped = scope_match_blocks(text);
        let mut reader = BufReader::new(scoped.as_bytes());
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
        // ~/.ssh and /etc/ssh: the two directories a relative `Include`
        // is resolved against (ssh_config(5)).
        let user_base = user.parent().map(Path::to_path_buf).unwrap_or_default();
        let system_base = system.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut text = std::fs::read_to_string(user)
            .map(|text| splice_includes(&text, &user_base, 0))
            .unwrap_or_default();
        if let Ok(system) =
            std::fs::read_to_string(system).map(|text| splice_includes(&text, &system_base, 0))
        {
            // A `Host *` line between the two files. Without it, every
            // directive the machine's file writes BEFORE its first
            // `Host` line would be parsed inside the LAST `Host` block
            // of the person's file, which is how Fedora's leading
            // `Include /etc/ssh/ssh_config.d/*.conf` and Ubuntu's
            // leading `HashKnownHosts yes` would end up scoped to
            // whatever host the person happened to name last. `Host *`
            // is also what those lines mean: a directive before the
            // first `Host` line applies to every host. The person's
            // file still wins every field, because the query merges
            // top down and only fills what is still unset, which is
            // ssh's own "first obtained value" rule.
            text.push_str("\n\nHost *\n");
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

/// Splice every `Include` into the text, where it stands.
///
/// ssh2-config knows the keyword no better than it knows `Match`, so
/// without this a config.d fragment is simply invisible, and Ubuntu,
/// Fedora and macOS all ship an `Include` line in
/// /etc/ssh/ssh_config while 1Password, Docker and corporate setups
/// write one into ~/.ssh/config. The per host `IdentityFile` and the
/// `ProxyJump` rules that live in those fragments are exactly what
/// this module exists to read, and a `ProxyJump` joy does not see is a
/// host refused by DNS error instead of by name (design D1.4).
///
/// Splicing the text in at the directive's own place is what ssh does:
/// "a Host or Match directive in an included file affects the rest of
/// the parent file" (ssh_config(5)). A relative path is resolved
/// against `base`, which stays ~/.ssh for everything the person's file
/// pulls in and /etc/ssh for everything the machine's file pulls in,
/// whatever directory the including file sits in, because that is how
/// ssh resolves it. A file that cannot be read is skipped with a debug
/// line, never with a failed fetch, and the depth is bounded so an
/// include that names its own file stops.
fn splice_includes(text: &str, base: &Path, depth: u32) -> String {
    /// Deeper than any real config, shallow enough to stop a cycle.
    const MAX_DEPTH: u32 = 8;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let (keyword, args) = split_directive(line);
        if !keyword.eq_ignore_ascii_case("include") {
            match with_joys_own_home(keyword, args) {
                Some(rewritten) => out.push_str(&format!("{keyword} {rewritten}")),
                None => out.push_str(line),
            }
            out.push('\n');
            continue;
        }
        if depth >= MAX_DEPTH {
            tracing::debug!(line, "ssh config Include nested too deep, not read");
            continue;
        }
        for path in include_paths(args, base) {
            match std::fs::read_to_string(&path) {
                Ok(inner) => {
                    out.push_str(&splice_includes(&inner, base, depth + 1));
                    out.push('\n');
                }
                Err(e) => {
                    tracing::debug!(file = %path.display(), error = %e, "ssh config Include not read")
                }
            }
        }
    }
    out
}

/// A leading `~` in a directive that names a file, expanded with the
/// home JOY reads, before ssh2-config expands it with the home the
/// `dirs` crate reads.
///
/// `None` when there is nothing to change, which is the normal line.
///
/// The two homes are the same on unix, where both read `HOME`. On
/// Windows they are not: joy reads `USERPROFILE` everywhere (store.rs,
/// [`home_dir`]), and `dirs::home_dir` asks the shell for the profile
/// folder and reads no environment variable at all, so an
/// `IdentityFile ~/.ssh/id_ed25519` came back under a different home
/// than every other path in the same run. One run reads one home.
fn with_joys_own_home(keyword: &str, args: &str) -> Option<String> {
    const NAMES_A_FILE: [&str; 5] = [
        "identityfile",
        "certificatefile",
        "identityagent",
        "userknownhostsfile",
        "globalknownhostsfile",
    ];
    if !NAMES_A_FILE
        .iter()
        .any(|name| keyword.eq_ignore_ascii_case(name))
    {
        return None;
    }
    let (quote, rest) = match args.strip_prefix('"') {
        Some(rest) => ("\"", rest),
        None => ("", args),
    };
    // `~/` and `~\` only: `~user` is not something ssh expands either.
    let rest = rest.strip_prefix('~')?;
    if !rest.starts_with('/') && !rest.starts_with('\\') {
        return None;
    }
    let home = home_dir()?;
    Some(format!("{quote}{}{rest}", home.display()))
}

/// The files one `Include` line names, in ssh's own order: every
/// argument, and for each of them every match of the pattern, sorted,
/// which is what glob(3) gives ssh.
fn include_paths(args: &str, base: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for argument in args.split_whitespace() {
        let argument = argument.trim_matches('"');
        if argument.is_empty() {
            continue;
        }
        let expanded = expand_tilde_string(argument.to_string());
        let path = PathBuf::from(&expanded);
        let path = if path.is_absolute() {
            path
        } else {
            base.join(path)
        };
        if expanded.contains(['*', '?', '[']) {
            let pattern = path.to_string_lossy().into_owned();
            match glob::glob(&pattern) {
                Ok(matches) => files.extend(matches.flatten().filter(|p| p.is_file())),
                Err(e) => tracing::debug!(pattern, error = %e, "ssh config Include pattern"),
            }
        } else {
            files.push(path);
        }
    }
    files
}

/// Keep a `Match` block out of the `Host` block above it.
///
/// ssh2-config 0.8 does not know the `Match` keyword at all: it is not
/// in its field list, so the line lands in the ignored fields and
/// EVERY directive under it stays in the scope of the PRECEDING `Host`
/// block. A config that ends with
///
/// ```text
/// Host *
///   ServerAliveInterval 60
///
/// Match host bastion.corp.example
///   ProxyJump gateway.corp.example
/// ```
///
/// would then carry a `ProxyJump` for `Host *`, and joy would refuse
/// every ssh remote on the machine instead of the one host that
/// carries the rule (design D1.4). `Match` blocks are ordinary:
/// 1Password writes one, corporate configs detect a VPN with
/// `Match exec`.
///
/// joy therefore rewrites the text before the parser sees it:
///
/// - `Match host <patterns>`, the one criterion that is itself a host
///   pattern list, becomes `Host <patterns>`; ssh separates those
///   patterns with commas and `Host` separates them with spaces, so
///   the commas become spaces;
/// - every other `Match` block (`exec`, `user`, `canonical`, `all`,
///   or several criteria at once) is dropped up to the next `Host` or
///   `Match` line. joy cannot evaluate those criteria, and applying
///   their directives to a host that may not match them would refuse
///   or reroute the wrong host. A `ProxyCommand` inside such a block
///   is therefore not seen, which costs the named sentence for that
///   one host and never costs a wrong refusal for another.
fn scope_match_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dropping = false;
    for line in text.lines() {
        let (keyword, args) = split_directive(line);
        if keyword.eq_ignore_ascii_case("host") {
            dropping = false;
            out.push_str(line);
            out.push('\n');
        } else if keyword.eq_ignore_ascii_case("match") {
            match match_host_patterns(args) {
                Some(patterns) => {
                    dropping = false;
                    out.push_str("Host ");
                    out.push_str(&patterns);
                    out.push('\n');
                }
                None => dropping = true,
            }
        } else if !dropping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The keyword and the rest of one config line, split the way
/// ssh2-config splits it: on the first `=` or the first space,
/// whichever comes first (parser.rs:623-643). A comment line has no
/// keyword.
fn split_directive(line: &str) -> (&str, &str) {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return ("", "");
    }
    let at = trimmed
        .find(|c: char| c == '=' || c.is_whitespace())
        .unwrap_or(trimmed.len());
    let (keyword, rest) = trimmed.split_at(at);
    (
        keyword,
        rest.trim().trim_start_matches('=').trim().trim_end(),
    )
}

/// The `Host` pattern list a `Match host a,b` line stands for, `None`
/// for every other `Match`.
fn match_host_patterns(args: &str) -> Option<String> {
    let mut words = args.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("host") {
        return None;
    }
    let patterns = words.next()?;
    // A second criterion (`Match host x user y`) is not a host pattern
    // list any more, and joy does not evaluate the other criteria.
    if words.next().is_some() {
        return None;
    }
    let patterns = patterns.replace(',', " ");
    let patterns = patterns.trim();
    (!patterns.is_empty()).then(|| patterns.to_string())
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
            let mut home = drive;
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

/// What the config says about the host of ONE contact, found through
/// the remote as the person configured it.
///
/// Normally the same as [`for_host`]. It differs when the config
/// renames the host: joy then dials the `HostName` ([`effective_url`])
/// and libgit2 hands its callbacks THAT address, so the `Host work`
/// block that produced the rewrite, and with it the `IdentityFile`,
/// the `IdentityAgent` and the `User` the person wrote under it, would
/// be invisible. `ssh git.example.com` would not read the `Host work`
/// block either, but joy is not answering `ssh git.example.com`, it is
/// answering `git fetch work` (design D1.4).
///
/// The alias is used only when it really resolves to the host being
/// dialled, so a remote that names a host of its own keeps its own
/// block.
pub fn for_contact(dialled: &str, configured: Option<&str>) -> HostSettings {
    let alias = (|| {
        let parsed = super::remote_url::RemoteUrl::parse(configured?)?;
        if parsed.transport != super::remote_url::Transport::Ssh || parsed.host == dialled {
            return None;
        }
        let settings = for_host(&parsed.host);
        settings
            .effective_host(&parsed.host)
            .eq_ignore_ascii_case(dialled)
            .then_some(settings)
    })();
    alias.unwrap_or_else(|| for_host(dialled))
}

/// The address joy really dials for a remote URL, with the person's
/// ssh config applied: `HostName` in place of the alias, and `Port`.
///
/// libgit2 reads no ssh config and opens the socket itself from the
/// URL, so `git@work:owner/repo.git` with `Host work / HostName
/// git.example.com / Port 2222` in the config is otherwise looked up
/// as the literal name `work` on port 22 and fails with a DNS error,
/// which names the wrong cause (design D1.4). The configured remote is
/// never rewritten on disk; this is the URL of ONE contact.
///
/// `None` when the config changes neither, so a remote that needs no
/// rewrite keeps the exact string the person wrote.
///
/// The form is kept: an scp-like remote stays scp-like, in the
/// bracketed form `[git@host:2222]:owner/repo.git` when a port has to
/// go in. The two forms do not mean the same path: the scp form yields
/// a path without a leading slash and the `ssh://` form yields one
/// with (net.c:522-530, :661-806, design D1.5), and libgit2 sends that
/// path to the server as it stands.
pub fn effective_url(url: &str) -> Option<String> {
    let parsed = super::remote_url::RemoteUrl::parse(url)?;
    if parsed.transport != super::remote_url::Transport::Ssh {
        return None;
    }
    let settings = for_host(&parsed.host);
    let host = settings.effective_host(&parsed.host).to_ascii_lowercase();
    let port = match parsed.port {
        // A port in the URL is ssh's command line, which beats the
        // config.
        Some(port) => Some(port),
        // ssh's own default; writing it in would only make the URL
        // longer than the person's.
        None => settings.port.filter(|port| *port != 22),
    };
    if host == parsed.host && port == parsed.port {
        return None;
    }
    let user = match &parsed.user {
        Some(user) => format!("{user}@"),
        None => String::new(),
    };
    let host = if parsed.bracketed {
        format!("[{host}]")
    } else {
        host
    };
    let dialled = if url.contains("://") {
        match port {
            Some(port) => format!("ssh://{user}{host}:{port}/{}", parsed.path),
            None => format!("ssh://{user}{host}/{}", parsed.path),
        }
    } else {
        match port {
            // The bracketed scp form, which is how an scp-like remote
            // carries a port at all (D1.5).
            Some(port) => format!("[{user}{host}:{port}]:{}", parsed.path),
            None => format!("{user}{host}:{}", parsed.path),
        }
    };
    tracing::debug!(
        url,
        dialled,
        "ssh config named another address for this host"
    );
    Some(dialled)
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

/// `SSH_AUTH_SOCK` pointed at the agent the config names for one host,
/// and put back when the contact is over.
///
/// The variable in joy's own environment is the ONLY way to reach
/// 1Password, Secretive or KeePassXC: libgit2 passes libssh2 no agent
/// path, and libssh2 reads the variable itself. It is process-wide by
/// nature, which is why this is a scope and not a setter: a per host
/// `IdentityAgent` that is never put back would make every later host
/// in the same process offer that agent, so one contact to a `Host
/// work` that names the 1Password socket would send every github.com
/// contact of a desktop session to 1Password as well.
///
/// The variable is touched at all only when the config names an agent
/// for this host and names a different one than the environment
/// already holds.
#[must_use = "the agent socket is put back when this scope is dropped"]
pub struct AgentScope {
    previous: Option<std::ffi::OsString>,
    applied: bool,
}

impl AgentScope {
    /// The scope for a host that names no agent: nothing is set and
    /// nothing is put back.
    pub fn none() -> AgentScope {
        AgentScope {
            previous: None,
            applied: false,
        }
    }

    /// Point this process at the agent `settings` names, for as long
    /// as the returned scope lives.
    pub fn apply(settings: &HostSettings) -> AgentScope {
        let Some(agent) = settings.identity_agent.as_deref() else {
            return AgentScope::none();
        };
        // `none` means "ask no agent", and the literal name of the
        // variable is the documented way to say "the one you have".
        if agent.eq_ignore_ascii_case("none") || agent == "SSH_AUTH_SOCK" {
            return AgentScope::none();
        }
        let previous = std::env::var_os("SSH_AUTH_SOCK");
        if previous.as_deref() == Some(std::ffi::OsStr::new(agent)) {
            return AgentScope::none();
        }
        // SAFETY (as far as edition 2021 allows): libssh2 reads this
        // variable itself and takes no agent path from libgit2, so
        // there is no other way to reach a per host agent. See the
        // open question in the package report.
        std::env::set_var("SSH_AUTH_SOCK", agent);
        tracing::debug!(agent, "ssh agent socket taken from IdentityAgent");
        AgentScope {
            previous,
            applied: true,
        }
    }

    /// The socket this scope points at, for a sentence about it.
    pub fn socket(&self) -> Option<String> {
        self.applied
            .then(|| std::env::var("SSH_AUTH_SOCK").ok())
            .flatten()
    }
}

impl Drop for AgentScope {
    fn drop(&mut self) {
        if !self.applied {
            return;
        }
        match self.previous.take() {
            Some(previous) => std::env::set_var("SSH_AUTH_SOCK", previous),
            None => std::env::remove_var("SSH_AUTH_SOCK"),
        }
    }
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

    /// The regression this guards: ssh2-config does not know `Match`,
    /// so without joy's own scoping every directive under a `Match`
    /// line stays inside the PRECEDING `Host` block. One
    /// `Match host bastion` with a `ProxyJump` under it would then
    /// refuse every ssh remote on the machine.
    #[test]
    fn a_match_block_never_reaches_the_host_block_above_it() {
        let config = SshConfigFile::parse(
            "Host *\n  ServerAliveInterval 60\n\n\
             Match exec \"nc -z vpn.corp.example 22\"\n  ProxyJump gateway.corp.example\n  User vpn\n",
        );
        assert!(
            config.settings("github.com").proxy.is_none(),
            "a Match block must not refuse a host it does not name"
        );
        assert_eq!(config.settings("github.com").user, None);
        assert!(config.settings("vpn.corp.example").proxy.is_none());
        // And the blocks after it are read again as they stand.
        let after = SshConfigFile::parse(
            "Match canonical\n  User nobody\n\nHost git.example.com\n  Port 2222\n",
        );
        assert_eq!(after.settings("git.example.com").port, Some(2222));
        assert_eq!(after.settings("git.example.com").user, None);
    }

    #[test]
    fn a_match_on_host_names_applies_to_those_hosts_and_to_no_other() {
        let config = SshConfigFile::parse(
            "Host *\n  ServerAliveInterval 60\n\n\
             Match host bastion.corp.example,jump.corp.example\n  ProxyJump gateway.corp.example\n",
        );
        for named in ["bastion.corp.example", "jump.corp.example"] {
            assert!(
                config
                    .settings(named)
                    .refusal()
                    .is_some_and(|s| s.contains("ProxyJump")),
                "{named} carries the rule"
            );
        }
        assert!(config.settings("github.com").refusal().is_none());
        // `Match host x user y` is a criterion joy does not evaluate,
        // so the block is dropped rather than applied to host x.
        let mixed =
            SshConfigFile::parse("Match host bastion.corp.example user deploy\n  ProxyJump g\n");
        assert!(mixed.settings("bastion.corp.example").proxy.is_none());
    }

    #[test]
    fn the_machines_file_is_not_read_inside_the_persons_last_host_block() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("config");
        let system = dir.path().join("ssh_config");
        std::fs::write(
            &user,
            "Host work\n  HostName git.example.com\n  Port 2222\n",
        )
        .unwrap();
        // Ubuntu's own file starts with directives that stand before
        // any Host line (HashKnownHosts among them); Fedora's starts
        // with an Include. They belong to every host, not to `work`.
        std::fs::write(
            &system,
            "HashKnownHosts yes\nStrictHostKeyChecking accept-new\n",
        )
        .unwrap();
        let config = SshConfigFile::parse_files(&user, &system);
        assert!(config.settings("github.com").hash_known_hosts);
        let work = config.settings("work");
        assert_eq!(work.host_name.as_deref(), Some("git.example.com"));
        assert_eq!(work.port, Some(2222));
    }

    /// The regression this guards: ssh2-config knows `Include` no
    /// better than it knows `Match`, and Ubuntu, Fedora and macOS all
    /// ship an `Include` line, so a config.d fragment carrying a per
    /// host `IdentityFile` or a `ProxyJump` would simply not be there.
    #[test]
    fn an_include_is_read_where_it_stands_and_relative_to_the_right_directory() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("config");
        std::fs::write(
            &user,
            "Include conf.d/*.conf

Host after
  Port 2200

Include nested.conf
",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("conf.d")).unwrap();
        std::fs::write(
            dir.path().join("conf.d").join("10-work.conf"),
            "Host work
  HostName git.example.com
  Port 2222
",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("conf.d").join("20-bastion.conf"),
            "Host bastion
  ProxyJump gateway
",
        )
        .unwrap();
        // A file whose own directives continue the block above it,
        // which is what ssh's textual splice does.
        std::fs::write(
            dir.path().join("nested.conf"),
            "  User deploy
",
        )
        .unwrap();
        let config = SshConfigFile::parse_files(&user, &dir.path().join("absent"));
        let work = config.settings("work");
        assert_eq!(work.host_name.as_deref(), Some("git.example.com"));
        assert_eq!(work.port, Some(2222));
        assert!(config
            .settings("bastion")
            .refusal()
            .is_some_and(|s| s.contains("ProxyJump")));
        // The block after the include is still read as its own.
        assert_eq!(config.settings("after").port, Some(2200));
        assert_eq!(config.settings("after").user.as_deref(), Some("deploy"));
        assert_eq!(config.settings("work").user, None);
        // A file that is not there is skipped, never fatal.
        let missing = dir.path().join("only-missing");
        std::fs::write(
            &missing,
            "Include /no/such/file
Host h
  Port 24
",
        )
        .unwrap();
        assert_eq!(
            SshConfigFile::parse_files(&missing, &dir.path().join("absent"))
                .settings("h")
                .port,
            Some(24)
        );
    }

    /// An include that names its own file must stop, not recurse for
    /// ever.
    #[test]
    fn an_include_cycle_stops() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("config");
        std::fs::write(
            &user,
            "Include config
Host h
  Port 25
",
        )
        .unwrap();
        assert_eq!(
            SshConfigFile::parse_files(&user, &dir.path().join("absent"))
                .settings("h")
                .port,
            Some(25)
        );
    }

    #[test]
    fn the_identity_agent_is_put_back_when_the_contact_is_over() {
        let before = std::env::var("SSH_AUTH_SOCK").ok();
        std::env::set_var("SSH_AUTH_SOCK", "/run/the-usual-agent");
        let one_password = HostSettings {
            identity_agent: Some("/run/1password/agent.sock".to_string()),
            ..HostSettings::default()
        };
        {
            let scope = AgentScope::apply(&one_password);
            assert_eq!(
                std::env::var("SSH_AUTH_SOCK").as_deref(),
                Ok("/run/1password/agent.sock")
            );
            assert_eq!(scope.socket().as_deref(), Some("/run/1password/agent.sock"));
        }
        assert_eq!(
            std::env::var("SSH_AUTH_SOCK").as_deref(),
            Ok("/run/the-usual-agent"),
            "the next host must not inherit this host's agent"
        );
        // A host that names none, or names the variable itself, or
        // names `none`, touches nothing at all.
        for settings in [
            HostSettings::default(),
            HostSettings {
                identity_agent: Some("SSH_AUTH_SOCK".to_string()),
                ..HostSettings::default()
            },
            HostSettings {
                identity_agent: Some("none".to_string()),
                ..HostSettings::default()
            },
        ] {
            let scope = AgentScope::apply(&settings);
            assert_eq!(
                std::env::var("SSH_AUTH_SOCK").as_deref(),
                Ok("/run/the-usual-agent")
            );
            assert_eq!(scope.socket(), None);
        }
        match before {
            Some(value) => std::env::set_var("SSH_AUTH_SOCK", value),
            None => std::env::remove_var("SSH_AUTH_SOCK"),
        }
    }

    #[test]
    fn a_wildcard_pattern_applies_to_the_hosts_it_matches() {
        let config = SshConfigFile::parse("Host *.example.com\n  User deploy\n  Port 2200\n");
        let inside = config.settings("git.example.com");
        assert_eq!(inside.user.as_deref(), Some("deploy"));
        assert_eq!(inside.port, Some(2200));
        assert_eq!(config.settings("github.com").user, None);
    }

    /// The home joy reads is the home a `~` in the config expands with.
    ///
    /// ssh2-config expands it with the `dirs` crate, which on Windows
    /// reads no environment variable, so the path came back under the
    /// machine's profile while every other path in the same run
    /// followed `USERPROFILE` (JOY-02A7-A2, found on the Windows
    /// runner).
    #[test]
    fn a_tilde_is_expanded_with_the_home_joy_reads() {
        let home = home_dir().expect("a home");
        assert_eq!(
            with_joys_own_home("IdentityFile", "~/.ssh/work_ed25519"),
            Some(format!("{}/.ssh/work_ed25519", home.display()))
        );
        assert_eq!(
            with_joys_own_home("identityfile", "\"~/my keys/id\""),
            Some(format!("\"{}/my keys/id\"", home.display()))
        );
        // Not every directive, and not a path that names no home.
        assert_eq!(with_joys_own_home("HostName", "~/nonsense"), None);
        assert_eq!(with_joys_own_home("IdentityFile", "/etc/ssh/id"), None);
        assert_eq!(with_joys_own_home("IdentityFile", "~someone/id"), None);
    }
}
