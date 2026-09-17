// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The credential resolver of design D1.1, assembled (package J4b).
//!
//! [`super::forge::Auth::Local`] keeps its name and its place; this
//! module is the body it grew. For one operation it answers three
//! questions, in this order:
//!
//! 1. Which transport carries the credential (D1.2)? The candidates of
//!    a configured ssh remote come FIRST, and the https twin is a
//!    consequence of "no working local ssh credential", never of "a
//!    token exists". A host whose memory says `ssh-worked` never goes
//!    to the twin, whatever tokens exist.
//! 2. What is the twin's address (D1.5)? The connector's `web-url` is
//!    the source of truth, the engine's table for github.com,
//!    gitlab.com and codeberg.org is the fallback, and four conditions
//!    refuse the twin outright.
//! 3. Which credential rides on it (D1.6, D1.7)? A forge token asked
//!    from the connector per HOST and cached with a TTL, so a 1 Hz chat
//!    poll spawns no connector per contact.
//!
//! What it never does: write into the person's `.git/config`. The
//! transport that authenticated, together with the credential source,
//! goes into joy's own state file (`<app_state_dir>/forge-state.json`,
//! mode 0600), and nowhere else.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::contact::{ContactDirection, Transport};
use super::forge::ForgeKind;
use crate::host::HostKind;

// ---------------------------------------------------------------------
// The transport memory (D1.2, D1.5)
// ---------------------------------------------------------------------

/// What joy learnt about a host's ssh transport. Three words, each with
/// its own consequence for the next operation (D1.2 rule 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportState {
    /// An ssh contact to this host authenticated. Such a host never
    /// goes to the twin, whatever tokens exist.
    SshWorked,
    /// joy established BEFORE a contact that this machine has no usable
    /// ssh credential for the host: no agent or no identity in it, and
    /// no key file that survived pre-validation (D1.2 rule 3a).
    NoSshCredential,
    /// An ssh contact failed with an authentication class failure
    /// (D1.2 rule 3b, `class == Ssh` with `code == Auth`).
    SshFailed,
}

impl TransportState {
    /// The word the state file carries, which is also the word D1.2
    /// uses.
    pub fn as_str(self) -> &'static str {
        match self {
            TransportState::SshWorked => "ssh-worked",
            TransportState::NoSshCredential => "no-ssh-credential",
            TransportState::SshFailed => "ssh-failed",
        }
    }

    /// Whether this state sends the next contact to the twin.
    fn wants_twin(self) -> bool {
        matches!(
            self,
            TransportState::NoSshCredential | TransportState::SshFailed
        )
    }
}

/// The ssh facts an entry was written under. `no-ssh-credential` is
/// dropped as soon as one of them changes (D1.2 rule 3a: "as soon as
/// `SSH_AUTH_SOCK` appears, an agent identity appears, or a candidate
/// key file's mtime changes").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshSignals {
    /// The socket `SSH_AUTH_SOCK` named, or the one the host's
    /// `IdentityAgent` names. `None` is "no agent was pointed at".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_socket: Option<String>,
    /// How many identities that agent held.
    #[serde(default)]
    pub agent_identities: u32,
    /// The modification time of every key file joy would consider for
    /// this host, by path. A file that is not there carries 0, so a key
    /// that APPEARS changes the map too.
    #[serde(default)]
    pub keys: BTreeMap<String, u64>,
}

/// One host's row in the state file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMemory {
    pub state: TransportState,
    /// When the row was written, in seconds since the epoch. The TTL of
    /// D1.2 is read from here.
    #[serde(default)]
    pub at: u64,
    /// The transport that authenticated: `ssh` or `https`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    /// The credential source that answered: `agent`, `key`, `helper` or
    /// `token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// The user name the token was sent under, remembered per host
    /// beside the transport (D1.6, last paragraph).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(default)]
    pub signals: SshSignals,
}

impl HostMemory {
    /// A row for `state`, stamped now.
    pub fn new(state: TransportState) -> HostMemory {
        HostMemory {
            state,
            at: unix_now(),
            transport: None,
            credential: None,
            shape: None,
            signals: SshSignals::default(),
        }
    }

    /// Say which transport and which credential carried the contact.
    pub fn with_credential(mut self, transport: Transport, credential: &str) -> HostMemory {
        self.transport = Some(match transport {
            Transport::Ssh => "ssh".to_string(),
            Transport::Https => "https".to_string(),
            Transport::Local => "local".to_string(),
        });
        self.credential = Some(credential.to_string());
        self
    }

    /// Say what the ssh probe saw, so the entry can be invalidated when
    /// the machine changes under it.
    pub fn with_signals(mut self, signals: SshSignals) -> HostMemory {
        self.signals = signals;
        self
    }

