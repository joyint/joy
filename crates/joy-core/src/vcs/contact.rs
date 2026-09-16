// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE forge contact (JOY-0268-2A, incident JP-00EF-CC; rebuilt for
//! JOY-0295-36, design forge-connection-ng D1.8 and D1.9): every network
//! verb of the git engine - clone, fetch, ls-remote, push, probe - goes
//! through [`run`], and only through it. One place therefore
//!
//!   - opens a span per contact (ADR JP-00ED-EE: every forge contact is
//!     a span with forge, verb, outcome and duration),
//!   - classifies a failure into what the person needs to know
//!     ([`Failure`]), reading the evidence libgit2 really hands out: the
//!     error code, the error class and the HTTP status number, never the
//!     prose (D1.8a - the prose carries an operating system tail in the
//!     user's own language, so matching words fails on every non English
//!     host),
//!   - spaces the contacts to one host by the host's budget, stated in
//!     HTTP REQUESTS per second (D1.9): a plain wait in line, never a
//!     refusal. One verb costs several requests, because the first
//!     request of a connection to a private repository is answered 401
//!     and replayed, and that is what the budget is spent on,
//!   - answers a 429 by slowing down, never by stopping (Horst,
//!     2026-08-29): the gap for the host is doubled per strike, the
//!     strike SURVIVES the next success and expires only with time, and
//!     every contact still goes out.
//!
//! The vocabulary in this module is the one every surface reads: the CLI
//! `--json` word (D3.8), the desktop banner (D4.7) and the platform
//! status word. A reader that does not know the new words maps them with
//! [`Failure::old_word`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---- the state vocabulary (D1.8a, D1.8b) -----------------------------

/// What a failed contact means for the person. One word per state, and
/// every one of them names a different next step; two states that would
/// lead to the same sentence are one state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Nobody is signed in for this host, or the login is spent.
    NeedsSignIn,
    /// The login exists, but the organisation has not approved Joy for
    /// this repository.
    NeedsOrgApproval,
    /// The organisation requires single sign-on for this login.
    NeedsSso,
    /// The repository can be read with this login and not written.
    NoPushRights,
    /// This machine has never seen (or no longer trusts) the host key.
    NeedsHostTrust,
    /// The login is valid but was granted too little access.
    ScopeMissing,
    /// The forge connector binary is not on this machine.
    PluginMissing,
    /// The connector on this machine speaks an older protocol.
    PluginOutdated,
    /// The TLS certificate chain is not trusted by this machine.
    TlsUntrusted,
    /// A proxy sits in front of the forge and wants a login of its own.
    ProxyAuth,
    /// The forge throttles us (HTTP 429, or a documented ban); it will
    /// serve us again.
    RateLimited,
    /// Nobody answered: DNS, connection, timeout, or the forge is down.
    Offline,
    /// The forge answered and refused, and no other state fits: the
    /// oracle's verdict on GitHub, or an ssh fetch the remote refused.
    Denied,
    /// Something else (a rejected ref, a missing branch, a local fault):
    /// the forge answered, so it is NOT "offline" - a banner that says
    /// so sends the person checking their network for a fault in their
    /// own checkout.
    Error,
}

impl Failure {
    /// The wire word the status carries (platform proto, desktop DTO,
    /// CLI `--json`): "" is reserved for healthy, so every failure has a
    /// word.
    pub fn reason(self) -> &'static str {
        match self {
            Failure::NeedsSignIn => "needs_sign_in",
            Failure::NeedsOrgApproval => "needs_org_approval",
            Failure::NeedsSso => "needs_sso",
            Failure::NoPushRights => "no_push_rights",
            Failure::NeedsHostTrust => "needs_host_trust",
            Failure::ScopeMissing => "scope_missing",
            Failure::PluginMissing => "plugin_missing",
            Failure::PluginOutdated => "plugin_outdated",
            Failure::TlsUntrusted => "tls_untrusted",
            Failure::ProxyAuth => "proxy_auth",
            Failure::RateLimited => "rate_limited",
            Failure::Offline => "offline",
            Failure::Denied => "denied",
            Failure::Error => "error",
        }
    }

    /// The same state in the FOUR words joy spoke before this design
    /// (D1.8b), for a reader that has not learnt the new ones: every
    /// refusal of a login reads as `denied`, every fault of the machine
    /// reads as `error`, and `rate_limited` and `offline` are unchanged.
    /// A reader that knows the new words never calls this.
    pub fn old_word(self) -> &'static str {
        match self {
            Failure::NeedsSignIn
            | Failure::NeedsOrgApproval
            | Failure::NeedsSso
            | Failure::NoPushRights
            | Failure::NeedsHostTrust
            | Failure::ScopeMissing
            | Failure::Denied => "denied",
            Failure::PluginMissing
            | Failure::PluginOutdated
            | Failure::TlsUntrusted
            | Failure::ProxyAuth
            | Failure::Error => "error",
            Failure::RateLimited => "rate_limited",
            Failure::Offline => "offline",
        }
    }

    /// The one plain sentence a surface shows for this state (D4.7). The
    /// raw libgit2 text never appears here; it goes to the detail line.
    pub fn sentence(self, host: &str) -> String {
        let forge = forge_name(host);
        match self {
            Failure::NeedsSignIn => format!("Sign in to {forge} to sync."),
            Failure::NeedsOrgApproval => {
                "Your organisation must approve Joy for this repository.".into()
            }
            Failure::NeedsSso => "Your organisation requires single sign-on for this login.".into(),
            Failure::NoPushRights => "You can read this repository but not write to it.".into(),
            Failure::NeedsHostTrust => {
                format!("This machine has never seen the host key of {host}.")
            }
            Failure::ScopeMissing => {
                format!("Your {forge} sign in does not allow this.")
            }
            Failure::PluginMissing => format!("The {forge} connector is missing on this machine."),
            Failure::PluginOutdated => {
                format!("The {forge} connector on this machine is too old.")
            }
            Failure::TlsUntrusted => format!(
                "The certificate for {host} is not trusted by this machine's certificate store."
            ),
            Failure::ProxyAuth => format!("The proxy {host} needs a user name and a password."),
            Failure::RateLimited => format!("{forge} is rate limiting us, retrying later."),
            Failure::Offline => format!("No connection to {host}."),
            Failure::Denied => format!("{forge} refuses this login for this repository."),
            Failure::Error => format!("{forge} answered with an error."),
        }
    }

    /// The ONE next step that belongs to the sentence, or `None` when
    /// there is nothing the person can do but wait.
    pub fn next_step(self) -> Option<&'static str> {
        match self {
            Failure::NeedsSignIn => Some("sign in"),
            Failure::NeedsOrgApproval => Some("open the approval page"),
            Failure::NeedsSso => Some("open the sign-on page"),
            Failure::NeedsHostTrust => Some("show the fingerprint"),
            Failure::ScopeMissing => Some("sign in again with wider access"),
            Failure::PluginMissing => Some("repair"),
            Failure::PluginOutdated => Some("show how to replace it"),
            Failure::TlsUntrusted => Some(CA_NEXT_STEP),
            Failure::ProxyAuth => Some("sign in to the proxy"),
            Failure::Offline => Some("retry"),
            Failure::NoPushRights | Failure::RateLimited | Failure::Denied | Failure::Error => None,
        }
    }
}

