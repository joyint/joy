// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! THE forge-connector registry, its resolution contract and its runner
//! (JOY-0293-12, package J1 of the forge connection NG design; the
//! original registry was JOY-0252-1A, epic JOY-0251-AA).
//!
//! All forge knowledge (host names, alias address formats, API access)
//! lives in the connector binary; this module knows only the binary
//! NAMES, where to look for them, the protocol number and the JSON query
//! protocol (docs/plugins.md, "Forge plugins").
//!
//! Three things happen here, in this order:
//!
//! 1. **Resolution** (D2.2): a spec names its binaries, `joy-forge`
//!    first and the legacy `joy-<forge>` name after it, and the search
//!    walks the directories the host registered, then the directory of
//!    the current executable, then PATH. The first hit wins.
//! 2. **The handshake** (D2.2a): the resolved binary is asked `version`
//!    once per path and mtime. It answers the protocol number, or it is
//!    a protocol 1 binary, which is detected without its cooperation
//!    (exit 2 with empty stdout, or any answer that does not parse).
//! 3. **The runner** (D2.3): [`run_once`] reads stdout concurrently with
//!    waiting under a per verb deadline and pipes stderr, [`run_stream`]
//!    reads newline delimited JSON events while the child runs, and both
//!    kill the child's whole process group when they give up.
//!
//! The ANSWER of the read verbs stays best effort by design: a missing
//! binary, a timeout or a garbled answer degrades to "no claim /
//! unknown", because identity resolution must never fail because a
//! connector is absent. What is no longer best effort is the REASON: it
//! is a typed [`PluginError`] every caller can tell apart (missing,
//! outdated, failed, timed out) and a warn line for the operator.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::host::HostKind;

/// The protocol this joy speaks. A connector that answers anything else
/// is outdated (or newer than this joy, which the same sentence covers).
pub const PROTOCOL: u32 = 2;

/// The one binary that carries every forge (D2.1). It is tried first in
/// every directory, so a fresh connector beside a stale `joy-<forge>`
/// wins wherever both are installed.
pub const COMBINED_BINARY: &str = "joy-forge";

/// The documented TEST HOOK of D2.2, and nothing else: a list of
/// directories (separated like PATH) searched before everything else.
/// It is not a product switch; no joy surface offers it.
pub const PLUGIN_DIR_ENV: &str = "JOY_PLUGIN_DIR";

/// One row per known forge connector, adapter-registry style
/// (JI-017A-85): data, not behavior. Adding a forge is one row here plus
/// a forge inside the connector binary; nothing else in joy-core
/// changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgePluginSpec {
    /// The forge id as `project.yaml`'s `forge:` override names it, and
    /// the first argument to the combined binary.
    pub id: &'static str,
    /// The forge's name as a person writes it, for the sentences a
    /// person reads.
    pub display: &'static str,
    /// The binaries that may answer for this forge, in the order they
    /// are tried inside every directory: the combined connector first,
    /// the legacy single-forge name after it (D2.2).
    pub binary_names: &'static [&'static str],
}

impl ForgePluginSpec {
    /// The legacy `joy-<forge>` name, i.e. every candidate that is not
    /// the combined binary. Used by the sentences that tell a person
    /// which stale file to remove.
    pub fn legacy_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.binary_names
            .iter()
            .copied()
            .filter(|name| *name != COMBINED_BINARY)
    }
}

/// The registry. Order matters only for the claims round-robin.
pub const FORGE_PLUGINS: &[ForgePluginSpec] = &[
    ForgePluginSpec {
        id: "github",
        display: "GitHub",
        binary_names: &[COMBINED_BINARY, "joy-github"],
    },
    ForgePluginSpec {
        id: "gitlab",
        display: "GitLab",
        binary_names: &[COMBINED_BINARY, "joy-gitlab"],
    },
    ForgePluginSpec {
        id: "gitea",
        display: "Gitea",
        binary_names: &[COMBINED_BINARY, "joy-gitea"],
    },
];

/// The registry's ids, in registry order. This is the list `--forge`
/// and `forge:` accept; error help text renders it, so it lives next
/// to the registry instead of being re-derived by every caller.
pub fn supported_ids() -> Vec<&'static str> {
    FORGE_PLUGINS.iter().map(|spec| spec.id).collect()
}

/// The registry row for a `forge:` override value, if any.
pub fn by_id(id: &str) -> Option<&'static ForgePluginSpec> {
    let id = id.trim().to_ascii_lowercase();
    FORGE_PLUGINS.iter().find(|spec| spec.id == id)
}

// ---------------------------------------------------------------------
// Resolution (D2.2)
// ---------------------------------------------------------------------

/// Where a connector was found. Part of every answer a person or an
/// operator reads, because "which file answered" is the first question
/// a stale binary raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundIn {
    /// The documented test hook, `JOY_PLUGIN_DIR`.
    TestHook,
    /// A directory the host registered with [`set_plugin_dirs`].
    Registered,
    /// The directory of the current executable.
    ExecutableDir,
    /// PATH.
    Path,
}

impl FoundIn {
    /// The word logs and `joy forge plugins` print.
    pub fn as_str(self) -> &'static str {
        match self {
            FoundIn::TestHook => PLUGIN_DIR_ENV,
            FoundIn::Registered => "registered directory",
            FoundIn::ExecutableDir => "executable directory",
            FoundIn::Path => "PATH",
        }
    }
}

impl std::fmt::Display for FoundIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A connector that exists on this machine, with everything the runner
/// and every sentence about it need: which forge it answers for, which
/// file answered, where that file was found, and what its handshake
/// said (D2.2, D2.2a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlugin {
    /// The forge id of the spec this was resolved for.
    pub id: &'static str,
    /// That forge's display name.
    pub display: &'static str,
    /// The names that were tried, in order.
    pub binary_names: &'static [&'static str],
    /// The file that answers.
    pub resolved_path: PathBuf,
    /// Which step of the search order found it.
    pub found_in: FoundIn,
    /// The protocol it speaks: [`PROTOCOL`], or 1 for a binary from
    /// before the handshake existed.
    pub protocol: u32,
    /// What its `version` verb calls it ("joy-forge 0.21.0"); `None`
    /// for a protocol 1 binary, which has no `version` verb.
    pub plugin_version: Option<String>,
}

impl ResolvedPlugin {
    /// Whether the file that answers is the combined connector, whose
    /// first argument is the forge id (`joy-forge github claims ...`).
    pub fn is_combined(&self) -> bool {
        self.resolved_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.eq_ignore_ascii_case(COMBINED_BINARY))
    }

    /// The `rm` line D2.2a prints beside an outdated connector. It names
    /// the file that answered and nothing else: joy never removes a
    /// binary it did not install.
    pub fn removal_line(&self) -> String {
        format!("rm {}", self.resolved_path.display())
    }
}