    /// Whether the row is still inside the 24 hour TTL of D1.2.
    pub fn fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.at) < MEMORY_TTL.as_secs()
    }

    /// The sentence a surface may show for "which credential did joy
    /// use here" (acceptance of J4b: the twin contact "says which
    /// credential it used"). It names no secret, only its source.
    pub fn sentence(&self, host: &str) -> String {
        let credential = match self.credential.as_deref() {
            Some("token") => "the access token from your forge sign in",
            Some("helper") => "the credential from your credential helper",
            Some("agent") => "an identity from your ssh agent",
            Some("key") => "one of your ssh key files",
            _ => "no credential",
        };
        let transport = match self.transport.as_deref() {
            Some("https") => "https",
            Some("ssh") => "ssh",
            _ => "the configured remote",
        };
        format!("joy reached {host} over {transport} with {credential}")
    }
}

/// How long a transport memory row stands (D1.2 rule 3).
pub const MEMORY_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The name of joy's own state file inside `app_state_dir()`.
pub const STATE_FILE: &str = "forge-state.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StateFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    hosts: BTreeMap<String, HostMemory>,
}

static STATE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Put the transport memory somewhere else than `app_state_dir()`.
///
/// The file is process state, exactly like the throttle's gap table
/// ([`super::contact::set_gaps`]) and the connector search path
/// ([`crate::forge_plugins::set_plugin_dirs`]), and it is set the same
/// way: by a host that knows better, and by the tests, which must never
/// write into the person's own state directory.
///
/// `None` puts it back where D1.2 says it lives, exactly as
/// `set_gaps("")` and `set_plugin_dirs(vec![])` restore their own
/// defaults. It does NOT switch the memory off: a host that asked for
/// the default and got no memory at all would lose the whole of D1.2's
/// transport memory without a word.
pub fn set_state_file(path: Option<PathBuf>) {
    *STATE_PATH.lock().unwrap_or_else(|e| e.into_inner()) = path;
}

/// Where the transport memory lives: `<app_state_dir>/forge-state.json`
/// (D1.2), or wherever [`set_state_file`] pointed it.
pub fn state_file() -> Option<PathBuf> {
    if let Some(override_path) = STATE_PATH.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Some(override_path);
    }
    crate::auth::session::app_state_dir()
        .ok()
        .map(|dir| dir.join(STATE_FILE))
}

fn read_state(path: &Path) -> StateFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<StateFile>(&text).ok())
        .unwrap_or_default()
}

