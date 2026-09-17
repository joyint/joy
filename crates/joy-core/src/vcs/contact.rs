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
//!   - holds back the one contact nobody asked for: an https remote
//!     with no credential is POLLED at most once every fifteen minutes
//!     per host ([`run_poll`], D1.9), and the refusal carries the
//!     sentence that says why. What "no credential" means is not what
//!     the caller hoped for but what the credential callbacks really
//!     handed over on this host ([`credential_answers`]): on the
//!     desktop `Auth::Local` claims one everywhere and a machine with
//!     no helper entry presents nothing. A person's own command goes
//!     through [`run`] and is never held,
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
            // D4.7's own row, word for word: the state is reached when
            // the connector answered `scope_missing` to a verb that
            // creates a project, which is the one verb of D2.7a whose
            // scope set is wider than the sync set.
            Failure::ScopeMissing => {
                format!("Your {forge} sign in does not allow creating projects.")
            }
            Failure::PluginMissing => format!("The {forge} connector is missing on this machine."),
            Failure::PluginOutdated => {
                format!("The {forge} connector on this machine is too old.")
            }
            Failure::TlsUntrusted => format!(
                "The certificate for {host} is not trusted by this machine's certificate store."
            ),
            // the proxy's OWN name when the evidence carries it
            // (D1.8c); `host` here is the forge, never the proxy, so
            // this fallback says "in front of" and names no wrong
            // machine
            Failure::ProxyAuth => {
                format!("A proxy in front of {host} needs a user name and a password.")
            }
            Failure::RateLimited => format!("{forge} is rate limiting us, retrying later."),
            Failure::Offline => format!("No connection to {host}."),
            Failure::Denied => format!("{forge} refuses this login for this repository."),
            Failure::Error => format!("{forge} answered with an error."),
        }
    }

    /// The ONE next step that belongs to the sentence, or `None` when
    /// there is nothing the person can do but wait.
    ///
    /// This is the BUTTON of D4.7 and nothing else: a surface renders it
    /// as a label, so it is always two or three words in the
    /// imperative. What is behind the button, which can be a whole
    /// sentence and can differ per operating system, is
    /// [`Failure::guidance`]. Mixing the two printed "add your
    /// organisation's CA with update-ca-certificates" on a button.
    pub fn next_step(self) -> Option<&'static str> {
        match self {
            Failure::NeedsSignIn => Some("sign in"),
            Failure::NeedsOrgApproval => Some("open the approval page"),
            Failure::NeedsSso => Some("open the sign-on page"),
            Failure::NeedsHostTrust => Some("show the fingerprint"),
            Failure::ScopeMissing => Some("sign in again with wider access"),
            Failure::PluginMissing => Some("repair"),
            Failure::PluginOutdated => Some("show how to replace it"),
            Failure::TlsUntrusted => Some("show what to do"),
            Failure::ProxyAuth => Some("sign in to the proxy"),
            Failure::Offline => Some("retry"),
            Failure::NoPushRights | Failure::RateLimited | Failure::Denied | Failure::Error => None,
        }
    }

    /// What is behind the button of [`Failure::next_step`]: the
    /// instruction a person follows, which is prose and never a label
    /// (D1.8c). Only the states whose next step needs one have it; for
    /// every other state the button says it all.
    pub fn guidance(self) -> Option<&'static str> {
        match self {
            Failure::TlsUntrusted => Some(CA_GUIDANCE),
            _ => None,
        }
    }
}