/// The directories the host registered, in the order they are searched.
fn registered_dirs() -> &'static RwLock<Vec<PathBuf>> {
    static DIRS: OnceLock<RwLock<Vec<PathBuf>>> = OnceLock::new();
    DIRS.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register the directories this host ships its connector in (D2.2,
/// step 1): the desktop passes the parent of its own executable, the
/// CLI its install directory. Called at startup, before the first verb.
///
/// It replaces the list rather than appending, so a host that decides
/// twice does not grow a search order nobody wrote down.
pub fn set_plugin_dirs(dirs: Vec<PathBuf>) {
    *registered_dirs().write().unwrap_or_else(|e| e.into_inner()) = dirs;
    // A directory list that changed may point at another file for the
    // same name, so nothing resolved under the old list may survive.
    resolution_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

/// The search order of D2.2, as directories: the test hook, then the
/// directories the host registered, then the directory of the current
/// executable, then PATH. Duplicates are dropped, so a directory that
/// is both registered and on PATH is searched once and reported by the
/// earlier of the two.
pub fn search_dirs() -> Vec<(PathBuf, FoundIn)> {
    let mut out: Vec<(PathBuf, FoundIn)> = Vec::new();
    let mut push = |dir: PathBuf, found_in: FoundIn| {
        if !dir.as_os_str().is_empty() && !out.iter().any(|(seen, _)| *seen == dir) {
            out.push((dir, found_in));
        }
    };
    if let Some(value) = std::env::var_os(PLUGIN_DIR_ENV) {
        for dir in std::env::split_paths(&value) {
            push(dir, FoundIn::TestHook);
        }
    }
    for dir in registered_dirs()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
    {
        push(dir.clone(), FoundIn::Registered);
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        push(dir, FoundIn::ExecutableDir);
    }
    if let Some(value) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&value) {
            push(dir, FoundIn::Path);
        }
    }
    out
}

/// Every file that could answer for `spec`, in resolution order: per
/// directory of [`search_dirs`], the names of `spec.binary_names` in
/// their own order. The first entry is what [`resolve_plugin`] uses;
/// the rest are what `joy forge plugins` reports as shadowed.
pub fn candidates(spec: &ForgePluginSpec) -> Vec<(PathBuf, FoundIn)> {
    let mut out = Vec::new();
    for (dir, found_in) in search_dirs() {
        for name in spec.binary_names {
            let file = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
            if is_executable_file(&file) {
                out.push((file, found_in));
            }
        }
    }
    out
}

/// Whether this path is a file this process could run. On unix that is
/// the execute bit; elsewhere the file's existence is the whole test
/// (Windows decides by extension, and the extension is already in the
/// name).
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.is_file() && may_execute(&meta)
}

#[cfg(unix)]
fn may_execute(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn may_execute(_meta: &std::fs::Metadata) -> bool {
    true
}

/// The resolution plus the handshake, cached per forge id for this
/// process (the handshake itself is cached per path and mtime, so a
/// connector replaced under a running joy is asked again).
pub fn resolve_plugin(spec: &ForgePluginSpec) -> Result<ResolvedPlugin, PluginError> {
    let (path, found_in) =
        candidates(spec)
            .into_iter()
            .next()
            .ok_or_else(|| PluginError::Missing {
                display: spec.display,
                names: spec.binary_names.iter().map(|n| (*n).to_string()).collect(),
            })?;
    if let Some(hit) = resolution_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(spec.id, path.clone(), mtime_of(&path)))
    {
        return Ok(hit.clone());
    }
    let handshake = handshake(spec, &path)?;
    let resolved = ResolvedPlugin {
        id: spec.id,
        display: spec.display,
        binary_names: spec.binary_names,
        resolved_path: path.clone(),
        found_in,
        protocol: handshake.protocol,
        plugin_version: handshake.plugin,
    };
    resolution_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert((spec.id, path.clone(), mtime_of(&path)), resolved.clone());
    Ok(resolved)
}

/// The resolution cache: forge id, file and its mtime to the answer.
/// The mtime is part of the key on purpose (D2.2a): `cargo install`
/// replacing the file under a running desktop must not keep answering
/// from the old handshake.
type CacheKey = (&'static str, PathBuf, Option<std::time::SystemTime>);

fn resolution_cache() -> &'static Mutex<HashMap<CacheKey, ResolvedPlugin>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, ResolvedPlugin>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mtime_of(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

// ---------------------------------------------------------------------
// The handshake (D2.2a)
// ---------------------------------------------------------------------

/// What `version` answers (D2.2a). Exactly one object, and anything
/// else means protocol 1.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct VersionAnswer {
    pub protocol: u32,
    #[serde(default)]
    pub plugin: Option<String>,
    #[serde(default)]
    pub forges: Vec<String>,
}

/// The handshake's verdict for one file.
struct Handshake {
    protocol: u32,
    plugin: Option<String>,
}

/// Ask one file what protocol it speaks, under the 5 s query class.
///
/// A protocol 1 binary needs no cooperation to be recognised: its clap
/// parser rejects the unknown subcommand and exits 2 with usage on
/// stderr and nothing on stdout. The rule of D2.2a is therefore: exit
/// code 2 with empty stdout, or any answer that does not parse as the
/// object above, means protocol 1.
fn handshake(spec: &ForgePluginSpec, path: &Path) -> Result<Handshake, PluginError> {
    let combined = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case(COMBINED_BINARY));
    let mut args: Vec<String> = Vec::new();
    if combined {
        args.push(spec.id.to_string());
    }
    args.push("version".to_string());
    let outcome = run_path(path, spec.id, "version", &args, &[], None, QUERY_TIMEOUT);
    if let Some(error) = outcome.spawn_error.clone() {
        return Err(PluginError::Spawn {
            display: spec.display,
            path: path.to_path_buf(),
            error,
        });
    }
    if outcome.timed_out {
        return Err(PluginError::TimedOut {
            display: spec.display,
            path: path.to_path_buf(),
            verb: "version".to_string(),
            timeout: QUERY_TIMEOUT,
        });
    }
    match outcome
        .stdout_json
        .as_ref()
        .and_then(|value| serde_json::from_value::<VersionAnswer>(value.clone()).ok())
    {
        Some(answer) if outcome.exit_code == Some(0) => Ok(Handshake {
            protocol: answer.protocol,
            plugin: answer.plugin,
        }),
        // Anything else is a binary from before the handshake existed.
        _ => Ok(Handshake {
            protocol: 1,
            plugin: None,
        }),
    }
}

// ---------------------------------------------------------------------
// The states every caller tells apart (D2.3)
// ---------------------------------------------------------------------

/// Why a connector call produced no answer. Every caller distinguishes
/// these, plus the connector's own `{"known":false}`, which is an
/// ANSWER and therefore not in here (D2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// No file with any of the names exists anywhere in the search
    /// order.
    Missing {
        display: &'static str,
        names: Vec<String>,
    },
    /// The file that answers speaks protocol 1 and the verb asked is
    /// not one a protocol 1 connector knows.
    Outdated {
        display: &'static str,
        path: PathBuf,
        protocol: u32,
        verb: String,
    },
    /// The file exists and could not be started.
    Spawn {
        display: &'static str,
        path: PathBuf,
        error: String,
    },
    /// The connector ran and refused: a non-zero exit, with whatever it
    /// wrote on stderr.
    Failed {
        display: &'static str,
        path: PathBuf,
        verb: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    /// The connector did not answer inside the verb's deadline and its
    /// process group was killed.
    TimedOut {
        display: &'static str,
        path: PathBuf,
        verb: String,
        timeout: Duration,
    },
    /// The connector answered something this joy cannot read.
    Unparsable {
        display: &'static str,
        path: PathBuf,
        verb: String,
        answer: String,
    },
}