/// Write the file at mode 0600 (D1.2). Best effort: a state directory
/// that cannot be written costs a memory row, never the operation.
fn write_state(path: &Path, state: &StateFile) {
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(json) = serde_json::to_string_pretty(state) else {
        return;
    };
    if write_private(path, json.as_bytes()).is_err() {
        tracing::debug!(path = %path.display(), "the forge state file could not be written");
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    // A file that already existed keeps its old mode, so the mode above
    // is only the CREATE mode; this is the one that holds either way.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    file.flush()
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// The lock beside the state file: two joy processes on one machine
/// read, change and write the same rows, and the loser of that race
/// would drop the winner's row. The primitive is J4a's
/// ([`crate::util::file_lock`]); a lock that cannot be had costs the
/// row, never the operation.
fn with_state<T>(work: impl FnOnce(&Path, &mut StateFile) -> T) -> Option<T> {
    let path = state_file()?;
    let lock_path = path.with_extension("json.lock");
    let _lock = crate::util::file_lock::exclusive(&lock_path, Duration::from_millis(500)).ok();
    let mut state = read_state(&path);
    Some(work(&path, &mut state))
}

/// What joy remembers about this host, if the row is still fresh. An
/// expired row is not read and not deleted: the next write replaces it.
pub fn recall(host: &str) -> Option<HostMemory> {
    let host = host.to_ascii_lowercase();
    let path = state_file()?;
    let state = read_state(&path);
    state
        .hosts
        .get(&host)
        .filter(|memory| memory.fresh(unix_now()))
        .cloned()
}

/// Write this host's row.
pub fn remember(host: &str, memory: HostMemory) {
    if host.is_empty() {
        return;
    }
    let host = host.to_ascii_lowercase();
    with_state(|path, state| {
        state.version = 1;
        state.hosts.insert(host, memory);
        write_state(path, state);
    });
}

/// Drop this host's row, because one of the facts it was written under
/// changed (D1.2 rule 3a).
pub fn forget(host: &str) {
    let host = host.to_ascii_lowercase();
    with_state(|path, state| {
        if state.hosts.remove(&host).is_some() {
            write_state(path, state);
        }
    });
}

/// The sentence for "which credential did joy use for this host", from
/// the memory of the last contact that authenticated.
pub fn used(host: &str) -> Option<String> {
    let memory = recall(host)?;
    memory.credential.as_ref()?;
    Some(memory.sentence(host))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// The ssh probe (D1.2 rule 3a)
// ---------------------------------------------------------------------

/// What this machine can offer a host over ssh, established BEFORE the
/// contact. `candidates` is the length of the chain of D1.4: zero is
/// "no usable ssh credential", which is trigger (a) of D1.2.
#[derive(Debug, Clone, Default)]
pub struct SshProbe {
    pub candidates: usize,
    pub signals: SshSignals,
    /// Why each missing candidate is missing, in the person's words.
    pub notes: Vec<String>,
}

impl SshProbe {
    /// A probe that found nothing, for the callers that need one
    /// without touching the machine.
    pub fn empty() -> SshProbe {
        SshProbe::default()
    }

    /// Whether the chain has anything at all to offer.
    pub fn usable(&self) -> bool {
        self.candidates > 0
    }
}

/// Ask the machine what it holds for `host`, without contacting the
/// forge: the agent probe of D1.4 and the key file pre-validation, plus
/// the facts the answer is invalidated by.
pub fn probe_ssh(host: &str, configured: Option<&str>, kind: HostKind) -> SshProbe {
    let settings = super::ssh_config::for_contact(host, configured);
    // The agent variable is pointed at this host's `IdentityAgent` for
    // as long as the probe lasts, exactly as a contact does it, so the
    // probe reads the agent the contact would read (D1.4).
    let _scope = super::ssh_config::AgentScope::apply(&settings);
    let agent = super::ssh_auth::probe_agent();
    let url_user = configured
        .and_then(super::remote_url::RemoteUrl::parse)
        .and_then(|parsed| parsed.user);
    let chain = super::ssh_auth::chain_for(
        host,
        url_user.as_deref(),
        &settings,
        kind,
        &agent,
        cfg!(windows),
    );
    let mut keys = BTreeMap::new();
    for path in super::ssh_auth::identity_files(&settings) {
        keys.insert(path.display().to_string(), mtime_of(&path));
    }
    let (agent_socket, agent_identities) = match &agent {
        super::ssh_auth::Agent::Missing => (None, 0),
        super::ssh_auth::Agent::Unreachable { socket, .. }
        | super::ssh_auth::Agent::Empty { socket } => (Some(socket.clone()), 0),
        super::ssh_auth::Agent::Ready { socket, identities } => (Some(socket.clone()), *identities),
    };
    SshProbe {
        candidates: chain.candidates.len(),
        signals: SshSignals {
            agent_socket,
            agent_identities,
            keys,
        },
        notes: chain.notes,
    }
}

fn mtime_of(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// The twin (D1.5)
// ---------------------------------------------------------------------

/// Why a remote has no https twin. Each one falls back to the
/// configured remote and says so (D1.5, "Refusal conditions").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwinRefusal {
    /// No connector claims the host and it is not one of the three the
    /// engine's own table knows.
    UnknownHost,
    /// Fewer than two path segments: there is no `owner/repo` to build
    /// a web address from.
    ShortPath,
    /// The host has no dot, so it is a local alias and not a forge
    /// address.
    NotAHostName,
    /// A connector claims the host but did not name its web base, and
    /// the engine's table does not know the host either. Only the
    /// connector knows where a self hosted instance answers, which may
    /// be a nested sub path or a different host entirely.
    NoWebBase,
    /// An `insteadOf` or a `pushInsteadOf` rule matches the twin.
    /// `git_remote_create_anonymous` applies those rules
    /// (remote.c:237-256) and git2 0.21 binds neither
    /// `git_remote_create_with_opts` nor
    /// `GIT_REMOTE_CREATE_SKIP_INSTEADOF`, so joy predicts the rule
    /// rather than meeting it.
    InsteadOf { rewritten: String },
}

impl TwinRefusal {
    /// The sentence the person reads for this refusal.
    pub fn sentence(&self, host: &str) -> String {
        match self {
            TwinRefusal::UnknownHost => format!(
                "no forge connector claims {host} and joy's own table does not know it, so there is no https address to try"
            ),
            TwinRefusal::ShortPath => format!(
                "the remote on {host} names no owner and repository, so joy cannot build its https address"
            ),
            TwinRefusal::NotAHostName => {
                format!("{host} is not a forge host name, so joy built no https address for it")
            }
            TwinRefusal::NoWebBase => format!(
                "the forge connector that claims {host} did not name its https address, so joy has no https address to try"
            ),
            TwinRefusal::InsteadOf { rewritten } => format!(
                "your git config rewrites the https address of {host} to {rewritten}, so joy stays on the remote you configured"
            ),
        }
    }
}

/// The engine's own table, for the three hosts of D1.5 and for nothing
/// else. `ssh.github.com` is github.com and `altssh.gitlab.com` is
/// gitlab.com, because the two public forges answer ssh under those
/// sub-domains.
fn table_host(host: &str) -> Option<&'static str> {
    match host.trim().to_ascii_lowercase().as_str() {
        "github.com" | "ssh.github.com" => Some("github.com"),
        "gitlab.com" | "altssh.gitlab.com" => Some("gitlab.com"),
        "codeberg.org" => Some("codeberg.org"),
        _ => None,
    }
}

/// The https twin of a remote from the engine's table alone, with the
/// refusals that do not need a connector.
///
/// The parse mirrors `git_net_url_parse_standard_or_scp`: the scp form
/// yields a path without a leading slash and the `ssh://` form with
/// one, and the bracketed port form `[git@host:2222]:owner/repo.git` is
/// not part of the host (joy's own parser, JOY-02A2-27).
///
/// On "the host does not resolve": no name lookup is made here. A twin
/// only exists for a host the table names by hand or a connector
/// claimed by answering `web-url` for it, so a name that resolves
/// nowhere cannot reach this point, and a DNS round trip per contact
/// would buy nothing.
pub fn twin_from_table(url: &str) -> Result<String, TwinRefusal> {
    let parsed = super::remote_url::RemoteUrl::parse(url).ok_or(TwinRefusal::NotAHostName)?;
    let host = parsed.host.as_str();
    if !host.contains('.') {
        return Err(TwinRefusal::NotAHostName);
    }
    let web = table_host(host).ok_or(TwinRefusal::UnknownHost)?;
    let path = twin_path(&parsed.path)?;
    Ok(format!("https://{web}/{path}"))
}

/// The `owner/repo` part of a twin, refused when there are fewer than
/// two segments (D1.5).
fn twin_path(path: &str) -> Result<String, TwinRefusal> {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    if path.split('/').filter(|s| !s.is_empty()).count() < 2 {
        return Err(TwinRefusal::ShortPath);
    }
    Ok(path.to_string())
}

/// The URL an `insteadOf` (fetch) or `pushInsteadOf` (push) rule
/// rewrites `url` to, or `None` when no rule matches.
///
/// This is libgit2's own longest prefix rule, reimplemented
/// (remote.c:3079-3140): the entry whose VALUE is the longest prefix of
/// the URL wins, and the replacement is the middle of the entry's name,
/// `url.<replacement>.insteadof`. On the push side git tries
/// `pushInsteadOf` first and falls back to `insteadOf`, which is what
/// `git_remote_create_with_opts` does when it fills `url` and `pushurl`
/// from the two globs.
pub fn insteadof_rewrite(
    config: &git2::Config,
    url: &str,
    direction: ContactDirection,
) -> Option<String> {
    // libgit2's own two patterns, ESCAPED dots and all: the glob is a
    // regex and an unescaped `url.*.insteadof` also matches
    // `url.<x>.pushinsteadof`, which would read a push rule on a fetch
    // (remote.c:3096-3102).
    let globs: &[&str] = match direction {
        ContactDirection::Push => &[r"url\..*\.pushinsteadof", r"url\..*\.insteadof"],
        ContactDirection::Fetch => &[r"url\..*\.insteadof"],
    };
    for glob in globs {
        if let Some(rewritten) = longest_prefix(config, glob, url) {
            return Some(rewritten);
        }
    }
    None
}

fn longest_prefix(config: &git2::Config, glob: &str, url: &str) -> Option<String> {
    let entries = config.entries(Some(glob)).ok()?;
    let mut best: Option<(usize, String)> = None;
    let _ = entries.for_each(|entry| {
        let Ok(prefix) = entry.value() else {
            return;
        };
        if prefix.is_empty() || !url.starts_with(prefix) {
            return;
        }
        if best.as_ref().is_some_and(|(len, _)| prefix.len() <= *len) {
            return;
        }
        // `url.<replacement>.insteadof`: the replacement is everything
        // between the first and the last dot, and the name libgit2
        // reports is already lowercased.
        let Ok(name) = std::str::from_utf8(entry.name_bytes()) else {
            return;
        };
        let Some(rest) = name.strip_prefix("url.") else {
            return;
        };
        let Some((replacement, _)) = rest.rsplit_once('.') else {
            return;
        };
        best = Some((
            prefix.len(),
            format!("{replacement}{}", &url[prefix.len()..]),
        ));
    });
    best.map(|(_, rewritten)| rewritten)
}

// ---------------------------------------------------------------------
// What a connector says about a host (D1.7, D2.4, D4.1c)
// ---------------------------------------------------------------------

/// The token a connector handed out for one host. It carries no
/// `Debug`, so no `{:?}` anywhere in joy can print it by accident, and
/// no field of it is ever logged.
#[derive(Clone)]
pub struct HostToken {
    pub token: String,
    /// The forge kind the connector claims, which decides the user name
    /// the twin presents beside the token (D1.6).
    pub kind: Option<ForgeKind>,
    /// Which login the token belongs to, for the surface (D4.1c). Never
    /// a secret.
    pub login: Option<String>,
    /// `keychain`, `file`, `gh`, `glab`, `tea` or `env`.
    pub source: Option<String>,
}

/// Everything a connector answered about one remote, cached per remote
/// (D1.7: "asked from the plugin per host, not per contact").
#[derive(Clone, Default)]
pub struct HostFacts {
    /// The forge kind the connector claims for the host, if one does.
    pub claimed: Option<ForgeKind>,
    /// Whether a connector claimed the host at all. A claimed host may
    /// have a twin even when it is in no table (D1.5).
    pub claimed_by_plugin: bool,
    /// The connector's own `web-url` answer, which beats the table.
    pub web_url: Option<String>,
    pub token: Option<HostToken>,
}

impl HostFacts {
    /// The facts of a machine with no connector at all, for the callers
    /// and tests that supply their own.
    pub fn none() -> HostFacts {
        HostFacts::default()
    }
}

struct CachedFacts {
    facts: HostFacts,
    until: Instant,
}

static FACTS: Mutex<Option<BTreeMap<String, CachedFacts>>> = Mutex::new(None);

/// The longest a connector answer is reused when it named no expiry
/// (D1.7: `min(expires_at - 60 s, 5 minutes)`).
const FACTS_TTL: Duration = Duration::from_secs(5 * 60);

/// How long the ABSENCE of a token is reused. D1.7 gives a TTL to a
/// token, not to its lack: a person who has just run `joy forge login`
/// in another process must not be told "nobody is signed in to
/// github.com" for the next five minutes by a desktop that cached the
/// refusal. The window exists only so that one operation's two legs and
/// the next tick of a poll do not each spawn a connector; five seconds
/// is five ticks of the fastest poll D1.9 allows, which is one second.
const NO_TOKEN_TTL: Duration = Duration::from_secs(5);

/// The grace D1.7 takes off a stated expiry.
const EXPIRY_GRACE: Duration = Duration::from_secs(60);

/// Forget everything a connector said about this host: a 401 does this
/// immediately and triggers one re-ask (D1.7).
pub fn invalidate_facts(host: &str) {
    let host = host.to_ascii_lowercase();
    FACTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(BTreeMap::new)
        .retain(|key, _| !key.starts_with(&format!("{host}\u{1}")));
}

/// Drop every cached connector answer (the tests, and a logout).
pub fn invalidate_all_facts() {
    FACTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(BTreeMap::new)
        .clear();
}

/// The key one connector answer is remembered under.
///
/// The ACCESS is part of it, because the call really carries `--for
/// read|write` and on a host with several logins the direction is what
/// tells a login that may only read the repository from one that may
/// push to it (D4.1c step 4). Without it a fetch that ran first would
/// lend its read scoped token to the push behind it, the forge would
/// answer 403 on receive-pack, and D1.8b would read that as
/// `no_push_rights` for a person who may push.
fn facts_key(host: &str, remote: &str, access: crate::forge_plugins::Access) -> String {
    format!(
        "{}\u{1}{remote}\u{1}{}",
        host.to_ascii_lowercase(),
        access.as_str()
    )
}

fn cached_facts(key: &str) -> Option<HostFacts> {
    let now = Instant::now();
    let mut guard = FACTS.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(BTreeMap::new);
    let entry = cache.get(key)?;
    if entry.until <= now {
        cache.remove(key);
        return None;
    }
    Some(entry.facts.clone())
}

fn cache_facts(key: &str, facts: &HostFacts, ttl: Duration) {
    FACTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(BTreeMap::new)
        .insert(
            key.to_string(),
            CachedFacts {
                facts: facts.clone(),
                until: Instant::now() + ttl,
            },
        );
}

/// How long a token answer may be reused: `min(expires_at - 60 s, 5
/// minutes)`, and never a negative span (D1.7).
fn token_ttl(expires_at: Option<&str>) -> Duration {
    let Some(expiry) = expires_at.and_then(parse_expiry) else {
        return FACTS_TTL;
    };
    let now = SystemTime::now();
    let left = expiry
        .duration_since(now)
        .unwrap_or(Duration::ZERO)
        .saturating_sub(EXPIRY_GRACE);
    left.min(FACTS_TTL)
}

fn parse_expiry(text: &str) -> Option<SystemTime> {
    let at = chrono::DateTime::parse_from_rfc3339(text.trim()).ok()?;
    let seconds = at.timestamp();
    if seconds < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(seconds as u64))
}

/// The login this project pinned for this host (D4.1c, "Where the pin
/// lives"): the per project app state file joy-core computes, under
/// `forgeLogin`. Never `project.yaml`, which is committed and synced to
/// the forge.
pub fn pinned_login(root: &Path, host: &str) -> Option<String> {
    let path = crate::auth::session::app_state_project_file(root).ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let state: serde_json::Value = serde_json::from_str(&text).ok()?;
    state
        .get("forgeLogin")?
        .get(host)?
        .as_str()
        .map(str::to_string)
        .filter(|login| !login.is_empty())
}

/// Ask the connectors what they know about this remote, at most once
/// per host per TTL (D1.7). Everything here is best effort: a machine
/// with no connector answers [`HostFacts::none`] and the resolver falls
/// back to the engine's table and to the machine's own credentials.
pub fn host_facts(
    remote: &str,
    root: Option<&Path>,
    kind: HostKind,
    direction: ContactDirection,
) -> HostFacts {
    let host = super::contact::host_of(remote);
    let access = match direction {
        ContactDirection::Push => crate::forge_plugins::Access::Write,
        ContactDirection::Fetch => crate::forge_plugins::Access::Read,
    };
    let key = facts_key(&host, remote, access);
    if let Some(facts) = cached_facts(&key) {
        return facts;
    }
    let mut context = match root {
        Some(root) => crate::forge_plugins::CallContext::in_project(root),
        None => crate::forge_plugins::CallContext::rootless(),
    }
    .with_host_kind(kind);
    if let Some(login) = root.and_then(|root| pinned_login(root, &host)) {
        context = context.with_facts(crate::forge_plugins::CallerFacts {
            login: Some(login),
            ..Default::default()
        });
    }
    let target = crate::forge_plugins::Target::remote(remote);
    let Some(spec) = crate::forge_plugins::FORGE_PLUGINS
        .iter()
        .find(|spec| crate::forge_plugins::claims(spec, &target, &context))
    else {
        // Nothing claimed it. Remembered as well, so a poll on a
        // machine with no connector spawns nothing per contact.
        let facts = HostFacts::none();
        cache_facts(&key, &facts, FACTS_TTL);
        return facts;
    };
    let answer = crate::forge_plugins::token_for(spec, &target, access, &context);
    let mut ttl = FACTS_TTL;
    let token = match answer {
        Ok(answer) if answer.known => {
            ttl = token_ttl(answer.expires_at.as_deref());
            answer
                .token
                .filter(|t| !t.is_empty())
                .map(|token| HostToken {
                    token,
                    kind: token_kind(spec.id, answer.username.as_deref()),
                    login: answer.login.clone(),
                    source: answer.source.clone(),
                })
        }
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(forge = %host, error = %e, "the forge connector answered no token");
            None
        }
    };
    // A token that is not there is not a token: it is remembered for
    // the short window above and re-asked, so a sign in that happened
    // in another process is seen within seconds (D1.7).
    if token.is_none() {
        ttl = NO_TOKEN_TTL;
    }
    let facts = HostFacts {
        claimed: ForgeKind::from_plugin_id(spec.id),
        claimed_by_plugin: true,
        web_url: crate::forge_plugins::web_url(spec, &target, &context),
        token,
    };
    cache_facts(&key, &facts, ttl.max(Duration::from_secs(1)));
    facts
}