/// What to do about an untrusted certificate, per operating system
/// (D1.8c): the machine's own certificate store is what has to change,
/// and joy never offers to skip the check.
#[cfg(target_os = "linux")]
const CA_GUIDANCE: &str = "add your organisation's CA with update-ca-certificates";
#[cfg(target_os = "macos")]
const CA_GUIDANCE: &str =
    "add your organisation's CA to the login or System keychain and mark it trusted";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const CA_GUIDANCE: &str = "your administrator must install the CA in the Windows certificate store";

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
    /// The proxy this contact went through (`proxy.acme.example:8080`),
    /// when joy configured one (D1.11). It is NOT the forge host, and a
    /// 407 names this machine and never [`ContactEvidence::host`]
    /// (D1.8c). `None` when no proxy was configured, which is when joy
    /// cannot name one.
    pub proxy: Option<String>,
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
            proxy: None,
        }
    }

    /// The same evidence for a contact that travelled through a proxy:
    /// `proxy` is the proxy's own `host:port`, which is the only name a
    /// 407 may carry (D1.8c).
    pub fn through_proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
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
/// The `GIT_ERROR_OS` prefixes of a connect that never came up, one per
/// build. Everything AFTER them is the operating system's own text
/// (FormatMessageW on Windows, `strerror` elsewhere, errors.c:182-203)
/// in the user's own language, so only the prefix is read.
///
/// `failed to connect to` covers both producers and is why the prefix
/// stops before the next word: WinHTTP writes the literal "failed to
/// connect to host" (winhttp.c:870), while every OpenSSL and Secure
/// Transport build interpolates the host name instead, "failed to
/// connect to codeberg.org: Connection refused"
/// (streams/socket.c:244). Matching the Windows wording alone made
/// every unreachable forge on Linux and macOS read as "Codeberg
/// answered with an error" instead of "No connection to codeberg.org".
const OS_OFFLINE_PREFIXES: [&str; 2] = ["failed to send request", "failed to connect to"];
/// The wait bound joy sets itself surfaces as libgit2's raw EAGAIN,
/// "SSL error: syscall failure: Resource temporarily unavailable"
/// (JOY-0278-85). libgit2 writes the whole literal below itself and
/// sets `GIT_ERROR_OS` (streams/openssl.c:325), so the operating
/// system's `strerror` tail is what follows it and is never matched
/// (D1.8a, step 4). It is silence, not a certificate fault, which is
/// why it is read before the certificate rules.
const SYSCALL_FAILURE_SENTENCE: &str = "ssl error: syscall failure";

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
/// What the oracle said for a host and when it was asked. D2.10 limits
/// the CALL to once per host per strike window, not the verdict: the
/// answer is remembered for the whole window, because the wall that
/// produced the first 403 produces the next one two seconds later and
/// the person may not watch the banner flip from "Your organisation
/// must approve Joy" to "GitHub answered with an error".
static ORACLE_ASKED: Mutex<Option<HashMap<String, OracleMemory>>> = Mutex::new(None);

/// One oracle call: when it was made, and what came back (`None` when
/// the answer could not be had).
struct OracleMemory {
    asked: Instant,
    answer: Option<OracleAnswer>,
}

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

/// The oracle's verdict for this host: asked at most once per host per
/// strike window (D2.10), and the answer of that one call is what every
/// contact inside the window reads.
fn ask_oracle(host: &str, status: u16) -> Option<OracleAnswer> {
    let now = Instant::now();
    {
        let mut guard = ORACLE_ASKED.lock().unwrap_or_else(|e| e.into_inner());
        let asked = guard.get_or_insert_with(HashMap::new);
        if let Some(memory) = asked.get(host) {
            if now.duration_since(memory.asked) < STRIKE_LASTS {
                // inside the window: the remembered answer, including a
                // remembered "it could not be had"
                return memory.answer.clone();
            }
        }
    }
    let oracle = ORACLE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .cloned();
    // without an installed oracle nothing is remembered: J3 may install
    // one between two contacts, and wave 0 ships without it
    let oracle = oracle?;
    let answer = oracle.ask(host, status);
    ORACLE_ASKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(
            host.to_string(),
            OracleMemory {
                asked: now,
                answer: answer.clone(),
            },
        );
    answer
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
    /// The ONE next step, as a BUTTON label of D4.7 ("sign in",
    /// "retry", "show what to do"), or `None` when there is nothing to
    /// do but wait.
    pub next_step: Option<String>,
    /// What is behind that button: the instruction in prose, when the
    /// step needs one (D1.8c, the per OS certificate sentence and the
    /// proxy's Windows integrated authentication case). A surface
    /// renders [`Verdict::next_step`] as the label and this as the
    /// text; it never prints this on a button.
    pub guidance: Option<String>,
    /// The URL the action opens, when the evidence carried one (D1.8b:
    /// for `needs_sso` the `X-GitHub-SSO` header's URL is the action).
    pub action: Option<String>,
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
    let decision = decide(evidence);
    let host = evidence.host.as_str();
    let failure = decision.failure;
    let sentence = decision
        .sentence
        .unwrap_or_else(|| match (failure, decision.wait) {
            // D4.7 writes the number down: "GitHub is rate limiting us,
            // retrying in 15 minutes." The state alone cannot know it,
            // the wait can.
            (Failure::RateLimited, Some(wait)) => rate_limited_sentence(host, wait),
            _ => failure.sentence(host),
        });
    Verdict {
        failure,
        sentence,
        next_step: failure.next_step().map(str::to_string),
        guidance: decision
            .guidance
            .or_else(|| failure.guidance().map(str::to_string)),
        action: None,
        wait: decision.wait,
        detail: format!("libgit2: {}", evidence.error.message()),
    }
}