/// The one next step for an untrusted certificate, per operating system
/// (D1.8c): the machine's own certificate store is what has to change,
/// and joy never offers to skip the check.
#[cfg(target_os = "linux")]
const CA_NEXT_STEP: &str = "add your organisation's CA with update-ca-certificates";
#[cfg(target_os = "macos")]
const CA_NEXT_STEP: &str =
    "add your organisation's CA to the login or System keychain and mark it trusted";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const CA_NEXT_STEP: &str =
    "your administrator must install the CA in the Windows certificate store";

// ---- the evidence a classifier is allowed to read (D1.8a) ------------

/// How the contact travelled. libgit2 produces completely different
/// errors per transport, so the transport is part of the evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Https,
    Ssh,
    /// A path or `file://` remote: neither of the two branches above
    /// applies, and neither one's rules may be borrowed for it. Not a
    /// state of D1.8a; joy really does contact local remotes (every
    /// engine test does), and letting them fall into the https branch
    /// would turn a local fault into "sign in to the forge".
    Local,
}

/// Which way the objects were meant to travel. A 403 on a fetch and a
/// 403 on a push mean two different things.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactDirection {
    Fetch,
    Push,
}

/// What joy put on the wire for this contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialSource {
    /// Nothing: an anonymous contact.
    NonePresented,
    /// A forge access token (the platform's, or the connector's).
    TokenPresented,
    /// A credential the machine's helper handed out.
    HelperPresented,
    /// A key the ssh agent holds.
    AgentPresented,
}

impl CredentialSource {
    /// Whether a credential rode with this contact, which is what
    /// decides the request weight of D1.9.
    pub fn is_some(self) -> bool {
        !matches!(self, CredentialSource::NonePresented)
    }
}

/// Everything the classifier is allowed to look at. The caller keeps the
/// libgit2 error INTACT (D1.8a: joy used to hand the classifier
/// `e.message()` alone and prefix "(offline?)" onto it, which made a 404
/// read as "no connection to github.com").
pub struct ContactEvidence {
    pub error: git2::Error,
    pub transport: Transport,
    pub direction: ContactDirection,
    pub credential: CredentialSource,
    /// Whether a credentialed contact to this host has already
    /// succeeded in this process ([`token_worked_before`]).
    pub token_worked_before: bool,
    pub host: String,
}

impl ContactEvidence {
    /// The evidence for a contact to `url`: the transport comes from the
    /// URL, and "did a token already work here" from this process's own
    /// memory.
    pub fn new(
        error: git2::Error,
        url: &str,
        direction: ContactDirection,
        credential: CredentialSource,
    ) -> Self {
        let host = host_of(url);
        ContactEvidence {
            error,
            transport: transport_of(url),
            direction,
            credential,
            token_worked_before: token_worked_before(&host),
            host,
        }
    }
}

/// The transport a remote URL names. An scp-style remote
/// (`git@github.com:o/r`) is ssh, a bare path is local.
pub fn transport_of(url: &str) -> Transport {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        Transport::Https
    } else if lower.starts_with("ssh://") || lower.starts_with("git+ssh://") {
        Transport::Ssh
    } else if lower.starts_with("file://") || lower.starts_with('/') || lower.starts_with('.') {
        Transport::Local
    } else if lower.contains('@') && lower.contains(':') {
        // scp syntax: user@host:path
        Transport::Ssh
    } else {
        Transport::Local
    }
}