/// The forge kind whose shape the twin should present (D1.6).
///
/// The connector's own `username` wins where it names one of the two
/// shapes D1.6 allows, because only the connector knows an instance
/// that presents the other one; otherwise the kind the connector id
/// names decides. The third shape, a token as the user name with an
/// empty password, is never one of the answers.
///
/// A name never DECIDES the kind on its own, because two families share
/// each of the two names: `x-access-token` is GitHub and GitHub
/// Enterprise Server, and `oauth2` is GitLab, Gitea, Forgejo and
/// Codeberg (D1.6). Where the connector's own id already names a family
/// of that shape, the id keeps it: the kind travels on into
/// `Auth::ClaimedToken` and into the remembered `shape`, and a Gitea
/// recorded as GitLab is inherited by the next reader of the row.
fn token_kind(plugin_id: &str, username: Option<&str>) -> Option<ForgeKind> {
    let claimed = ForgeKind::from_plugin_id(plugin_id);
    let of_shape = |shape: &str| claimed.filter(|kind| kind.token_user() == shape);
    match username.map(str::trim) {
        Some("x-access-token") => of_shape("x-access-token").or(Some(ForgeKind::GitHub)),
        Some("oauth2") => of_shape("oauth2").or(Some(ForgeKind::GitLab)),
        _ => claimed,
    }
}

