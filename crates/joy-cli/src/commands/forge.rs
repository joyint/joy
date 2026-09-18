// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! `joy forge`: the CLI's door to a forge (design D3.10, package J10).
//!
//! Until now a person at a terminal had no way to sign joy in to a
//! forge: every sentence that said "sign in" pointed at a foreign CLI
//! or at `joy forge setup`, which never existed. This group is that
//! door, and it is one door with two sources behind it:
//!
//! - `joy forge login` runs the connector's own `login` verb through
//!   the streaming runner and prints the verification URL and code on
//!   stderr while the connector polls. joy never opens a browser.
//! - `joy forge login --token-stdin` reads ONE line from stdin and
//!   hands it to `token-store`, which validates it before storing. The
//!   token is never an argument, so no process list can carry it; there
//!   is no `--token <value>` and there will not be one.
//!
//! `status`, `logout` and `plugins` are the three answers a person
//! needs beside it: who am I here, remove it, and which binary
//! answered.
//!
//! Output follows what the CLI already does (ADR-036): the answer goes
//! to stdout, every progress line and diagnostic to stderr, and with
//! `--json` stdout carries exactly one envelope `{"version":1,"data":
//! {...}}`. Exit codes: 0 on success, 1 on every forge failure, and the
//! `state` word carries which failure it was. 2 stays clap's.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use joy_core::forge_plugins::{
    self, interactive, Access, CallContext, CallerFacts, CancelToken, ForgePluginSpec, ForgeToken,
    FoundIn, PluginError, ResolvedPlugin, Target, COMBINED_BINARY, PROTOCOL,
};
use joy_core::host::HostKind;
use joy_core::vcs::contact::{host_of, Failure};

use crate::output;

#[derive(Args)]
pub struct ForgeArgs {
    #[command(subcommand)]
    command: ForgeCommand,
}

#[derive(Subcommand)]
enum ForgeCommand {
    /// Sign in to a forge on this machine
    Login(LoginArgs),
    /// Show which forge hosts this machine is signed in to
    Status(StatusArgs),
    /// Remove the credential this machine holds for a forge
    Logout(LogoutArgs),
    /// Show which connector binary answers for which forge
    Plugins,
}

#[derive(Args)]
struct LoginArgs {
    /// Forge host to sign in to (default: the host of this project's remote)
    #[arg(long, value_name = "HOST")]
    host: Option<String>,

    /// Read one token from stdin instead of running the forge's sign in flow
    #[arg(long = "token-stdin")]
    token_stdin: bool,

    /// What the credential is for
    #[arg(long = "for", value_name = "ACCESS", value_enum)]
    access: Option<AccessArg>,

    /// Sign in as this forge login
    #[arg(long, value_name = "NAME")]
    login: Option<String>,
}

#[derive(Args)]
struct StatusArgs {
    /// Show this host only
    #[arg(long, value_name = "HOST")]
    host: Option<String>,
}

#[derive(Args)]
struct LogoutArgs {
    /// Forge host to sign out of
    #[arg(long, value_name = "HOST", conflicts_with = "all")]
    host: Option<String>,

    /// Sign out of every host this machine holds a credential for
    #[arg(long)]
    all: bool,

    /// Sign out this forge login
    #[arg(long, value_name = "NAME")]
    login: Option<String>,
}

/// The word list of `--for`, which is the connector's own (D2.7a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AccessArg {
    Read,
    Write,
    Create,
    Release,
}

impl From<AccessArg> for Access {
    fn from(value: AccessArg) -> Self {
        match value {
            AccessArg::Read => Access::Read,
            AccessArg::Write => Access::Write,
            AccessArg::Create => Access::Create,
            AccessArg::Release => Access::Release,
        }
    }
}

pub fn run(args: ForgeArgs) -> Result<()> {
    match args.command {
        ForgeCommand::Login(args) => login(args),
        ForgeCommand::Status(args) => status(args),
        ForgeCommand::Logout(args) => logout(args),
        ForgeCommand::Plugins => plugins(),
    }
}

// ---------------------------------------------------------------------
// The sentences that point at this door (D3.10)
// ---------------------------------------------------------------------

/// The one sentence every "sign in to the forge" text in this CLI ends
/// with. It names a host, because `joy forge login` without one only
/// works inside a project whose remote a connector claims.
pub fn sign_in_line(host: &str) -> String {
    let host = host.trim();
    if host.is_empty() {
        "run `joy forge login --host <host>`".to_string()
    } else {
        format!("run `joy forge login --host {host}`")
    }
}

/// What to do about a failed contact, in the CLI's own words: the state
/// decides, and the three states this door answers name this door
/// (D3.10, D3.8). Every other state keeps the classifier's own next
/// step, so the vocabulary stays the classifier's and not a second one.
pub fn action_line(failure: Failure, host: &str) -> Option<String> {
    match failure {
        Failure::NeedsSignIn | Failure::ScopeMissing | Failure::NeedsSso => {
            Some(sign_in_line(host))
        }
        Failure::PluginMissing | Failure::PluginOutdated => {
            Some("run `joy forge plugins` to see which binary answered".to_string())
        }
        // `Failure::next_step` is deliberately NOT read here: it is the
        // BUTTON of D4.7 and says "retry" or "show the fingerprint",
        // which is a label on a surface that has buttons and nonsense
        // after "= help:". The prose seam is `guidance`, and a state
        // that has none gets no help line rather than a wrong one.
        other => other.guidance().map(|guidance| guidance.to_string()),
    }
}

