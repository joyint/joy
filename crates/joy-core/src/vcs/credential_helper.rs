// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! joy's own credential helper runner (design D1.3).
//!
//! `git2::Cred::credential_helper` is not used anywhere in joy, and
//! this module is the reason. git2 builds the string
//! `git credential-<name>` for every short helper name and runs it
//! through `sh -c` (cred.rs:310-316, :395), which spawns a GIT PROCESS
//! for a credential - on a product that has no git binary to spawn
//! (ADR: git2 only), on Windows where `sh` may not be on PATH at all,
//! and only ever with the `get` operation, which is why a revoked Git
//! Credential Manager entry is replayed on every single contact.
//!
//! What joy does instead:
//!
//! - the helper binary is spawned BY ITS OWN NAME,
//!   `git-credential-<name>`, found in Git for Windows' own directories
//!   through its registry keys or in git's libexec directory on unix;
//! - the config lookup includes the PORT, which git2 drops
//!   (cred.rs:323-330), and reads EVERY value of a multivar, not only
//!   the last one, with an empty value resetting the chain the way git
//!   does it;
//! - a shell shaped value (`!...`) is split with a quote-aware argv
//!   splitter, because `gh` always single-quotes its Windows path, and
//!   only a value that genuinely needs a shell gets one: Git for
//!   Windows' own `usr\bin\sh.exe` on Windows, `/bin/sh` on unix,
//!   never `sh` from PATH;
//! - `store` and `erase` are run beside `get`, so an accepted
//!   credential is remembered and a refused one is forgotten instead of
//!   being replayed;
//! - the helper's stderr is captured and quoted in the failure, which
//!   is the difference between "authentication failed" and "helper
//!   'manager': fatal: Cannot prompt because user interactivity has
//!   been disabled.";
//! - the environment that silences a helper's own dialogs is set PER
//!   SPAWN through `Command::env` (never `std::env::set_var`, which
//!   would leak into every other thread of a desktop app) and only for
//!   the host kinds that have nobody to answer a dialog.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::remote_url::RemoteUrl;
use crate::host::HostKind;

/// How long a helper may take when nobody can answer it. A worker
/// needs this bound, because a helper that waits for a dialog nobody
/// sees would otherwise stop the sync for ever.
const QUIET_DEADLINE: Duration = Duration::from_secs(30);

/// How long a helper may take when a person CAN answer it. A Git
/// Credential Manager window a person is typing into is not a hang, so
/// the bound is generous, but there is one: this call sits inside
/// libgit2's credentials callback, and a dialog that opened behind
/// another window and is never answered would otherwise hang the whole
/// fetch with no way out (design D1.3, D1.10).
const PROMPT_DEADLINE: Duration = Duration::from_secs(600);

/// How long an answer is reused without asking the helper again
/// (design D1.7: a 1 Hz poll must not spawn a .NET process per
/// contact).
const CACHE_TTL: Duration = Duration::from_secs(300);

/// What joy asks a helper about: git's credential protocol, one
/// `key=value` line each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub protocol: String,
    /// The host, with `:port` when the URL carries one and in brackets
    /// when it is an IPv6 literal. git2 writes no host at all for an IP
    /// literal (cred.rs:214-220), so a helper keyed on an internal
    /// address never answers there.
    pub host: String,
    /// The repository path, sent only when `useHttpPath` is set.
    pub path: Option<String>,
    /// The user name, when the URL or the config names one.
    pub username: Option<String>,
    /// The URL as configured, for the most specific config key.
    pub url: String,
}

impl Request {
    /// The request for a remote URL; `None` for a transport no helper
    /// answers for (ssh has its own chain, design D1.2).
    pub fn for_url(url: &str) -> Option<Request> {
        let parsed = RemoteUrl::parse(url)?;
        if !parsed.transport.takes_helper() {
            return None;
        }
        Some(Request {
            protocol: parsed.transport.protocol().to_string(),
            host: parsed.host_field(),
            path: Some(parsed.path.clone()).filter(|p| !p.is_empty()),
            username: parsed.user.clone(),
            url: url.to_string(),
        })
    }

    /// `<protocol>://<host>[:<port>]`: the middle config key and the
    /// key of the per-host cache.
    pub fn host_key(&self) -> String {
        format!("{}://{}", self.protocol, self.host)
    }

    /// The lines joy writes on the helper's stdin, terminated by the
    /// blank line that ends a credential description.
    fn lines(&self, with_path: bool, credential: Option<&Credential>) -> String {
        let mut text = format!("protocol={}\nhost={}\n", self.protocol, self.host);
        if with_path {
            if let Some(path) = &self.path {
                text.push_str(&format!("path={path}\n"));
            }
        }
        let username = credential
            .map(|c| c.username.clone())
            .or_else(|| self.username.clone());
        if let Some(username) = username {
            text.push_str(&format!("username={username}\n"));
        }
        if let Some(credential) = credential {
            text.push_str(&format!("password={}\n", credential.password));
        }
        text.push('\n');
        text
    }
}

/// The three operations of git's credential protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Get,
    Store,
    Erase,
}

impl Op {
    fn word(self) -> &'static str {
        match self {
            Op::Get => "get",
            Op::Store => "store",
            Op::Erase => "erase",
        }
    }
}

/// One credential a helper answered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub username: String,
    pub password: String,
    /// The configured value that produced it, for the detail line and
    /// for `store` and `erase` later.
    pub helper: String,
}

/// A helper that could not be run, or ran and said no, with the text
/// it wrote on stderr. The text is the whole point: GCM's own sentence
/// names the cause, joy's would not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperFailure {
    pub helper: String,
    pub detail: String,
}

impl std::fmt::Display for HelperFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "helper '{}': {}", self.helper, self.detail)
    }
}

impl std::error::Error for HelperFailure {}

// ---- the config chain -------------------------------------------------

/// Every helper the config names for this request, most specific
/// first, with the empty value resetting the values of its own key.
///
/// git reads all three keys and all values of each; git2 reads two
/// keys without the port and keeps only the last value of each
/// (config_list.c:160-201, cred.rs:323-330).
///
/// The reset is per key, and that is a considered difference from git.
/// git resets in config-READ order, where `credential.<exact url>`
/// normally stands in the repository's own config and is therefore
/// read after a global `credential.helper`; joy walks the keys most
/// specific first, as D1.3 lists them, so honouring the reset across
/// keys would let an empty global `credential.helper` wipe the
/// entries written for one exact URL. Inside one key the values ARE in
/// config order, so the empty value resets them the way git does it:
/// "the empty string ... resets the helper list to empty"
/// (git-config(1), credential.helper), which is how a repository
/// switches its global helper off.
pub fn configured_helpers(config: &git2::Config, request: &Request) -> Vec<String> {
    let mut chain = Vec::new();
    for key in helper_keys(request) {
        let mut from_key = Vec::new();
        for value in multivar(config, &format!("credential.{key}helper")) {
            let value = value.trim().to_string();
            if value.is_empty() {
                from_key.clear();
            } else {
                from_key.push(value);
            }
        }
        chain.append(&mut from_key);
    }
    chain
}