/// The HTTP status number libgit2 reports, from EXACTLY the two formats
/// the two http transports produce and from nothing else (D1.8a):
/// `unexpected http status code: %d` (http.c:282, every non Windows
/// build) and `request failed with status code: %lu` (winhttp.c:1274,
/// every Windows build). Both are a C integer printed by libgit2 itself,
/// so the digits are ASCII and locale independent, and no other number
/// in the sentence (a branch name, a path, a byte count) can be mistaken
/// for one.
pub fn http_status(message: &str) -> Option<u16> {
    const FORMATS: [&str; 2] = [
        "unexpected http status code: ",
        "request failed with status code: ",
    ];
    let lower = message.to_ascii_lowercase();
    for format in FORMATS {
        if let Some(at) = lower.find(format) {
            let digits: String = lower[at + format.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(status) = digits.parse::<u16>() {
                return Some(status);
            }
        }
    }
    None
}

/// The seven sentences WinHTTP's status callback sets for a certificate
/// it will not accept (winhttp.c:718-740). They carry class `Http`, not
/// `Ssl`, and they are libgit2's OWN English literals with nothing
/// appended, which is why they may be matched as text.
const WINHTTP_CERTIFICATE_SENTENCES: [&str; 7] = [
    "ssl certificate issued for different common name",
    "ssl certificate has expired",
    "ssl certificate signed by unknown ca",
    "ssl certificate is invalid",
    "certificate revocation check failed",
    "ssl certificate was revoked",
    "security libraries could not be loaded",
];

/// libgit2's own English literals that may be matched as text, because
/// libgit2 writes them whole and appends nothing of the operating
/// system's (D1.8a, step 4).
const PROXY_AUTH_SENTENCES: [&str; 2] = [
    // http.c:165-169 with server type "proxy" (http.c:98)
    "proxy authentication required but no callback set",
    // http.c:206-208
    "proxy requires authentication that we do not support",
];
const REPLAY_SENTENCE: &str = "too many redirects or authentication replays";
/// GIT_ERROR_OS prefixes of the WinHTTP connect path (winhttp.c:950,
/// :870). Everything AFTER them is FormatMessageW output in the user's
/// own language, so only the prefix is read.
const OS_OFFLINE_PREFIXES: [&str; 2] = ["failed to send request", "failed to connect to host"];
/// The wait bound joy sets itself surfaces as libgit2's raw EAGAIN,
/// "SSL error: syscall failure: Resource temporarily unavailable"
/// (JOY-0278-85). It carries class `Ssl` and has nothing to do with a
/// certificate, so it is read before the certificate rules.
const SYSCALL_FAILURE_SENTENCES: [&str; 2] =
    ["syscall failure", "resource temporarily unavailable"];

// ---- the forge family behind a host ----------------------------------

/// Which forge software answers on a host. The 403 rules of D2.10 differ
/// per family, so the family is evidence too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostFamily {
    GitHub,
    /// gitlab.com itself: its documented ban is 15 minutes.
    GitLabCom,
    /// A self managed GitLab: the documented default ban is one hour.
    GitLabSelfManaged,
    /// Codeberg, Forgejo, Gitea: the oracle is never asked here.
    Gitea,
    Unknown,
}

static FAMILIES: Mutex<Option<HashMap<String, HostFamily>>> = Mutex::new(None);

/// Teach joy what software a self hosted host runs (the connector reads
/// it from `forges.yaml`, D2.5). Without an entry the family is guessed
/// from the host name, which is right for the three public forges and
/// for the usual `gitlab.<company>` and `git.<company>` names.
pub fn set_host_family(host: &str, family: HostFamily) {
    let mut guard = FAMILIES.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(host.to_ascii_lowercase(), family);
}

/// The forge family behind a host.
pub fn host_family(host: &str) -> HostFamily {
    let host = host.to_ascii_lowercase();
    if let Some(family) = FAMILIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|f| f.get(&host).copied())
    {
        return family;
    }
    if host == "github.com" || host.ends_with(".github.com") {
        HostFamily::GitHub
    } else if host == "gitlab.com" || host.ends_with(".gitlab.com") {
        HostFamily::GitLabCom
    } else if host.contains("gitlab") {
        HostFamily::GitLabSelfManaged
    } else if host == "codeberg.org" || host.contains("gitea") || host.contains("forgejo") {
        HostFamily::Gitea
    } else {
        HostFamily::Unknown
    }
}

/// The documented wait after GitLab's failed authentication ban (D2.10):
/// gitlab.com answers 403 for 15 minutes, a self managed instance
/// defaults to one hour. The ban "cannot be cleared by authenticating"
/// and sends no response header, so the number can only come from here.
fn gitlab_ban_wait(family: HostFamily) -> Option<Duration> {
    match family {
        HostFamily::GitLabCom => Some(Duration::from_secs(15 * 60)),
        HostFamily::GitLabSelfManaged => Some(Duration::from_secs(60 * 60)),
        _ => None,
    }
}

// ---- the rate limit oracle (D2.10) -----------------------------------

/// What the forge's own rate limit endpoint said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OracleAnswer {
    /// A limit, with the wait `x-ratelimit-reset` or `retry-after` named.
    RateLimited { wait: Option<Duration> },
    /// The login is fine, the organisation has not approved Joy.
    NeedsOrgApproval,
    /// The forge refuses, and no waiting helps.
    Denied,
}

/// The hook J3 fills with the connector call `GET /rate_limit`. It is
/// asked ONLY under the conditions of D2.10, which this module enforces:
/// status exactly 403 or 429, transport https, a GitHub host, and not
/// more than once per host per strike window.
pub trait RateLimitOracle: Send + Sync {
    /// The forge's own verdict, or `None` when it could not be had.
    fn ask(&self, host: &str, status: u16) -> Option<OracleAnswer>;
}