impl PluginError {
    /// The state name, the word every surface and every log uses. The
    /// two that D2.2a names explicitly are `plugin_missing` and
    /// `plugin_outdated`.
    pub fn state(&self) -> &'static str {
        match self {
            PluginError::Missing { .. } => "plugin_missing",
            PluginError::Outdated { .. } => "plugin_outdated",
            PluginError::TimedOut { .. } => "plugin_timed_out",
            PluginError::Spawn { .. }
            | PluginError::Failed { .. }
            | PluginError::Unparsable { .. } => "plugin_failed",
        }
    }

    /// The file that answered, where there is one.
    pub fn resolved_path(&self) -> Option<&Path> {
        match self {
            PluginError::Missing { .. } => None,
            PluginError::Outdated { path, .. }
            | PluginError::Spawn { path, .. }
            | PluginError::Failed { path, .. }
            | PluginError::TimedOut { path, .. }
            | PluginError::Unparsable { path, .. } => Some(path),
        }
    }
}

/// The first `limit` characters of a connector's own text, on one line.
/// Its message is what a person needs; its layout is not.
fn one_line(text: &str, limit: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let cut: String = joined.chars().take(limit).collect();
    format!("{cut}...")
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Missing { display, names } => write!(
                f,
                "no {display} connector is installed (looked for {} beside joy and on PATH)\n  \
                 = help: `cargo install joy-cli` ships {COMBINED_BINARY}",
                names.join(", ")
            ),
            PluginError::Outdated {
                display,
                path,
                protocol,
                ..
            } => write!(
                f,
                "the {display} connector at {} speaks protocol {protocol}, this joy needs \
                 protocol {PROTOCOL}. Install the new connector (`cargo install joy-cli` ships \
                 {COMBINED_BINARY}), then remove the old one: rm {}",
                path.display(),
                path.display()
            ),
            PluginError::Spawn {
                display,
                path,
                error,
            } => write!(
                f,
                "the {display} connector at {} could not be started: {error}",
                path.display()
            ),
            PluginError::Failed {
                display,
                path,
                verb,
                exit_code,
                stderr,
            } => {
                let code = match exit_code {
                    Some(code) => code.to_string(),
                    None => "a signal".to_string(),
                };
                let text = one_line(stderr, 400);
                if text.is_empty() {
                    write!(
                        f,
                        "the {display} connector at {} refused `{verb}` (exit {code}) and said \
                         nothing",
                        path.display()
                    )
                } else {
                    write!(
                        f,
                        "the {display} connector at {} refused `{verb}` (exit {code}): {text}",
                        path.display()
                    )
                }
            }
            PluginError::TimedOut {
                display,
                path,
                verb,
                timeout,
            } => write!(
                f,
                "the {display} connector at {} did not answer `{verb}` within {} s and was \
                 stopped",
                path.display(),
                timeout.as_secs()
            ),
            PluginError::Unparsable {
                display,
                path,
                verb,
                answer,
            } => write!(
                f,
                "the {display} connector at {} answered `{verb}` with something this joy could \
                 not read: {}",
                path.display(),
                one_line(answer, 200)
            ),
        }
    }
}

impl std::error::Error for PluginError {}

// ---------------------------------------------------------------------
// The runner (D2.3)
// ---------------------------------------------------------------------

/// How long a connector may take per query class. Queries are local
/// parses or one forge API call; anything slower must not stall a `joy`
/// command.
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the repository-facing verbs may take: several forge API
/// calls (the file, the repository, GitLab's branch protection; the
/// pages of a tree), each bounded by the connector itself. `token` is
/// in this class because `gh` alone allows 60 s per keyring read.
const STORE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the release verb may take: it talks to the forge's release
/// API (possibly view + edit/create), so it gets network patience the
/// read queries must not have.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long `login` may take to say its first word (D2.3): the device
/// grant's first round trip and nothing more.
const LOGIN_FIRST_EVENT: Duration = Duration::from_secs(15);

/// The cap on the rest of a `login` (D2.3): the verification event's own
/// `expires_in` decides, and this is the most it may ask for.
const LOGIN_TOTAL_CAP: Duration = Duration::from_secs(900);

/// The deadline for one verb (D2.3). Every verb of the catalogue is
/// named; anything else gets the careful class, because a verb nobody
/// wrote down must not be the one that stalls a command.
pub fn timeout_for(verb: &str) -> Duration {
    match verb {
        "claims" | "identity" | "resolve" | "web-url" | "version" => QUERY_TIMEOUT,
        "store" | "files" | "repositories" | "create-repository" | "token" | "token-store"
        | "logout" => STORE_TIMEOUT,
        "release" => RELEASE_TIMEOUT,
        "login" => LOGIN_TOTAL_CAP,
        _ => QUERY_TIMEOUT,
    }
}

/// What one connector call did, whatever it did (D2.3). Every field is
/// filled on every path, so no caller has to guess why it got nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginOutcome {
    /// The exit code, `None` when a signal ended it or it never ran.
    pub exit_code: Option<i32>,
    /// Stdout parsed as one JSON value, when it is one. For
    /// [`run_stream`] this is the LAST event that parsed, which is the
    /// `result` or `error` object of an event stream.
    pub stdout_json: Option<serde_json::Value>,
    /// Stdout as it arrived. Kept beside the parsed value because an
    /// answer nobody could parse is exactly what a reader needs to see.
    /// Empty for [`run_stream`], whose output went to the sink.
    pub stdout_text: String,
    /// Stderr as it arrived: piped and captured, so the connector's own
    /// message reaches the error instead of a terminal nobody watches.
    pub stderr_text: String,
    /// The deadline was reached and the process group was killed.
    pub timed_out: bool,
    /// The child never started, with the reason.
    pub spawn_error: Option<String>,
}

/// A stop signal shared between a caller and a running connector
/// (D2.3). Cancelling kills the child's whole process group.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token nobody has cancelled yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stop the call this token belongs to. Idempotent.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether somebody asked for the call to stop.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The two bounds of a streaming call (D2.3). `login` is the verb that
/// needs both: 15 s until its first event, then that event's own
/// `expires_in`, capped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBounds {
    /// How long the connector may take to send its first event.
    pub first_event: Duration,
    /// The most the rest of the call may take, and the cap on anything
    /// the sink asks for.
    pub total: Duration,
}

impl StreamBounds {
    /// The bounds of D2.3 for a verb.
    pub fn for_verb(verb: &str) -> Self {
        match verb {
            "login" => StreamBounds {
                first_event: LOGIN_FIRST_EVENT,
                total: LOGIN_TOTAL_CAP,
            },
            other => StreamBounds {
                first_event: timeout_for(other),
                total: timeout_for(other),
            },
        }
    }
}

/// Where the events of [`run_stream`] go while the connector runs.
///
/// The sink may extend the call: returning `Some(d)` sets the remaining
/// deadline to `d`, capped by [`StreamBounds::total`]. That is how the
/// `login` verb's second bound works, its `verification` event carrying
/// the `expires_in` the forge granted (D2.3).
pub trait EventSink {
    /// One newline delimited JSON object the connector flushed.
    fn event(&mut self, event: &serde_json::Value) -> Option<Duration>;

    /// A stdout line that is not one JSON object. Ignored by default:
    /// a connector that prints noise must not break the stream.
    fn line_noise(&mut self, _line: &str) {}
}

impl<F: FnMut(&serde_json::Value)> EventSink for F {
    fn event(&mut self, event: &serde_json::Value) -> Option<Duration> {
        self(event);
        None
    }
}