/// Whether the repository path belongs in the request
/// (`credential.useHttpPath`).
///
/// Unlike the helper chain this is a single value: the most specific
/// key that carries one decides, and inside that key the last value
/// wins, the way git reads every single-valued setting.
pub fn use_http_path(config: &git2::Config, request: &Request) -> bool {
    most_specific(config, request, "useHttpPath")
        .and_then(|value| git2::Config::parse_bool(value.as_str()).ok())
        .unwrap_or(false)
}

/// The user name the config names for this request, if any.
pub fn configured_username(config: &git2::Config, request: &Request) -> Option<String> {
    most_specific(config, request, "username")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The value of `credential.<...>.<name>` from the most specific key
/// that carries one.
fn most_specific(config: &git2::Config, request: &Request, name: &str) -> Option<String> {
    helper_keys(request)
        .into_iter()
        .find_map(|key| multivar(config, &format!("credential.{key}{name}")).pop())
}

/// The three key prefixes of D1.3, least specific LAST so that the
/// chain reads most specific first.
fn helper_keys(request: &Request) -> Vec<String> {
    let mut keys = vec![format!("{}.", request.url)];
    let host_key = request.host_key();
    if !keys.iter().any(|k| k.trim_end_matches('.') == host_key) {
        keys.push(format!("{host_key}."));
    }
    keys.push(String::new());
    keys
}

/// Every value of a multivar, in config order. An absent key is an
/// empty list, never an error: a repository without a credential
/// section is the normal case.
fn multivar(config: &git2::Config, name: &str) -> Vec<String> {
    let mut values = Vec::new();
    if let Ok(entries) = config.multivar(name, None) {
        let _ = entries.for_each(|entry| {
            values.push(entry.value().unwrap_or_default().to_string());
        });
    }
    values
}

// ---- what a configured value means ------------------------------------

/// A resolved helper: what joy spawns, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spawn {
    /// The binary itself, with its arguments. No shell involved.
    Direct { program: PathBuf, args: Vec<String> },
    /// A value that genuinely needs a shell (a pipe, a variable, a
    /// redirection), run by a shell joy names by its absolute path.
    Shell { shell: PathBuf, command: String },
}

/// Where a short helper name is looked up, and which shell runs a
/// shell shaped value. Built from the machine in production and handed
/// in by the tests.
#[derive(Debug, Clone, Default)]
pub struct Search {
    pub dirs: Vec<PathBuf>,
    pub shell: Option<PathBuf>,
    /// Directories put IN FRONT of the child's own PATH, because the
    /// helper's own bootstrap needs them.
    ///
    /// Git Credential Manager is a .NET application that runs git
    /// itself for parts of its work, and the first thing its bootstrap
    /// does is look for `git.exe` on PATH; without it the helper dies
    /// with "Failed to locate git.exe executable on the path" and no
    /// credential is obtained. joy already knows where Git for Windows
    /// keeps it, from the same registry keys it resolves the helper
    /// with (D1.3), so it hands the child that directory. This is the
    /// HELPER spawning git, not joy: joy's own rule is that it never
    /// builds a git command line, and it does not (ADR: git2 only).
    pub child_path: Vec<PathBuf>,
}

impl Search {
    /// This machine's places, in the order D1.3 names them.
    pub fn of_this_machine() -> Search {
        let mut dirs = Vec::new();
        let mut shell = None;
        let child_path = git_binary_dirs();
        #[cfg(windows)]
        {
            for install in git_for_windows_dirs() {
                dirs.push(install.join("mingw64").join("bin"));
                dirs.push(install.join("mingw64").join("libexec").join("git-core"));
                dirs.push(install.join("cmd"));
                let candidate = install.join("usr").join("bin").join("sh.exe");
                if shell.is_none() && candidate.is_file() {
                    shell = Some(candidate);
                }
            }
            for libexec in git_for_windows_values("LibexecPath") {
                dirs.push(PathBuf::from(libexec));
            }
        }
        #[cfg(not(windows))]
        {
            // git's own libexec, reached from the git binary's place
            // WITHOUT running git: joy never spawns it (ADR: git2 only).
            if let Some(bin) = which("git") {
                if let Some(prefix) = bin.parent().and_then(|dir| dir.parent()) {
                    dirs.push(prefix.join("libexec").join("git-core"));
                    // Debian and its derivatives ship git-core under
                    // lib, not libexec.
                    dirs.push(prefix.join("lib").join("git-core"));
                }
            }
            let sh = PathBuf::from("/bin/sh");
            if sh.is_file() {
                shell = Some(sh);
            }
        }
        dirs.extend(path_dirs());
        // PATH repeats a directory often enough, and rarely next to
        // itself, so `dedup` alone would leave the repeats in and stat
        // each of them again for every helper name.
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|dir| seen.insert(dir.clone()));
        Search {
            dirs,
            shell,
            child_path,
        }
    }

    /// The binary called `git-credential-<name>`, if this machine has
    /// one. joy never falls back to the string `git credential-<name>`:
    /// that is a git process, and joy spawns none.
    pub fn named_helper(&self, name: &str) -> Option<PathBuf> {
        let plain = format!("git-credential-{name}");
        let names: Vec<String> = if cfg!(windows) {
            vec![
                format!("{plain}.exe"),
                format!("{plain}.cmd"),
                format!("{plain}.bat"),
                plain.clone(),
            ]
        } else {
            vec![plain.clone()]
        };
        for dir in &self.dirs {
            for candidate in &names {
                let path = dir.join(candidate);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }
}

/// What a configured value turns into, or why it cannot be run.
pub fn resolve(value: &str, search: &Search) -> Result<Spawn, HelperFailure> {
    let failure = |detail: String| HelperFailure {
        helper: value.to_string(),
        detail,
    };
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix('!') {
        let rest = rest.trim();
        if needs_shell(rest) {
            let shell = search.shell.clone().ok_or_else(|| {
                failure(
                    "this helper needs a shell and joy found none (no /bin/sh, no Git for Windows sh.exe)"
                        .to_string(),
                )
            })?;
            return Ok(Spawn::Shell {
                shell,
                command: rest.to_string(),
            });
        }
        let argv = split_argv(rest)
            .ok_or_else(|| failure("the helper value has an unterminated quote".to_string()))?;
        let mut argv = argv.into_iter();
        let program = argv
            .next()
            .ok_or_else(|| failure("the helper value is empty".to_string()))?;
        return Ok(Spawn::Direct {
            program: PathBuf::from(program),
            args: argv.collect(),
        });
    }
    if is_absolute(trimmed) {
        // An absolute path is used as it stands. Only when the whole
        // value is not a file does joy read arguments out of it, which
        // is how `/usr/bin/git-credential-foo --timeout 5` still runs
        // without a shell.
        let whole = PathBuf::from(trimmed);
        if whole.is_file() || !trimmed.contains(' ') {
            return Ok(Spawn::Direct {
                program: whole,
                args: Vec::new(),
            });
        }
        let argv = split_argv(trimmed)
            .ok_or_else(|| failure("the helper value has an unterminated quote".to_string()))?;
        let mut argv = argv.into_iter();
        let program = argv
            .next()
            .ok_or_else(|| failure("the helper value is empty".to_string()))?;
        return Ok(Spawn::Direct {
            program: PathBuf::from(program),
            args: argv.collect(),
        });
    }
    if trimmed.is_empty() {
        return Err(failure("the helper value is empty".to_string()));
    }
    let (name, args) = match split_argv(trimmed) {
        Some(argv) if !argv.is_empty() => {
            let mut argv = argv.into_iter();
            (argv.next().unwrap_or_default(), argv.collect::<Vec<_>>())
        }
        _ => (trimmed.to_string(), Vec::new()),
    };
    match search.named_helper(&name) {
        Some(program) => Ok(Spawn::Direct { program, args }),
        None => Err(failure(format!(
            "no binary named git-credential-{name} was found; joy does not fall back to a git process"
        ))),
    }
}

/// Whether a shell shaped value needs a real shell, or is just a
/// command line with quotes in it. gh's own value,
/// `!'C:\Program Files\GitHub CLI\gh.exe' auth git-credential`, is the
/// second kind and must not be handed to a shell that may not exist.
pub fn needs_shell(value: &str) -> bool {
    let mut quote = None;
    for c in value.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => quote = Some(c),
                '|' | '&' | ';' | '<' | '>' | '(' | ')' | '$' | '`' | '\n' | '*' | '?' | '['
                | ']' | '{' | '}' | '~' => return true,
                _ => {}
            },
        }
    }
    false
}