/// The classifier state behind a `state` word this command prints, where
/// the word names a failed contact at all. `unsupported`, `cancelled`,
/// `busy` and `error` are not contact states and have no next step this
/// command can name: mapping them onto `needs_sign_in` told every one of
/// them to run the command that had just refused.
fn failure_of_state(state: &str) -> Option<Failure> {
    match state {
        // a credential that is spent is a credential to renew, and the
        // door that renews it is this one
        "needs_sign_in" | "expired" => Some(Failure::NeedsSignIn),
        "scope_missing" => Some(Failure::ScopeMissing),
        "plugin_missing" => Some(Failure::PluginMissing),
        "plugin_outdated" => Some(Failure::PluginOutdated),
        "offline" => Some(Failure::Offline),
        "rate_limited" => Some(Failure::RateLimited),
        "denied" => Some(Failure::Denied),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Which connector, for which host
// ---------------------------------------------------------------------

/// The host and the connector one command works on.
struct Door {
    spec: &'static ForgePluginSpec,
    target: Target,
    host: String,
    ctx: CallContext,
}

/// The project this command runs in, when it runs in one. A forge
/// command needs no project: `--host` works on a bare machine, which is
/// the rootless invocation of D2.3.
fn project_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    joy_core::store::find_project_root(&cwd)
}

fn context(login: Option<&str>) -> CallContext {
    let ctx = match project_root() {
        Some(root) => CallContext::in_project(root),
        None => CallContext::rootless(),
    };
    match login {
        Some(login) => ctx.with_facts(CallerFacts {
            login: Some(login.to_string()),
            ..CallerFacts::default()
        }),
        None => ctx,
    }
}

/// A forge command that could not do what it was asked, in the ONE
/// vocabulary of D3.8: a state word an agent reads, a sentence a person
/// reads, and the host both of them are about.
struct Refusal {
    host: String,
    state: String,
    message: String,
    /// The ONE next step, or `None` when this state has none that this
    /// command can name. A refusal whose own message already carries a
    /// help line gets `None` through [`Refusal::speaks_for_itself`]:
    /// "run the command that just failed" under a message that says why
    /// it cannot work contradicts the sentence above it.
    action: Option<String>,
}

impl Refusal {
    fn new(host: &str, state: &str, message: impl Into<String>) -> Refusal {
        let action = failure_of_state(state).and_then(|failure| action_line(failure, host));
        Refusal {
            host: host.to_string(),
            state: state.to_string(),
            message: message.into(),
            action,
        }
    }

    /// A refusal whose message is its own instruction: no second help
    /// line is added under it.
    fn speaks_for_itself(mut self) -> Refusal {
        self.action = None;
        self
    }

    /// The refusal a connector call produced: the connector's own
    /// sentence, under the state joy-core named for it.
    fn of(host: &str, error: PluginError) -> Refusal {
        Refusal::new(host, error.state(), error.to_string())
    }
}

/// The registry row that claims this target, with the reason when no
/// row could be asked at all.
fn responsible(
    host: &str,
    target: &Target,
    ctx: &CallContext,
) -> Result<&'static ForgePluginSpec, Refusal> {
    let mut first_failure: Option<PluginError> = None;
    for spec in forge_plugins::FORGE_PLUGINS {
        match forge_plugins::claims_full(spec, target, ctx) {
            Ok(true) => return Ok(spec),
            Ok(false) => {}
            Err(error) => {
                if first_failure.is_none() {
                    first_failure = Some(error);
                }
            }
        }
    }
    match first_failure {
        Some(error) => Err(Refusal::of(host, error)),
        None => {
            let what = match target {
                Target::Host(host) => format!("host {host}"),
                Target::Remote(url) => format!("remote {url}"),
            };
            Err(Refusal::new(
                host,
                "unsupported",
                format!(
                    "no forge connector claims the {what}\n  \
                     = help: add the instance to forges.yaml, or name a host a connector knows"
                ),
            ))
        }
    }
}