// ---------------------------------------------------------------------
// The plan (D1.2)
// ---------------------------------------------------------------------

/// Which of the two addresses a leg dials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    /// The remote as the person configured it.
    Configured,
    /// The https twin of D1.5, never written into `.git/config`.
    Twin,
}

/// What a leg presents.
#[derive(Clone)]
pub enum LegCredential {
    /// The machine's own chain: the ssh candidates of D1.4, or the
    /// credential helper of D1.3.
    Machine,
    /// A forge token from a connector, with the helper chain behind it
    /// inside the same contact (D1.2 rule 1).
    Token(HostToken),
}

impl LegCredential {
    /// The word the transport memory and the evidence carry.
    pub fn word(&self) -> &'static str {
        match self {
            LegCredential::Machine => "machine",
            LegCredential::Token(_) => "token",
        }
    }

    /// Whether this leg carries a credential of its own.
    pub fn is_token(&self) -> bool {
        matches!(self, LegCredential::Token(_))
    }
}

/// Never the token, whatever the format string says.
impl std::fmt::Debug for LegCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LegCredential::Machine => f.write_str("Machine"),
            LegCredential::Token(token) => f
                .debug_struct("Token")
                .field("kind", &token.kind)
                .field("login", &token.login)
                .field("source", &token.source)
                .finish(),
        }
    }
}