/// Split a command line into argv, honouring single and double
/// quotes. `None` when a quote is never closed.
///
/// A backslash keeps its literal meaning unless it stands in front of
/// a character a shell would escape (a space, a quote, another
/// backslash, and inside double quotes `$` and a backtick). That is
/// the one rule that reads a Windows path and an escaped space the way
/// both were meant: `"C:\Program Files\gh.exe"` stays a path, and
/// `/opt/my\ helper` stays one word.
pub fn split_argv(value: &str) -> Option<Vec<String>> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => {
                let next = chars.peek().copied();
                match next {
                    Some('"') | Some('\\') | Some('$') | Some('`') => {
                        current.push(chars.next().unwrap_or_default());
                    }
                    _ => current.push(c),
                }
            }
            Some(_) => current.push(c),
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                '\\' => {
                    started = true;
                    let next = chars.peek().copied();
                    match next {
                        Some(' ') | Some('\'') | Some('"') | Some('\\') => {
                            current.push(chars.next().unwrap_or_default());
                        }
                        _ => current.push(c),
                    }
                }
                c if c.is_whitespace() => {
                    if started {
                        argv.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                c => {
                    started = true;
                    current.push(c);
                }
            },
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        argv.push(current);
    }
    Some(argv)
}

fn is_absolute(value: &str) -> bool {
    if value.starts_with('/') {
        return true;
    }
    // A Windows path, recognised on every host so that a config
    // written on Windows reads the same everywhere.
    let bytes = value.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    value.starts_with("\\\\")
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

/// The first `name` on PATH. Used to FIND git's libexec directory, not
/// to run git.
#[cfg(not(windows))]
fn which(name: &str) -> Option<PathBuf> {
    path_dirs().into_iter().find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
    })
}

/// Where this machine keeps `git.exe` for a helper's OWN bootstrap
/// (see [`Search::child_path`]). Only a directory that really holds it
/// is named, so the child's PATH grows by what the helper needs and by
/// nothing else.
#[cfg(windows)]
fn git_binary_dirs() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for install in git_for_windows_dirs() {
        for holds_git in [
            install.join("cmd"),
            install.join("bin"),
            install.join("mingw64").join("bin"),
        ] {
            if holds_git.join("git.exe").is_file() && !found.contains(&holds_git) {
                found.push(holds_git);
            }
        }
    }
    found
}

/// On unix a helper that wants git finds it the same way joy found the
/// helper: on the PATH the child inherits. Nothing is prepended.
#[cfg(not(windows))]
fn git_binary_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// Git for Windows' install paths, from its own registry keys
/// (install.iss:157-162 writes them), user hive first.
#[cfg(windows)]
fn git_for_windows_dirs() -> Vec<PathBuf> {
    git_for_windows_values("InstallPath")
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(windows)]
fn git_for_windows_values(value: &str) -> Vec<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let mut found = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        if let Some(text) = registry_string(root, "Software\\GitForWindows", value) {
            if !text.is_empty() && !found.contains(&text) {
                found.push(text);
            }
        }
    }
    found
}