/// The remotes of this project, the one joy really CONTACTS first:
/// `origin`, or the first configured one, which is the selection D1.1
/// fixes for the whole engine (`origin_or_first`). A checkout whose
/// first configured remote is not `origin` was told to sign in to a
/// host it never pushes to, while the throttle key, the twin source and
/// the contact itself used the other one.
fn remote_urls(root: &Path) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    if let Some(url) = joy_core::vcs::forge::remote_url(root) {
        urls.push(url);
    }
    for (_, url) in joy_core::vcs::default_vcs()
        .all_remotes(root)
        .unwrap_or_default()
    {
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

/// The door one login, logout or status row works through: the target
/// the verbs take, the host its sentences name, and the connector
/// responsible for it.
fn door(host: Option<&str>, login: Option<&str>) -> Result<Door, Refusal> {
    let ctx = context(login);
    let target = match host {
        Some(host) => Target::host(host.trim().to_ascii_lowercase()),
        None => {
            let root = project_root().ok_or_else(|| {
                Refusal::new(
                    "",
                    "error",
                    "no host given and this directory is not a Joy project\n  \
                     = help: pass --host <host>",
                )
            })?;
            let remotes = remote_urls(&root);
            if remotes.is_empty() {
                return Err(Refusal::new(
                    "",
                    "error",
                    "this project has no git remote, so joy cannot tell which forge you mean\n  \
                     = help: pass --host <host>",
                ));
            }
            // The plugin's `claims` decides whose remote this is
            // (D3.10): joy never parses a forge URL itself.
            let claimed = remotes.iter().find(|url| {
                forge_plugins::FORGE_PLUGINS
                    .iter()
                    .any(|spec| forge_plugins::claims(spec, &Target::remote(url.as_str()), &ctx))
            });
            match claimed {
                Some(url) => Target::remote(url.clone()),
                None => {
                    return Err(Refusal::new(
                        &host_of(&remotes[0]),
                        "unsupported",
                        "no forge connector claims a remote of this project\n  \
                         = help: pass --host <host>, or run `joy forge plugins` to see which \
                         binary answered",
                    ))
                }
            }
        }
    };
    let host = match &target {
        Target::Host(host) => host.clone(),
        Target::Remote(url) => host_of(url),
    };
    let spec = responsible(&host, &target, &ctx)?;
    Ok(Door {
        spec,
        target,
        host,
        ctx,
    })
}

// ---------------------------------------------------------------------
// login
// ---------------------------------------------------------------------

/// The answer of a finished `joy forge login` (D3.10).
#[derive(Serialize)]
struct LoginPayload {
    host: String,
    state: &'static str,
    login: Option<String>,
    user_id: Option<String>,
    emails: Vec<String>,
    /// How the credential was obtained, as the connector reported it;
    /// `token` for the stdin path. `null` when the connector's `result`
    /// event named none: which grant a forge runs is the connector's
    /// knowledge, and joy does not guess it.
    source: Option<String>,
    stored: Option<String>,
    scopes: Option<String>,
    expires_at: Option<String>,
}

/// The answer of a `joy forge login` that did not sign anybody in.
#[derive(Serialize)]
struct LoginFailurePayload {
    host: String,
    state: String,
    message: String,
    action: String,
}

fn login(args: LoginArgs) -> Result<()> {
    let door = match door(args.host.as_deref(), args.login.as_deref()) {
        Ok(door) => door,
        Err(refusal) => return refused(refusal),
    };
    let resolved = match forge_plugins::resolve_plugin(door.spec) {
        Ok(resolved) => resolved,
        Err(error) => return refused(Refusal::of(&door.host, error)),
    };
    // The token paste is the HEADLESS door (D2.4): a CI runner, a
    // Linux server and a Windows host with no browser are exactly the
    // machines it was written for, so it is not what the refusal below
    // is about. It reads one line from stdin and asks nobody anything.
    if args.token_stdin {
        return login_with_token(&door, &resolved);
    }
    // Layer 3 of D3.11, after the claims call that named the connector
    // and before the `login` verb is started: a host with nobody at it
    // cannot type a verification code, and the refusal is instant
    // rather than a fifteen minute wait for one. joy-core refuses the
    // same call for itself; this refusal exists so the sentence reaches
    // the caller without the login spawn, and so that it can be true: a
    // hook and a piped run are not delegation sessions, and telling
    // them they are sends a person looking for an agent that does not
    // exist. Both sentences name the headless door, because refusing
    // without one would leave a CI runner with nothing to do.
    match door.ctx.host_kind {
        HostKind::Delegated => {
            return refused(
                Refusal::new(&door.host, "needs_sign_in", interactive::NO_PERSON_HERE)
                    .speaks_for_itself(),
            )
        }
        HostKind::Background => {
            return refused(
                Refusal::new(
                    &door.host,
                    "needs_sign_in",
                    "joy forge login needs a person at this machine; this process has no \
                     terminal to ask at. Run it in a terminal, or store a token with joy forge \
                     login --token-stdin",
                )
                .speaks_for_itself(),
            )
        }
        HostKind::Interactive => {}
    }
    let access: Access = args.access.map(Access::from).unwrap_or_default();
    let mut progress = LoginProgress::default();
    let cancel = CancelToken::new();
    let outcome = interactive::login(
        &resolved,
        &door.target,
        access,
        &mut progress,
        &cancel,
        &door.ctx,
    );
    // Whatever the wait left standing on the terminal goes before
    // anything is printed over it: a refusal written into "Still
    // waiting, 13 minutes left." reads as both at once (JOY-02A9-48).
    progress.clear();
    match outcome {
        Ok(result) if result.known => signed_in(
            &door.host,
            LoginPayload {
                host: door.host.clone(),
                state: "signed-in",
                login: result.login,
                user_id: result.user_id,
                emails: result.emails,
                source: result.source,
                stored: result.stored,
                scopes: result.scopes,
                expires_at: result.expires_at,
            },
        ),
        // A `result` event that says `known: false` is an answer, not a
        // failure of the call (D2.3).
        Ok(_) => refused(Refusal::new(
            &door.host,
            "needs_sign_in",
            "the connector finished without a credential",
        )),
        Err(error) => {
            // The connector's own `error` event carries the reason; the
            // sink kept it, because the runner's error keeps the
            // message alone.
            let (state, message) = match progress.failure.take() {
                Some((code, message)) => (state_of_code(&code).to_string(), message),
                // Nothing was refused: joy itself stopped the call, so
                // the sentence says which sign in it stopped and how
                // much of the code was left (D3.10: one sentence, one
                // next step).
                None if error.state() == "plugin_timed_out" => {
                    ("expired".to_string(), progress.stopped_sentence(&door.host))
                }
                None => (error.state().to_string(), error.to_string()),
            };
            refused(Refusal::new(&door.host, &state, message))
        }
    }
}

/// The token paste of D3.10: one line from stdin, never an argument.
fn login_with_token(door: &Door, resolved: &ResolvedPlugin) -> Result<()> {
    let token = match read_token_line(&door.host) {
        Ok(token) => token,
        Err(refusal) => return refused(refusal),
    };
    match interactive::token_store(resolved, &door.target, &token, &door.ctx) {
        Ok(noted) => {
            // Whatever the connector said while it worked reaches the
            // person here, on both paths. It used to be read off the
            // pipe and dropped on a zero exit, which is how a keychain
            // that refused and fell back to a file stayed invisible
            // (JOY-02A8-F4).
            print_note(noted.note.as_deref());
            let answer = noted.answer;
            if answer.known {
                return signed_in(
                    &door.host,
                    LoginPayload {
                        host: answer.host.unwrap_or_else(|| door.host.clone()),
                        state: "signed-in",
                        login: answer.login,
                        user_id: None,
                        emails: Vec::new(),
                        source: Some("token".to_string()),
                        stored: answer.source,
                        scopes: answer.scopes,
                        expires_at: answer.expires_at,
                    },
                );
            }
            let state = answer
                .reason
                .as_deref()
                .map(state_of_code)
                .unwrap_or("needs_sign_in");
            // The fallback stays the refusal sentence, and it is
            // reached only where the connector named no reason at all:
            // a connector that could not REACH the forge names
            // `offline` and its own sentence, and neither is overwritten
            // here (JOY-02A8-F4).
            let message = answer
                .message
                .unwrap_or_else(|| "the forge did not accept this token".to_string());
            refused(Refusal::new(&door.host, state, message))
        }
        Err(error) => refused(Refusal::of(&door.host, error)),
    }
}

/// What the connector said while it answered, on stderr, under the
/// answer it belongs to (D3.10: diagnostics go to stderr, in both
/// output modes, so one JSON envelope stays one JSON envelope).
fn print_note(note: Option<&str>) {
    let Some(note) = note else {
        return;
    };
    for line in note.lines().filter(|line| !line.trim().is_empty()) {
        eprintln!("  = note: {}", line.trim_end());
    }
}

/// Read ONE token from stdin. The trailing CR/LF goes, an empty line is
/// refused, and at a terminal the line is not echoed. The token is
/// never put in an error message.
///
/// Every way out of here is a [`Refusal`], because every answer of this
/// command is exactly one envelope and a caller in `--json` mode reads
/// a `state` (D3.10). An `anyhow` error would leave stdout empty.
fn read_token_line(host: &str) -> Result<String, Refusal> {
    use std::io::BufRead;
    let line = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Paste the token (it is not shown): ")
            .map_err(|error| token_refusal(host, format!("--token-stdin: {error}")))?
    } else {
        let mut line = String::new();
        let read = std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|error| token_refusal(host, format!("--token-stdin: {error}")))?;
        if read == 0 {
            return Err(token_refusal(
                host,
                "--token-stdin: stdin closed before a token was read",
            ));
        }
        line
    };
    let line = line.trim_end_matches('\n').trim_end_matches('\r');
    if line.trim().is_empty() {
        return Err(token_refusal(host, "--token-stdin: empty input"));
    }
    Ok(line.to_string())
}