/// One contact the resolver is willing to make.
#[derive(Debug, Clone)]
pub struct Leg {
    pub way: Way,
    pub url: String,
    pub transport: Transport,
    pub credential: LegCredential,
}

/// The candidate order of D1.2 for one operation, with the sentences
/// that say why it looks the way it does.
///
/// At most two legs, which is D1.2's "at most two contacts per
/// operation": a third attempt happens only after a person acted.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The host the memory and the ownership join key on: the host of
    /// the CONFIGURED remote, whatever the twin dials. It is the person's
    /// own address for this forge, so `ssh.github.com` stays
    /// `ssh.github.com` in their row and in the join.
    ///
    /// The THROTTLE is not keyed on it and must not be: D1.9's budget is
    /// requests per second per host per machine, and the machine that
    /// counts them is the one a socket is opened to. Each leg is
    /// therefore charged to the host it really dials, which is
    /// github.com for the twin of an `ssh.github.com` remote.
    pub host: String,
    pub legs: Vec<Leg>,
    pub notes: Vec<String>,
    /// What the machine held for this host when the plan was made. The
    /// memory row a contact writes records these, so the row can be
    /// dropped as soon as one of them changes (D1.2 rule 3a).
    pub probe: SshProbe,
}

impl Plan {
    /// The one leg of a caller that has its own credential (the
    /// platform's token), which is the engine as it was.
    pub fn single(url: &str, credential: LegCredential) -> Plan {
        Plan {
            host: super::contact::host_of(url),
            legs: vec![Leg {
                way: Way::Configured,
                url: url.to_string(),
                transport: super::contact::transport_of(url),
                credential,
            }],
            notes: Vec::new(),
            probe: SshProbe::empty(),
        }
    }