static ORACLE: Mutex<Option<Arc<dyn RateLimitOracle>>> = Mutex::new(None);
static ORACLE_ASKED: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// Install the oracle (J3). Until one is installed every 403 answers
/// from the table alone, which is what wave 0 ships.
pub fn set_oracle(oracle: Arc<dyn RateLimitOracle>) {
    *ORACLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(oracle);
}

#[cfg(test)]
pub(crate) fn clear_oracle() {
    *ORACLE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    ORACLE_ASKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .clear();
}

/// Ask the oracle, at most once per host per strike window.
fn ask_oracle(host: &str, status: u16) -> Option<OracleAnswer> {
    let oracle = ORACLE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .cloned()?;
    {
        let now = Instant::now();
        let mut guard = ORACLE_ASKED.lock().unwrap_or_else(|e| e.into_inner());
        let asked = guard.get_or_insert_with(HashMap::new);
        if let Some(last) = asked.get(host) {
            if now.duration_since(*last) < STRIKE_LASTS {
                return None;
            }
        }
        asked.insert(host.to_string(), now);
    }
    oracle.ask(host, status)
}

// ---- the classifier (D1.8b) ------------------------------------------

/// The classifier's whole answer: the state, the one sentence, the one
/// next step, the wait the host asked for and the detail line's first
/// part. [`classify`] is this without the words.
#[derive(Clone, Debug)]
pub struct Verdict {
    pub failure: Failure,
    /// The plain sentence for the surface. Never libgit2's own text.
    pub sentence: String,
    /// The ONE next step, or `None` when there is nothing to do.
    pub next_step: Option<String>,
    /// How long to wait, when the forge or its documentation said so.
    pub wait: Option<Duration>,
    /// libgit2's own verdict, for the log and for the details view a
    /// person opens deliberately. It never reaches the banner or a
    /// tooltip (D1.8b, wording rules).
    pub detail: String,
}

/// The state this failure is (D1.8b). The evidence is read in the order
/// of D1.8a: the error code, the error class, the HTTP status number,
/// and only then libgit2's own English literals.
pub fn classify(evidence: &ContactEvidence) -> Failure {
    verdict(evidence).failure
}

/// [`classify`] with the words, the wait and the detail line.
pub fn verdict(evidence: &ContactEvidence) -> Verdict {
    let (failure, wait, sentence) = decide(evidence);
    let host = evidence.host.as_str();
    Verdict {
        failure,
        sentence: sentence.unwrap_or_else(|| failure.sentence(host)),
        next_step: failure.next_step().map(str::to_string),
        wait,
        detail: format!("libgit2: {}", evidence.error.message()),
    }
}

fn decide(ev: &ContactEvidence) -> (Failure, Option<Duration>, Option<String>) {
    use git2::{ErrorClass as Class, ErrorCode as Code};
    let code = ev.error.code();
    let class = ev.error.class();
    let message = ev.error.message().to_ascii_lowercase();
    let family = host_family(&ev.host);
    let plain = |f: Failure| (f, None, None);

    // ssh first: its class is unambiguous, and an ssh host key refusal
    // must not be read as an https certificate problem.
    if class == Class::Ssh || (ev.transport == Transport::Ssh && code == Code::Auth) {
        return match code {
            Code::Auth => plain(Failure::NeedsSignIn),
            Code::Certificate => plain(Failure::NeedsHostTrust),
            _ => {
                // the message is the remote's own stderr
                // (ssh_libssh2.c:138): it goes to the detail line, never
                // to the banner
                match ev.direction {
                    ContactDirection::Push => plain(Failure::NoPushRights),
                    ContactDirection::Fetch => plain(Failure::Denied),
                }
            }
        };
    }

    // The wait bound joy sets itself (JOY-0278-85) arrives as an SSL
    // syscall failure and is not a certificate fault: it is silence.
    if SYSCALL_FAILURE_SENTENCES
        .iter()
        .any(|s| message.contains(s))
    {
        return plain(Failure::Offline);
    }

    // A proxy that wants a login of its own, before the generic http
    // rules: both sentences are libgit2's own literals.
    if PROXY_AUTH_SENTENCES.iter().any(|s| message.contains(s)) {
        return plain(Failure::ProxyAuth);
    }

    // The certificate chain.
    if (code == Code::Certificate && ev.transport == Transport::Https)
        || class == Class::Ssl
        || (class == Class::Http
            && WINHTTP_CERTIFICATE_SENTENCES
                .iter()
                .any(|s| message.contains(s)))
    {
        return plain(Failure::TlsUntrusted);
    }

    // The HTTP status number, the only digits in the sentence joy trusts.
    if let Some(status) = http_status(&message) {
        return decide_by_status(ev, family, status);
    }

    // libgit2 says the replay count ran out: the exact cause is unclear,
    // and it says so itself (http.c:439-440). A login is what is
    // missing, not a refusal of one.
    if message.contains(REPLAY_SENTENCE) {
        return plain(Failure::NeedsSignIn);
    }

    // Nobody answered.
    if code == Code::Timeout
        || class == Class::Net
        || (class == Class::Os && OS_OFFLINE_PREFIXES.iter().any(|p| message.starts_with(p)))
    {
        return plain(Failure::Offline);
    }

    // An https 401 that libgit2 turned into GIT_EAUTH itself.
    if code == Code::Auth && ev.transport == Transport::Https {
        return plain(Failure::NeedsSignIn);
    }

    plain(Failure::Error)
}