/// Nothing usable came in on stdin. The state is `error`, because no
/// forge refused anything: the input did not arrive. The help line says
/// how a token gets in, and it is not "run this command again".
fn token_refusal(host: &str, message: impl Into<String>) -> Refusal {
    let message = message.into();
    Refusal::new(
        host,
        "error",
        format!(
            "{message}\n  \
             = help: pipe ONE token line into joy, or run it on a terminal to be asked for it"
        ),
    )
    .speaks_for_itself()
}

/// The states of D3.10, from the `code` of the connector's `error`
/// event and from the `reason` of a `token` answer.
fn state_of_code(code: &str) -> &'static str {
    match code {
        "access_denied" => "denied",
        "expired_token" | "expired" => "expired",
        "cancelled" | "canceled" => "cancelled",
        "unsupported" | "device_flow_disabled" => "unsupported",
        "invalid_scope" | "scope_missing" => "scope_missing",
        "network" | "offline" => "offline",
        "rate_limited" => "rate_limited",
        "no-login" | "no-keychain" | "no-login-for-repo" | "needs_sign_in" => "needs_sign_in",
        // The organisation's own wall (D2.7c): nobody signs their way
        // through it, so it is never `needs_sign_in`.
        "needs_org_approval" => "needs_org_approval",
        "busy" => "busy",
        _ => "error",
    }
}

fn signed_in(host: &str, payload: LoginPayload) -> Result<()> {
    if output::is_json() {
        output::emit(payload)?;
        return Ok(());
    }
    let who = payload.login.as_deref().unwrap_or("this machine");
    println!("Signed in to {host} as {who}.");
    if let Some(scopes) = payload.scopes.as_deref().filter(|s| !s.is_empty()) {
        println!("  access: {scopes}");
    }
    if let Some(stored) = payload.stored.as_deref() {
        println!("  stored in: {stored}");
    }
    if let Some(expires) = payload.expires_at.as_deref() {
        println!("  expires: {expires}");
    }
    Ok(())
}

/// Every state other than `signed-in` ends the command with exit 1
/// (D3.10). In `--json` mode the envelope is printed first and the
/// process then exits with the code, exactly as `joy auth status` does.
fn refused(refusal: Refusal) -> Result<()> {
    let Refusal {
        host,
        state,
        message,
        action,
    } = refusal;
    if output::is_json() {
        output::emit(LoginFailurePayload {
            host,
            state,
            message,
            action: action.unwrap_or_default(),
        })?;
        std::process::exit(1);
    }
    eprintln!("{message}");
    eprintln!("  = note: state {state}");
    if let Some(action) = action {
        eprintln!("  = help: {action}");
    }
    std::process::exit(1);
}

/// What the person sees while the connector polls the forge: the two
/// lines of D3.10 on stderr, and the countdown the CONNECTOR reports.
/// The CLI never opens a browser and never counts the time itself: the
/// seconds it prints are the `seconds_left` of the event it just read,
/// so the countdown, the connector's own deadline and the runner's
/// bound are one number and not three (JOY-02A9-48).
#[derive(Default)]
struct LoginProgress {
    /// The last `error` event, kept because the runner's error carries
    /// the message but not the connector's own code.
    failure: Option<(String, String)>,
    /// The width of the progress line standing on the terminal, which
    /// has to be wiped before anything else is written over it. Without
    /// this the refusal was printed INTO "Still waiting, 13 minutes
    /// left." and the person read both at once.
    standing: usize,
    /// The last countdown the connector reported, for the sentence a
    /// stopped call ends with.
    seconds_left: Option<u64>,
    /// The connector has said its last word (`result` or `error`).
    /// Nothing is printed after it, and the call has only as long as it
    /// takes the process to exit.
    finished: bool,
}

/// How long a connector has to exit after its last event. The loop ends
/// on end of file long before this; the bound exists so that a
/// connector which says `error` and then hangs cannot hold a person's
/// terminal for the rest of the code's life.
const AFTER_THE_LAST_WORD: std::time::Duration = std::time::Duration::from_secs(5);

impl LoginProgress {
    /// Wipe the progress line, if one is standing.
    fn clear(&mut self) {
        if self.standing == 0 {
            return;
        }
        eprint!("\r{:width$}\r", "", width = self.standing);
        let _ = std::io::stderr().flush();
        self.standing = 0;
    }

    /// One progress line, in place of the last one.
    fn say(&mut self, line: &str) {
        if !std::io::stderr().is_terminal() {
            return;
        }
        let pad = self.standing.saturating_sub(line.chars().count());
        eprint!("\r{line}{:pad$}", "", pad = pad);
        let _ = std::io::stderr().flush();
        self.standing = line.chars().count();
    }

    /// The sentence for a sign in joy itself stopped: it names which
    /// sign in it was and how much of the code's life was left, because
    /// "the forge plugin did not answer in time and was stopped" tells
    /// a person who was standing at the forge's page nothing they can
    /// act on. The next step is the help line under it, which is this
    /// command's own door.
    fn stopped_sentence(&self, host: &str) -> String {
        match self.seconds_left {
            Some(left) if left > 0 => format!(
                "joy stopped the sign in to {host} while it was still waiting for you; \
                 the code had {} left.",
                minutes(left)
            ),
            _ => format!("joy stopped the sign in to {host} before it finished."),
        }
    }
}

impl forge_plugins::EventSink for LoginProgress {
    fn event(&mut self, event: &serde_json::Value) -> Option<std::time::Duration> {
        let kind = event.get("event").and_then(|e| e.as_str()).unwrap_or("");
        match kind {
            "verification" => {
                let url = event
                    .get("url_complete")
                    .and_then(|u| u.as_str())
                    .or_else(|| event.get("url").and_then(|u| u.as_str()))
                    .unwrap_or_default();
                let code = event
                    .get("code")
                    .and_then(|c| c.as_str())
                    .unwrap_or_default();
                let seconds = event.get("expires_in").and_then(|e| e.as_u64());
                self.clear();
                eprintln!("Open {url}");
                eprintln!("Enter the code {code}");
                if let Some(seconds) = seconds {
                    self.seconds_left = Some(seconds);
                    eprintln!(
                        "Waiting for you; the code is good for {}.",
                        minutes(seconds)
                    );
                }
                // The forge's own `expires_in` sets the rest of the
                // call's deadline, capped by the runner (D2.3).
                seconds.map(std::time::Duration::from_secs)
            }
            // A wait AFTER the connector's last word is not a wait: the
            // stream is over and a countdown printed under a refusal is
            // noise on top of the sentence that matters.
            "waiting" if !self.finished => {
                if let Some(left) = event.get("seconds_left").and_then(|s| s.as_u64()) {
                    self.seconds_left = Some(left);
                    // The reason of D2.4, where the connector named
                    // one: a poll that is waiting out a name that does
                    // not resolve says so instead of counting silently.
                    let line = match event.get("reason").and_then(|r| r.as_str()) {
                        Some(reason) if !reason.trim().is_empty() => {
                            format!("Still waiting, {} left ({}).", minutes(left), reason.trim())
                        }
                        _ => format!("Still waiting, {} left.", minutes(left)),
                    };
                    self.say(&line);
                }
                None
            }
            "error" => {
                let code = event
                    .get("code")
                    .and_then(|c| c.as_str())
                    .unwrap_or("error")
                    .to_string();
                let message = event
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("the sign in did not finish")
                    .to_string();
                self.failure = Some((code, message));
                self.finished = true;
                self.clear();
                // The connector has spoken; what is left is its exit.
                Some(AFTER_THE_LAST_WORD)
            }
            "result" => {
                self.finished = true;
                self.clear();
                Some(AFTER_THE_LAST_WORD)
            }
            _ => None,
        }
    }
}