/// One `REG_SZ` value, or `None` when the key, the value or the
/// installation is not there.
#[cfg(windows)]
fn registry_string(
    root: windows_sys::Win32::System::Registry::HKEY,
    subkey: &str,
    value: &str,
) -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};

    let subkey = wide(subkey);
    let value = wide(value);
    let mut size: u32 = 0;
    let probe = unsafe {
        RegGetValueW(
            root,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if probe != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
    let mut size_again = size;
    let read = unsafe {
        RegGetValueW(
            root,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size_again,
        )
    };
    if read != ERROR_SUCCESS {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---- running one helper -----------------------------------------------

/// What a helper answered.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Answer {
    pub username: Option<String>,
    pub password: Option<String>,
    /// The helper said "stop asking anybody else".
    pub quit: bool,
}

/// How one helper call is made: what goes on the child's input, what
/// the child is allowed to do, and what its PATH needs in front of it.
///
/// One value for all three, so that `get` and the `store` and `erase`
/// that follow it reach the same binary in the same way, and so that
/// adding a fourth fact later does not add a fourth parameter.
#[derive(Debug, Clone, Default)]
pub struct Manner {
    /// `path=` rides on the request (`credential.useHttpPath`).
    pub with_path: bool,
    /// Who is at this machine, which decides whether the helper may
    /// raise a window of its own (D1.10).
    pub kind: HostKind,
    /// [`Search::child_path`], for the helper's own bootstrap.
    pub child_path: Vec<PathBuf>,
}

/// Run one helper for one operation.
pub fn run(
    spawn: &Spawn,
    label: &str,
    op: Op,
    request: &Request,
    credential: Option<&Credential>,
    manner: &Manner,
) -> Result<Answer, HelperFailure> {
    let Manner {
        with_path,
        kind,
        child_path,
    } = manner;
    let (with_path, kind) = (*with_path, *kind);
    let failure = |detail: String| HelperFailure {
        helper: label.to_string(),
        detail,
    };
    let mut command = match spawn {
        Spawn::Direct { program, args } => {
            let mut command = joy_process::command(program);
            command.args(args);
            command.arg(op.word());
            command
        }
        Spawn::Shell { shell, command } => {
            let mut spawned = joy_process::command(shell);
            spawned.arg("-c");
            spawned.arg(format!("{command} {}", op.word()));
            spawned
        }
    };
    // Per spawn, never through the process environment: a desktop app
    // sets these for ONE child, not for every thread it runs
    // (design D1.3). An interactive host sets none of them, because
    // there the helper's own window is the way in.
    if !kind.may_prompt() {
        command.env("GCM_INTERACTIVE", "never");
        command.env("GCM_GUI_PROMPT", "0");
        command.env("GIT_TERMINAL_PROMPT", "0");
    }
    // The helper's own bootstrap may need a binary joy knows where to
    // find and the child's inherited PATH does not name: Git Credential
    // Manager looks for `git.exe` before it does anything at all
    // (`Search::child_path`). Per spawn, in front of what the child
    // would have had, and never through this process's own environment.
    if !child_path.is_empty() {
        let mut dirs: Vec<PathBuf> = child_path.to_vec();
        for dir in path_dirs() {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        match std::env::join_paths(&dirs) {
            Ok(joined) => {
                command.env("PATH", joined);
            }
            Err(e) => tracing::debug!(error = %e, "the helper's PATH was left as it was"),
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| failure(format!("could not be started: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        // Write errors are ignored the way git ignores them: a helper
        // that answers without reading is not an error.
        let _ = stdin.write_all(request.lines(with_path, credential).as_bytes());
    }
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = pipe.read_to_string(&mut text);
        }
        text
    });
    let err_reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.read_to_string(&mut text);
        }
        text
    });
    let bound = if kind.may_prompt() {
        PROMPT_DEADLINE
    } else {
        QUIET_DEADLINE
    };
    // A helper that may open its own window is answered by a PERSON,
    // and the wait for that is not silence: joy's own contact bound is
    // held open while this runs, and THIS deadline is the one that
    // applies (design D1.3, `super::bound`).
    let _hold = kind.may_prompt().then(super::bound::hold);
    let status = wait_bounded(&mut child, bound);
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    let status = match status {
        Ok(status) => status,
        Err(detail) => return Err(failure(quoted(&detail, &stderr))),
    };
    if !status.success() {
        let code = status
            .code()
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "killed by a signal".to_string());
        // The exit code is kept here rather than in the detail: the
        // detail is the helper's own sentence (see `quoted`).
        tracing::debug!(helper = label, op = op.word(), status = %code, "credential helper refused");
        return Err(failure(quoted(&code, &stderr)));
    }
    Ok(parse_answer(&stdout))
}