fn decide_by_status(
    ev: &ContactEvidence,
    family: HostFamily,
    status: u16,
) -> (Failure, Option<Duration>, Option<String>) {
    let plain = |f: Failure| (f, None, None);
    match status {
        401 => (Failure::NeedsSignIn, None, None),
        429 => {
            let wait = match (family, ev.transport) {
                (HostFamily::GitHub, Transport::Https) => match ask_oracle(&ev.host, status) {
                    Some(OracleAnswer::RateLimited { wait }) => wait,
                    // the oracle answered something else for a 429: the
                    // 429 stands, the strike table names the wait
                    _ => None,
                },
                _ => None,
            };
            (Failure::RateLimited, wait, None)
        }
        403 | 404 if ev.direction == ContactDirection::Push && ev.token_worked_before => {
            plain(Failure::NoPushRights)
        }
        403 if ev.direction == ContactDirection::Fetch && ev.token_worked_before => {
            match (family, ev.transport) {
                (HostFamily::GitHub, Transport::Https) => match ask_oracle(&ev.host, status) {
                    Some(OracleAnswer::RateLimited { wait }) => (Failure::RateLimited, wait, None),
                    Some(OracleAnswer::NeedsOrgApproval) => plain(Failure::NeedsOrgApproval),
                    Some(OracleAnswer::Denied) => plain(Failure::Denied),
                    None => plain(Failure::Error),
                },
                // GitLab's failed authentication ban: it sends no header
                // and cannot be cleared by signing in, so the wait comes
                // from GitLab's own documentation and the connector is
                // NOT asked (D2.10).
                (HostFamily::GitLabCom, _) | (HostFamily::GitLabSelfManaged, _) => (
                    Failure::RateLimited,
                    gitlab_ban_wait(family),
                    Some(format!(
                        "{} refused this login for a while after too many failed sign ins.",
                        forge_name(&ev.host)
                    )),
                ),
                _ => plain(Failure::Error),
            }
        }
        404 if ev.direction == ContactDirection::Fetch => {
            if family == HostFamily::GitHub && ev.token_worked_before {
                plain(Failure::NeedsOrgApproval)
            } else {
                (
                    Failure::Error,
                    None,
                    Some(format!(
                        "{} does not have this repository (renamed, deleted or not visible to this login)",
                        ev.host
                    )),
                )
            }
        }
        502..=504 => (
            Failure::Offline,
            None,
            Some(format!("{} is not answering right now", ev.host)),
        ),
        _ => plain(Failure::Error),
    }
}

/// Whether this failure is the one D1.7 answers with ONE cache
/// invalidation and one re-ask of the connector: a token that worked
/// before was presented and the host now asks for a login. Anything else
/// is a sign in, not a refresh, and joy re-asks at most once.
pub fn wants_token_refresh(evidence: &ContactEvidence) -> bool {
    classify(evidence) == Failure::NeedsSignIn
        && evidence.credential == CredentialSource::TokenPresented
        && evidence.token_worked_before
}

// ---- the connector's own answers (D1.8b, first four rows) ------------

/// What the forge connector said, in the words D2 gives it. The runner
/// (J1) produces this; the mapping to a state lives here with the rest
/// of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginEvidence {
    /// The binary is not there, or it could not be started.
    Missing,
    /// It answered protocol 1 to a protocol 2 verb.
    Outdated,
    /// It answered `scope_missing`.
    ScopeMissing,
    /// It answered `needs_sso`; the URL from the `X-GitHub-SSO` header
    /// is the action (D2.7c).
    NeedsSso { url: Option<String> },
}

/// The state a connector answer means (D1.8b).
pub fn classify_plugin(evidence: &PluginEvidence) -> Failure {
    match evidence {
        PluginEvidence::Missing => Failure::PluginMissing,
        PluginEvidence::Outdated => Failure::PluginOutdated,
        PluginEvidence::ScopeMissing => Failure::ScopeMissing,
        PluginEvidence::NeedsSso { .. } => Failure::NeedsSso,
    }
}

// ---- the detail line (D1.8b, wording rules) --------------------------

/// The list of sources joy tried, in the fixed grammar of D1.8b:
/// `source: outcome` parts joined by `"; "`, for example
/// `agent: no identities; key ~/.ssh/id_ed25519: passphrase needed
/// (skipped, background); helper 'manager': fatal: ...; no forge login
/// for github.com`. It goes to the log and to a details view the person
/// opens deliberately. It never goes onto the banner and never into a
/// tooltip.
#[derive(Clone, Debug, Default)]
pub struct DetailLine {
    parts: Vec<String>,
}

impl DetailLine {
    pub fn new() -> Self {
        DetailLine::default()
    }

    /// One source and what it answered.
    pub fn tried(&mut self, source: impl std::fmt::Display, outcome: impl std::fmt::Display) {
        self.parts.push(format!("{source}: {outcome}"));
    }

    /// A part that has no source of its own ("no forge login for
    /// github.com").
    pub fn note(&mut self, note: impl std::fmt::Display) {
        self.parts.push(note.to_string());
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

impl std::fmt::Display for DetailLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.parts.join("; "))
    }
}

// ---- the error every contact returns ---------------------------------

/// The error every contact returns: the sentence for the person, the
/// state behind it, the detail line for the log, and - when the forge
/// limits us - the moment the gate opens again.
#[derive(Debug)]
pub struct ContactError {
    pub failure: Failure,
    pub message: String,
    /// libgit2's own text and the sources joy tried; never the banner.
    pub detail: Option<String>,
    pub next_try: Option<SystemTime>,
}