/// Run one connector call to its end, reading stdout CONCURRENTLY with
/// waiting (D2.3).
///
/// Concurrently is the whole point: reading stdout only after the child
/// exited deadlocks every answer above the pipe buffer (64 KiB on
/// Linux), because the child blocks writing while joy blocks waiting.
/// A repository listing passes that bound easily.
///
/// Stderr is piped and captured for the same reason the read verbs
/// stopped inheriting it: a connector's message belongs in the error a
/// caller renders, not on a terminal that may not exist.
pub fn run_once(
    spec: &ResolvedPlugin,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
) -> PluginOutcome {
    let verb = verb_of(spec, args);
    run_path(&spec.resolved_path, spec.id, verb, args, env, None, timeout)
}

/// [`run_once`] with a working directory. A connector call needs no
/// project root (D2.3, `claims --host github.com` with nothing on
/// disk); a caller that has one passes it so a connector that reads
/// project.yaml still can.
pub fn run_once_in(
    spec: &ResolvedPlugin,
    args: &[String],
    env: &[(String, String)],
    root: Option<&Path>,
    timeout: Duration,
) -> PluginOutcome {
    let verb = verb_of(spec, args);
    run_path(&spec.resolved_path, spec.id, verb, args, env, root, timeout)
}

/// The verb inside an argument list: the first argument, or the second
/// when the combined binary's first argument is the forge id.
fn verb_of<'a>(spec: &ResolvedPlugin, args: &'a [String]) -> &'a str {
    let index = usize::from(spec.is_combined());
    args.get(index).map(String::as_str).unwrap_or("")
}

/// The one place a connector call can fail silently, so the one place
/// that says so (forge connection NG, D5 and D2.3, packages J1 and
/// P1a).
///
/// The ANSWER of a read verb stays best effort: every failure still
/// degrades to "unknown" in the caller. What changes is that the
/// failure leaves a trace. Without it, a connector that is missing,
/// shadowed by a stale binary, refused or slow turns into "unknown" for
/// the person and into nothing at all for the operator, which is
/// exactly how a broken server image survived a working day.
///
/// Every line carries the connector and the verb, because both are what
/// a reader needs to act: the binary to look for and the question that
/// was asked.
fn run_path(
    path: &Path,
    plugin: &str,
    verb: &str,
    args: &[String],
    env: &[(String, String)],
    root: Option<&Path>,
    timeout: Duration,
) -> PluginOutcome {
    let mut outcome = PluginOutcome::default();
    let mut child = match spawn(path, args, env, root) {
        Ok(child) => child,
        Err(e) => {
            tracing::warn!(
                plugin,
                verb,
                path = %path.display(),
                error = %e,
                "the forge plugin could not be started"
            );
            outcome.spawn_error = Some(e.to_string());
            return outcome;
        }
    };
    let group = ProcessGroup::of(&child);
    let out_reader = drain(child.stdout.take());
    let err_reader = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    outcome.timed_out = true;
                    group.kill(&mut child);
                    let _ = child.wait();
                    tracing::warn!(
                        plugin,
                        verb,
                        path = %path.display(),
                        timeout_secs = timeout.as_secs(),
                        "the forge plugin did not answer in time and was stopped"
                    );
                    break None;
                }
                std::thread::sleep(POLL_GAP);
            }
            Err(e) => {
                tracing::warn!(
                    plugin,
                    verb,
                    path = %path.display(),
                    error = %e,
                    "the forge plugin could not be waited for"
                );
                group.kill(&mut child);
                let _ = child.wait();
                outcome.spawn_error = Some(e.to_string());
                break None;
            }
        }
    };
    outcome.stdout_text = out_reader.join();
    outcome.stderr_text = err_reader.join();
    outcome.stdout_json = serde_json::from_str(outcome.stdout_text.trim()).ok();
    if let Some(status) = status {
        outcome.exit_code = status.code();
        if !status.success() {
            let code = status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "a signal".to_string());
            tracing::warn!(
                plugin,
                verb,
                path = %path.display(),
                code = %code,
                stderr = %one_line(&outcome.stderr_text, 200),
                "the forge plugin refused the verb"
            );
        }
    }
    outcome
}

/// Wait for a child until `deadline`, without blocking past it.
/// `None` means it was still running when the deadline passed.
fn wait_until(child: &mut Child, deadline: Instant) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_GAP),
            _ => return None,
        }
    }
}

/// How often the runner looks whether the child is done. Small enough
/// that a 5 s deadline is kept to the tenth of a second, large enough
/// that a 1 Hz poll costs nothing.
const POLL_GAP: Duration = Duration::from_millis(20);