/// A duration a person reads: minutes while there are minutes, seconds
/// at the end.
fn minutes(seconds: u64) -> String {
    match seconds {
        1 => "1 second".to_string(),
        0..=90 => format!("{seconds} seconds"),
        _ => format!("{} minutes", seconds / 60),
    }
}

// ---------------------------------------------------------------------
// status
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct StatusPayload {
    hosts: Vec<HostRow>,
    /// What the whole answer says: `signed-in` when one host is,
    /// `expired` when a credential is there but past its date, `none`
    /// when this machine holds none - which is also what an empty
    /// `hosts` means (D3.10). Without it a `--json` caller read
    /// `{"hosts":[]}` and exit 1 and had nothing to act on
    /// (JOY-02A7-A2 finding 8).
    state: &'static str,
    /// The one next step, present exactly when the command exits 1: the
    /// same sentence the human answer prints under `= help`.
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<String>,
}

#[derive(Serialize)]
struct HostRow {
    host: String,
    forge: Option<&'static str>,
    login: Option<String>,
    state: &'static str,
    source: String,
    scopes: Option<String>,
    expires_at: Option<String>,
    plugin: Option<PluginRef>,
}

#[derive(Serialize)]
struct PluginRef {
    id: &'static str,
    binary: String,
    path: PathBuf,
    protocol: u32,
}