impl std::fmt::Display for ContactError {
    /// The plain sentence and nothing else: the detail line is read
    /// deliberately through [`detail_of`], because libgit2's own text
    /// never belongs on a surface (D1.8b, wording rules).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ContactError {}

/// Turn the evidence of one failed contact into the error joy carries
/// upwards: the classifier decides the state, the state decides the
/// sentence, and libgit2's own words go to the detail line.
pub fn failed(evidence: &ContactEvidence) -> anyhow::Error {
    let verdict = verdict(evidence);
    let message = match verdict.next_step {
        Some(step) => format!("{} ({step})", verdict.sentence),
        None => verdict.sentence,
    };
    anyhow::Error::new(ContactError {
        failure: verdict.failure,
        message,
        detail: Some(verdict.detail),
        next_try: verdict.wait.map(|w| SystemTime::now() + w),
    })
}

/// The meaning of any error that came out of the engine: a
/// [`ContactError`] says it directly. An error that never passed the
/// classifier is NOT guessed at from its prose (D1.8a) - it is a fault
/// the forge answered to, which is exactly `error`.
pub fn failure_of(error: &anyhow::Error) -> Failure {
    match error.downcast_ref::<ContactError>() {
        Some(c) => c.failure,
        None => Failure::Error,
    }
}

/// The next-try moment an error carries, if the forge limits us.
pub fn next_try_of(error: &anyhow::Error) -> Option<SystemTime> {
    error
        .downcast_ref::<ContactError>()
        .and_then(|c| c.next_try)
}

/// The detail line an error carries, for the log and the details view.
pub fn detail_of(error: &anyhow::Error) -> Option<String> {
    error
        .downcast_ref::<ContactError>()
        .and_then(|c| c.detail.clone())
}

// ---- the budget, in HTTP requests per second (D1.9) -------------------

/// The canonical budget table of D1.9, in HTTP REQUESTS per second per
/// host per machine. Every millisecond figure joy uses is `1000 /
/// budget` computed from this table, and no millisecond figure is
/// written down anywhere else. Codeberg's 0.9 is a measured ceiling of
/// about 1.1 requests per second minus the buffer (JP-00EF-CC); GitHub's
/// 1.0 and the unknown host's 1.0 are the design's decided values;
/// gitlab.com's 5.0 is its documented headroom.
const BUDGETS: [(&str, f64); 4] = [
    ("codeberg.org", 0.9),
    ("github.com", 1.0),
    ("gitlab.com", 5.0),
    ("default", 1.0),
];

/// The milliseconds one request costs on a host, derived from
/// [`BUDGETS`] and from nothing else.
fn derived_gaps() -> HashMap<String, Duration> {
    BUDGETS
        .iter()
        .map(|(host, budget)| {
            (
                host.to_string(),
                Duration::from_millis((1000.0 / budget).round() as u64),
            )
        })
        .collect()
}

/// What one verb costs in HTTP requests (D1.9). The numbers are the
/// design's table for the engine AFTER this package: `download_ref`
/// holds its connection, so a fetch is one connection, not two.
///
/// The first request of a new connection to a private repository carries
/// no `Authorization` header and is answered 401, and the credential
/// callback then replays it (httpclient.c:566-568), which is why a
/// credentialed verb costs one request more than an anonymous one. An
/// ssh contact makes no HTTP request at all and pays the anonymous
/// column; a local remote pays nothing, because it rides no forge's
/// bucket.
pub fn requests(verb: &str, transport: Transport, credentialed: bool) -> u32 {
    if transport == Transport::Local {
        return 0;
    }
    let (anonymous, private) = match verb {
        "ls-remote" | "probe" => (1, 2),
        "fetch" | "clone" => (2, 3),
        "push" => (3, 3),
        // an unknown verb is charged like the most expensive one joy has
        _ => (3, 3),
    };
    if transport == Transport::Ssh || !credentialed {
        anonymous
    } else {
        private
    }
}

/// The poll period for a verb on a host (D1.9, one rule):
/// `requests(verb) / budget`, rounded up to the next whole second. A
/// private chat poll on codeberg.org is two requests against a 0.9
/// budget, which is 2.222 s and therefore a 3 s period. `projects` open
/// projects on one host divide the budget, so the period is multiplied
/// by their number.
///
/// A5 and P2 use the number computed here and never a number of their
/// own.
pub fn poll_period(host: &str, verb: &str, transport: Transport, credentialed: bool) -> Duration {
    poll_period_for(host, verb, transport, credentialed, 1)
}

/// [`poll_period`] for `projects` open projects on the same host.
pub fn poll_period_for(
    host: &str,
    verb: &str,
    transport: Transport,
    credentialed: bool,
    projects: u32,
) -> Duration {
    // No anonymous polling (D1.9): an https remote nobody is signed in
    // for is checked once every 15 minutes per host, whatever the
    // budget would allow.
    if transport == Transport::Https && !credentialed {
        return ANONYMOUS_POLL_INTERVAL;
    }
    let cost = gap_for(host) * requests(verb, transport, credentialed) * projects.max(1);
    let seconds = (cost.as_millis() as u64).div_ceil(1000);
    Duration::from_secs(seconds)
}

/// The no anonymous polling rule of D1.9: a remote with no credential is
/// polled at most once every fifteen minutes per host.
pub const ANONYMOUS_POLL_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// The sentence the surface shows while a host is polled anonymously, so
/// a person is never left wondering why their chat is slow.
pub fn anonymous_poll_reason(host: &str) -> String {
    format!(
        "Nobody is signed in for {host}, so it is checked once every 15 minutes. Sign in to sync at full speed."
    )
}

static ANONYMOUS_POLLS: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// The gate for an anonymous poll: `None` when the poll may go out (and
/// the slot is taken), `Some(next try)` when it may not. A person's own
/// command is not a poll and never asks this.
pub fn anonymous_poll_gate(host: &str) -> Option<SystemTime> {
    let now = Instant::now();
    let mut guard = ANONYMOUS_POLLS.lock().unwrap_or_else(|e| e.into_inner());
    let polls = guard.get_or_insert_with(HashMap::new);
    if let Some(last) = polls.get(host) {
        let waited = now.duration_since(*last);
        if waited < ANONYMOUS_POLL_INTERVAL {
            return Some(SystemTime::now() + (ANONYMOUS_POLL_INTERVAL - waited));
        }
    }
    polls.insert(host.to_string(), now);
    None
}

#[cfg(test)]
pub(crate) fn reset_anonymous_polls() {
    ANONYMOUS_POLLS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .clear();
}

// ---- the per-host throttle -------------------------------------------

struct Throttle {
    /// milliseconds per HTTP REQUEST, per host
    gaps: HashMap<String, Duration>,
    /// when the next contact to a host may leave
    next_free: HashMap<String, Instant>,
}

static THROTTLE: Mutex<Option<Throttle>> = Mutex::new(None);

fn with_throttle<T>(f: impl FnOnce(&mut Throttle) -> T) -> T {
    let mut guard = THROTTLE.lock().unwrap_or_else(|e| e.into_inner());
    let t = guard.get_or_insert_with(|| Throttle {
        gaps: derived_gaps(),
        next_free: HashMap::new(),
    });
    f(t)
}

/// `host=ms,host=ms,default=ms` into a table; unknown shapes are
/// skipped. The milliseconds are per HTTP REQUEST, not per contact.
pub fn parse_gaps(spec: &str) -> HashMap<String, Duration> {
    spec.split(',')
        .filter_map(|entry| {
            let (host, ms) = entry.split_once('=')?;
            let ms: u64 = ms.trim().parse().ok()?;
            Some((host.trim().to_ascii_lowercase(), Duration::from_millis(ms)))
        })
        .collect()
}

/// Install per-host gaps over the derived table (the platform hands its
/// JOYINT_FORGE_MIN_GAP_MS here; hosts left out keep the budget table's
/// own value).
pub fn set_gaps(spec: &str) {
    let parsed = parse_gaps(spec);
    with_throttle(|t| {
        let mut gaps = derived_gaps();
        gaps.extend(parsed);
        t.gaps = gaps;
    });
}

/// The milliseconds ONE request costs on this host. A remote with no
/// host name is a path on this machine and costs nothing.
fn gap_for(host: &str) -> Duration {
    if host.is_empty() {
        return Duration::ZERO;
    }
    with_throttle(|t| {
        t.gaps
            .get(host)
            .or_else(|| t.gaps.get("default"))
            .copied()
            .unwrap_or(Duration::from_millis(1000))
    })
}

/// Wait for this host's turn: the gap the verb's requests cost, doubled
/// once per standing strike. Takes the slot on the way out, so
/// concurrent callers line up one behind the other instead of leaving
/// together.
fn take_turn(host: &str, verb: &str, transport: Transport, credentialed: bool) -> Duration {
    let mut gap = gap_for(host) * requests(verb, transport, credentialed);
    // A forge that said 429 is never stopped, only slowed (Horst,
    // 2026-08-29): `strikes` is the exponent it is documented to be.
    let strikes = strikes_for(host);
    if strikes > 0 {
        gap *= 1u32 << strikes.min(MAX_STRIKE_EXPONENT);
    }
    if gap.is_zero() {
        return Duration::ZERO;
    }
    let waited = with_throttle(|t| {
        let now = Instant::now();
        let free = t.next_free.get(host).copied().unwrap_or(now);
        let start = free.max(now);
        t.next_free.insert(host.to_string(), start + gap);
        start.saturating_duration_since(now)
    });
    if !waited.is_zero() {
        std::thread::sleep(waited);
    }
    waited
}

#[cfg(test)]
pub(crate) fn reset_throttle() {
    with_throttle(|t| t.next_free.clear());
}

// ---- the per-host gate -----------------------------------------------

/// How long a strike stands. It is NOT cleared by the next success
/// (D1.9): a forge that limited us a second ago has not changed its mind
/// because one request got through, and clearing on success is what let
/// joy run straight back into the limit.
const STRIKE_LASTS: Duration = Duration::from_secs(600);

/// The doubling stops here: a 1000 ms request gap becomes 64 s, which is
/// longer than any poll period joy computes, and it is still a wait and
/// never a refusal.
const MAX_STRIKE_EXPONENT: u32 = 6;

struct Limit {
    until: Instant,
    /// consecutive 429s, the doubling exponent
    strikes: u32,
}

static LIMITS: Mutex<Option<HashMap<String, Limit>>> = Mutex::new(None);

fn with_limits<T>(f: impl FnOnce(&mut HashMap<String, Limit>) -> T) -> T {
    let mut guard = LIMITS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// The host part of a forge URL (`https://codeberg.org/o/r.git` ->
/// `codeberg.org`, `git@github.com:o/r` -> `github.com`); empty for a
/// path on this machine.
pub fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let rest = rest.rsplit('@').next().unwrap_or(rest);
    rest.split(['/', ':'])
        .next()
        .unwrap_or(rest)
        .to_ascii_lowercase()
}

/// A user-facing name for the forge behind a URL or a host.
pub fn forge_name(url: &str) -> String {
    let host = host_of(url);
    match host.as_str() {
        "github.com" => "GitHub".into(),
        "gitlab.com" => "GitLab".into(),
        "codeberg.org" => "Codeberg".into(),
        "" => "the forge".into(),
        other => other.to_string(),
    }
}

/// When the gate for this host opens, if a strike still stands.
pub fn limited_until(host: &str) -> Option<SystemTime> {
    let now = Instant::now();
    with_limits(|limits| {
        limits
            .get(host)
            .filter(|l| l.until > now)
            .map(|l| SystemTime::now() + (l.until - now))
    })
}

/// How many strikes still stand for this host: the doubling exponent. An
/// expired strike is forgotten here, which is the only way a strike ends.
fn strikes_for(host: &str) -> u32 {
    let now = Instant::now();
    with_limits(|limits| match limits.get(host) {
        Some(limit) if limit.until > now => limit.strikes,
        Some(_) => {
            limits.remove(host);
            0
        }
        None => 0,
    })
}

/// The gate's answer for a checkout's forge, for status displays:
/// `Some(next try)` while the forge limits us.
pub fn limited_for(repo_dir: &Path) -> Option<SystemTime> {
    let url = super::forge::remote_url(repo_dir)?;
    limited_until(&host_of(&url))
}

/// Record a 429 (or a documented ban) and return when the strike ends.
/// `wait` is the forge's own number when it named one.
fn strike(host: &str, wait: Option<Duration>) -> SystemTime {
    let now = Instant::now();
    let lasts = wait.unwrap_or(STRIKE_LASTS);
    with_limits(|limits| {
        let entry = limits.entry(host.to_string()).or_insert(Limit {
            until: now,
            strikes: 0,
        });
        if entry.until <= now {
            // the previous strike had already run out
            entry.strikes = 0;
        }
        entry.strikes = entry.strikes.saturating_add(1);
        entry.until = now + lasts;
        SystemTime::now() + lasts
    })
}

#[cfg(test)]
pub(crate) fn reset_limits() {
    with_limits(|limits| limits.clear());
}

// ---- what a credential has already achieved on a host ----------------

static TOKEN_WORKED: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// Whether a credentialed contact to this host has already succeeded in
/// this process. It is the difference between "you cannot read this" and
/// "you can read this and not write it" (D1.8b, the push rules), and
/// between a 404 that means "no such repository" and a 404 that means
/// "your organisation has not approved Joy".
pub fn token_worked_before(host: &str) -> bool {
    TOKEN_WORKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|t| t.get(host).copied())
        .unwrap_or(false)
}