    /// Whether the plan ever leaves the configured remote.
    pub fn uses_twin(&self) -> bool {
        self.legs.iter().any(|leg| leg.way == Way::Twin)
    }

    /// The one sentence that says why the plan looks the way it does.
    pub fn why(&self) -> String {
        self.notes.join("; ")
    }
}

/// The candidate order for one operation, from facts that are already
/// gathered. Pure: every machine fact, every connector answer and the
/// git config are arguments, so the rule of D1.2 can be read and tested
/// without a forge, an agent or a connector.
///
/// `kind` is the host kind of D1.1, and it decides exactly one thing
/// here: whether the twin may be dialled with no credential at all
/// (D1.2, "No anonymous polling").
pub fn plan_with(
    configured: &str,
    facts: &HostFacts,
    memory: Option<&HostMemory>,
    probe: &SshProbe,
    kind: HostKind,
    insteadof: &dyn Fn(&str) -> Option<String>,
) -> Plan {
    let host = super::contact::host_of(configured);
    let transport = super::contact::transport_of(configured);
    let machine = Leg {
        way: Way::Configured,
        url: configured.to_string(),
        transport,
        credential: LegCredential::Machine,
    };
    let mut notes = Vec::new();
    if transport != Transport::Ssh {
        // D1.2 rule 1: the forge token first, then a credential from
        // joy's own helper runner, all inside ONE contact. The token
        // rides the configured remote; there is no twin to build.
        let credential = match (&facts.token, transport) {
            (Some(token), Transport::Https) => LegCredential::Token(token.clone()),
            _ => LegCredential::Machine,
        };
        return Plan {
            host,
            legs: vec![Leg {
                credential,
                ..machine
            }],
            notes,
            probe: probe.clone(),
        };
    }

    // An ssh remote. The twin is a consequence of "no working local ssh
    // credential", never of "a token exists" (D1.2 rule 2).
    let twin = twin_leg(configured, &host, facts, kind, insteadof, &mut notes);
    let state = memory.map(|memory| memory.state);
    if state == Some(TransportState::SshWorked) {
        notes.push(format!(
            "ssh worked for {host} before, so joy stays on it whatever tokens exist"
        ));
        return Plan {
            host,
            legs: vec![machine],
            notes,
            probe: probe.clone(),
        };
    }
    let no_credential = !probe.usable();
    if no_credential {
        notes.extend(probe.notes.iter().cloned());
    }
    let wants_twin = no_credential || state.is_some_and(TransportState::wants_twin);
    let Some(twin) = twin else {
        return Plan {
            host,
            legs: vec![machine],
            notes,
            probe: probe.clone(),
        };
    };
    if wants_twin {
        if let Some(state) = state {
            notes.push(format!(
                "joy remembers {} for {host}, so it tries the https address first",
                state.as_str()
            ));
        }
        // Nothing to offer over ssh at all: the second contact would
        // present the same empty chain, so there is no second leg.
        let legs = if no_credential {
            vec![twin]
        } else {
            vec![twin, machine]
        };
        return Plan {
            host,
            legs,
            notes,
            probe: probe.clone(),
        };
    }
    Plan {
        host,
        legs: vec![machine, twin],
        notes,
        probe: probe.clone(),
    }
}