fn status(args: StatusArgs) -> Result<()> {
    let ctx = context(None);
    let hosts = host_set(args.host.as_deref(), &ctx);
    let rows: Vec<(HostRow, Option<String>)> =
        hosts.iter().map(|host| host_row(host, &ctx)).collect();
    let signed_in = rows.iter().any(|(row, _)| row.state == "signed-in");
    // The state of the ANSWER, which is the state of the best row: a
    // machine signed in to one host of three is signed in
    // (JOY-02A7-A2 finding 8).
    let state = if signed_in {
        "signed-in"
    } else if rows.iter().any(|(row, _)| row.state == "expired") {
        "expired"
    } else {
        "none"
    };
    // ONE next step for both surfaces, so the envelope's `help` is
    // literally the sentence the human answer prints. A machine that
    // knows no host at all keeps the line it has always had; a machine
    // with rows gets the step the rows themselves decide, which is the
    // configuration and not a login when nothing claims them
    // (JOY-02A8-F4 finding 1).
    let help = if rows.is_empty() {
        sign_in_line("")
    } else {
        status_help(&rows)
    };
    if output::is_json() {
        // The notes are diagnostics and go to stderr in this mode too,
        // so stdout stays exactly one envelope (D3.10).
        for (_, note) in &rows {
            print_note(note.as_deref());
        }
        output::emit(StatusPayload {
            hosts: rows.into_iter().map(|(row, _)| row).collect(),
            state,
            help: (!signed_in).then(|| help.clone()),
        })?;
        if !signed_in {
            eprintln!("  = help: {help}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if rows.is_empty() {
        println!("No forge host is known on this machine.");
        eprintln!("  = help: {help}");
        std::process::exit(1);
    }
    for (row, note) in &rows {
        let login = row.login.as_deref().unwrap_or("-");
        let forge = row.forge.unwrap_or("-");
        println!(
            "{}  {}  {}  {}  {}",
            row.host, forge, login, row.state, row.source
        );
        if let Some(scopes) = row.scopes.as_deref().filter(|s| !s.is_empty()) {
            println!("  access: {scopes}");
        }
        if let Some(expires) = row.expires_at.as_deref() {
            println!("  expires: {expires}");
        }
        match &row.plugin {
            Some(plugin) => println!(
                "  connector: {} (protocol {}) at {}",
                plugin.binary,
                plugin.protocol,
                plugin.path.display()
            ),
            None => println!("  connector: none answered for this host"),
        }
        print_note(note.as_deref());
    }
    if !signed_in {
        eprintln!("  = help: {help}");
        std::process::exit(1);
    }
    Ok(())
}

/// The sentence for a host no connector claims. `joy forge login` is
/// not it: there is no door to knock on until the instance is
/// configured, and sending somebody to a command that will refuse them
/// for the same reason is the wrong next step (JOY-02A8-F4, D2.5).
const NOTHING_CLAIMS_ANY_HOST: &str =
    "add the instance to forges.yaml, or name a host a connector knows";

/// The ONE next step under a `joy forge status` where nothing is signed
/// in (D3.8: one next step, never a list).
///
/// Three cases, because one help line for all of them was wrong in two
/// of them: it named `rows[0].host` whatever that row was, so a machine
/// whose first row was an unclaimed host was sent to sign in to it,
/// and a machine with several rows was told about one of them with no
/// word about the others (JOY-02A8-F4).
///
/// - No row has a connector: the instance is not configured, and the
///   step is `forges.yaml`.
/// - Exactly one row has a connector and is not signed in: that host is
///   named, because there is nothing to choose between.
/// - Several: the host is left out, and the person picks from the rows
///   printed right above.
fn status_help(rows: &[(HostRow, Option<String>)]) -> String {
    let claimed: Vec<&HostRow> = rows
        .iter()
        .map(|(row, _)| row)
        .filter(|row| row.forge.is_some())
        .collect();
    match claimed.as_slice() {
        [] => NOTHING_CLAIMS_ANY_HOST.to_string(),
        [only] => sign_in_line(&only.host),
        _ => sign_in_line(""),
    }
}

/// One row of `joy forge status`: who this machine is on this host, and
/// which binary answered, with whatever the connector said while it
/// answered (JOY-02A8-F4).
fn host_row(host: &str, ctx: &CallContext) -> (HostRow, Option<String>) {
    let target = Target::host(host.to_string());
    let spec = forge_plugins::FORGE_PLUGINS
        .iter()
        .find(|spec| forge_plugins::claims(spec, &target, ctx));
    let Some(spec) = spec else {
        return (
            HostRow {
                host: host.to_string(),
                forge: None,
                login: None,
                state: "none",
                source: "none".to_string(),
                scopes: None,
                expires_at: None,
                plugin: None,
            },
            None,
        );
    };
    let resolved = forge_plugins::resolve_plugin(spec).ok();
    let noted = resolved.as_ref().and_then(|resolved| {
        forge_plugins::query_resolved_noted::<ForgeToken>(
            resolved,
            "token",
            Some(&target),
            &[],
            ctx,
        )
        .ok()
    });
    let (token, note) = match noted {
        Some(noted) => (Some(noted.answer), noted.note),
        None => (None, None),
    };
    let known = token.as_ref().is_some_and(|token| token.known);
    let expires_at = token.as_ref().and_then(|token| token.expires_at.clone());
    let state = match (known, expires_at.as_deref().map(is_past)) {
        (true, Some(true)) => "expired",
        (true, _) => "signed-in",
        (false, _) => "none",
    };
    (
        HostRow {
            host: host.to_string(),
            forge: Some(spec.id),
            login: token.as_ref().and_then(|token| token.login.clone()),
            state,
            source: token
                .as_ref()
                .and_then(|token| token.source.clone())
                .unwrap_or_else(|| "none".to_string()),
            scopes: token.as_ref().and_then(|token| token.scopes.clone()),
            expires_at,
            plugin: resolved.as_ref().map(plugin_ref),
        },
        note,
    )
}

fn plugin_ref(resolved: &ResolvedPlugin) -> PluginRef {
    PluginRef {
        id: resolved.id,
        binary: binary_name(resolved),
        path: resolved.resolved_path.clone(),
        protocol: resolved.protocol,
    }
}

fn binary_name(resolved: &ResolvedPlugin) -> String {
    resolved
        .resolved_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(COMBINED_BINARY)
        .to_string()
}

/// Whether an RFC 3339 instant has passed. An unreadable one has not:
/// joy does not call a credential expired because it could not read a
/// date.
fn is_past(instant: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(instant) {
        Ok(when) => when < chrono::Utc::now(),
        Err(_) => false,
    }
}

/// The host set of D3.10: the hosts of `forges.yaml`, the hosts of this
/// project's remotes, and the hosts joy's own credential file holds. A
/// credential that lives in the operating system's keychain alone
/// cannot be enumerated (`Entry::new` has no listing, D2.6), so such a
/// host appears here through `forges.yaml`, through a remote or through
/// `--host`.
fn host_set(explicit: Option<&str>, ctx: &CallContext) -> Vec<String> {
    if let Some(host) = explicit {
        return vec![host.trim().to_ascii_lowercase()];
    }
    let mut hosts: Vec<String> = Vec::new();
    let mut add = |host: String| {
        let host = host.trim().to_ascii_lowercase();
        if !host.is_empty() && !hosts.contains(&host) {
            hosts.push(host);
        }
    };
    for host in forges_yaml_hosts() {
        add(host);
    }
    if let Some(root) = ctx.root.as_deref() {
        for (_, url) in joy_core::vcs::default_vcs()
            .all_remotes(root)
            .unwrap_or_default()
        {
            add(host_of(&url));
        }
    }
    for host in stored_credential_hosts() {
        add(host);
    }
    hosts
}

/// joy's own configuration directory, the one `forges.yaml` and the
/// fallback credential file live in (D2.5, D2.6).
fn joy_config_dir() -> Option<PathBuf> {
    joy_core::store::global_config_path()
        .parent()
        .map(Path::to_path_buf)
}

/// The hosts an operator configured. Only the `host` key is read here;
/// every other key of D2.5 belongs to the connector.
fn forges_yaml_hosts() -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        #[serde(default)]
        host: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Shape {
        Bare(Vec<Entry>),
        Keyed { forges: Vec<Entry> },
    }
    let Some(file) = joy_config_dir().map(|dir| dir.join("forges.yaml")) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Vec::new();
    };
    match serde_yaml_ng::from_str::<Shape>(&text) {
        Ok(Shape::Bare(entries)) | Ok(Shape::Keyed { forges: entries }) => {
            entries.into_iter().map(|entry| entry.host).collect()
        }
        Err(error) => {
            eprintln!("joy: {} is not readable: {error}", file.display());
            Vec::new()
        }
    }
}