fn note_credential_worked(host: &str) {
    if host.is_empty() {
        return;
    }
    TOKEN_WORKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(host.to_string(), true);
}

#[cfg(test)]
pub(crate) fn reset_token_memory() {
    TOKEN_WORKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .clear();
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run one forge contact against `url`: the throttle first, then the
/// verb, then the verdict - a span around all of it. `verb` names the
/// contact (`clone`, `fetch`, `ls-remote`, `push`, `probe`), and
/// `credentialed` says whether a credential rides with it, which is what
/// decides how many HTTP requests it costs (D1.9).
pub fn run<T>(
    url: &str,
    verb: &'static str,
    credentialed: bool,
    work: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let host = host_of(url);
    let transport = transport_of(url);
    let span = tracing::info_span!("forge.contact", verb, forge = %host);
    let _s = span.enter();
    let waited = take_turn(&host, verb, transport, credentialed);
    if !waited.is_zero() {
        tracing::debug!(
            waited_ms = waited.as_millis() as u64,
            requests = requests(verb, transport, credentialed),
            "forge contact throttled"
        );
    }
    let started = Instant::now();
    match work() {
        Ok(value) => {
            if credentialed {
                note_credential_worked(&host);
            }
            tracing::debug!(
                took_ms = started.elapsed().as_millis() as u64,
                "forge contact ok"
            );
            Ok(value)
        }
        Err(e) => {
            let failure = failure_of(&e);
            let next_try = match failure {
                Failure::RateLimited => {
                    let wait =
                        next_try_of(&e).and_then(|t| t.duration_since(SystemTime::now()).ok());
                    Some(strike(&host, wait))
                }
                _ => next_try_of(&e),
            };
            if failure == Failure::RateLimited {
                // the sentence an operator reads on the board (JP-00F7-31)
                tracing::error!(
                    forge = %host,
                    slowed_until_unix = next_try.map(unix).unwrap_or(0),
                    "{host} is limiting this instance: contacts to it run at half speed per strike for ten minutes; lower JOYINT_FORGE_MIN_GAP_MS's rate for this host if it repeats"
                );
            }
            tracing::error!(
                outcome = failure.reason(),
                took_ms = started.elapsed().as_millis() as u64,
                error = %e,
                detail = detail_of(&e).unwrap_or_default(),
                "forge contact failed"
            );
            Err(anyhow::Error::new(ContactError {
                failure,
                message: e.to_string(),
                detail: detail_of(&e),
                next_try,
            }))
        }
    }
}

/// [`run`] for a checkout: the forge is the checkout's remote.
pub fn run_for<T>(
    repo_dir: &Path,
    verb: &'static str,
    credentialed: bool,
    work: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let url = super::forge::remote_url(repo_dir).unwrap_or_default();
    run(&url, verb, credentialed, work)
}

#[cfg(test)]
mod tests;