/// The rate limit sentence with the number the forge or its
/// documentation named (D4.7). Under a minute reads as one minute: a
/// person waiting is told to wait, not given a stopwatch.
fn rate_limited_sentence(host: &str, wait: Duration) -> String {
    let minutes = wait.as_secs().div_ceil(60).max(1);
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    format!(
        "{} is rate limiting us, retrying in {minutes} {unit}.",
        forge_name(host)
    )
}

/// What the classifier decided: the state, plus the parts of the
/// sentence a state alone cannot know (the proxy's own name, the wait,
/// the instruction that differs from the state's usual one). The BUTTON
/// belongs to the state and is never decided here (D4.7).
struct Decision {
    failure: Failure,
    wait: Option<Duration>,
    sentence: Option<String>,
    guidance: Option<String>,
}

/// A state that says everything about itself.
fn plain(failure: Failure) -> Decision {
    Decision {
        failure,
        wait: None,
        sentence: None,
        guidance: None,
    }
}

fn decide(ev: &ContactEvidence) -> Decision {
    use git2::{ErrorClass as Class, ErrorCode as Code};
    let code = ev.error.code();
    let class = ev.error.class();
    let message = ev.error.message().to_ascii_lowercase();
    let family = host_family(&ev.host);

    // ssh first, and by the CODE before the class (D1.8a reads the code
    // first): an ssh host key refusal must not be read as an https
    // certificate problem, and it does not always carry class `Ssh`.
    // libgit2 refuses an unknown host key itself with
    // `GIT_ERROR_SSH`/`GIT_ECERTIFICATE` (ssh_libssh2.c:759-767), but
    // when joy's own `certificate_check` closure refuses (D1.4a, J4h)
    // the class is whatever the closure set, or `GIT_ERROR_NET` when it
    // set nothing at all, while the code stays `GIT_ECERTIFICATE`.
    // Reading the class alone classified joy's own host key refusal as
    // `offline` and told a person with an unknown host key "No
    // connection to github.com." with a retry button.
    if class == Class::Ssh
        || (ev.transport == Transport::Ssh && matches!(code, Code::Auth | Code::Certificate))
    {
        match code {
            Code::Auth => return plain(Failure::NeedsSignIn),
            Code::Certificate => return plain(Failure::NeedsHostTrust),
            // GIT_EEOF and ONLY GIT_EEOF is the row of D1.8b: libgit2
            // read the remote's own stderr and returns GIT_EEOF with it
            // as the message (ssh_libssh2.c:136-140).
            //
            // That the code REACHES this classifier is read off the
            // source, because no test here can make a real forge refuse
            // an ssh push: `ssh_stream_read` returns GIT_EEOF,
            // `git_smart__recv` hands a negative return value back
            // unchanged (smart.c:16-35), the ref advertisement returns
            // it unchanged again (smart_protocol.c:57-59, :290-296),
            // and git2 builds its error from the RETURN code plus
            // `git_error_last` (git2 0.21 error.rs:34-63), so
            // `error.code()` is `Eof` and `error.class()` is `Ssh`.
            // Both a refused fetch and a refused push read the
            // advertisement first, so both arrive here. Every other
            // generic GIT_ERROR_SSH is a transport fault of this
            // machine, not a refusal of this login: "failed to start
            // SSH session" (:578), "unable to initialize libssh2"
            // (:1118), "unable to get the host key" (:748), "error
            // reading known_hosts" (:443, :466). Calling those `denied`
            // told a person their forge refuses them and blocked writes
            // for a key exchange that broke.
            Code::Eof => {
                // the remote's own sentence goes to the detail line,
                // never to the banner
                return match ev.direction {
                    ContactDirection::Push => plain(Failure::NoPushRights),
                    ContactDirection::Fetch => plain(Failure::Denied),
                };
            }
            // fall through to the rules that read the code and the
            // class: a timeout is a timeout on ssh too, and what nothing
            // explains is `error`
            _ => {}
        }
    }

    // The wait bound joy sets itself (JOY-0278-85) arrives as an SSL
    // syscall failure and is not a certificate fault: it is silence.
    if message.contains(SYSCALL_FAILURE_SENTENCE) {
        return plain(Failure::Offline);
    }

    // A proxy that wants a login of its own, before the generic http
    // rules: both sentences are libgit2's own literals.
    if PROXY_AUTH_SENTENCES.iter().any(|s| message.contains(s)) {
        return proxy_auth(ev, &message);
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

fn decide_by_status(ev: &ContactEvidence, family: HostFamily, status: u16) -> Decision {
    match status {
        401 => plain(Failure::NeedsSignIn),
        429 => {
            // D1.8b: the wait comes from the plugin on GitHub only,
            // "otherwise from the strike table". The strike table's
            // number is STRIKE_LASTS, the same window `run` sets when
            // it records the strike, so a 429 on any host can name a
            // number and the sentence of D4.7 reads "Codeberg is rate
            // limiting us, retrying in 10 minutes." instead of
            // "retrying later."
            let oracle_wait = match (family, ev.transport) {
                (HostFamily::GitHub, Transport::Https) => match ask_oracle(&ev.host, status) {
                    Some(OracleAnswer::RateLimited { wait }) => wait,
                    // the oracle answered something else for a 429: the
                    // 429 stands, the strike table names the wait
                    _ => None,
                },
                _ => None,
            };
            Decision {
                failure: Failure::RateLimited,
                wait: Some(oracle_wait.unwrap_or(STRIKE_LASTS)),
                sentence: None,
                guidance: None,
            }
        }
        403 | 404 if ev.direction == ContactDirection::Push && ev.token_worked_before => {
            plain(Failure::NoPushRights)
        }
        403 if ev.direction == ContactDirection::Fetch && ev.token_worked_before => {
            match (family, ev.transport) {
                (HostFamily::GitHub, Transport::Https) => match ask_oracle(&ev.host, status) {
                    Some(OracleAnswer::RateLimited { wait }) => Decision {
                        // an oracle that named no number leaves the
                        // strike table's own window, as for a 429
                        wait: Some(wait.unwrap_or(STRIKE_LASTS)),
                        failure: Failure::RateLimited,
                        sentence: None,
                        guidance: None,
                    },
                    Some(OracleAnswer::NeedsOrgApproval) => plain(Failure::NeedsOrgApproval),
                    Some(OracleAnswer::Denied) => plain(Failure::Denied),
                    // no oracle installed, or it could not answer: joy
                    // does not invent a wall it has no evidence for
                    None => plain(Failure::Error),
                },
                // GitLab's failed authentication ban: it sends no header
                // and cannot be cleared by signing in, so the wait comes
                // from GitLab's own documentation and the connector is
                // NOT asked (D2.10).
                (HostFamily::GitLabCom, _) | (HostFamily::GitLabSelfManaged, _) => Decision {
                    failure: Failure::RateLimited,
                    wait: gitlab_ban_wait(family),
                    sentence: Some(format!(
                        "{} refused this login for a while after too many failed sign ins.",
                        forge_name(&ev.host)
                    )),
                    guidance: None,
                },
                _ => plain(Failure::Error),
            }
        }
        404 if ev.direction == ContactDirection::Fetch => {
            if family == HostFamily::GitHub && ev.token_worked_before {
                plain(Failure::NeedsOrgApproval)
            } else {
                Decision {
                    failure: Failure::Error,
                    wait: None,
                    sentence: Some(format!(
                        "{} does not have this repository (renamed, deleted or not visible to this login)",
                        ev.host
                    )),
                    guidance: None,
                }
            }
        }
        502..=504 => Decision {
            failure: Failure::Offline,
            wait: None,
            sentence: Some(format!("{} is not answering right now", ev.host)),
            guidance: None,
        },
        _ => plain(Failure::Error),
    }
}

/// The two proxy 407 texts of D1.8c, told apart.
///
/// Both mean `proxy_auth`, and both name the PROXY, which is the one
/// machine in the sentence: the forge host has nothing to do with it
/// and naming it produced "The proxy github.com needs a user name and a
/// password."
///
/// The second text means something different per operating system.
/// libgit2 resolves the NTLM and Negotiate schemes to
/// `git_http_auth_dummy` on everything but Windows (auth.c:65-71), so
/// off Windows it means the proxy offered nothing but Windows
/// integrated authentication and no password joy could ask for would
/// help. Telling that person to "sign in to the proxy" sends them at
/// the one door that cannot open.
fn proxy_auth(ev: &ContactEvidence, message: &str) -> Decision {
    let sentence = ev
        .proxy
        .as_ref()
        .map(|proxy| format!("The proxy {proxy} needs a user name and a password."));
    let unsupported = message.contains(PROXY_AUTH_SENTENCES[1]);
    let guidance = if unsupported && !cfg!(windows) {
        Some(format!(
            "this proxy requires Windows integrated authentication, which joy cannot do on this system; ask for a proxy password or a bypass rule for {}",
            ev.host
        ))
    } else {
        // the alternative of D1.8c, on the detail line and only when
        // joy knows the proxy's own name: inventing one would send a
        // person at a machine that does not exist
        ev.proxy
            .as_ref()
            .map(|proxy| format!("or set http.proxy to http://user@{proxy}"))
    };
    Decision {
        failure: Failure::ProxyAuth,
        wait: None,
        sentence,
        guidance,
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

/// [`classify_plugin`] with the words and the action, so a connector
/// answer reaches a surface the same way a libgit2 failure does. The
/// `needs_sso` row of D1.8b says the `X-GitHub-SSO` header's URL IS the
/// action, so it is carried here and not thrown away; `host` names the
/// forge the connector answered for, which is what the sentences of
/// D4.7 put in front of "connector".
pub fn plugin_verdict(evidence: &PluginEvidence, host: &str) -> Verdict {
    let failure = classify_plugin(evidence);
    Verdict {
        failure,
        sentence: failure.sentence(host),
        next_step: failure.next_step().map(str::to_string),
        guidance: failure.guidance().map(str::to_string),
        action: match evidence {
            PluginEvidence::NeedsSso { url } => url.clone(),
            _ => None,
        },
        wait: None,
        // no libgit2 was involved: the connector's answer is the whole
        // evidence, and it is already said in plain words
        detail: String::new(),
    }
}

/// One connector answer as the error joy carries upwards, the way
/// [`failed`] does it for a libgit2 failure.
pub fn plugin_failed(evidence: &PluginEvidence, host: &str) -> anyhow::Error {
    error_of(plugin_verdict(evidence, host))
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
    error_of(verdict(evidence))
}

/// One verdict as the error joy carries upwards. The sentence and the
/// button are what a person reads; the instruction behind the button
/// joins libgit2's own words on the detail line, which is where D1.8c
/// puts the proxy's alternative and where a surface that cannot render
/// a button finds the instruction at all.
fn error_of(verdict: Verdict) -> anyhow::Error {
    let message = match &verdict.next_step {
        Some(step) => format!("{} ({step})", verdict.sentence),
        None => verdict.sentence.clone(),
    };
    let detail = match &verdict.guidance {
        Some(guidance) if verdict.detail.is_empty() => guidance.clone(),
        Some(guidance) => format!("{guidance}; {}", verdict.detail),
        None => verdict.detail.clone(),
    };
    anyhow::Error::new(ContactError {
        failure: verdict.failure,
        message,
        detail: (!detail.is_empty()).then_some(detail),
        next_try: verdict.wait.map(|w| SystemTime::now() + w),
    })
}

/// A local fault of the git engine, carried TYPED so the contact
/// boundary can tell libgit2's own text from joy's own sentences
/// (D1.8b: the raw libgit2 text never becomes the sentence a person
/// reads for a failed contact).
///
/// Outside a contact - reading a checkout, writing an index, resolving
/// a ref - it is still what a developer needs and it displays
/// unchanged. Inside one, [`run`] moves it to the detail line and the
/// state provides the sentence.
#[derive(Debug)]
pub struct EngineFault {
    message: String,
}

/// The fault of one libgit2 call on local data, in libgit2's own words
/// and under joy's own prefix ("git: reference not found").
pub fn engine_fault(what: &str, e: &git2::Error) -> anyhow::Error {
    anyhow::Error::new(EngineFault {
        message: format!("{what}: {}", e.message()),
    })
}

impl std::fmt::Display for EngineFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineFault {}

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
    // budget would allow. What "signed in" means is not what the caller
    // hopes but what joy really handed over here
    // ([`credential_answers`]).
    if transport == Transport::Https && !credential_answers(host, credentialed) {
        return ANONYMOUS_POLL_INTERVAL;
    }
    let cost = gap_for(host) * requests(verb, transport, credentialed) * projects.max(1);
    let seconds = (cost.as_millis() as u64).div_ceil(1000);
    // never zero: a remote on this machine costs no budget, and the
    // platform may set a host's gap to 0 through JOYINT_FORGE_MIN_GAP_MS.
    // A5 and P2 use this number and no number of their own, so a zero
    // here would be their busy loop.
    Duration::from_secs(seconds.max(1))
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

/// The gate for an anonymous poll, ASKED: `None` when the poll may go
/// out, `Some(next try)` when it may not. It takes nothing; the slot is
/// taken by [`note_anonymous_poll`] once the poll has really been made,
/// so a poll that never reached the forge (the host was down) does not
/// burn a fifteen minute window it never used.
///
/// A person's own command is not a poll and never asks this, which is
/// why [`run`] does not charge it and [`run_poll`] does.
pub fn anonymous_poll_due(host: &str) -> Option<SystemTime> {
    let now = Instant::now();
    let mut guard = ANONYMOUS_POLLS.lock().unwrap_or_else(|e| e.into_inner());
    let polls = guard.get_or_insert_with(HashMap::new);
    let last = polls.get(host)?;
    let waited = now.duration_since(*last);
    (waited < ANONYMOUS_POLL_INTERVAL)
        .then(|| SystemTime::now() + (ANONYMOUS_POLL_INTERVAL - waited))
}

/// Take this host's anonymous poll slot: the next one waits the fifteen
/// minutes of D1.9.
pub fn note_anonymous_poll(host: &str) {
    ANONYMOUS_POLLS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(host.to_string(), Instant::now());
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

/// What one turn at the throttle reserved. The reservation for the NEXT
/// contact is made on the way out, with the gap that was in force at
/// that moment, so a strike recorded afterwards has to widen it
/// ([`widen_for_strike`]).
struct Turn {
    /// how long this contact waited in line
    waited: Duration,
    /// the moment this contact was let go, which is where the next
    /// host's slot is measured from
    start: Instant,
    /// what this verb costs on this host BEFORE the strike doubling
    base_gap: Duration,
}

/// Wait for this host's turn: the gap the verb's requests cost, doubled
/// once per standing strike. Takes the slot on the way out, so
/// concurrent callers line up one behind the other instead of leaving
/// together.
fn take_turn(host: &str, verb: &str, transport: Transport, credentialed: bool) -> Turn {
    let base_gap = gap_for(host) * requests(verb, transport, credentialed);
    // A forge that said 429 is never stopped, only slowed (Horst,
    // 2026-08-29): `strikes` is the exponent it is documented to be.
    let gap = base_gap * strike_factor(strikes_for(host));
    if gap.is_zero() {
        return Turn {
            waited: Duration::ZERO,
            start: Instant::now(),
            base_gap,
        };
    }
    let (waited, start) = with_throttle(|t| {
        let now = Instant::now();
        let free = t.next_free.get(host).copied().unwrap_or(now);
        let start = free.max(now);
        t.next_free.insert(host.to_string(), start + gap);
        (start.saturating_duration_since(now), start)
    });
    if !waited.is_zero() {
        std::thread::sleep(waited);
    }
    Turn {
        waited,
        start,
        base_gap,
    }
}

/// How much wider the gap is after `strikes` strikes: the exponent of
/// D1.9, capped.
fn strike_factor(strikes: u32) -> u32 {
    1u32 << strikes.min(MAX_STRIKE_EXPONENT)
}

/// Push the reservation this contact made forward, because the contact
/// came back with a 429.
///
/// D1.9 and J5's acceptance criterion say "after a 429 the NEXT contact
/// to that host waits at least twice the gap". The slot for that next
/// contact was already reserved by [`take_turn`] on the way out, with
/// the gap that was in force before the forge said 429, so recording
/// the strike alone reached only the contact after next: the very
/// contact that runs straight back into the limit went out on the old
/// gap.
fn widen_for_strike(host: &str, turn: &Turn) {
    if turn.base_gap.is_zero() {
        return;
    }
    let free = turn.start + turn.base_gap * strike_factor(strikes_for(host));
    with_throttle(|t| {
        let entry = t.next_free.entry(host.to_string()).or_insert(free);
        if *entry < free {
            *entry = free;
        }
    });
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

/// Record a 429 (or a documented ban) and return when the strike window
/// ends. The window is always [`STRIKE_LASTS`] (D1.9: "the gap is
/// `gap * 2^strikes` capped, until `STRIKE_LASTS` (600 s) has
/// elapsed"). The forge's own retry-after is a different number and
/// belongs to the caller's next try, never to the doubling: a
/// retry-after of a few seconds, which is what J3's oracle hands back,
/// would otherwise end the doubling before it did anything.
fn strike(host: &str) -> SystemTime {
    let now = Instant::now();
    let lasts = STRIKE_LASTS;
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

thread_local! {
    /// Whether the credential callback of THIS contact handed libgit2 a
    /// credential. libgit2 calls the callback on the contact's own
    /// thread, synchronously, so a thread local is exactly the scope of
    /// one contact.
    static PRESENTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The credential callback says it handed something over (the engine
/// calls this; J4b's resolver keeps calling it). Without it a contact
/// that ended in `Cred::default()`, which is "I have nothing", would be
/// remembered as a credential that worked, and the next 404 on
/// github.com would read as "Your organisation must approve Joy" for a
/// repository that simply does not exist.
pub fn note_credential_presented() {
    PRESENTED.with(|p| p.set(true));
}

/// Read and clear the flag above.
fn take_credential_presented() -> bool {
    PRESENTED.with(|p| p.replace(false))
}

/// Whether joy has ever really handed a credential to this host in this
/// process. `None` until a contact to the host has been made.
static CREDENTIAL_PRESENTED: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// Whether joy has anything to present to this host.
///
/// `claimed` is what the CALLER says its `Auth` carries, and on the
/// desktop that is a hope rather than a fact: `Auth::Local` is "the
/// machine's own credentials" and claims a credential for every host,
/// while a machine with no helper entry and no agent key for this host
/// presents nothing at all (the callback ends in `Cred::default()`,
/// which is "I have nothing"). Keying the no anonymous polling rule of
/// D1.9 on the claim alone meant the rule never fired on the desktop
/// and an https remote nobody is signed in for was polled every two
/// seconds.
///
/// So the claim is overruled by what the credential callbacks really
/// handed over here ([`note_credential_presented`]), which [`run`]
/// records per host. Until a contact has been made there is nothing to
/// overrule it with, so the first contact goes out on the claim and
/// teaches this memory what it is worth.
pub fn credential_answers(host: &str, claimed: bool) -> bool {
    claimed
        && CREDENTIAL_PRESENTED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(host).copied())
            .unwrap_or(true)
}

fn note_credential_answer(host: &str, presented: bool) {
    if host.is_empty() {
        return;
    }
    CREDENTIAL_PRESENTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(host.to_string(), presented);
}

#[cfg(test)]
pub(crate) fn reset_credential_memory() {
    CREDENTIAL_PRESENTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .clear();
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
    let turn = take_turn(&host, verb, transport, credentialed);
    if !turn.waited.is_zero() {
        tracing::debug!(
            waited_ms = turn.waited.as_millis() as u64,
            requests = requests(verb, transport, credentialed),
            "forge contact throttled"
        );
    }
    let started = Instant::now();
    take_credential_presented();
    let outcome = work();
    let presented = take_credential_presented();
    match outcome {
        Ok(value) => {
            // a contact that never had to present anything (a public
            // repository answers the first request) proves nothing about
            // a credential
            if credentialed && presented {
                note_credential_worked(&host);
            }
            // but it does prove what joy has for this host: the forge
            // answered and joy handed nothing over, which is what the
            // no anonymous polling rule of D1.9 needs to know
            note_credential_answer(&host, presented);
            tracing::debug!(
                took_ms = started.elapsed().as_millis() as u64,
                "forge contact ok"
            );
            Ok(value)
        }
        Err(e) => {
            let failure = failure_of(&e);
            // what joy really had for this host: a credential it
            // presented, or - when the host answered by ASKING for one
            // and joy had nothing - the proof that it has none. Every
            // other failure (nobody answered, a certificate, a local
            // fault) says nothing either way and leaves the memory
            // alone, so a network outage never makes joy believe it is
            // signed out.
            if presented {
                note_credential_answer(&host, true);
            } else if failure == Failure::NeedsSignIn {
                note_credential_answer(&host, false);
            }
            let next_try = match failure {
                Failure::RateLimited => {
                    // the strike window paces the next contacts to this
                    // host; the forge's own number, when it named one,
                    // is what this caller is told to wait
                    let window_ends = strike(&host);
                    // and the NEXT contact pays the doubled gap at
                    // once: its slot was reserved on the way out, with
                    // the gap that was in force before the 429
                    widen_for_strike(&host, &turn);
                    Some(next_try_of(&e).unwrap_or(window_ends))
                }
                _ => next_try_of(&e),
            };
            if failure == Failure::RateLimited {
                // the sentence an operator reads on the board (JP-00F7-31)
                tracing::error!(
                    forge = %host,
                    slowed_until_unix = next_try.map(unix).unwrap_or(0),
                    "{host} is limiting this instance: every contact to it is spaced 2^strikes wider (up to 64 times the host's gap) while the strike stands, which is ten minutes after the last one; lower JOYINT_FORGE_MIN_GAP_MS's rate for this host if it repeats"
                );
            }
            tracing::error!(
                outcome = failure.reason(),
                took_ms = started.elapsed().as_millis() as u64,
                error = %e,
                detail = detail_of(&e).unwrap_or_default(),
                "forge contact failed"
            );
            // An error that never passed the classifier keeps its own
            // sentence, because joy wrote it and it is a plain one
            // ("branch x not found on the forge"). The one exception is
            // libgit2's own text, which arrives typed as an
            // [`EngineFault`] and never becomes the person-facing
            // sentence of a contact (D1.8b, wording rules): it moves to
            // the detail line and the state says the rest.
            let (message, detail) = match e.downcast_ref::<EngineFault>() {
                Some(fault) => (
                    format!("The {verb} could not be completed in this checkout."),
                    Some(fault.to_string()),
                ),
                None => (e.to_string(), detail_of(&e)),
            };
            Err(anyhow::Error::new(ContactError {
                failure,
                message,
                detail,
                next_try,
            }))
        }
    }
}

/// [`run`] for a POLL: a contact nobody asked for, made by a loop that
/// watches a forge. It is the one contact the no anonymous polling rule
/// of D1.9 holds back: an https remote with no credential is polled at
/// most once every fifteen minutes per host, whatever the budget would
/// allow, and the refusal carries the sentence that says why
/// ([`anonymous_poll_reason`]) so the surface is never silent about it.
///
/// A person's own command goes through [`run`] and is never held.
pub fn run_poll<T>(
    url: &str,
    verb: &'static str,
    credentialed: bool,
    work: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let host = host_of(url);
    let anonymous = |host: &str| {
        transport_of(url) == Transport::Https && !credential_answers(host, credentialed)
    };
    if anonymous(&host) {
        if let Some(next_try) = anonymous_poll_due(&host) {
            tracing::debug!(forge = %host, "anonymous poll held back (D1.9)");
            return Err(anyhow::Error::new(ContactError {
                // a held poll is a WAIT joy imposes on itself, not a
                // refusal by the forge. `rate_limited` is the word for
                // that, and it is one of the two words the pre-NG
                // reader already knows (D1.8b, createForgeSync.ts:98-118):
                // `needs_sign_in` reads as "denied" there and would
                // have blocked writes for a poll that was merely
                // throttled.
                failure: Failure::RateLimited,
                message: anonymous_poll_reason(&host),
                detail: None,
                next_try: Some(next_try),
            }));
        }
    }
    let result = run(url, verb, credentialed, work);
    // The slot is taken AFTER the poll, and only for a poll that really
    // went out with nothing to present and really reached the forge. A
    // poll that found nobody home burnt no window, and a poll that did
    // present a credential never rode this gate at all. `run` has just
    // taught `credential_answers` what this host is worth, so this
    // second look is the honest one.
    let reached = !matches!(&result, Err(e) if failure_of(e) == Failure::Offline);
    if reached && anonymous(&host) {
        note_anonymous_poll(&host);
    }
    result
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

/// [`run_poll`] for a checkout: the forge is the checkout's remote.
pub fn run_poll_for<T>(
    repo_dir: &Path,
    verb: &'static str,
    credentialed: bool,
    work: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let url = super::forge::remote_url(repo_dir).unwrap_or_default();
    run_poll(&url, verb, credentialed, work)
}

#[cfg(test)]
mod tests;