/// The hosts joy's own fallback credential file holds (D2.6). The host
/// NAMES are read and nothing else: the tokens beside them are the
/// connector's business and are never read here.
fn stored_credential_hosts() -> Vec<String> {
    let Some(file) = joy_config_dir().map(|dir| dir.join("forge-tokens.json")) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    value
        .get("hosts")
        .and_then(|hosts| hosts.as_object())
        .map(|hosts| hosts.keys().cloned().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------
// logout
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct LogoutPayload {
    host: String,
    removed: bool,
    revoked: bool,
    source: Option<String>,
    /// The foreign command that removes a foreign credential, because
    /// joy removes none itself (D2.6).
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

#[derive(Serialize)]
struct LogoutAllPayload {
    hosts: Vec<LogoutPayload>,
}

fn logout(args: LogoutArgs) -> Result<()> {
    if !args.all && args.host.is_none() {
        // A refusal and not a `bail!`: in `--json` mode stdout carries
        // exactly one envelope with a `state` an agent reads, and an
        // `anyhow` error would leave it empty (D3.10).
        return refused(
            Refusal::new(
                "",
                "error",
                "joy forge logout needs a host\n  \
                 = help: pass --host <host>, or --all to sign out everywhere",
            )
            .speaks_for_itself(),
        );
    }
    if args.all {
        let ctx = context(args.login.as_deref());
        let hosts = host_set(None, &ctx);
        let mut answers: Vec<LogoutPayload> = Vec::new();
        for host in hosts {
            let door = match door(Some(&host), args.login.as_deref()) {
                Ok(door) => door,
                // A host nothing claims holds no credential of joy's.
                Err(_) => continue,
            };
            match logout_one(&door) {
                Ok(answer) => answers.push(answer),
                Err(refusal) => return refused(refusal),
            }
        }
        if output::is_json() {
            output::emit(LogoutAllPayload { hosts: answers })?;
            return Ok(());
        }
        if answers.is_empty() {
            println!("No forge credential on this machine.");
        }
        for answer in &answers {
            print_logout(answer);
        }
        return Ok(());
    }
    let door = match door(args.host.as_deref(), args.login.as_deref()) {
        Ok(door) => door,
        Err(refusal) => return refused(refusal),
    };
    let answer = match logout_one(&door) {
        Ok(answer) => answer,
        Err(refusal) => return refused(refusal),
    };
    if output::is_json() {
        output::emit(answer)?;
        return Ok(());
    }
    print_logout(&answer);
    Ok(())
}

fn logout_one(door: &Door) -> Result<LogoutPayload, Refusal> {
    let outcome = interactive::logout(door.spec, &door.target, &door.ctx)
        .map_err(|error| Refusal::of(&door.host, error))?;
    if let Some(refusal) = refused_removal(door, &outcome) {
        return Err(refusal);
    }
    if outcome.removed {
        // The credential is gone, so what it achieved on this host goes
        // with it: a 404 after this is a 404 again and not "your
        // organisation must approve Joy" (D1.8b, JOY-02A9-48).
        joy_core::vcs::contact::forget_token_worked(&door.host);
    }
    let command = outcome
        .command
        .clone()
        .or_else(|| foreign_removal(outcome.source.as_deref(), &door.host));
    Ok(LogoutPayload {
        host: door.host.clone(),
        removed: outcome.removed,
        revoked: outcome.revoked,
        source: outcome.source,
        command,
    })
}

/// A `logout` that removed nothing and said WHY is a refusal, not an
/// answer (D3.8). Under a delegation session the connector answers
/// `removed: false, revoked: false` with the sentence of G2: a
/// delegated process may use the credential this machine holds and may
/// never sign it out. `removed: false` alone drops that sentence, so
/// the caller learns nothing and an agent retries a call that can never
/// work. The same holds for the refresh lock (`busy`), for a host with
/// several of joy's own logins, and for a state directory that would
/// not take the delete.
///
/// The state word is the connector's own `reason` through the CLI's
/// one list, so `unsupported` and `busy` read here as they do
/// everywhere else, and a `reason` the connector named none for is
/// `error`. A sentence beside a foreign `command` is NOT this: that
/// answer already names the tool that owns the credential, and
/// [`print_logout`] prints it.
fn refused_removal(door: &Door, outcome: &interactive::LogoutOutcome) -> Option<Refusal> {
    if outcome.removed || outcome.command.is_some() {
        return None;
    }
    let message = outcome.message.as_deref()?.trim();
    if message.is_empty() {
        return None;
    }
    let state = outcome
        .reason
        .as_deref()
        .map(state_of_code)
        .unwrap_or("error");
    // The connector's sentence carries its own next step ("sign out on
    // the machine that owns the session", "try again in a moment",
    // "say which one with --login"), so no second help line goes under
    // it.
    Some(Refusal::new(&door.host, state, message).speaks_for_itself())
}

/// The foreign command that removes a foreign credential (D3.10), when
/// the connector named none itself.
fn foreign_removal(source: Option<&str>, host: &str) -> Option<String> {
    match source? {
        "gh" => Some(format!("gh auth logout --hostname {host}")),
        "glab" => Some(format!("glab auth logout --hostname {host}")),
        "tea" => Some(format!("tea logins delete <the login for {host}>")),
        _ => None,
    }
}

fn print_logout(answer: &LogoutPayload) {
    let host = &answer.host;
    match (&answer.command, answer.removed) {
        (Some(command), _) => {
            let source = answer.source.as_deref().unwrap_or("another tool");
            println!("the token for {host} comes from {source}; run `{command}` to remove it");
        }
        (None, true) => {
            let where_from = answer
                .source
                .as_deref()
                .map(|source| format!(" ({source})"))
                .unwrap_or_default();
            let revoked = if answer.revoked {
                " and revoked at the forge"
            } else {
                ", and the forge offers no revocation"
            };
            println!("Removed the credential for {host}{where_from}{revoked}.");
        }
        (None, false) => println!("No credential for {host} on this machine."),
    }
}

// ---------------------------------------------------------------------
// plugins
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct PluginsPayload {
    plugins: Vec<PluginRow>,
}

#[derive(Serialize)]
struct PluginRow {
    id: &'static str,
    binary: Option<String>,
    path: Option<PathBuf>,
    found_in: Option<&'static str>,
    protocol: Option<u32>,
    version: Option<String>,
    problem: Option<String>,
    /// The `rm` line for a stale binary that shadows nothing but sits
    /// beside the one that answers (D2.2a). joy never removes a binary
    /// it did not install, so this is a line to read, not an action.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shadowed: Vec<String>,
}

fn plugins() -> Result<()> {
    let mut rows: Vec<PluginRow> = Vec::new();
    for spec in forge_plugins::FORGE_PLUGINS {
        let candidates = forge_plugins::candidates(spec);
        let shadowed: Vec<String> = candidates
            .iter()
            .skip(1)
            .map(|(path, _)| format!("rm {}", path.display()))
            .collect();
        match forge_plugins::resolve_plugin(spec) {
            Ok(resolved) => {
                let found_in = candidates
                    .first()
                    .map(|(_, found_in): &(PathBuf, FoundIn)| found_in.as_str());
                let legacy_shadow = candidates.iter().skip(1).any(|(path, _)| {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .is_some_and(|stem| spec.legacy_names().any(|name| name == stem))
                });
                let problem = if resolved.protocol < PROTOCOL {
                    Some("plugin_outdated".to_string())
                } else if legacy_shadow {
                    Some("shadowed-legacy".to_string())
                } else {
                    None
                };
                rows.push(PluginRow {
                    id: spec.id,
                    binary: Some(binary_name(&resolved)),
                    path: Some(resolved.resolved_path.clone()),
                    found_in,
                    protocol: Some(resolved.protocol),
                    version: resolved.plugin_version.clone(),
                    problem,
                    shadowed,
                });
            }
            Err(error) => rows.push(PluginRow {
                id: spec.id,
                binary: None,
                path: error.resolved_path().map(Path::to_path_buf),
                found_in: None,
                protocol: None,
                version: None,
                problem: Some(error.state().to_string()),
                shadowed,
            }),
        }
    }
    if output::is_json() {
        output::emit(PluginsPayload { plugins: rows })?;
        return Ok(());
    }
    for row in &rows {
        match (&row.path, row.protocol) {
            (Some(path), Some(protocol)) => println!(
                "{}  {}  protocol {}  {}  {}",
                row.id,
                path.display(),
                protocol,
                row.found_in.unwrap_or("-"),
                row.version.as_deref().unwrap_or("-")
            ),
            _ => println!("{}  no connector installed", row.id),
        }
        if let Some(problem) = row.problem.as_deref() {
            println!("  problem: {problem}");
        }
        for line in &row.shadowed {
            println!("  another binary for this forge is installed and unused; {line}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one sentence every "sign in" text in this CLI ends with.
    #[test]
    fn the_sign_in_line_names_the_host_and_the_new_door() {
        assert_eq!(
            sign_in_line("github.com"),
            "run `joy forge login --host github.com`"
        );
        assert_eq!(sign_in_line(""), "run `joy forge login --host <host>`");
    }

    /// D3.8: the CLI has ONE failure vocabulary, and the states this
    /// door answers point at this door.
    #[test]
    fn the_action_line_points_at_the_door_that_fixes_the_state() {
        assert_eq!(
            action_line(Failure::NeedsSignIn, "codeberg.org").as_deref(),
            Some("run `joy forge login --host codeberg.org`")
        );
        assert_eq!(
            action_line(Failure::PluginMissing, "github.com").as_deref(),
            Some("run `joy forge plugins` to see which binary answered")
        );
        // Everything else keeps the classifier's own PROSE, and a
        // state that has none says nothing: `next_step` is the button
        // label of D4.7 ("retry", "show the fingerprint"), and a help
        // line that reads "= help: retry" is not an instruction.
        assert_eq!(action_line(Failure::Offline, "github.com"), None);
        assert_eq!(action_line(Failure::RateLimited, "github.com"), None);
        assert_eq!(action_line(Failure::NeedsHostTrust, "github.com"), None);
        assert_eq!(
            action_line(Failure::TlsUntrusted, "github.com").as_deref(),
            Failure::TlsUntrusted.guidance(),
            "the one state with prose behind the button keeps it"
        );
    }

    /// The states that are not a failed contact get NO next step. The
    /// one that mattered: every unknown state used to be told to run
    /// `joy forge login --host <host>`, which is the command that had
    /// just refused.
    #[test]
    fn a_state_with_no_next_step_is_given_none() {
        for state in ["unsupported", "cancelled", "busy", "error"] {
            assert_eq!(failure_of_state(state), None, "{state}");
            assert_eq!(
                Refusal::new("example.invalid", state, "the connector said so").action,
                None,
                "{state}"
            );
        }
        assert_eq!(
            Refusal::new("codeberg.org", "expired", "the credential is spent").action,
            Some("run `joy forge login --host codeberg.org`".to_string()),
            "a spent credential is renewed at this door"
        );
    }

    /// A refusal whose own message carries a help line gets no second
    /// one: the D3.11 refusal says "store a token with joy forge login
    /// --token-stdin", and "run `joy forge login --host x`" under it
    /// contradicts it.
    #[test]
    fn a_refusal_that_says_what_to_do_gets_no_second_help_line() {
        let refusal = Refusal::new(
            "example.invalid",
            "needs_sign_in",
            interactive::NO_PERSON_HERE,
        )
        .speaks_for_itself();
        assert_eq!(refusal.action, None);
        assert_eq!(refusal.state, "needs_sign_in");
        assert_eq!(
            token_refusal("github.com", "--token-stdin: empty input").action,
            None
        );
    }

    /// One row for the help line cases below.
    fn row(
        host: &str,
        forge: Option<&'static str>,
        state: &'static str,
    ) -> (HostRow, Option<String>) {
        (
            HostRow {
                host: host.to_string(),
                forge,
                login: None,
                state,
                source: "none".to_string(),
                scopes: None,
                expires_at: None,
                plugin: None,
            },
            None,
        )
    }

    /// JOY-02A8-F4: the help line under a `joy forge status` that found
    /// nobody signed in used to be `sign_in_line(rows[0].host)` in every
    /// case. That is wrong twice: it sent a person to sign in to a host
    /// no connector claims, where the door cannot open at all, and with
    /// several rows it named whichever host happened to be first and
    /// said nothing about the rest.
    #[test]
    fn the_status_help_names_a_host_only_where_there_is_one_to_name() {
        // Nothing claims the host: the step is the configuration, not
        // a sign in.
        assert_eq!(
            status_help(&[row("nowhere.example", None, "none")]),
            "add the instance to forges.yaml, or name a host a connector knows"
        );
        // One claimed host and nothing to choose between.
        assert_eq!(
            status_help(&[row("github.com", Some("github"), "none")]),
            "run `joy forge login --host github.com`"
        );
        // One claimed host beside an unclaimed one: the claimed one is
        // the one that can be signed in to.
        assert_eq!(
            status_help(&[
                row("nowhere.example", None, "none"),
                row("github.com", Some("github"), "none"),
            ]),
            "run `joy forge login --host github.com`"
        );
        // Several: the person picks from the rows printed above, and
        // the help line does not pick for them.
        assert_eq!(
            status_help(&[
                row("github.com", Some("github"), "none"),
                row("codeberg.org", Some("gitea"), "none"),
            ]),
            "run `joy forge login --host <host>`"
        );
    }

    #[test]
    fn the_connector_codes_become_the_states_of_the_design() {
        assert_eq!(state_of_code("access_denied"), "denied");
        assert_eq!(state_of_code("expired_token"), "expired");
        assert_eq!(state_of_code("device_flow_disabled"), "unsupported");
        assert_eq!(state_of_code("invalid_scope"), "scope_missing");
        assert_eq!(state_of_code("network"), "offline");
        assert_eq!(state_of_code("no-login"), "needs_sign_in");
        assert_eq!(state_of_code("something new"), "error");
    }

    #[test]
    fn a_foreign_credential_names_the_foreign_command() {
        assert_eq!(
            foreign_removal(Some("gh"), "github.com").as_deref(),
            Some("gh auth logout --hostname github.com")
        );
        assert_eq!(foreign_removal(Some("keychain"), "github.com"), None);
        assert_eq!(foreign_removal(None, "github.com"), None);
    }

    #[test]
    fn an_unreadable_expiry_is_not_an_expired_credential() {
        assert!(!is_past("not a date"));
        assert!(is_past("2001-01-01T00:00:00Z"));
        assert!(!is_past("2999-01-01T00:00:00Z"));
    }
}