/// The child's status, killing it when the bound runs out. Every host
/// kind has a bound: an unbounded `wait` here is a hung fetch, because
/// this runs inside libgit2's credentials callback.
fn wait_bounded(
    child: &mut std::process::Child,
    bound: Duration,
) -> Result<std::process::ExitStatus, String> {
    let deadline = Instant::now() + bound;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => return Err(format!("did not finish: {e}")),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "did not answer within {} seconds and was stopped",
                bound.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The failure detail: what the helper itself said, and what happened
/// when it said nothing.
///
/// The helper's own sentence stands alone, because it is the one that
/// names the cause and because design D1.3 names the result exactly:
/// "helper 'manager': fatal: Cannot prompt because user interactivity
/// has been disabled." An exit code in front of it would add a number
/// nobody can act on to a sentence that already says everything. The
/// code is not lost: [`run`] logs it beside the detail.
///
/// The FIRST sentence and no more. Git Credential Manager is a .NET
/// application and prints its unhandled exceptions with a full stack
/// trace under the one line that says what went wrong; joining all of
/// them put a hundred frames of `at GitCredentialManager...` into a
/// line a person reads on a banner. The rest is not lost either: the
/// whole stderr goes to the log.
fn quoted(what: &str, stderr: &str) -> String {
    match first_sentence(stderr) {
        Some(said) => said,
        None => what.to_string(),
    }
}

/// The first sentence of a helper's stderr: its first non-empty line, cut
/// at the first full stop that has more text behind it.
fn first_sentence(stderr: &str) -> Option<String> {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let sentence = match line.find(". ") {
        Some(at) => &line[..=at],
        None => line,
    };
    Some(sentence.to_string())
}

fn parse_answer(stdout: &str) -> Answer {
    let mut answer = Answer::default();
    for line in stdout.lines() {
        if line.is_empty() {
            break;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "username" => answer.username = Some(value.to_string()),
            "password" => answer.password = Some(value.to_string()),
            "quit" => answer.quit = git2::Config::parse_bool(value).unwrap_or(false),
            _ => {}
        }
    }
    answer
}

// ---- the chain, the cache and the outcome -----------------------------

struct Remembered {
    request: Request,
    credential: Credential,
    /// EVERY helper of the chain as it was resolved for `get`, in
    /// order, kept so that `store` and `erase` reach the same binaries
    /// without looking them up again. Not only the one that answered:
    /// git runs both operations on the whole chain, which is what
    /// fills a `cache` helper standing in front of `manager` and what
    /// erases a revoked entry a second helper still holds (D1.3).
    chain: Vec<(String, Spawn)>,
    /// How `get` ran, so `store` and `erase` reach the same binaries in
    /// the same way.
    manner: Manner,
}

struct State {
    /// Answers, per `<protocol>://<host>[:<port>]`.
    cache: HashMap<String, (Instant, Credential)>,
    /// What was handed to libgit2 last and not yet confirmed, per
    /// host key. Only a FRESH helper answer lands here, so a poll that
    /// reuses the cached credential spawns nothing at all.
    presented: HashMap<String, Remembered>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<T>(body: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| State {
        cache: HashMap::new(),
        presented: HashMap::new(),
    });
    body(state)
}

/// Ask the configured chain for a credential for `url`.
///
/// `Ok(None)` means "nobody answered", which is not a failure: a
/// repository with no helper configured is the normal case. `Err`
/// means a helper ran and said something worth repeating.
pub fn get(
    config: &git2::Config,
    url: &str,
    username: Option<&str>,
    kind: HostKind,
) -> Result<Option<Credential>, HelperFailure> {
    get_from(config, url, username, kind, &Search::of_this_machine())
}

/// [`get`] with the search path handed in, so a test can point the
/// lookup at a directory of its own instead of the machine's.
pub fn get_from(
    config: &git2::Config,
    url: &str,
    username: Option<&str>,
    kind: HostKind,
    search: &Search,
) -> Result<Option<Credential>, HelperFailure> {
    let Some(mut request) = Request::for_url(url) else {
        return Ok(None);
    };
    if request.username.is_none() {
        request.username = username
            .map(str::to_string)
            .or_else(|| configured_username(config, &request));
    }
    let key = request.host_key();
    if let Some(hit) = with_state(|state| match state.cache.get(&key) {
        Some((at, credential)) if at.elapsed() < CACHE_TTL => Some(credential.clone()),
        _ => None,
    }) {
        return Ok(Some(hit));
    }
    let manner = Manner {
        with_path: use_http_path(config, &request),
        kind,
        child_path: search.child_path.clone(),
    };
    // The whole chain is resolved before the first helper runs, so
    // that `store` and `erase` can reach every one of them later, the
    // way git does: a `cache` helper standing in front of `manager`
    // only ever fills up on `store`, and it never sees the credential
    // if the runner stops looking after the helper that answered.
    // A failure keeps the place of its helper in the chain, so the one
    // that is reported is still the first thing that went wrong.
    let values = configured_helpers(config, &request);
    let mut failures: Vec<Option<HelperFailure>> = values.iter().map(|_| None).collect();
    let mut resolved: Vec<(usize, String, Spawn)> = Vec::new();
    for (at, value) in values.into_iter().enumerate() {
        match resolve(&value, search) {
            Ok(spawn) => resolved.push((at, value, spawn)),
            Err(e) => {
                tracing::debug!(helper = %value, detail = %e.detail, "credential helper not resolved");
                failures[at] = Some(e);
            }
        }
    }
    let chain: Vec<(String, Spawn)> = resolved
        .iter()
        .map(|(_, value, spawn)| (value.clone(), spawn.clone()))
        .collect();
    let mut username = request.username.clone();
    let mut password = None;
    for (at, value, spawn) in &resolved {
        let mut quit = false;
        match run(spawn, value, Op::Get, &request, None, &manner) {
            Ok(answer) => {
                if username.is_none() {
                    username = answer.username;
                }
                if password.is_none() {
                    password = answer.password;
                }
                quit = answer.quit;
            }
            Err(e) => {
                tracing::debug!(helper = %value, detail = %e.detail, "credential helper failed");
                failures[*at] = Some(e);
            }
        }
        if username.is_some() && password.is_some() {
            let credential = Credential {
                username: username.clone().unwrap_or_default(),
                password: password.clone().unwrap_or_default(),
                helper: value.clone(),
            };
            with_state(|state| {
                state
                    .cache
                    .insert(key.clone(), (Instant::now(), credential.clone()));
                state.presented.insert(
                    key.clone(),
                    Remembered {
                        request: request.clone(),
                        credential: credential.clone(),
                        chain: chain.clone(),
                        manner: manner.clone(),
                    },
                );
            });
            return Ok(Some(credential));
        }
        if quit {
            // git's own order: the credential is checked for
            // completeness BEFORE `quit` is honoured (credential.c,
            // `credential_fill`), so a helper that answers username,
            // password and quit=1 in one block is believed and its
            // credential is used, not thrown away.
            break;
        }
    }
    match failures.into_iter().flatten().next() {
        Some(failure) => Err(failure),
        None => Ok(None),
    }
}

/// Tell the chain how its credential fared.
///
/// Every helper of the chain hears it, not only the one that answered:
/// git runs `store` and `erase` over the whole configured list
/// (credential.c, `credential_approve` and `credential_reject`). That
/// is what fills a `cache` helper standing in front of `manager`, and
/// it is the only way a revoked entry a second helper still holds is
/// erased instead of replayed on the next contact.
fn tell(remembered: &Remembered, op: Op) {
    for (label, spawn) in &remembered.chain {
        if let Err(e) = run(
            spawn,
            label,
            op,
            &remembered.request,
            Some(&remembered.credential),
            &remembered.manner,
        ) {
            tracing::debug!(helper = %label, op = op.word(), detail = %e.detail, "credential helper did not take the outcome");
        }
    }
}

/// The contact that used the last helper credential for this URL
/// succeeded: the helper is told to `store` it. Called once per fresh
/// answer, never per contact, so a 1 Hz poll spawns nothing.
pub fn accepted(url: &str) {
    let Some(request) = Request::for_url(url) else {
        return;
    };
    let key = request.host_key();
    let Some(remembered) = with_state(|state| state.presented.remove(&key)) else {
        return;
    };
    tell(&remembered, Op::Store);
}

/// The credential was refused: the helper is told to `erase` it and
/// joy forgets it. This is the replay bug git2 cannot fix, because it
/// never runs anything but `get` (cred.rs:395, :415).
pub fn refused(url: &str) {
    let Some(request) = Request::for_url(url) else {
        return;
    };
    let key = request.host_key();
    let Some(remembered) = with_state(|state| {
        state.cache.remove(&key);
        state.presented.remove(&key)
    }) else {
        return;
    };
    tell(&remembered, Op::Erase);
}

/// Forget what is waiting for a verdict for this URL's host, without
/// telling any helper anything.
///
/// A contact calls this before it starts: a fresh answer that the last
/// contact left behind because that contact failed for a reason that
/// had nothing to do with the credential (no route, 500, a timeout)
/// must not be `store`d later because some other contact to the same
/// host succeeded with another credential. The cached answer itself
/// stays, so this costs no helper spawn (design D1.3, D1.7).
pub fn forget_presented(url: &str) {
    let Some(request) = Request::for_url(url) else {
        return;
    };
    let key = request.host_key();
    with_state(|state| state.presented.remove(&key));
}

/// Drop every cached answer. For the tests and for a host that knows
/// its credentials changed.
pub fn forget_all() {
    with_state(|state| {
        state.cache.clear();
        state.presented.clear();
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn config_from(text: &str, dir: &Path) -> git2::Config {
        let path = dir.join("gitconfig");
        std::fs::write(&path, text).unwrap();
        git2::Config::open(&path).unwrap()
    }

    fn request(url: &str) -> Request {
        Request::for_url(url).unwrap()
    }

    #[test]
    fn the_chain_reads_every_value_most_specific_first() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_from(
            "[credential]\n\thelper = global-one\n\thelper = global-two\n\
             [credential \"https://gitea.example.com:8443\"]\n\thelper = host-one\n\
             [credential \"https://gitea.example.com:8443/o/r.git\"]\n\thelper = exact\n",
            dir.path(),
        );
        let chain = configured_helpers(&config, &request("https://gitea.example.com:8443/o/r.git"));
        assert_eq!(chain, vec!["exact", "host-one", "global-one", "global-two"]);
    }

    #[test]
    fn the_port_is_part_of_the_key_unlike_git2() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_from(
            "[credential \"https://gitea.example.com:8443\"]\n\thelper = with-port\n\
             [credential \"https://gitea.example.com\"]\n\thelper = without-port\n",
            dir.path(),
        );
        let chain = configured_helpers(&config, &request("https://gitea.example.com:8443/o/r.git"));
        assert_eq!(chain, vec!["with-port"]);
    }

    #[test]
    fn an_empty_value_resets_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_from(
            "[credential]\n\thelper = never-runs\n\thelper =\n\thelper = the-only-one\n",
            dir.path(),
        );
        let chain = configured_helpers(&config, &request("https://github.com/o/r.git"));
        assert_eq!(chain, vec!["the-only-one"]);
    }

    #[test]
    fn use_http_path_and_the_configured_username_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_from(
            "[credential]\n\tuseHttpPath = false\n\tusername = everyone\n             [credential \"https://gitea.example.com\"]\n\tuseHttpPath = true\n\tusername = deploy\n",
            dir.path(),
        );
        let req = request("https://gitea.example.com/o/r.git");
        // The host's own setting wins over the global one.
        assert!(use_http_path(&config, &req));
        assert_eq!(
            configured_username(&config, &req).as_deref(),
            Some("deploy")
        );
        let elsewhere = request("https://github.com/o/r.git");
        assert!(!use_http_path(&config, &elsewhere));
        assert_eq!(
            configured_username(&config, &elsewhere).as_deref(),
            Some("everyone")
        );
    }

    #[test]
    fn the_request_lines_are_gits_own() {
        let req = request("https://gitea.example.com:8443/owner/repo.git");
        assert_eq!(
            req.lines(false, None),
            "protocol=https\nhost=gitea.example.com:8443\n\n"
        );
        assert_eq!(
            req.lines(true, None),
            "protocol=https\nhost=gitea.example.com:8443\npath=owner/repo.git\n\n"
        );
        let with_credential = req.lines(
            false,
            Some(&Credential {
                username: "u".into(),
                password: "p".into(),
                helper: "h".into(),
            }),
        );
        assert_eq!(
            with_credential,
            "protocol=https\nhost=gitea.example.com:8443\nusername=u\npassword=p\n\n"
        );
    }

    #[test]
    fn an_ip_literal_host_is_written_out() {
        let req = request("https://10.0.0.7/o/r.git");
        assert!(req.lines(false, None).contains("host=10.0.0.7\n"));
    }

    #[test]
    fn an_ssh_remote_has_no_helper_request_at_all() {
        assert!(Request::for_url("git@github.com:o/r.git").is_none());
        assert!(Request::for_url("ssh://git@github.com/o/r.git").is_none());
    }

    #[test]
    fn ghs_single_quoted_windows_path_is_split_not_shelled() {
        let value = "!'C:\\Program Files\\GitHub CLI\\gh.exe' auth git-credential";
        assert!(!needs_shell(value.strip_prefix('!').unwrap()));
        let search = Search::default();
        let spawn = resolve(value, &search).unwrap();
        assert_eq!(
            spawn,
            Spawn::Direct {
                program: PathBuf::from("C:\\Program Files\\GitHub CLI\\gh.exe"),
                args: vec!["auth".to_string(), "git-credential".to_string()],
            }
        );
    }

    #[test]
    fn a_value_that_really_needs_a_shell_gets_one_by_absolute_path() {
        let search = Search {
            shell: Some(PathBuf::from("/bin/sh")),
            ..Search::default()
        };
        let spawn = resolve("!f() { echo password=x; }; f", &search).unwrap();
        match spawn {
            Spawn::Shell { shell, command } => {
                assert_eq!(shell, PathBuf::from("/bin/sh"));
                assert_eq!(command, "f() { echo password=x; }; f");
            }
            other => panic!("expected a shell spawn, got {other:?}"),
        }
    }

    #[test]
    fn a_short_name_becomes_the_binarys_own_name_never_a_git_process() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join(if cfg!(windows) {
            "git-credential-manager.exe"
        } else {
            "git-credential-manager"
        });
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
        let search = Search {
            dirs: vec![dir.path().to_path_buf()],
            ..Search::default()
        };
        let spawn = resolve("manager", &search).unwrap();
        assert_eq!(
            spawn,
            Spawn::Direct {
                program: binary,
                args: Vec::new()
            }
        );
    }

    #[test]
    fn a_short_name_with_no_binary_is_named_and_never_guessed() {
        let search = Search {
            dirs: vec![PathBuf::from("/nowhere/at/all")],
            ..Search::default()
        };
        let failure = resolve("manager", &search).unwrap_err();
        assert!(
            failure.detail.contains("git-credential-manager"),
            "{failure}"
        );
        assert!(!failure.detail.contains("git credential-"), "{failure}");
    }

    #[test]
    fn an_absolute_path_is_used_as_it_stands() {
        let search = Search::default();
        let spawn = resolve("/usr/local/bin/my-helper", &search).unwrap();
        assert_eq!(
            spawn,
            Spawn::Direct {
                program: PathBuf::from("/usr/local/bin/my-helper"),
                args: Vec::new()
            }
        );
    }

    #[test]
    fn argv_splitting_keeps_quoted_spaces_together() {
        assert_eq!(
            split_argv("'/a b/c' one 'two three'").unwrap(),
            vec!["/a b/c", "one", "two three"]
        );
        assert_eq!(
            split_argv("\"C:\\Program Files\\x.exe\" get").unwrap(),
            vec!["C:\\Program Files\\x.exe", "get"]
        );
        assert_eq!(
            split_argv("/opt/my\\ helper get").unwrap(),
            vec!["/opt/my helper", "get"]
        );
        assert_eq!(split_argv("'unterminated"), None);
    }

    // ---- the fake helpers -------------------------------------------
    //
    // These run a real child process, so the script they run is written
    // in the interpreter of the machine that runs the test: a `sh`
    // script on unix, a `.cmd` batch file on Windows (`windows_leg`
    // below). Both legs run in CI, because the blocker this module was
    // rewritten for - a helper whose own bootstrap dies because the
    // child's PATH names no git - can only fail on Windows
    // (JOY-02A7-A2).

    /// The cache, the "presented" record and the fake scripts are
    /// process-wide, so the tests that run a helper run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// A freshly written script can answer ETXTBSY when another thread
    /// of this process happens to be between `fork` and `exec` while
    /// the write handle is still open. That is a property of running
    /// tests in threads, not of the runner, so the test retries.
    #[cfg(unix)]
    fn get_retrying(
        config: &git2::Config,
        url: &str,
        kind: HostKind,
        search: &Search,
    ) -> Result<Option<Credential>, HelperFailure> {
        for _ in 0..10 {
            match get_from(config, url, None, kind, search) {
                Err(e) if e.detail.contains("Text file busy") => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                other => return other,
            }
        }
        get_from(config, url, None, kind, search)
    }

    #[cfg(unix)]
    fn fake_helper(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_is_asked_in_gits_own_protocol_and_answers_a_credential() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_all();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("asked");
        fake_helper(
            dir.path(),
            "git-credential-fake",
            &format!(
                "echo \"op=$1\" >> {log}\ncat >> {log}\necho username=x-access-token\necho password=s3cret",
                log = log.display()
            ),
        );
        // A git binary that would shout if joy ever spawned one. joy
        // spawns the helper by its own name, so this file stays absent.
        fake_helper(
            dir.path(),
            "git",
            &format!("touch {}", dir.path().join("git-was-run").display()),
        );
        let config = config_from("[credential]\n\thelper = fake\n", dir.path());
        let search = Search {
            dirs: vec![dir.path().to_path_buf()],
            ..Search::default()
        };
        let credential = get_retrying(
            &config,
            "https://ghes.internal.example/o/r.git",
            HostKind::Background,
            &search,
        )
        .unwrap()
        .expect("a credential");
        assert_eq!(credential.username, "x-access-token");
        assert_eq!(credential.password, "s3cret");
        let asked = std::fs::read_to_string(&log).unwrap();
        assert!(asked.contains("op=get"), "{asked}");
        assert!(asked.contains("protocol=https"), "{asked}");
        assert!(asked.contains("host=ghes.internal.example"), "{asked}");
        assert!(
            !dir.path().join("git-was-run").exists(),
            "joy spawned a git process"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_quiet_host_disarms_the_helpers_own_prompts_and_an_interactive_one_does_not() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for (kind, expected) in [
            (HostKind::Background, "never 0 0"),
            (HostKind::Delegated, "never 0 0"),
            (HostKind::Interactive, "unset unset unset"),
        ] {
            forget_all();
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("env");
            fake_helper(
                dir.path(),
                "git-credential-fake",
                &format!(
                    "echo \"${{GCM_INTERACTIVE:-unset}} ${{GCM_GUI_PROMPT:-unset}} ${{GIT_TERMINAL_PROMPT:-unset}}\" > {log}\necho password=p\necho username=u",
                    log = log.display()
                ),
            );
            let config = config_from("[credential]\n\thelper = fake\n", dir.path());
            let search = Search {
                dirs: vec![dir.path().to_path_buf()],
                ..Search::default()
            };
            get_retrying(&config, "https://github.com/o/r.git", kind, &search)
                .unwrap()
                .expect("a credential");
            assert_eq!(
                std::fs::read_to_string(&log).unwrap().trim(),
                expected,
                "for {kind}"
            );
        }
        forget_all();
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_that_refuses_is_quoted_with_its_own_sentence() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_all();
        let dir = tempfile::tempdir().unwrap();
        fake_helper(
            dir.path(),
            "git-credential-manager",
            "echo 'fatal: Cannot prompt because user interactivity has been disabled.' >&2\nexit 1",
        );
        let config = config_from("[credential]\n\thelper = manager\n", dir.path());
        let search = Search {
            dirs: vec![dir.path().to_path_buf()],
            ..Search::default()
        };
        let failure = get_retrying(
            &config,
            "https://github.com/o/r.git",
            HostKind::Background,
            &search,
        )
        .unwrap_err();
        assert_eq!(
            failure.to_string(),
            "helper 'manager': fatal: Cannot prompt because user interactivity has been disabled."
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_accepted_credential_is_stored_and_a_refused_one_is_erased() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for (outcome, expected) in [("accepted", "store"), ("refused", "erase")] {
            forget_all();
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("ops");
            fake_helper(
                dir.path(),
                "git-credential-fake",
                &format!(
                    "echo \"$1\" >> {log}\nif [ \"$1\" = get ]; then echo username=u; echo password=p; fi",
                    log = log.display()
                ),
            );
            let config = config_from("[credential]\n\thelper = fake\n", dir.path());
            let search = Search {
                dirs: vec![dir.path().to_path_buf()],
                ..Search::default()
            };
            let url = "https://codeberg.org/o/r.git";
            get_retrying(&config, url, HostKind::Background, &search)
                .unwrap()
                .expect("a credential");
            match outcome {
                "accepted" => accepted(url),
                _ => refused(url),
            }
            let ops = std::fs::read_to_string(&log).unwrap();
            assert_eq!(
                ops.split_whitespace().collect::<Vec<_>>(),
                vec!["get", expected],
                "for {outcome}"
            );
        }
        forget_all();
    }

    #[cfg(unix)]
    #[test]
    fn a_second_ask_inside_the_ttl_spawns_nothing() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_all();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("spawns");
        fake_helper(
            dir.path(),
            "git-credential-fake",
            &format!(
                "if [ \"$1\" = get ]; then echo . >> {log}; fi\necho username=u\necho password=p",
                log = log.display()
            ),
        );
        let config = config_from("[credential]\n\thelper = fake\n", dir.path());
        let search = Search {
            dirs: vec![dir.path().to_path_buf()],
            ..Search::default()
        };
        let url = "https://gitlab.com/o/r.git";
        for _ in 0..3 {
            get_retrying(&config, url, HostKind::Background, &search)
                .unwrap()
                .expect("a credential");
        }
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().lines().count(),
            1,
            "a poll must not spawn a helper per contact"
        );
        // A refusal drops the cached answer, so the next ask asks again.
        refused(url);
        get_retrying(&config, url, HostKind::Background, &search)
            .unwrap()
            .expect("a credential");
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);
        forget_all();
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_shaped_value_runs_under_the_shell_joy_names() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_all();
        let dir = tempfile::tempdir().unwrap();
        let config = config_from(
            "[credential]\n\thelper = \"!f() { echo username=u; echo password=$((1+1)); }; f\"\n",
            dir.path(),
        );
        let search = Search {
            shell: Some(PathBuf::from("/bin/sh")),
            ..Search::default()
        };
        let credential = get_retrying(
            &config,
            "https://github.com/o/r.git",
            HostKind::Background,
            &search,
        )
        .unwrap()
        .expect("a credential");
        assert_eq!(credential.password, "2");
        forget_all();
    }

    /// Every host kind has a bound, the interactive one included. The
    /// call sits inside libgit2's credentials callback, so a dialog a
    /// person never answers would otherwise hang the whole fetch with
    /// no way out (design D1.3, D1.10). The bound under test is a
    /// short one; what is pinned is that `wait_bounded` takes a
    /// duration and not an option, so there is no "wait for ever"
    /// branch left to fall into.
    #[cfg(unix)]
    #[test]
    fn a_helper_that_never_answers_is_stopped_whoever_is_watching() {
        assert!(
            PROMPT_DEADLINE > QUIET_DEADLINE,
            "a person typing is not a hang, but it is not for ever either"
        );
        let mut child = joy_process::command("/bin/sh")
            .args(["-c", "sleep 30"])
            .stdout(Stdio::null())
            .spawn()
            .expect("a sleeping child");
        let started = Instant::now();
        let detail = wait_bounded(&mut child, Duration::from_millis(150))
            .expect_err("a child that never answers is stopped");
        assert!(detail.contains("was stopped"), "{detail}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// The first sentence of a helper's stderr and nothing behind it.
    ///
    /// Git Credential Manager is a .NET application: an unhandled
    /// exception arrives as one sentence followed by a stack trace, and
    /// the detail line a person reads is the sentence (design D1.3).
    #[test]
    fn a_helpers_stack_trace_stays_out_of_the_detail_line() {
        let gcm = "fatal: Cannot prompt because user interactivity has been disabled.\n\
             Unhandled exception. System.Exception: Failed to locate git.exe executable on the path\n\
                at GitCredentialManager.Application.Execute()\n\
                at GitCredentialManager.Program.Main(String[] args)\n";
        assert_eq!(
            quoted("exit 1", gcm),
            "fatal: Cannot prompt because user interactivity has been disabled."
        );
        // A line that runs two sentences together keeps the first.
        assert_eq!(
            quoted("exit 1", "error: no such host. try again later.\n"),
            "error: no such host."
        );
        // A helper that said nothing keeps what happened to it.
        assert_eq!(quoted("exit 128", "  \n\n"), "exit 128");
    }

    // ---- the Windows leg --------------------------------------------

    #[cfg(windows)]
    mod windows_leg {
        use super::*;

        /// A `.cmd` helper, which is what a helper installed beside Git
        /// for Windows can be and what `named_helper` already probes
        /// for. `std::process::Command` runs a batch file through
        /// `cmd.exe` itself, so joy spawns the helper by its own name
        /// here too and never builds a command line for a shell.
        fn cmd_helper(dir: &Path, name: &str, body: &str) -> PathBuf {
            let path = dir.join(format!("{name}.cmd"));
            std::fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
            path
        }

        /// The protocol exchange, on Windows, against a real child.
        #[test]
        fn a_cmd_helper_is_asked_in_gits_own_protocol_and_answers() {
            let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            forget_all();
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("asked");
            cmd_helper(
                dir.path(),
                "git-credential-fake",
                &format!(
                    "echo op=%1>>\"{log}\"\r\nmore >>\"{log}\"\r\necho username=x-access-token\r\necho password=s3cret",
                    log = log.display()
                ),
            );
            let config = config_from("[credential]\n\thelper = fake\n", dir.path());
            let search = Search {
                dirs: vec![dir.path().to_path_buf()],
                ..Search::default()
            };
            let credential = get_from(
                &config,
                "https://ghes.internal.example/o/r.git",
                None,
                HostKind::Background,
                &search,
            )
            .unwrap()
            .expect("a credential");
            assert_eq!(credential.username, "x-access-token");
            assert_eq!(credential.password, "s3cret");
            let asked = std::fs::read_to_string(&log).unwrap();
            assert!(asked.contains("op=get"), "{asked}");
            assert!(asked.contains("protocol=https"), "{asked}");
            assert!(asked.contains("host=ghes.internal.example"), "{asked}");
            forget_all();
        }

        /// THE Windows blocker (JOY-02A7-A2 finding 1): Git Credential
        /// Manager's own bootstrap looks for `git.exe` on the PATH of
        /// the process joy spawns, and dies with "Failed to locate
        /// git.exe executable on the path" when the inherited PATH has
        /// none. joy knows where Git for Windows keeps it and puts that
        /// directory in front of the child's PATH.
        #[test]
        fn the_childs_path_carries_the_directory_git_lives_in() {
            let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            forget_all();
            let dir = tempfile::tempdir().unwrap();
            let git_dir = dir.path().join("cmd");
            std::fs::create_dir_all(&git_dir).unwrap();
            let log = dir.path().join("path");
            cmd_helper(
                dir.path(),
                "git-credential-manager",
                &format!(
                    "echo %PATH%>\"{log}\"\r\necho username=u\r\necho password=p",
                    log = log.display()
                ),
            );
            let config = config_from("[credential]\n\thelper = manager\n", dir.path());
            let search = Search {
                dirs: vec![dir.path().to_path_buf()],
                child_path: vec![git_dir.clone()],
                ..Search::default()
            };
            get_from(
                &config,
                "https://github.com/o/r.git",
                None,
                HostKind::Background,
                &search,
            )
            .unwrap()
            .expect("a credential");
            let seen = std::fs::read_to_string(&log).unwrap();
            assert!(
                seen.to_lowercase()
                    .starts_with(&git_dir.display().to_string().to_lowercase()),
                "the helper's own PATH must start with the directory git lives in: {seen}"
            );
            forget_all();
        }

        /// And the machine's own search names that directory when Git
        /// for Windows is installed: every entry holds `git.exe`, and a
        /// machine without Git for Windows gets an empty list rather
        /// than a guess.
        #[test]
        fn the_machines_own_child_path_only_names_directories_with_git() {
            let search = Search::of_this_machine();
            for dir in &search.child_path {
                assert!(
                    dir.join("git.exe").is_file(),
                    "{} was named without git.exe in it",
                    dir.display()
                );
            }
        }
    }

    #[test]
    fn an_answer_is_read_up_to_the_blank_line() {
        let answer = parse_answer("username=u\npassword=p\n\nquit=1\n");
        assert_eq!(answer.username.as_deref(), Some("u"));
        assert_eq!(answer.password.as_deref(), Some("p"));
        assert!(!answer.quit);
        assert!(parse_answer("quit=1\n").quit);
    }
}