/// The twin leg, or `None` with the refusal written into `notes`.
fn twin_leg(
    configured: &str,
    host: &str,
    facts: &HostFacts,
    kind: HostKind,
    insteadof: &dyn Fn(&str) -> Option<String>,
    notes: &mut Vec<String>,
) -> Option<Leg> {
    // The address the person's ssh config really names: `git@work:o/r`
    // with `HostName github.com` has the twin of github.com, while the
    // configured ssh URL stays the ssh candidate (D1.5).
    let dialled = super::ssh_config::effective_url(configured);
    let source = dialled.as_deref().unwrap_or(configured);
    let url = match facts.web_url.clone() {
        Some(url) => url,
        None => match twin_from_table(source) {
            Ok(url) => url,
            // A host the table does not know is refused by two
            // different sentences, because the two are two different
            // situations for the person: nobody claims this host at
            // all, or a connector claims it and did not say where its
            // web base is.
            Err(TwinRefusal::UnknownHost) if facts.claimed_by_plugin => {
                notes.push(TwinRefusal::NoWebBase.sentence(host));
                return None;
            }
            Err(refusal) => {
                notes.push(refusal.sentence(host));
                return None;
            }
        },
    };
    if let Some(rewritten) = insteadof(&url) {
        notes.push(TwinRefusal::InsteadOf { rewritten }.sentence(host));
        return None;
    }
    // "Nobody is signed in" is NOT one of D1.5's four refusals, and
    // D1.2 allows exactly one contact without a credential: "A person
    // initiated one off operation (a clone, an explicit 'check now') may
    // contact an https remote with no credential. A poll or a worker
    // tick may not." A public repository whose remote is ssh, on a
    // machine with no readable key and nobody signed in, is reachable
    // over the twin and over nothing else; refusing it here reported an
    // ssh fault for a repository anyone may read.
    let credential = match facts.token.clone() {
        Some(token) => LegCredential::Token(token),
        None if kind == HostKind::Interactive => {
            notes.push(format!(
                "nobody is signed in to {host}, so joy tries its https address with whatever this machine holds"
            ));
            LegCredential::Machine
        }
        None => {
            notes.push(format!(
                "nobody is signed in to {host}, so joy has no https credential to try"
            ));
            return None;
        }
    };
    Some(Leg {
        way: Way::Twin,
        url,
        transport: Transport::Https,
        credential,
    })
}

/// Whether this failure is the ssh authentication class failure that is
/// trigger (b) of D1.2: `class == Ssh` with `code == Auth`, read from
/// the error's own fields and never from its prose (D1.8a).
pub fn is_ssh_auth_failure(error: &git2::Error) -> bool {
    error.class() == git2::ErrorClass::Ssh && error.code() == git2::ErrorCode::Auth
}

thread_local! {
    /// Whether the contact running on this thread failed with trigger
    /// (b) of D1.2. It is read off the RAW libgit2 error at the one
    /// place that still holds one, because the classifier folds an ssh
    /// authentication failure and an https 401 into the same state and
    /// only one of the two may send an operation to the twin.
    static SSH_AUTH_FAILED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The engine hands every failed contact's raw error here before it is
/// classified.
pub fn note_contact_error(transport: Transport, error: &git2::Error) {
    if transport == Transport::Ssh && is_ssh_auth_failure(error) {
        SSH_AUTH_FAILED.with(|failed| failed.set(true));
    }
}

/// Whether the contact that just ended was an ssh authentication
/// failure. Reading it clears it, so one contact's refusal is never read
/// as the next one's.
pub fn took_ssh_auth_failure() -> bool {
    SSH_AUTH_FAILED.with(|failed| failed.replace(false))
}

/// Run `work` against a state file of its own, one case at a time.
///
/// The file is process state; every test module in the crate that
/// writes it takes this one lock, so a case never reads another's rows.
#[cfg(test)]
pub(crate) fn with_state_file<T>(work: impl FnOnce(&Path) -> T) -> T {
    static SERIAL: Mutex<()> = Mutex::new(());
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Put back whatever was there, which for a plain `cargo test` is
    // the default: `set_state_file(None)` now MEANS the default, so a
    // case that ended must not leave the next one writing rows into the
    // person's own state directory.
    let before = STATE_PATH.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(STATE_FILE);
    set_state_file(Some(path.clone()));
    let out = work(&path);
    set_state_file(before);
    out
}

#[cfg(test)]
mod tests;