/// Run a connector that speaks while it works, one newline delimited
/// JSON object per line (D2.3).
///
/// Every line reaches `sink` as it arrives, not when the child exits,
/// which is what makes `login` usable: the caller shows the
/// verification code while the connector is still polling the forge.
/// `cancel` and the bounds both end the call by killing the child's
/// whole process group.
pub fn run_stream(
    spec: &ResolvedPlugin,
    args: &[String],
    env: &[(String, String)],
    sink: &mut dyn EventSink,
    cancel: &CancelToken,
    bounds: StreamBounds,
    root: Option<&Path>,
) -> PluginOutcome {
    let verb = verb_of(spec, args).to_string();
    let path = spec.resolved_path.clone();
    let mut outcome = PluginOutcome::default();
    let mut child = match spawn(&path, args, env, root) {
        Ok(child) => child,
        Err(e) => {
            tracing::warn!(
                plugin = spec.id,
                verb = %verb,
                path = %path.display(),
                error = %e,
                "the forge plugin could not be started"
            );
            outcome.spawn_error = Some(e.to_string());
            return outcome;
        }
    };
    let group = ProcessGroup::of(&child);
    let err_reader = drain(child.stderr.take());
    let (lines_tx, lines_rx) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take();
    let line_reader = std::thread::spawn(move || {
        let Some(stdout) = stdout else { return };
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            if lines_tx.send(line).is_err() {
                return;
            }
        }
    });
    let mut deadline = Instant::now() + bounds.first_event;
    let cap = Instant::now() + bounds.total;
    let mut cancelled = false;
    loop {
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let now = Instant::now();
        if now >= deadline || now >= cap {
            outcome.timed_out = true;
            tracing::warn!(
                plugin = spec.id,
                verb = %verb,
                path = %path.display(),
                timeout_secs = bounds.total.as_secs(),
                "the forge plugin did not answer in time and was stopped"
            );
            break;
        }
        match lines_rx.recv_timeout(POLL_GAP) {
            Ok(line) => match serde_json::from_str::<serde_json::Value>(line.trim()) {
                Ok(event) => {
                    let asked = sink.event(&event);
                    outcome.stdout_json = Some(event);
                    let next = asked.unwrap_or(bounds.total);
                    deadline = (Instant::now() + next).min(cap);
                }
                Err(_) => sink.line_noise(&line),
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if cancelled || outcome.timed_out {
        group.kill(&mut child);
    }
    // A connector that closed stdout and then hung would hold the
    // caller for ever on a plain `wait`, which is the same failure the
    // deadline exists to prevent. The cap bounds this wait too, and
    // whatever is left of the process group after it is ended.
    if wait_until(&mut child, cap.max(Instant::now() + POLL_GAP)).is_none() {
        outcome.timed_out = true;
        group.kill(&mut child);
        let _ = child.wait();
    }
    outcome.exit_code = child.wait().ok().and_then(|status| status.code());
    drop(lines_rx);
    let _ = line_reader.join();
    outcome.stderr_text = err_reader.join();
    if outcome.exit_code.is_some_and(|code| code != 0) && !outcome.timed_out && !cancelled {
        tracing::warn!(
            plugin = spec.id,
            verb = %verb,
            path = %path.display(),
            code = outcome.exit_code.unwrap_or_default(),
            stderr = %one_line(&outcome.stderr_text, 200),
            "the forge plugin refused the verb"
        );
    }
    outcome
}

/// Build and start one connector process: no stdin, both output pipes
/// captured, its own process group, and the caller's environment
/// additions on the CHILD only (a multi-account host must never widen
/// its own process environment with a token).
fn spawn(
    path: &Path,
    args: &[String],
    env: &[(String, String)],
    root: Option<&Path>,
) -> std::io::Result<Child> {
    let mut command = joy_process::command(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    if let Some(root) = root {
        command.current_dir(root);
    }
    own_process_group(&mut command);
    command.spawn()
}

/// Give the child a process group of its own, so the runner can end the
/// connector AND whatever it started (D2.3): `child.kill()` alone
/// leaves a `gh` or a `curl` grandchild running with the pipe still
/// open.
#[cfg(unix)]
fn own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Windows has no process groups in this sense; the child is put into a
/// job object after the spawn instead (see [`ProcessGroup`]).
#[cfg(not(unix))]
fn own_process_group(_command: &mut Command) {}

/// The handle the runner ends a connector with: the child's process
/// group on unix, a job object on Windows (D2.3).
struct ProcessGroup {
    #[cfg(unix)]
    pid: i32,
    #[cfg(windows)]
    job: Option<windows::Job>,
}

impl ProcessGroup {
    #[cfg(unix)]
    fn of(child: &Child) -> Self {
        ProcessGroup {
            pid: child.id() as i32,
        }
    }

    #[cfg(windows)]
    fn of(child: &Child) -> Self {
        ProcessGroup {
            job: windows::Job::holding(child),
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn of(_child: &Child) -> Self {
        ProcessGroup {}
    }

    /// End the connector and everything it started. Best effort by
    /// definition: a process that already exited is not an error here.
    #[cfg(unix)]
    fn kill(&self, child: &mut Child) {
        // The child is the leader of its own group (process_group(0)),
        // so its pid IS the group id and one killpg reaches the
        // connector and every `gh` or `curl` below it.
        unsafe { libc::killpg(self.pid, libc::SIGKILL) };
        let _ = child.kill();
    }

    #[cfg(windows)]
    fn kill(&self, child: &mut Child) {
        if let Some(job) = &self.job {
            job.terminate();
        }
        let _ = child.kill();
    }

    #[cfg(not(any(unix, windows)))]
    fn kill(&self, child: &mut Child) {
        let _ = child.kill();
    }
}

/// The job object that holds a connector and its grandchildren on
/// Windows (D2.3). `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` means the whole
/// tree also dies when joy itself dies, which is what a person expects
/// after closing the window a login was started from.
#[cfg(windows)]
mod windows {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub(super) struct Job(HANDLE);

    // The handle is owned by this value and only ever used through the
    // two calls below, both of which Windows allows from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// A job object holding `child`.
        ///
        /// The child is assigned right after the spawn rather than
        /// created suspended, because std gives no hook between the two.
        /// A grandchild started in that window would escape the job; a
        /// connector's first act is its own start-up, so the window is
        /// microseconds and the child itself is always held.
        pub(super) fn holding(child: &Child) -> Option<Self> {
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return None;
            }
            let job = Job(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(limits) as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if set == 0 {
                return None;
            }
            let assigned =
                unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) };
            if assigned == 0 {
                return None;
            }
            Some(job)
        }

        /// End every process in the job.
        pub(super) fn terminate(&self) {
            unsafe { TerminateJobObject(self.0, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Read one pipe to its end on a thread of its own, so a connector that
/// fills the pipe buffer keeps running instead of blocking on a write
/// nobody is reading.
struct Drain(Option<std::thread::JoinHandle<String>>);

impl Drain {
    fn join(self) -> String {
        self.0
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default()
    }
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Drain {
    let Some(mut pipe) = pipe else {
        return Drain(None);
    };
    Drain(Some(std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = pipe.read_to_end(&mut buffer);
        String::from_utf8_lossy(&buffer).into_owned()
    })))
}

// ---------------------------------------------------------------------
// What a call carries (D2.3, D1.10, D4.1c)
// ---------------------------------------------------------------------

/// Caller facts a multi-account host hands to the connector: the
/// platform's session knows who acts (forge login, user id, a token in
/// an env var), while a single-person device passes none and the
/// connector finds its own facts (e.g. the forge CLI's config).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerFacts {
    /// The login this call is pinned to (D4.1c). Travels as `--login`
    /// on every protocol 2 call.
    pub login: Option<String>,
    pub user_id: Option<String>,
    /// Name of an environment variable holding a forge token — the token
    /// itself must never appear in a process list. When `token_value` is
    /// set too, the variable is injected into the PLUGIN's environment
    /// only (a multi-account host must never widen its own process env).
    pub token_env: Option<String>,
    pub token_value: Option<String>,
}

/// What a verb is asked about: a remote URL, or a bare host for the
/// calls that have no repository (D2.3, "all verbs accept `--host`").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Remote(String),
    Host(String),
}

impl Target {
    /// The target of a remote URL.
    pub fn remote(url: impl Into<String>) -> Self {
        Target::Remote(url.into())
    }

    /// The target of a bare host name.
    pub fn host(host: impl Into<String>) -> Self {
        Target::Host(host.into())
    }

    /// Whether this target is a remote URL, the only one a protocol 1
    /// connector understands.
    pub fn is_remote(&self) -> bool {
        matches!(self, Target::Remote(_))
    }

    fn args(&self) -> [String; 2] {
        match self {
            Target::Remote(url) => ["--remote".to_string(), url.clone()],
            Target::Host(host) => ["--host".to_string(), host.clone()],
        }
    }
}

/// Everything a connector call carries besides its verb and its target:
/// the project root if there is one (there need not be), the host kind
/// of D1.10 and the caller's facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallContext {
    /// The project root, when the caller has one. `None` is the normal
    /// case for the host-only verbs (D2.3, rootless invocation).
    pub root: Option<PathBuf>,
    /// Who is behind this process. Travels as `--host-kind` on every
    /// protocol 2 call, so the connector can skip any step that would
    /// raise an operating system dialog nobody can answer (D1.10).
    pub host_kind: HostKind,
    /// Who acts, and where their token is.
    pub facts: CallerFacts,
}

impl Default for CallContext {
    /// The host kind this process decided at its entry point, never a
    /// guess: a library that never asked gets `Background`, which is the
    /// careful answer.
    fn default() -> Self {
        CallContext {
            root: None,
            host_kind: crate::host::process_host(),
            facts: CallerFacts::default(),
        }
    }
}

impl CallContext {
    /// A call with no project on disk (D2.3).
    pub fn rootless() -> Self {
        Self::default()
    }

    /// A call from inside a project.
    pub fn in_project(root: impl Into<PathBuf>) -> Self {
        CallContext {
            root: Some(root.into()),
            ..Self::default()
        }
    }

    /// Say who is behind this call explicitly, instead of taking the
    /// process-wide answer.
    pub fn with_host_kind(mut self, kind: HostKind) -> Self {
        self.host_kind = kind;
        self
    }

    /// Hand the connector the caller's facts.
    pub fn with_facts(mut self, facts: CallerFacts) -> Self {
        self.facts = facts;
        self
    }

    fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The variable a token travels in, injected into the CHILD only.
    fn env(&self) -> Vec<(String, String)> {
        match (
            self.facts.token_env.as_deref(),
            self.facts.token_value.as_deref(),
        ) {
            (Some(var), Some(value)) => vec![(var.to_string(), value.to_string())],
            _ => Vec::new(),
        }
    }
}

/// The verbs a protocol 1 connector still answers, with `--remote` only
/// (D2.2a). Everything else asked of one is `plugin_outdated`.
const LEGACY_VERBS: &[&str] = &["claims", "identity", "resolve", "store", "files", "release"];

/// The argument list for one call, in the order the connector's parser
/// sees it: the forge id for the combined binary, the verb, the target,
/// the verb's own arguments, then the protocol fields.
fn call_args(
    resolved: &ResolvedPlugin,
    verb: &str,
    target: Option<&Target>,
    extra: &[&str],
    ctx: &CallContext,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if resolved.is_combined() {
        args.push(resolved.id.to_string());
    }
    args.push(verb.to_string());
    if let Some(target) = target {
        args.extend(target.args());
    }
    args.extend(extra.iter().map(|arg| (*arg).to_string()));
    if let Some(var) = ctx.facts.token_env.as_deref() {
        args.push("--token-env".to_string());
        args.push(var.to_string());
    }
    if let Some(login) = ctx.facts.login.as_deref() {
        // Protocol 1 knows `--login` on `identity` alone; on protocol 2
        // it is the login pin of D4.1c and travels on every call.
        if resolved.protocol >= PROTOCOL || verb == "identity" {
            args.push("--login".to_string());
            args.push(login.to_string());
        }
    }
    if resolved.protocol >= PROTOCOL {
        args.push("--host-kind".to_string());
        args.push(ctx.host_kind.as_str().to_string());
    }
    args
}

/// Whether this connector may be asked this verb at all (D2.2a): a
/// protocol 1 binary answers the six old verbs with `--remote`, and
/// nothing else.
fn refuse_outdated(
    resolved: &ResolvedPlugin,
    verb: &str,
    target: Option<&Target>,
) -> Result<(), PluginError> {
    if resolved.protocol >= PROTOCOL {
        return Ok(());
    }
    let answerable = LEGACY_VERBS.contains(&verb) && target.map(Target::is_remote).unwrap_or(true);
    if answerable {
        return Ok(());
    }
    Err(PluginError::Outdated {
        display: resolved.display,
        path: resolved.resolved_path.clone(),
        protocol: resolved.protocol,
        verb: verb.to_string(),
    })
}

/// Ask one verb and read its one JSON answer, with every failure named
/// (D2.3). This is the door every verb of the catalogue goes through,
/// the ones J3 adds included.
pub fn query<T: serde::de::DeserializeOwned>(
    spec: &ForgePluginSpec,
    verb: &str,
    target: Option<&Target>,
    extra: &[&str],
    ctx: &CallContext,
) -> Result<T, PluginError> {
    let resolved = resolve_plugin(spec)?;
    query_resolved(&resolved, verb, target, extra, ctx)
}

/// [`query`] against a connector that is already resolved, so a caller
/// that asks several verbs resolves once.
pub fn query_resolved<T: serde::de::DeserializeOwned>(
    resolved: &ResolvedPlugin,
    verb: &str,
    target: Option<&Target>,
    extra: &[&str],
    ctx: &CallContext,
) -> Result<T, PluginError> {
    refuse_outdated(resolved, verb, target)?;
    let args = call_args(resolved, verb, target, extra, ctx);
    let outcome = run_once_in(resolved, &args, &ctx.env(), ctx.root(), timeout_for(verb));
    if let Some(error) = outcome.spawn_error {
        return Err(PluginError::Spawn {
            display: resolved.display,
            path: resolved.resolved_path.clone(),
            error,
        });
    }
    if outcome.timed_out {
        return Err(PluginError::TimedOut {
            display: resolved.display,
            path: resolved.resolved_path.clone(),
            verb: verb.to_string(),
            timeout: timeout_for(verb),
        });
    }
    if outcome.exit_code != Some(0) {
        // A protocol 1 binary that slipped past the handshake (a file
        // replaced between the two calls) still exits 2 with an empty
        // stdout, and that is the detector of D2.2a.
        if outcome.exit_code == Some(2) && outcome.stdout_text.trim().is_empty() {
            return Err(PluginError::Outdated {
                display: resolved.display,
                path: resolved.resolved_path.clone(),
                protocol: 1,
                verb: verb.to_string(),
            });
        }
        return Err(PluginError::Failed {
            display: resolved.display,
            path: resolved.resolved_path.clone(),
            verb: verb.to_string(),
            exit_code: outcome.exit_code,
            stderr: outcome.stderr_text,
        });
    }
    let value = outcome.stdout_json.ok_or_else(|| PluginError::Unparsable {
        display: resolved.display,
        path: resolved.resolved_path.clone(),
        verb: verb.to_string(),
        answer: outcome.stdout_text.clone(),
    })?;
    serde_json::from_value(value).map_err(|e| PluginError::Unparsable {
        display: resolved.display,
        path: resolved.resolved_path.clone(),
        verb: verb.to_string(),
        answer: format!("{}: {e}", one_line(&outcome.stdout_text, 200)),
    })
}

// ---------------------------------------------------------------------
// The verbs joy-core itself asks
// ---------------------------------------------------------------------

/// A forge connector's identity answer (docs/plugins.md).
#[derive(Debug, Clone, Deserialize)]
pub struct ForgeIdentity {
    pub known: bool,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    /// Verified addresses the connector vouches for; possibly empty when
    /// its source cannot list them.
    #[serde(default)]
    pub emails: Vec<String>,
}

#[derive(Deserialize)]
struct ClaimsAnswer {
    claims: bool,
}

/// Whether this connector claims the target. False on every failure:
/// the claims round is the one place that must never fail loudly, and
/// the reason is in the log and in [`claims_full`].
pub fn claims(spec: &ForgePluginSpec, target: &Target, ctx: &CallContext) -> bool {
    claims_full(spec, target, ctx).unwrap_or(false)
}

/// [`claims`] with the reason (D2.3: missing, outdated, failed, timed
/// out are four different facts, and `false` is a fifth).
pub fn claims_full(
    spec: &ForgePluginSpec,
    target: &Target,
    ctx: &CallContext,
) -> Result<bool, PluginError> {
    query::<ClaimsAnswer>(spec, "claims", Some(target), &[], ctx).map(|a| a.claims)
}

/// Who is ACTING on the forge; `None` on every failure or
/// `known:false`. [`identity_full`] tells the two apart.
pub fn identity(
    spec: &ForgePluginSpec,
    target: Option<&Target>,
    ctx: &CallContext,
) -> Option<ForgeIdentity> {
    identity_full(spec, target, ctx)
        .ok()
        .filter(|identity| identity.known)
}

/// [`identity`] with `known:false` kept apart from every failure.
pub fn identity_full(
    spec: &ForgePluginSpec,
    target: Option<&Target>,
    ctx: &CallContext,
) -> Result<ForgeIdentity, PluginError> {
    let mut extra: Vec<&str> = Vec::new();
    if let Some(id) = ctx.facts.user_id.as_deref() {
        extra.extend(["--user-id", id]);
    }
    query(spec, "identity", target, &extra, ctx)
}

/// Whose address is this? PURE by contract: the connector answers from
/// the address alone, never from ambient state. `None` on every failure
/// or `known:false`.
pub fn resolve(spec: &ForgePluginSpec, email: &str, ctx: &CallContext) -> Option<ForgeIdentity> {
    query::<ForgeIdentity>(spec, "resolve", None, &["--email", email], ctx)
        .ok()
        .filter(|identity| identity.known)
}

/// What a forge says about a repository's joy store (JP-013C-11), the
/// answer of the `store` query. A multi-account host asks it instead of
/// cloning the repository to find out.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum StoreAnswer {
    /// The store is there and readable: the content of its project.yaml.
    Store {
        project_yaml: String,
        /// The repository's size in BYTES, normalised by the connector
        /// because the unit is forge knowledge (D2.4). Optional, and
        /// its absence is not an error.
        #[serde(default)]
        size_bytes: Option<u64>,
    },
    /// The repository is there but holds no store; whether the caller may
    /// push one, and the branch the forge names as the default (the first
    /// push of an empty repository goes there).
    Missing {
        may_create: bool,
        #[serde(default)]
        default_branch: Option<String>,
        #[serde(default)]
        size_bytes: Option<u64>,
    },
    /// The forge does not show the caller the repository: deleted, or no
    /// access. Forges answer both the same way on purpose.
    Gone,
    /// The forge could not be asked (network, a refused token, an answer
    /// the connector did not expect). Never a verdict on the repository.
    Unknown,
}

/// Does the repository at `target` hold a joy store, and may the caller
/// create one? `None` on every connector failure and on `unknown`: the
/// question stayed unanswered.
pub fn store(spec: &ForgePluginSpec, target: &Target, ctx: &CallContext) -> Option<StoreAnswer> {
    query::<StoreAnswer>(spec, "store", Some(target), &[], ctx)
        .ok()
        .filter(|answer| *answer != StoreAnswer::Unknown)
}

/// The files a repository's default branch carries (JAPP-0293-A7), the
/// answer of the `files` query. A listing the forge cut off, or the
/// connector bounded, says so.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum FilesAnswer {
    Files { paths: Vec<String>, truncated: bool },
    Unknown,
}

/// The files of the repository at `target`; `None` on every connector
/// failure and on `unknown`.
pub fn files(spec: &ForgePluginSpec, target: &Target, ctx: &CallContext) -> Option<FilesAnswer> {
    query::<FilesAnswer>(spec, "files", Some(target), &[], ctx)
        .ok()
        .filter(|answer| *answer != FilesAnswer::Unknown)
}

/// What the release verb answered (JOY-0256-64). `unsupported` is the
/// connector saying "my forge has no release backend yet", and the
/// caller then keeps the tag-only publish instead of failing the whole
/// release.
#[derive(Debug, Deserialize)]
pub struct ReleaseOutcome {
    /// The release URL when the forge reports one.
    pub url: Option<String>,
    /// The connector has no release capability for its forge (yet).
    #[serde(default)]
    pub unsupported: bool,
}

/// Create (or complete) the release for `tag` through the connector.
///
/// The one verb that reports its failure instead of degrading: a
/// release nobody made must not look like a release that was made, so
/// the caller gets the named reason (and with it the connector's own
/// stderr, which is now captured rather than inherited).
pub fn release(
    spec: &ForgePluginSpec,
    tag: &str,
    title: &str,
    notes_file: &Path,
    ctx: &CallContext,
) -> Result<ReleaseOutcome, PluginError> {
    let notes_path = notes_file.to_string_lossy().into_owned();
    query(
        spec,
        "release",
        None,
        &["--tag", tag, "--title", title, "--notes-file", &notes_path],
        ctx,
    )
}

/// The connector responsible for this project: the `forge:` override
/// when it names a registered forge, else the first registry row that
/// claims one of the remotes. `None` = nobody is responsible (a
/// local-only project, or no connector installed) and every caller
/// proceeds exactly as before.
pub fn responsible_plugin(
    forge_override: Option<&str>,
    ctx: &CallContext,
    remotes: &[(String, String)],
) -> Option<&'static ForgePluginSpec> {
    if let Some(id) = forge_override {
        // An explicit override is the operator's word: no claims round.
        return by_id(id);
    }
    if remotes.is_empty() {
        return None;
    }
    FORGE_PLUGINS.iter().find(|spec| {
        remotes
            .iter()
            .any(|(_, url)| claims(spec, &Target::remote(url.as_str()), ctx))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_answers_by_id_case_insensitively() {
        assert_eq!(by_id("github").map(|s| s.binary_names), Some(GITHUB_NAMES));
        assert_eq!(by_id(" GitLab ").map(|s| s.id), Some("gitlab"));
        assert_eq!(by_id("sourcehut"), None);
    }

    const GITHUB_NAMES: &[&str] = &[COMBINED_BINARY, "joy-github"];

    /// The name order of D2.2: the combined connector is tried first in
    /// EVERY directory, so a fresh `joy-forge` beside a stale
    /// `joy-github` in `~/.cargo/bin` wins.
    #[test]
    fn the_combined_binary_is_the_first_name_of_every_forge() {
        for spec in FORGE_PLUGINS {
            assert_eq!(spec.binary_names.first(), Some(&COMBINED_BINARY));
            assert_eq!(spec.binary_names.len(), 2);
            assert_eq!(
                spec.legacy_names().collect::<Vec<_>>(),
                vec![format!("joy-{}", spec.id)]
            );
        }
    }

    #[test]
    fn identity_answers_parse_and_unknown_filters_out() {
        let known: ForgeIdentity = serde_json::from_str(
            r#"{"known":true,"login":"alice","user_id":"12345","emails":["a@example.com"]}"#,
        )
        .unwrap();
        assert!(known.known);
        assert_eq!(known.emails, vec!["a@example.com"]);
        let unknown: ForgeIdentity = serde_json::from_str(r#"{"known":false}"#).unwrap();
        assert!(!unknown.known);
        assert!(unknown.emails.is_empty());
    }

    #[test]
    fn store_answers_parse_in_every_state() {
        let parse = |raw: &str| serde_json::from_str::<StoreAnswer>(raw).unwrap();
        assert_eq!(
            parse(r#"{"state":"store","project_yaml":"name: x\n"}"#),
            StoreAnswer::Store {
                project_yaml: "name: x\n".into(),
                size_bytes: None,
            }
        );
        // D2.4's optional field, in bytes whatever the forge counts in
        assert_eq!(
            parse(r#"{"state":"store","project_yaml":"x","size_bytes":4096}"#),
            StoreAnswer::Store {
                project_yaml: "x".into(),
                size_bytes: Some(4096),
            }
        );
        assert_eq!(
            parse(r#"{"state":"missing","may_create":true,"default_branch":"main"}"#),
            StoreAnswer::Missing {
                may_create: true,
                default_branch: Some("main".into()),
                size_bytes: None,
            }
        );
        // a connector that names no default branch still parses
        assert_eq!(
            parse(r#"{"state":"missing","may_create":false}"#),
            StoreAnswer::Missing {
                may_create: false,
                default_branch: None,
                size_bytes: None,
            }
        );
        let files = serde_json::from_str::<FilesAnswer>(
            r#"{"state":"files","paths":["VISION.md"],"truncated":false}"#,
        )
        .unwrap();
        assert_eq!(
            files,
            FilesAnswer::Files {
                paths: vec!["VISION.md".into()],
                truncated: false
            }
        );
        assert_eq!(parse(r#"{"state":"gone"}"#), StoreAnswer::Gone);
        assert_eq!(parse(r#"{"state":"unknown"}"#), StoreAnswer::Unknown);
    }

    /// The handshake object of D2.2a, exactly as the design writes it.
    #[test]
    fn the_version_answer_parses() {
        let answer: VersionAnswer = serde_json::from_str(
            r#"{"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}"#,
        )
        .unwrap();
        assert_eq!(answer.protocol, PROTOCOL);
        assert_eq!(answer.plugin.as_deref(), Some("joy-forge 0.21.0"));
        assert_eq!(answer.forges.len(), 3);
    }

    /// The deadlines of D2.3, per verb and not per call site.
    #[test]
    fn every_verb_of_the_catalogue_has_its_deadline() {
        for verb in ["claims", "identity", "resolve", "web-url", "version"] {
            assert_eq!(timeout_for(verb), Duration::from_secs(5), "{verb}");
        }
        for verb in [
            "store",
            "files",
            "repositories",
            "create-repository",
            "token",
        ] {
            assert_eq!(timeout_for(verb), Duration::from_secs(30), "{verb}");
        }
        assert_eq!(timeout_for("release"), Duration::from_secs(120));
        let login = StreamBounds::for_verb("login");
        assert_eq!(login.first_event, Duration::from_secs(15));
        assert_eq!(login.total, Duration::from_secs(900));
        // a verb nobody wrote down gets the careful class
        assert_eq!(timeout_for("something-new"), Duration::from_secs(5));
    }

    /// Every call carries the host kind (D1.10) and the login pin
    /// (D4.1c), and the combined binary carries its forge id first.
    #[test]
    fn a_protocol_two_call_carries_the_host_kind_and_the_pin() {
        let resolved = resolved_stub("/opt/joy/joy-forge", 2);
        let ctx = CallContext::rootless()
            .with_host_kind(HostKind::Delegated)
            .with_facts(CallerFacts {
                login: Some("scotty".into()),
                token_env: Some("JOY_FORGE_TOKEN".into()),
                ..CallerFacts::default()
            });
        let args = call_args(
            &resolved,
            "store",
            Some(&Target::host("git.acme.com")),
            &[],
            &ctx,
        );
        assert_eq!(
            args,
            vec![
                "github",
                "store",
                "--host",
                "git.acme.com",
                "--token-env",
                "JOY_FORGE_TOKEN",
                "--login",
                "scotty",
                "--host-kind",
                "delegated",
            ]
        );
    }

    /// A protocol 1 connector is spoken to the way it understands: no
    /// forge id, no `--host-kind`, no pin, and `--login` only on the one
    /// verb that always took it.
    #[test]
    fn a_protocol_one_call_carries_none_of_the_new_fields() {
        let resolved = resolved_stub("/home/s/.cargo/bin/joy-github", 1);
        let ctx = CallContext::rootless()
            .with_host_kind(HostKind::Interactive)
            .with_facts(CallerFacts {
                login: Some("scotty".into()),
                ..CallerFacts::default()
            });
        let target = Target::remote("git@github.com:o/r.git");
        assert_eq!(
            call_args(&resolved, "claims", Some(&target), &[], &ctx),
            vec!["claims", "--remote", "git@github.com:o/r.git"]
        );
        assert_eq!(
            call_args(&resolved, "identity", None, &[], &ctx),
            vec!["identity", "--login", "scotty"]
        );
    }

    /// D2.2a: the six old verbs with `--remote` are all a protocol 1
    /// connector may be asked; everything else is `plugin_outdated`
    /// with the resolved path and the `rm` line.
    #[test]
    fn a_protocol_one_connector_is_outdated_for_everything_new() {
        let resolved = resolved_stub("/home/s/.cargo/bin/joy-github", 1);
        let remote = Target::remote("git@github.com:o/r.git");
        let host = Target::host("github.com");
        assert!(refuse_outdated(&resolved, "claims", Some(&remote)).is_ok());
        assert!(refuse_outdated(&resolved, "release", None).is_ok());
        for (verb, target) in [
            ("claims", Some(&host)),
            ("token", Some(&host)),
            ("login", Some(&host)),
            ("web-url", Some(&remote)),
        ] {
            let error = refuse_outdated(&resolved, verb, target)
                .expect_err("a protocol 1 connector cannot answer this");
            assert_eq!(error.state(), "plugin_outdated");
            let text = error.to_string();
            assert!(text.contains("/home/s/.cargo/bin/joy-github"), "{text}");
            assert!(text.contains("speaks protocol 1"), "{text}");
            assert!(text.contains("rm /home/s/.cargo/bin/joy-github"), "{text}");
        }
        assert_eq!(
            resolved.removal_line(),
            "rm /home/s/.cargo/bin/joy-github".to_string()
        );
    }

    /// A connector that is not installed says so by name, and the state
    /// is the one D2.2a gives it.
    #[test]
    fn a_missing_connector_names_what_was_looked_for() {
        let error = PluginError::Missing {
            display: "GitHub",
            names: vec![COMBINED_BINARY.to_string(), "joy-github".to_string()],
        };
        assert_eq!(error.state(), "plugin_missing");
        assert!(error.resolved_path().is_none());
        let text = error.to_string();
        assert!(text.contains("joy-forge, joy-github"), "{text}");
        assert!(text.contains("no GitHub connector is installed"), "{text}");
    }

    /// The search order of D2.2, and the rule that a directory named
    /// twice is searched once.
    #[test]
    fn the_search_order_puts_the_executable_directory_before_path() {
        let dirs = search_dirs();
        let exe_at = dirs
            .iter()
            .position(|(_, found_in)| *found_in == FoundIn::ExecutableDir);
        let path_at = dirs
            .iter()
            .position(|(_, found_in)| *found_in == FoundIn::Path);
        if let (Some(exe_at), Some(path_at)) = (exe_at, path_at) {
            assert!(exe_at < path_at, "{dirs:?}");
        }
        let mut seen: Vec<&PathBuf> = Vec::new();
        for (dir, _) in &dirs {
            assert!(!seen.contains(&dir), "{dir:?} twice in {dirs:?}");
            seen.push(dir);
        }
    }

    /// The connector's own words reach the error, on one line and
    /// bounded, because a person needs the message and not its layout.
    #[test]
    fn a_refusal_carries_the_connector_text() {
        let error = PluginError::Failed {
            display: "Gitea",
            path: PathBuf::from("/usr/local/bin/joy-forge"),
            verb: "store".to_string(),
            exit_code: Some(3),
            stderr: "  the token was refused\n  by git.acme.com\n".to_string(),
        };
        assert_eq!(error.state(), "plugin_failed");
        let text = error.to_string();
        assert!(text.contains("/usr/local/bin/joy-forge"), "{text}");
        assert!(text.contains("exit 3"), "{text}");
        assert!(
            text.contains("the token was refused by git.acme.com"),
            "{text}"
        );
    }

    fn resolved_stub(path: &str, protocol: u32) -> ResolvedPlugin {
        ResolvedPlugin {
            id: "github",
            display: "GitHub",
            binary_names: GITHUB_NAMES,
            resolved_path: PathBuf::from(path),
            found_in: FoundIn::Path,
            protocol,
            plugin_version: None,
        }
    }
}
