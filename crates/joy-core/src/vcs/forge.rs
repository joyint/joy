// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The one headless git engine for forge sync (JOY-0265-D7), over git2
//! (JP-0034-A2: one git engine everywhere; gix cannot push). Grown from
//! the platform's runtime-checkout layer and shared by the platform, the
//! desktop app, and everything else that syncs a checkout with a forge.
//!
//! Two mechanics live under the one vcs roof, each with its reason: the
//! CLI verbs in [`super`] run the git BINARY because user hooks and user
//! config must fire; everything here is HEADLESS work (server worker,
//! app worker) that must never fire hooks and authenticates with tokens
//! or the machine's stored credentials.
//!
//! FETCH_HEAD is never written or read here. It is the one file git
//! updates without a lock, and sharing it tore syncs apart twice: a
//! chats fetch of a ref the forge does not have left it EMPTY (read as
//! "corrupted loose reference", JP-00DB-61), and an app worker racing
//! the user's own `git pull` produced "Cannot rebase onto multiple
//! branches" (JAPP-0198-EA). Fetches update the remote-tracking ref
//! only, and fast-forwards read THAT.

use std::path::{Path, PathBuf};

use crate::host::HostKind;

/// The forge family a host belongs to, as far as authentication goes
/// (design D1.6). It comes from the plugin that claims the host
/// (`claims`), and from the engine's own table for the three hosts
/// every joy knows when no plugin claimed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeKind {
    GitHub,
    /// GitHub Enterprise Server. The same shape as github.com on a
    /// host name that says nothing about it, which is why the claim
    /// matters: without it the first attempt is a guess.
    GitHubEnterprise,
    GitLab,
    /// Gitea, Forgejo and Codeberg: one implementation, one shape.
    Gitea,
}

impl ForgeKind {
    /// The kind a forge plugin's id names (`forge_plugins::FORGE_PLUGINS`).
    pub fn from_plugin_id(id: &str) -> Option<ForgeKind> {
        match id.trim().to_ascii_lowercase().as_str() {
            "github" => Some(ForgeKind::GitHub),
            "github-enterprise" | "ghes" => Some(ForgeKind::GitHubEnterprise),
            "gitlab" => Some(ForgeKind::GitLab),
            "gitea" | "forgejo" | "codeberg" => Some(ForgeKind::Gitea),
            _ => None,
        }
    }

    /// The user name this forge expects beside an access token.
    ///
    /// Every forge takes the token as the PASSWORD; each one wants its
    /// own name in front of it. GitHub documents `x-access-token`,
    /// GitLab requires `oauth2` (doc/api/oauth2.md:409-417), and the
    /// Gitea family takes the token as the password under any name and
    /// is given `oauth2` too (forgejo services/auth/method/util.go:60-67).
    pub fn token_user(self) -> &'static str {
        match self {
            ForgeKind::GitHub | ForgeKind::GitHubEnterprise => "x-access-token",
            ForgeKind::GitLab | ForgeKind::Gitea => "oauth2",
        }
    }
}

/// The engine's own table, for github.com, gitlab.com and codeberg.org
/// and for nothing else (design D1.1, D1.5: "Engine fallback table for
/// github.com, gitlab.com and codeberg.org only").
///
/// There is no guess by name here, and that is the point of D1.6. A
/// substring rule would classify a Gitea reachable at
/// `github.internal.example` as GitHub Enterprise, send it
/// `x-access-token` and, because the second shape is offered only to a
/// host no table knows, never offer it `oauth2` either: a valid token
/// would look refused. That is exactly what the deleted
/// `basic_auth_user` did with `lower.contains("gitlab")`. A
/// self-hosted host is identified by the plugin that claims it (D2
/// `claims`), never by its name.
///
/// The sub-domain form is part of the table because the two public
/// forges answer ssh under one: ssh.github.com is github.com and
/// altssh.gitlab.com is gitlab.com (D1.5).
pub fn known_forge_kind(host: &str) -> Option<ForgeKind> {
    let host = host.trim().to_ascii_lowercase();
    let host = host.split(':').next().unwrap_or(&host);
    let is = |name: &str| host == name || host.ends_with(&format!(".{name}"));
    if is("github.com") {
        return Some(ForgeKind::GitHub);
    }
    if is("gitlab.com") {
        return Some(ForgeKind::GitLab);
    }
    if is("codeberg.org") {
        return Some(ForgeKind::Gitea);
    }
    None
}

/// The user name to send beside an access token for this host.
///
/// The shape "token as the user name with an EMPTY password" is never
/// sent (design D1.6). It is what joy sent to Codeberg and to every
/// unknown host until now; on GitLab it always fails and counts
/// towards the failed-authentication ban, and on Codeberg it produced
/// "server requires authentication that we do not support" for a token
/// that would have been accepted (JP-00D8-94). An unknown host gets
/// `oauth2`, the name GitLab requires, the Gitea family accepts and
/// GitHub ignores.
pub fn token_user(host: &str, claimed: Option<ForgeKind>) -> &'static str {
    claimed
        .or_else(|| known_forge_kind(host))
        .map(ForgeKind::token_user)
        .unwrap_or("oauth2")
}

/// How this checkout talks to its forge.
///
/// The platform authenticates with the account's OAuth token; the
/// desktop app with whatever the person's machine holds (ssh-agent,
/// credential helper). One enum instead of two engines, so every
/// caller gets the same retry discipline and the same honest errors.
///
/// Two of the four variants carry a host fact the other two default:
/// the forge kind a plugin claimed (design D1.6) and the host kind
/// (design D1.1). They are separate variants and not fields so that
/// every caller written before the resolver keeps compiling and keeps
/// working, under the quiet defaults.
pub enum Auth {
    /// A forge access token (platform), for a host whose forge kind
    /// joy reads from the host name.
    Token(String),
    /// A forge access token for a host whose forge kind a plugin
    /// claimed: the credential has the right shape on the FIRST
    /// attempt, which is what a GitHub Enterprise Server host whose
    /// name says nothing needs.
    ClaimedToken(String, ForgeKind),
    /// The machine's own credentials (desktop), for a host that has
    /// not said who it is. Defaults to [`HostKind::Background`], the
    /// quiet one: a host that never named its kind must not be handed
    /// a prompt nobody will answer.
    Local,
    /// The machine's own credentials for a host that named its kind
    /// (design D1.1): the whole prompt rule of design D1.10 hangs off
    /// this word.
    LocalAs(HostKind),
}

/// How long a forge contact waits on a silent forge, set once for the
/// process (JP-0115-EC). Codeberg's own proxy answered 504 after 30
/// seconds of silence on 2026-09-03, and until then libgit2 waited with
/// it; every contact in the loop paid the full thirty. A forge that has
/// not accepted the connection after ten seconds, or has sent nothing for
/// fifteen, is not answering; a slow but living transfer keeps sending
/// and is not touched by either bound. Both are socket bounds, not a cap
/// on the transfer as a whole.
const CONNECT_BOUND_MS: i32 = 10_000;
const SILENCE_BOUND_MS: i32 = 15_000;

fn bound_forge_waits() {
    static BOUND: std::sync::Once = std::sync::Once::new();
    BOUND.call_once(|| {
        // SAFETY: libgit2 global options, set before any remote is opened
        // and never changed afterwards; the Once serialises the one call.
        unsafe {
            if let Err(e) = git2::opts::set_server_connect_timeout_in_milliseconds(CONNECT_BOUND_MS)
            {
                tracing::debug!(error = %e, "forge connect timeout not set");
            }
            if let Err(e) = git2::opts::set_server_timeout_in_milliseconds(SILENCE_BOUND_MS) {
                tracing::debug!(error = %e, "forge socket timeout not set");
            }
        }
    });
}

/// What a forge contact's libgit2 error means, for the person
/// (JOY-0295-36, design D1.8a). The error is handed to the classifier
/// WHOLE - code, class, status number and message - because joy used to
/// pass `e.message()` alone and prefix "(offline?)" onto it, which made
/// a 404 over https read as "no connection to github.com". The plain
/// sentence comes back from the state; libgit2's own words go to the
/// detail line and never to a surface.
fn contact_failed(
    url: &str,
    auth: &Auth,
    direction: super::contact::ContactDirection,
    e: git2::Error,
) -> anyhow::Error {
    let transport = super::contact::transport_of(url);
    let mut evidence = super::contact::ContactEvidence::new(
        e,
        url,
        direction,
        auth.credential_evidence(transport),
    );
    // The proxy THIS contact really went through (D1.8c): a 407 names
    // the proxy and never the forge, and the name is joy's own
    // decision, so it travels in the cell `proxy::options_for` filled
    // rather than through fifteen call sites.
    if let Some(proxy) = super::proxy::current() {
        evidence = evidence.through_proxy(proxy);
    }
    super::contact::failed(&evidence)
}

/// THE proxy options of one contact (D1.11), with the refusal of a
/// proxy joy cannot speak to turned into the failure the caller
/// returns. No socket is opened for a refused proxy.
fn proxy_for(url: &str, repo: Option<&git2::Repository>) -> anyhow::Result<super::proxy::Proxy> {
    super::proxy::options_for(url, repo).map_err(|refused| anyhow::anyhow!("{refused}"))
}

/// The remote URL a contact travels over, for the evidence above.
fn remote_url_of(remote: &git2::Remote<'_>) -> String {
    remote.url().unwrap_or_default().to_string()
}

/// Note a credential the callback really handed to libgit2, and hand it
/// on unchanged. A callback arm that ends in `Cred::default()` (which
/// is "I have nothing to offer") notes nothing, so joy never remembers
/// an anonymous contact as a credential that worked (D1.8b: it is what
/// tells a 404 that means "no such repository" from a 404 that means
/// "your organisation has not approved Joy").
fn presented(cred: Result<git2::Cred, git2::Error>) -> Result<git2::Cred, git2::Error> {
    if cred.is_ok() {
        super::contact::note_credential_presented();
    }
    cred
}

impl Auth {
    /// A token for a host joy identifies by its name.
    pub fn token(token: impl Into<String>) -> Self {
        Auth::Token(token.into())
    }

    /// Whether a credential rides with a contact under this auth, which
    /// is what decides its request weight (D1.9): a credentialed
    /// request to a private repository is answered 401 once and
    /// replayed, so it costs one request more than an anonymous one.
    fn credentialed(&self) -> bool {
        match self {
            Auth::Token(token) | Auth::ClaimedToken(token, _) => !token.is_empty(),
            Auth::Local | Auth::LocalAs(_) => true,
        }
    }

    /// What joy presented, for the evidence of D1.8a. This is the
    /// coarse answer the engine can give before a contact is made; what
    /// the chain below really handed over is noted per contact through
    /// [`presented`], and J4b refines this to the source that actually
    /// answered the callback.
    fn credential_evidence(
        &self,
        transport: super::contact::Transport,
    ) -> super::contact::CredentialSource {
        use super::contact::CredentialSource;
        match self {
            Auth::Token(token) | Auth::ClaimedToken(token, _) if !token.is_empty() => {
                CredentialSource::TokenPresented
            }
            Auth::Token(_) | Auth::ClaimedToken(..) => CredentialSource::NonePresented,
            Auth::Local | Auth::LocalAs(_) => match transport {
                super::contact::Transport::Ssh => CredentialSource::AgentPresented,
                _ => CredentialSource::HelperPresented,
            },
        }
    }

    /// A token for a host a plugin claimed, whose shape is therefore
    /// right on the first attempt.
    pub fn token_for(token: impl Into<String>, forge: ForgeKind) -> Self {
        Auth::ClaimedToken(token.into(), forge)
    }

    /// The machine's own credentials, for a host that names its kind.
    pub fn local(kind: HostKind) -> Self {
        Auth::LocalAs(kind)
    }

    /// Who is at the other end (design D1.1). A token host is the
    /// platform or a worker, which is never asked anything.
    pub fn host_kind(&self) -> HostKind {
        match self {
            Auth::Token(_) | Auth::ClaimedToken(..) => HostKind::Background,
            Auth::Local => HostKind::default(),
            Auth::LocalAs(kind) => *kind,
        }
    }

    /// The forge kind a plugin claimed for this host, if any.
    fn claimed_kind(&self) -> Option<ForgeKind> {
        match self {
            Auth::ClaimedToken(_, forge) => Some(*forge),
            _ => None,
        }
    }

    /// The credentials callback for this host, as the closure itself.
    ///
    /// It is built here and not inside [`Auth::callbacks`] so that it
    /// can be driven the way libgit2 drives it: git2 0.21 keeps the
    /// installed callback in a private field
    /// (remote_callbacks.rs:20-29), so a `RemoteCallbacks` cannot be
    /// asked what it would answer, and the shape decision of D1.6
    /// would have no test.
    fn credential_source(
        &self,
        source: CredSource,
    ) -> impl FnMut(&str, Option<&str>, git2::CredentialType) -> Result<git2::Cred, git2::Error> + 'static
    {
        let kind = self.host_kind();
        let token = match self {
            Auth::Token(token) | Auth::ClaimedToken(token, _) => {
                Some(token.clone()).filter(|t| !t.is_empty())
            }
            Auth::Local | Auth::LocalAs(_) => None,
        };
        let claimed = self.claimed_kind();
        // The same resolver for both: for a token host it serves the
        // two cases a token cannot, a remote an insteadOf rule rewrote
        // to ssh and a token the forge refused.
        let mut chain = LocalChain::new(kind);
        let mut attempts = 0u32;
        let mut token_refused = false;
        move |url: &str, username: Option<&str>, allowed: git2::CredentialType| {
            // Design D1.6: honour the `allowed` mask. Without this, an
            // insteadOf rewrite to ssh answers "authentication
            // callback returned unsupported credentials type"
            // (ssh_libssh2.c:415-418) and the person is told the token
            // is wrong.
            let Some(token) = token
                .as_deref()
                .filter(|_| allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT))
            else {
                return chain.credential(url, username, allowed, &source);
            };
            attempts += 1;
            let host = host_of_url(url);
            match attempts {
                1 => presented(git2::Cred::userpass_plaintext(
                    token_user(&host, claimed),
                    token,
                )),
                // Only for a host nobody claimed and no table knows: a
                // self-hosted forge joy has not met. Never the
                // empty-password shape (D1.6).
                2 if claimed.is_none() && known_forge_kind(&host).is_none() => presented(
                    git2::Cred::userpass_plaintext(other_token_user(&host), token),
                ),
                _ => {
                    if !token_refused {
                        token_refused = true;
                        // The chain's sentence would otherwise name
                        // only what came after the token.
                        chain.note(format!(
                            "{host} refused the access token (sent as {})",
                            token_user(&host, claimed)
                        ));
                    }
                    chain.credential(url, username, allowed, &source)
                }
            }
        }
    }

    /// THE callbacks of one contact. Both slots are filled here and
    /// nowhere else: the credential resolver of D1.1, and the ONE
    /// `certificate_check` closure of D1.4a, which git2 0.21 holds one
    /// of per contact (remote_callbacks.rs:27) and which decides the
    /// ssh host key and lets libgit2 decide the TLS chain.
    fn callbacks(&self, source: CredSource) -> git2::RemoteCallbacks<'static> {
        bound_forge_waits();
        let mut callbacks = git2::RemoteCallbacks::new();
        // built before the resolver takes `source`: both read the
        // remote URL as the person configured it, which is what carries
        // the port and the `Host` alias (D1.4)
        let trust = super::certificates::check(self.host_kind(), source.configured.clone());
        callbacks.credentials(self.credential_source(source));
        callbacks.certificate_check(trust);
        callbacks
    }
}

/// The other of the two names, for the one host joy knows nothing
/// about. The empty-password shape is not one of the two.
fn other_token_user(host: &str) -> &'static str {
    match token_user(host, None) {
        "oauth2" => "x-access-token",
        _ => "oauth2",
    }
}

fn host_of_url(url: &str) -> String {
    super::remote_url::RemoteUrl::parse(url)
        .map(|parsed| parsed.host)
        .unwrap_or_default()
}

/// One candidate the resolver offers libgit2, in the order of design
/// D1.2.
enum Step {
    Agent,
    Key {
        path: PathBuf,
        public: Option<PathBuf>,
        passphrase: Option<String>,
    },
    Helper,
}

/// What was handed over last. libgit2 re-enters the credentials
/// callback while the answer is `GIT_EAUTH` (ssh_libssh2.c:855-880),
/// so a second call IS the refusal of the first answer, and the only
/// place joy can see one from inside the callback.
enum Presented {
    Agent,
    Key(PathBuf),
    Helper,
}

/// The resolver of design D1.1 for one contact: built on the first
/// callback invocation from the URL libgit2 is really contacting
/// (after insteadOf and after a redirect), then walked one candidate
/// per re-entry. There is no three-attempt cap any more, only the
/// length of the chain.
struct LocalChain {
    kind: HostKind,
    /// Notes from before the chain was built (the token the forge
    /// refused), so that the one sentence at the end names everything
    /// that was tried and not only the ssh and helper half.
    prelude: Vec<String>,
    state: Option<ChainState>,
}

struct ChainState {
    kind: HostKind,
    host: String,
    /// The user name, decided once and never changed: libgit2 keeps
    /// the first one for the whole contact (ssh_libssh2.c:865).
    user: String,
    steps: std::collections::VecDeque<Step>,
    /// Why each missing candidate is missing, in the person's words.
    notes: Vec<String>,
    presented: Option<Presented>,
    usernames: u32,
    defaulted: bool,
    /// `SSH_AUTH_SOCK` pointed at this host's `IdentityAgent`, for as
    /// long as this contact lasts. Dropped with the chain, which is
    /// dropped with the callbacks of this contact, so the socket of a
    /// `Host work` does not follow every later github.com contact of
    /// the same desktop process (design D1.4).
    _agent: super::ssh_config::AgentScope,
}

impl LocalChain {
    fn new(kind: HostKind) -> LocalChain {
        LocalChain {
            kind,
            prelude: Vec::new(),
            state: None,
        }
    }

    /// Add something the person should read in the final sentence.
    fn note(&mut self, note: String) {
        match &mut self.state {
            Some(state) => state.notes.push(note),
            None => self.prelude.push(note),
        }
    }

    fn credential(
        &mut self,
        url: &str,
        username_from_url: Option<&str>,
        allowed: git2::CredentialType,
        source: &CredSource,
    ) -> Result<git2::Cred, git2::Error> {
        if self.state.is_none() {
            let mut state = ChainState::prepare(
                url,
                username_from_url,
                self.kind,
                source.configured.as_deref(),
            );
            let mut notes = std::mem::take(&mut self.prelude);
            notes.append(&mut state.notes);
            state.notes = notes;
            self.state = Some(state);
        }
        let state = self.state.as_mut().expect("the chain was just prepared");
        if allowed.contains(git2::CredentialType::USERNAME) {
            return state.user_name();
        }
        state.note_refusal(url);
        loop {
            let Some(step) = state.take_next(allowed) else {
                if !state.defaulted && allowed.contains(git2::CredentialType::DEFAULT) {
                    state.defaulted = true;
                    // nothing to offer: NOT a credential, so it is not
                    // noted as one (D1.8b, [`presented`])
                    return git2::Cred::default();
                }
                return Err(git2::Error::from_str(&state.exhausted()));
            };
            match step {
                Step::Agent => {
                    state.presented = Some(Presented::Agent);
                    return presented(git2::Cred::ssh_key_from_agent(&state.user));
                }
                Step::Key {
                    path,
                    public,
                    passphrase,
                } => {
                    state.presented = Some(Presented::Key(path.clone()));
                    return presented(git2::Cred::ssh_key(
                        &state.user,
                        public.as_deref(),
                        &path,
                        passphrase.as_deref(),
                    ));
                }
                Step::Helper => match source.config.as_ref() {
                    Some(config) => {
                        match super::credential_helper::get(
                            config,
                            url,
                            username_from_url,
                            state.kind,
                        ) {
                            Ok(Some(credential)) => {
                                state.presented = Some(Presented::Helper);
                                return presented(git2::Cred::userpass_plaintext(
                                    &credential.username,
                                    &credential.password,
                                ));
                            }
                            Ok(None) => state.notes.push(format!(
                                "no credential helper is configured for {}",
                                state.host
                            )),
                            Err(failure) => state.notes.push(failure.to_string()),
                        }
                    }
                    None => state.notes.push(
                        "there is no git configuration to read a credential helper from"
                            .to_string(),
                    ),
                },
            }
        }
    }
}

impl ChainState {
    fn prepare(
        url: &str,
        username_from_url: Option<&str>,
        kind: HostKind,
        configured: Option<&str>,
    ) -> ChainState {
        let parsed = super::remote_url::RemoteUrl::parse(url);
        let host = parsed
            .as_ref()
            .map(|p| p.host.clone())
            .unwrap_or_else(|| url.to_string());
        let url_user = username_from_url
            .map(str::to_string)
            .or_else(|| parsed.as_ref().and_then(|p| p.user.clone()));
        let mut notes = Vec::new();
        let mut steps = std::collections::VecDeque::new();
        let mut user = url_user.clone().unwrap_or_else(|| "git".to_string());
        let mut agent_scope = super::ssh_config::AgentScope::none();
        match parsed.as_ref().map(|p| p.transport) {
            Some(super::remote_url::Transport::Ssh) => {
                let settings = super::ssh_config::for_contact(&host, configured);
                match settings.refusal() {
                    // A host joy cannot reach at all. The contact is
                    // refused before this by `guard_transport`; this
                    // is the second line of defence, for a URL that
                    // only appeared after an insteadOf rewrite.
                    Some(sentence) => notes.push(sentence),
                    None => {
                        // Before the agent is probed, because the probe
                        // reads the same variable libssh2 will read.
                        agent_scope = super::ssh_config::AgentScope::apply(&settings);
                        let agent = super::ssh_auth::probe_agent();
                        let chain = super::ssh_auth::chain_for(
                            &host,
                            url_user.as_deref(),
                            &settings,
                            kind,
                            &agent,
                            cfg!(windows),
                        );
                        user = chain.user;
                        notes.extend(chain.notes);
                        for candidate in chain.candidates {
                            steps.push_back(match candidate {
                                super::ssh_auth::SshCandidate::Agent => Step::Agent,
                                super::ssh_auth::SshCandidate::Key {
                                    path,
                                    public,
                                    passphrase,
                                } => Step::Key {
                                    path,
                                    public,
                                    passphrase,
                                },
                            });
                        }
                    }
                }
            }
            Some(transport) if transport.takes_helper() => steps.push_back(Step::Helper),
            _ => {}
        }
        ChainState {
            kind,
            host,
            user,
            steps,
            notes,
            presented: None,
            usernames: 0,
            defaulted: false,
            _agent: agent_scope,
        }
    }

    /// The one user name, however often libgit2 asks for it.
    fn user_name(&mut self) -> Result<git2::Cred, git2::Error> {
        self.usernames += 1;
        if self.usernames > 3 {
            return Err(git2::Error::from_str(&format!(
                "{} kept asking for a user name; joy answered {} every time",
                self.host, self.user
            )));
        }
        git2::Cred::username(&self.user)
    }

    /// A re-entry means the forge refused what was offered last.
    fn note_refusal(&mut self, url: &str) {
        match self.presented.take() {
            Some(Presented::Agent) => self.notes.push(format!(
                "the ssh agent's identities were refused by {}",
                self.host
            )),
            Some(Presented::Key(path)) => self.notes.push(format!(
                "key {} was refused by {}",
                path.display(),
                self.host
            )),
            Some(Presented::Helper) => {
                self.notes.push(format!(
                    "the credential from your credential helper was refused by {}",
                    self.host
                ));
                // The half git2 never runs: the helper is told to
                // erase it, so the next contact does not replay a
                // revoked entry (cred.rs:395, :415).
                super::credential_helper::refused(url);
            }
            None => {}
        }
    }

    fn take_next(&mut self, allowed: git2::CredentialType) -> Option<Step> {
        while let Some(step) = self.steps.pop_front() {
            let fits = match &step {
                Step::Agent | Step::Key { .. } => allowed.contains(git2::CredentialType::SSH_KEY),
                Step::Helper => allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT),
            };
            if fits {
                return Some(step);
            }
            self.notes.push(format!(
                "{} does not accept the credential joy had for it",
                self.host
            ));
        }
        None
    }

    /// Everything joy tried and everything it could not try, in one
    /// sentence. This is what the person reads instead of libgit2's
    /// "error authenticating".
    fn exhausted(&self) -> String {
        let mut sentence = format!("no usable credential for {}", self.host);
        if !self.notes.is_empty() {
            sentence.push_str(": ");
            sentence.push_str(&self.notes.join("; "));
        }
        sentence
    }
}

/// Refuse a remote joy cannot speak to BEFORE libgit2 opens a socket
/// (design D1.4). A host behind `ProxyCommand` or `ProxyJump` would
/// otherwise be contacted directly and fail with a DNS or connect
/// error that names the wrong cause.
fn guard_transport(url: Option<&str>) -> anyhow::Result<()> {
    guard_transport_with(url, super::known_hosts::user_file_refusal)
}

/// [`guard_transport`] with the known_hosts verdict handed in.
///
/// The production verdict is read once per process from the machine's
/// own `~/.ssh/known_hosts` ([`super::known_hosts::user_file_refusal`],
/// a `OnceLock` around the real HOME), which is exactly what a test
/// cannot arrange. The rule this guard carries is the branch, not the
/// reading, so the branch is a function of two arguments and the
/// reading is passed in.
fn guard_transport_with(
    url: Option<&str>,
    known_hosts_refusal: impl FnOnce() -> Option<String>,
) -> anyhow::Result<()> {
    if let Some(sentence) = url.and_then(super::ssh_config::refusal_for_url) {
        anyhow::bail!("{sentence}");
    }
    // An ssh contact reads `~/.ssh/known_hosts` inside libgit2 before
    // joy's own callback runs, and ONE line libssh2 cannot parse makes
    // it discard the whole file and end the connection with "error
    // reading known_hosts" (D1.4a). Said here, once per process, with
    // the line number. Only an ssh contact reads that file at all, so
    // no other transport pays for the check.
    if url.map(super::contact::transport_of) == Some(super::contact::Transport::Ssh) {
        if let Some(sentence) = known_hosts_refusal() {
            anyhow::bail!("{sentence}");
        }
    }
    Ok(())
}

/// [`guard_transport`] for a remote that is already open.
fn guard_remote(remote: &git2::Remote<'_>) -> anyhow::Result<()> {
    guard_transport(remote.url().ok())
}

/// The remote joy really contacts for this checkout: the configured
/// one ([`origin_or_first`]), refused when joy cannot speak to it at
/// all, and dialled at the address the person's ssh config names.
///
/// libgit2 reads no ssh config and opens the socket itself from the
/// URL, so `git@work:owner/repo.git` with `Host work / HostName
/// git.example.com / Port 2222` would be looked up as the literal
/// name `work` on port 22 and fail with a DNS error (design D1.4).
/// Where the config renames the host or moves the port, joy therefore
/// hands libgit2 an anonymous remote on the real address. The
/// configured remote and `.git/config` are never touched, and a remote
/// the config does not rename is returned exactly as it is, so the
/// named remote (with its refspecs) stays the normal case.
fn contact_remote(repo: &git2::Repository) -> anyhow::Result<git2::Remote<'_>> {
    let remote = origin_or_first(repo)?;
    guard_remote(&remote)?;
    let Some(url) = remote.url().ok() else {
        return Ok(remote);
    };
    if rewritten_by_insteadof(repo, url) {
        return Ok(remote);
    }
    let Some(dialled) = super::ssh_config::effective_url(url) else {
        return Ok(remote);
    };
    drop(remote);
    repo.remote_anonymous(&dialled).map_err(err)
}

/// Whether an `insteadOf` rule rewrites this URL, in which case joy
/// leaves the address alone.
///
/// libgit2 applies those rules itself, to the URL as CONFIGURED
/// (remote.c:254-255, :509-510), and git applies them before ssh ever
/// reads its config. joy's own `HostName` rewrite runs before libgit2
/// builds the remote, so it would hide such a rule and dial the alias's
/// `HostName` where the person meant the rule's target. Which rule wins
/// (the longest prefix) is J4b's prediction (design D1.5); the question
/// here is only whether there is one at all.
fn rewritten_by_insteadof(repo: &git2::Repository, url: &str) -> bool {
    let Ok(config) = repo.config() else {
        return false;
    };
    let mut matched = false;
    for glob in ["url.*.insteadof", "url.*.pushinsteadof"] {
        let Ok(entries) = config.entries(Some(glob)) else {
            continue;
        };
        let _ = entries.for_each(|entry| {
            if let Ok(prefix) = entry.value() {
                if !prefix.is_empty() && url.starts_with(prefix) {
                    matched = true;
                }
            }
        });
    }
    matched
}

/// `origin`, or the first configured remote — a checkout the product
/// made always has `origin`, but a repo a person wired by hand may not
/// (the desktop opens those too).
fn origin_or_first<'r>(repo: &'r git2::Repository) -> anyhow::Result<git2::Remote<'r>> {
    match repo.find_remote("origin") {
        Ok(remote) => Ok(remote),
        Err(_) => {
            let remotes = repo.remotes().map_err(err)?;
            let name = remotes
                .get(0)
                .map_err(|_| anyhow::anyhow!("no remote configured"))?
                .ok_or_else(|| anyhow::anyhow!("remote name is not utf-8"))?
                .to_string();
            repo.find_remote(&name).map_err(err)
        }
    }
}

/// The process state libgit2 needs before joy reads anything through it:
/// git's system config rule below, and the one TLS trust decision of
/// D1.12. Every entry into libgit2 in this file calls this FIRST, and
/// that includes every call that only reads git config, because the
/// first of the two decides what git config even is.
fn git_environment() {
    git_config_environment();
    // The one process wide TLS trust decision of D1.12, which must run
    // before the first contact and does so here, because every entry
    // into libgit2 passes through this function. It reads git config,
    // so it runs after the search path above is settled.
    crate::apply_ca_locations();
}

/// git's own rule for its system config, which libgit2 does not know: with
/// `GIT_CONFIG_NOSYSTEM` true, the system-wide gitconfig is not read
/// (git-config(1)). libgit2 always adds it, so a script that isolates
/// HOME and sets the variable still found an identity from the machine's
/// /etc/gitconfig in joy, where git itself found none (JOY-028D-46).
/// The search path is process state, so it is changed only when the
/// variable changes, under one lock.
fn git_config_environment() {
    static APPLIED: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);
    let nosystem = std::env::var("GIT_CONFIG_NOSYSTEM").is_ok_and(|value| git_bool(&value));
    let mut applied = APPLIED.lock().unwrap_or_else(|e| e.into_inner());
    if *applied == Some(nosystem) {
        return;
    }
    // SAFETY: libgit2's search paths are global; every change goes through
    // this lock, and joy reads config only after passing through here.
    let result = unsafe {
        if nosystem {
            git2::opts::set_search_path(git2::ConfigLevel::System, "")
        } else {
            git2::opts::reset_search_path(git2::ConfigLevel::System)
        }
    };
    if result.is_ok() {
        *applied = Some(nosystem);
    }
}

/// A boolean the way git reads one from its environment: true, yes, on or
/// a non-zero number; anything else, the empty value included, is false.
fn git_bool(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    matches!(value.as_str(), "true" | "yes" | "on") || value.parse::<i64>().is_ok_and(|n| n != 0)
}

/// Open the repository that holds `dir` — `discover`, not `open`: the
/// desktop opens project roots that may sit inside a larger repo, and
/// for an exact root (every platform checkout) discover is the same
/// thing.
fn open(dir: &Path) -> Result<git2::Repository, git2::Error> {
    git_environment();
    git2::Repository::discover(dir)
}

/// Everything the credential resolver needs about one contact beside
/// the URL libgit2 hands it.
pub(crate) struct CredSource {
    /// The credential-helper config for a repo: repo-level settings
    /// (insteadOf, per-repo helpers) included; the global config as
    /// the fallback when there is no repo yet (clone).
    config: Option<git2::Config>,
    /// The remote URL as the person configured it.
    ///
    /// libgit2 hands the callback the URL it is DIALLING, which after
    /// joy's own `HostName` rewrite ([`contact_remote`]) names the real
    /// host. The `Host work` block that produced that rewrite, and with
    /// it the `IdentityFile`, the `IdentityAgent` and the `User` the
    /// person wrote under it, would then be invisible to the chain
    /// (design D1.4).
    configured: Option<String>,
}

/// The source for a contact on a repository: its config, and the URL
/// of the remote the contact runs over ([`origin_or_first`]).
fn cred_source(repo: Option<&git2::Repository>) -> CredSource {
    git_environment();
    match repo {
        Some(r) => CredSource {
            config: r.config().and_then(|mut c| c.snapshot()).ok(),
            configured: origin_or_first(r)
                .ok()
                .and_then(|remote| remote.url().ok().map(str::to_string)),
        },
        None => CredSource {
            config: git2::Config::open_default().ok(),
            configured: None,
        },
    }
}

impl CredSource {
    /// Nothing to read: no config and no configured remote. What the
    /// shape tests hand the resolver, so that the decision under test
    /// is the one the URL and the claim make.
    #[cfg(test)]
    fn none() -> CredSource {
        CredSource {
            config: None,
            configured: None,
        }
    }
}

/// The source for a contact that has no repository yet: a clone, where
/// the URL the caller named IS the configured remote.
fn cred_source_for_url(url: &str) -> CredSource {
    let mut source = cred_source(None);
    source.configured = Some(url.to_string());
    source
}

/// A fault of libgit2 on this machine's own data: the index, a ref, a
/// tree, a checkout. It reads exactly as it always did ("git: reference
/// not found"), but it is TYPED, so the contact boundary can tell
/// libgit2's words from joy's own plain sentences and keep them off the
/// surface when such a fault happens inside a contact (D1.8b, wording
/// rules).
fn err(e: git2::Error) -> anyhow::Error {
    super::contact::engine_fault("git", &e)
}

/// Clone a forge URL into `dest` using the account token.
pub fn clone(url: &str, auth: &Auth, dest: &Path) -> anyhow::Result<()> {
    super::contact::run(url, "clone", auth.credentialed(), || {
        clone_raw(url, auth, dest)
    })
}

fn clone_raw(url: &str, auth: &Auth, dest: &Path) -> anyhow::Result<()> {
    guard_transport(Some(url))?;
    std::fs::create_dir_all(dest.parent().expect("checkout dir has a parent"))?;
    // Before anything that reads git config, the proxy decision
    // included: a clone is the one verb that reaches libgit2 without
    // going through `open`, and `options_for` opens the default config
    // (JOY-028D-46, the invariant above `git_config_environment`).
    git_environment();
    // The proxy of D1.11, before the first socket: a proxy joy cannot
    // speak to (SOCKS) is refused here by name and nothing is dialled.
    let proxy = proxy_for(url, None)?;
    let mut fetch = git2::FetchOptions::new();
    fetch.remote_callbacks(auth.callbacks(cred_source_for_url(url)));
    fetch.proxy_options(proxy.options());
    // The same address the other verbs dial (see `contact_remote`): a
    // clone from an ssh alias reaches the `HostName` the person's ssh
    // config names, which libgit2 reads nothing of (design D1.4).
    let dialled = super::ssh_config::effective_url(url);
    let cloned = git2::build::RepoBuilder::new()
        .fetch_options(fetch)
        .clone(dialled.as_deref().unwrap_or(url), dest)
        .map_err(|e| contact_failed(url, auth, super::contact::ContactDirection::Fetch, e))?;
    if dialled.is_some() {
        // The new checkout keeps the remote the caller named, not the
        // address joy dialled it at: the alias is the person's, the
        // rewrite belongs to one contact.
        cloned.remote_set_url("origin", url).map_err(err)?;
    }
    drop(cloned);
    // the clone machinery runs libgit2's update_tips once; nothing here
    // ever reads FETCH_HEAD, so the checkout starts without one
    std::fs::remove_file(dest.join(".git/FETCH_HEAD")).ok();
    Ok(())
}

/// Fetch the current branch and fast-forward to the remote state. Diverged
/// histories are an error (manual merge; the YAML merge driver stays the
/// known follow-up).
pub fn pull_ff(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    let span = tracing::info_span!("git.pull_ff", repo = %repo_dir.display());
    let _s = span.enter();
    let result = pull_ff_inner(repo_dir, auth);
    if let Err(e) = &result {
        // divergence is the NORMAL case under write-behind sync (ADR
        // JAPP-00D8) — callers branch on it; only real failures are errors
        if e.to_string().contains("diverged") {
            tracing::debug!(repo = %repo_dir.display(), "git pull: histories diverged");
        } else {
            tracing::error!(repo = %repo_dir.display(), error = %e, "git pull failed");
        }
    }
    result
}

fn pull_ff_inner(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    fetch_branch(repo_dir, auth)?;
    ff_from_tracking(repo_dir)
}

/// The remote-tracking ref for `branch`: the branch's configured
/// upstream when it has one, else `refs/remotes/<remote>/<branch>` for
/// the resolved remote ([`origin_or_first`]) — never a hardwired
/// "origin", because a hand-wired repo may call its remote differently.
fn tracking_ref_name(repo: &git2::Repository, branch: &str) -> anyhow::Result<String> {
    if let Ok(b) = repo.find_branch(branch, git2::BranchType::Local) {
        if let Ok(upstream) = b.upstream() {
            if let Ok(name) = upstream.get().name() {
                return Ok(name.to_string());
            }
        }
    }
    let remote = origin_or_first(repo)?;
    let remote_name = remote.name().ok().flatten().unwrap_or("origin").to_string();
    Ok(format!("refs/remotes/{remote_name}/{branch}"))
}

/// The one low-level fetch: download the objects for `src` and point
/// `dst` at the advertised tip OURSELVES. libgit2's own update_tips is
/// never run, because it truncates FETCH_HEAD unconditionally (the
/// UPDATE_FETCHHEAD flag only guards the writing of entries, see
/// libgit2 remote.c truncate_fetch_head) — and that bare truncation is
/// the whole torn-FETCH_HEAD class (JP-00DB-61, JAPP-0198-EA).
/// `Ok(None)` when the forge does not advertise `src`.
fn download_ref(
    repo: &git2::Repository,
    auth: &Auth,
    src: &str,
    dst: &str,
) -> anyhow::Result<Option<git2::Oid>> {
    // The address the ssh config really names, not the alias the
    // remote was written as (design D1.4).
    let mut remote = contact_remote(repo)?;
    let url = remote_url_of(&remote);
    let proxy = proxy_for(&url, Some(repo))?;
    // ONE connection for the advertisement AND the download (D1.9): the
    // RemoteConnection disconnects on drop, and joy used to drop it
    // before `remote.download`, so git_remote_download reconnected and
    // paid the 401 challenge a second time. Downloading through
    // `connection.remote()` while the connection is alive removes one
    // handshake and one challenge per fetch, which is two HTTP requests
    // of the host's budget.
    let mut connection = remote
        .connect_auth(
            git2::Direction::Fetch,
            Some(auth.callbacks(cred_source(Some(repo)))),
            Some(proxy.options()),
        )
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Fetch, e))?;
    // an empty advertisement (freshly created forge) is a plain
    // empty list since git2 0.21, and an honest "nothing there"
    let advertised = connection
        .list()
        // the advertisement is a forge contact like any other: its
        // failure is read by the classifier, never copied raw onto a
        // surface (D1.8b, wording rules)
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Fetch, e))?
        .iter()
        .find(|r| r.name() == src)
        .map(|r| r.oid());
    let Some(tip) = advertised else {
        return Ok(None);
    };
    let mut opts = git2::FetchOptions::new();
    opts.remote_callbacks(auth.callbacks(cred_source(Some(repo))));
    opts.proxy_options(proxy.options());
    let refspec = format!("+{src}:{dst}");
    connection
        .remote()
        .download(&[refspec.as_str()], Some(&mut opts))
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Fetch, e))?;
    drop(connection);
    repo.reference(dst, tip, true, "joy-vcs: fetch")
        .map_err(err)?;
    Ok(Some(tip))
}

/// The NETWORK half of a pull: fetch the working branch into its
/// remote-tracking ref. FETCH_HEAD is not touched (see [`download_ref`]).
/// Touches no working tree and no local branch. Every caller of git work
/// on a shared checkout holds that checkout's gate (JP-00DB-61: one git
/// process per checkout).
///
/// A branch that is gone from the forge (renamed or deleted) is said out
/// loud instead of surfacing as a phantom state.
pub fn fetch_branch(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    super::contact::run_for(repo_dir, "fetch", auth.credentialed(), || {
        fetch_branch_raw(repo_dir, auth)
    })
}

fn fetch_branch_raw(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    let span = tracing::info_span!("git.fetch", repo = %repo_dir.display());
    let _s = span.enter();
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?;
    let branch = head
        .shorthand()
        .map_err(|_| anyhow::anyhow!("detached HEAD"))?
        .to_string();
    let src = format!("refs/heads/{branch}");
    let dst = tracking_ref_name(&repo, &branch)?;
    match download_ref(&repo, auth, &src, &dst)? {
        Some(_) => Ok(()),
        None => anyhow::bail!("branch {branch} not found on the forge (renamed or deleted?)"),
    }
}

/// The LOCAL half of a pull: fast-forward the working branch onto its
/// remote-tracking ref, which [`fetch_branch`] just updated. Fast, no
/// network; the caller holds the project gate.
pub fn ff_from_tracking(repo_dir: &Path) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?;
    let branch = head
        .shorthand()
        .map_err(|_| anyhow::anyhow!("detached HEAD"))?
        .to_string();
    let tracking = repo
        .find_reference(&tracking_ref_name(&repo, &branch)?)
        .map_err(err)?;
    let remote_commit = repo.reference_to_annotated_commit(&tracking).map_err(err)?;
    let (analysis, _) = repo.merge_analysis(&[&remote_commit]).map_err(err)?;
    if analysis.is_fast_forward() {
        let refname = format!("refs/heads/{branch}");
        let mut reference = repo.find_reference(&refname).map_err(err)?;
        reference
            .set_target(remote_commit.id(), "joy-vcs: fast-forward")
            .map_err(err)?;
        repo.set_head(&refname).map_err(err)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(err)?;
    } else if !analysis.is_up_to_date() {
        anyhow::bail!("local and remote histories diverged; resolve on the forge side");
    }
    Ok(())
}

/// Stage `.joy/` changes and commit them as the acting account; returns the
/// commit id, or None when the tree is clean.
pub fn commit_joy(
    repo_dir: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<Option<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let mut status_opts = git2::StatusOptions::new();
    status_opts
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(".joy");
    let dirty = !repo
        .statuses(Some(&mut status_opts))
        .map_err(err)?
        .is_empty();
    if !dirty {
        return Ok(None);
    }
    let mut index = repo.index().map_err(err)?;
    index
        .add_all([".joy"], git2::IndexAddOption::DEFAULT, None)
        .map_err(err)?;
    index.write().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let signature = signature_now(repo_dir, author_name, author_email)?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(err)?;
    Ok(Some(oid.to_string()))
}

/// Push the current branch back to the forge.
/// Ask the forge whether it would let us WRITE, without writing anything
/// (operator 2026-07-27).
///
/// A public repository can be read by anyone and written by nobody without
/// credentials, so a project can look perfectly healthy until the first
/// push — which may be hours later, in the middle of something. The push
/// side of the protocol answers this question during its handshake: the
/// connection authenticates and the server advertises its refs, and no
/// object and no ref is sent. A refusal here is exactly the refusal a real
/// push would meet.
pub fn probe_write_access(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    super::contact::run_for(repo_dir, "probe", auth.credentialed(), || {
        probe_write_access_raw(repo_dir, auth)
    })
}

fn probe_write_access_raw(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    let span = tracing::info_span!("git.probe_write", repo = %repo_dir.display());
    let _s = span.enter();
    let repo = open(repo_dir).map_err(err)?;
    // The address the ssh config really names, not the alias the
    // remote was written as (design D1.4).
    let mut remote = contact_remote(&repo)?;
    let url = remote_url_of(&remote);
    let proxy = proxy_for(&url, Some(&repo))?;
    remote
        .connect_auth(
            git2::Direction::Push,
            Some(auth.callbacks(cred_source(Some(&repo)))),
            Some(proxy.options()),
        )
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Push, e))?;
    let _ = remote.disconnect();
    Ok(())
}

pub fn push(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    super::contact::run_for(repo_dir, "push", auth.credentialed(), || {
        push_raw(repo_dir, auth)
    })
}

fn push_raw(repo_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    let span = tracing::info_span!("git.push", repo = %repo_dir.display());
    let _s = span.enter();
    let result = (|| -> anyhow::Result<()> {
        let repo = open(repo_dir).map_err(err)?;
        let head = repo.head().map_err(err)?;
        let branch = head
            .shorthand()
            .map_err(|_| anyhow::anyhow!("detached HEAD"))?
            .to_string();
        // The address the ssh config really names, not the alias the
        // remote was written as (design D1.4).
        let mut remote = contact_remote(&repo)?;
        let url = remote_url_of(&remote);
        let proxy = proxy_for(&url, Some(&repo))?;
        let mut opts = git2::PushOptions::new();
        opts.remote_callbacks(auth.callbacks(cred_source(Some(&repo))));
        opts.proxy_options(proxy.options());
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        remote
            .push(&[refspec.as_str()], Some(&mut opts))
            .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Push, e))?;
        Ok(())
    })();
    if let Err(e) = &result {
        // some callers defer a failed push to the write-behind worker; the
        // event still carries the cause with the repo context
        tracing::error!(repo = %repo_dir.display(), error = %e, "git push failed");
    }
    result
}

// ---- generic ref plumbing (chats and other side refs) ------------------
//
// The chat SEMANTICS (adopt / fast-forward / message-union merge) live in
// joy-chat-store; these are the raw verbs it composes. They never write
// FETCH_HEAD either.

/// Fetch one ref into a local destination ref. `Ok(false)` when the
/// forge does not have the source ref (first-ever sync, or the ref was
/// removed) — the stale destination is deleted then, so reconciles run
/// against nothing rather than a stale state.
pub fn fetch_ref(repo_dir: &Path, auth: &Auth, src: &str, dst: &str) -> anyhow::Result<bool> {
    super::contact::run_for(repo_dir, "fetch", auth.credentialed(), || {
        fetch_ref_raw(repo_dir, auth, src, dst)
    })
}

fn fetch_ref_raw(repo_dir: &Path, auth: &Auth, src: &str, dst: &str) -> anyhow::Result<bool> {
    let repo = open(repo_dir).map_err(err)?;
    match download_ref(&repo, auth, src, dst)? {
        Some(_) => Ok(true),
        None => {
            if let Ok(mut stale) = repo.find_reference(dst) {
                stale.delete().ok();
            }
            Ok(false)
        }
    }
}

// ---- the ONE per-checkout gate (JP-00DB-61) ----------------------------

/// Gates keyed by canonical checkout path. Lazy, never dropped: a gate
/// is a few bytes and a process touches a handful of checkouts.
static CHECKOUT_GATES: std::sync::Mutex<
    Option<std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>>,
> = std::sync::Mutex::new(None);

/// THE per-checkout gate (JP-00DB-61): one git actor per checkout,
/// whoever the actor is. Every path that MOVES refs on a checkout —
/// commits, syncs, polls — takes this gate first; reads stay lock-free.
/// It lives in the engine so no host grows a private twin again (the
/// desktop did on 2026-08-27, and a command's answer lost the race
/// against a poll). Per process; cross-process safety is git's own
/// ref locking plus the store's compare-and-swap.
pub fn checkout_gate(repo_dir: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    let key = repo_dir
        .canonicalize()
        .unwrap_or_else(|_| repo_dir.to_path_buf());
    CHECKOUT_GATES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(std::collections::HashMap::new)
        .entry(key)
        .or_default()
        .clone()
}

/// Push one local ref to the same name on the forge.
pub fn push_ref(repo_dir: &Path, auth: &Auth, refname: &str) -> anyhow::Result<()> {
    super::contact::run_for(repo_dir, "push", auth.credentialed(), || {
        push_ref_raw(repo_dir, auth, refname)
    })
}

fn push_ref_raw(repo_dir: &Path, auth: &Auth, refname: &str) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    // The address the ssh config really names, not the alias the
    // remote was written as (design D1.4).
    let mut remote = contact_remote(&repo)?;
    let url = remote_url_of(&remote);
    let proxy = proxy_for(&url, Some(&repo))?;
    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(auth.callbacks(cred_source(Some(&repo))));
    opts.proxy_options(proxy.options());
    let refspec = format!("{refname}:{refname}");
    remote
        .push(&[refspec.as_str()], Some(&mut opts))
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Push, e))?;
    Ok(())
}

/// The oid the forge holds for `refname`, without fetching anything
/// (JP-008B-24: polls compare hashes and fetch only on a change). `None`
/// when the forge does not have the ref. Callers hold a registered
/// project, so the remote always advertises at least its working branch
/// (a fully ref-less remote trips a git2 empty-list edge).
///
/// A caller that wants two refs asks [`ls_remote_refs`] for both at
/// once: the advertisement it reads carries every ref anyway.
pub fn ls_remote_ref(
    repo_dir: &Path,
    auth: &Auth,
    refname: &str,
) -> anyhow::Result<Option<String>> {
    Ok(ls_remote_refs(repo_dir, auth, &[refname])?.remove(refname))
}

/// The oids the forge holds for SEVERAL refs, from one advertisement
/// (D1.9). `connection.list()` downloads the forge's whole ref list, so
/// asking for a second ref costs nothing on top; asking twice costs a
/// second connection and, on a private https remote, a second 401
/// challenge. One poll tick that watches two refs therefore makes one
/// contact. Refs the forge does not have are absent from the map.
pub fn ls_remote_refs(
    repo_dir: &Path,
    auth: &Auth,
    refnames: &[&str],
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    super::contact::run_for(repo_dir, "ls-remote", auth.credentialed(), || {
        ls_remote_refs_raw(repo_dir, auth, refnames)
    })
}

/// [`ls_remote_ref`] as a POLL: a contact no person asked for, made by
/// a loop that watches the forge. A poll is the one contact the no
/// anonymous polling rule of D1.9 holds back, so a public https remote
/// nobody is signed in for is asked at most once every fifteen minutes
/// per host and the refusal says why. Every other caller asks
/// [`ls_remote_ref`].
pub fn ls_remote_ref_poll(
    repo_dir: &Path,
    auth: &Auth,
    refname: &str,
) -> anyhow::Result<Option<String>> {
    Ok(ls_remote_refs_poll(repo_dir, auth, &[refname])?.remove(refname))
}

/// [`ls_remote_refs`] as a poll; see [`ls_remote_ref_poll`].
pub fn ls_remote_refs_poll(
    repo_dir: &Path,
    auth: &Auth,
    refnames: &[&str],
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    super::contact::run_poll_for(repo_dir, "ls-remote", auth.credentialed(), || {
        ls_remote_refs_raw(repo_dir, auth, refnames)
    })
}

fn ls_remote_refs_raw(
    repo_dir: &Path,
    auth: &Auth,
    refnames: &[&str],
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let repo = open(repo_dir).map_err(err)?;
    // The address the ssh config really names, not the alias the
    // remote was written as (design D1.4).
    let mut remote = contact_remote(&repo)?;
    let url = remote_url_of(&remote);
    let proxy = proxy_for(&url, Some(&repo))?;
    let connection = remote
        .connect_auth(
            git2::Direction::Fetch,
            Some(auth.callbacks(cred_source(Some(&repo)))),
            Some(proxy.options()),
        )
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Fetch, e))?;
    let found = connection
        .list()
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Fetch, e))?
        .iter()
        .filter(|r| refnames.contains(&r.name()))
        .map(|r| (r.name().to_string(), r.oid().to_string()))
        .collect();
    Ok(found)
}

/// Pull with a REAL merge (ADR JAPP-00D8): fetch, fast-forward when
/// possible, otherwise three-way-merge the histories. Conflicting
/// `.joy/*.yaml` files merge through joy-core's YAML engine (the same
/// logic as the git merge driver); joycrypt blobs and everything else
/// take the forge side — under write-behind only `.joy` is written
/// locally. Divergence is the NORMAL case under write-behind, not an
/// error. The merge commit carries the acting member (JP-00DE-11): the
/// write that made the checkout dirty is whose work this merge finishes.
pub fn pull_merge(
    repo_dir: &Path,
    auth: &Auth,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<()> {
    // the shared fetch half: honest about a vanished branch, and it
    // never touches FETCH_HEAD (JP-00DB-61)
    fetch_branch(repo_dir, auth)?;
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?;
    let branch = head
        .shorthand()
        .map_err(|_| anyhow::anyhow!("detached HEAD"))?
        .to_string();
    let tracking = repo
        .find_reference(&tracking_ref_name(&repo, &branch)?)
        .map_err(err)?;
    let remote_commit = repo.reference_to_annotated_commit(&tracking).map_err(err)?;
    let (analysis, _) = repo.merge_analysis(&[&remote_commit]).map_err(err)?;
    if analysis.is_up_to_date() {
        return Ok(());
    }
    if analysis.is_fast_forward() {
        let refname = format!("refs/heads/{branch}");
        let mut reference = repo.find_reference(&refname).map_err(err)?;
        reference
            .set_target(remote_commit.id(), "joy-vcs: fast-forward")
            .map_err(err)?;
        repo.set_head(&refname).map_err(err)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(err)?;
        return Ok(());
    }
    // three-way merge
    let local = repo
        .find_commit(
            head.target()
                .ok_or_else(|| anyhow::anyhow!("unborn HEAD"))?,
        )
        .map_err(err)?;
    let theirs = repo.find_commit(remote_commit.id()).map_err(err)?;
    let mut index = repo.merge_commits(&local, &theirs, None).map_err(err)?;
    resolve_conflicts_yaml_aware(&repo, &mut index)?;
    let tree_id = index.write_tree_to(&repo).map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let sig = signature_now(repo_dir, author_name, author_email)?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "chore: merge forge changes [no-item]",
        &tree,
        &[&local, &theirs],
    )
    .map_err(err)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
        .map_err(err)?;
    repo.cleanup_state().ok();
    Ok(())
}

/// Resolve every conflict of a merged `index` in place: conflicting
/// `.joy/*.yaml` files merge through joy-core's YAML engine (the same
/// logic as the git merge driver); joycrypt blobs and everything else
/// take the `theirs` side. Shared by [`pull_merge`] (theirs = the forge)
/// and [`land_branch_yaml`] (theirs = the joywork branch — which only
/// carries `.joy` changes, so the theirs-wins arm stays theoretical there).
fn resolve_conflicts_yaml_aware(
    repo: &git2::Repository,
    index: &mut git2::Index,
) -> anyhow::Result<()> {
    if index.has_conflicts() {
        let conflicts: Vec<_> = index
            .conflicts()
            .map_err(err)?
            .filter_map(|c| c.ok())
            .collect();
        for conflict in conflicts {
            let path_bytes = conflict
                .our
                .as_ref()
                .or(conflict.their.as_ref())
                .or(conflict.ancestor.as_ref())
                .map(|e| e.path.clone())
                .unwrap_or_default();
            let path = String::from_utf8_lossy(&path_bytes).to_string();
            let read = |entry: &Option<git2::IndexEntry>| -> Vec<u8> {
                entry
                    .as_ref()
                    .and_then(|e| repo.find_blob(e.id).ok())
                    .map(|b| b.content().to_vec())
                    .unwrap_or_default()
            };
            let ours_bytes = read(&conflict.our);
            let theirs_bytes = read(&conflict.their);
            let base_bytes = read(&conflict.ancestor);
            let merged: Vec<u8> = if path.starts_with(".joy/")
                && path.ends_with(".yaml")
                && !crate::merge::is_joycrypt_blob(&ours_bytes)
                && !crate::merge::is_joycrypt_blob(&theirs_bytes)
            {
                let doc = crate::merge::merge_yaml_doc(
                    &String::from_utf8_lossy(&base_bytes),
                    &String::from_utf8_lossy(&ours_bytes),
                    &String::from_utf8_lossy(&theirs_bytes),
                )
                .map_err(|e| anyhow::anyhow!("joy-yaml merge of {path}: {e}"))?;
                doc.into_bytes()
            } else if !theirs_bytes.is_empty() {
                // non-joy or encrypted content: the forge side wins
                theirs_bytes
            } else {
                ours_bytes
            };
            let blob = repo.blob(&merged).map_err(err)?;
            let mut entry = conflict
                .our
                .or(conflict.their)
                .or(conflict.ancestor)
                .ok_or_else(|| anyhow::anyhow!("empty conflict entry"))?;
            entry.id = blob;
            entry.flags &= !0x3000; // clear the stage bits: stage 0 (merged)
            index.add(&entry).map_err(err)?;
            index.remove_path(std::path::Path::new(&path)).ok();
            index.add(&entry).map_err(err)?;
        }
    }
    Ok(())
}

// ---- repo lifecycle (seeding, harnesses, volume facts) ------------------

/// Initialize a bare repository on `branch` (a local stand-in forge for
/// harnesses).
pub fn init_bare(dir: &Path, branch: &str) -> anyhow::Result<()> {
    git_environment();
    let repo = git2::Repository::init_bare(dir).map_err(err)?;
    repo.set_head(&format!("refs/heads/{branch}"))
        .map_err(err)?;
    Ok(())
}

/// Initialize a plain repository on `branch`.
pub fn init_repo(dir: &Path, branch: &str) -> anyhow::Result<()> {
    git_environment();
    let repo = git2::Repository::init(dir).map_err(err)?;
    repo.set_head(&format!("refs/heads/{branch}"))
        .map_err(err)?;
    Ok(())
}

// ---- working-copy verbs of joy init (JOY-0288-72) ------------------------
//
// joy init used to reach these through the git binary, which a host
// without one (the platform) cannot run. They answer the way the git
// commands they replace did, for every caller of the Vcs trait.

/// Is `dir` inside a repository's working tree? A bare repository is not.
pub fn is_worktree(dir: &Path) -> bool {
    open(dir)
        .map(|repo| repo.workdir().is_some())
        .unwrap_or(false)
}

/// Initialize a repository in `dir` the way `git init` does: on the
/// person's `init.defaultBranch` when their git config names one.
pub fn init_worktree(dir: &Path) -> anyhow::Result<()> {
    git_environment();
    let mut options = git2::RepositoryInitOptions::new();
    let configured = git2::Config::open_default()
        .ok()
        .and_then(|config| config.get_string("init.defaultBranch").ok())
        .filter(|branch| !branch.trim().is_empty());
    if let Some(branch) = configured.as_deref() {
        options.initial_head(branch);
    }
    git2::Repository::init_opts(dir, &options).map_err(err)?;
    Ok(())
}

/// The person's `user.email`, answered like `git config user.email` in
/// the current directory: the repository's config when it runs inside
/// one (local over global over system), else the global and system files.
pub fn user_email() -> Option<String> {
    git_environment();
    let config = open(Path::new("."))
        .and_then(|repo| repo.config())
        .or_else(|_| git2::Config::open_default())
        .ok()?;
    config
        .get_string("user.email")
        .ok()
        .filter(|email| !email.trim().is_empty())
}

/// A value of the repository's own config file (`git config --local`).
pub fn local_config_get(dir: &Path, key: &str) -> Option<String> {
    let repo = open(dir).ok()?;
    let local = repo
        .config()
        .ok()?
        .open_level(git2::ConfigLevel::Local)
        .ok()?;
    local.get_string(key).ok()
}

/// Write a value into the repository's own config file
/// (`git config --local <key> <value>`).
pub fn local_config_set(dir: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let repo = open(dir).map_err(err)?;
    let mut local = repo
        .config()
        .map_err(err)?
        .open_level(git2::ConfigLevel::Local)
        .map_err(err)?;
    local.set_str(key, value).map_err(err)
}

/// Every configured remote as `(name, url)`, in the repository's order.
pub fn remotes(dir: &Path) -> Vec<(String, String)> {
    let Ok(repo) = open(dir) else {
        return Vec::new();
    };
    let Ok(names) = repo.remotes() else {
        return Vec::new();
    };
    (0..names.len())
        .filter_map(|i| {
            let name = names.get(i).ok()??;
            let remote = repo.find_remote(name).ok()?;
            let url = remote.url().ok()?.to_string();
            Some((name.to_string(), url))
        })
        .collect()
}

/// `path`, given relative to `dir`, as the repository sees it: relative
/// to its working tree, with forward slashes. `dir` may be a subdirectory.
fn workdir_relative(repo: &git2::Repository, dir: &Path, path: &str) -> Option<String> {
    let workdir = repo.workdir()?.canonicalize().ok()?;
    let here = dir.canonicalize().ok()?;
    let prefix = here.strip_prefix(&workdir).ok()?;
    let joined = prefix.join(path);
    Some(
        joined
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

/// Does a `.gitignore` rule match `path` (relative to `dir`), tracked or
/// not? `git check-ignore`'s question; errors count as not ignored.
pub fn is_ignored(dir: &Path, path: &str) -> bool {
    let Ok(repo) = open(dir) else {
        return false;
    };
    workdir_relative(&repo, dir, path)
        .and_then(|rel| repo.is_path_ignored(rel).ok())
        .unwrap_or(false)
}

/// Stage `paths` (relative to `dir`) the way `git add` does: new and
/// changed files go in, deleted ones come out, untracked ignored ones stay
/// out.
pub fn stage_paths(dir: &Path, paths: &[&str]) -> anyhow::Result<()> {
    let repo = open(dir).map_err(err)?;
    let specs: Vec<String> = paths
        .iter()
        .map(|path| {
            workdir_relative(&repo, dir, path)
                .ok_or_else(|| anyhow::anyhow!("{path} is outside the working tree"))
        })
        .collect::<anyhow::Result<_>>()?;
    let mut index = repo.index().map_err(err)?;
    index
        .add_all(specs.iter(), git2::IndexAddOption::DEFAULT, None)
        .map_err(err)?;
    index.update_all(specs.iter(), None).map_err(err)?;
    index.write().map_err(err)
}

/// Is `dir` itself a repository, bare or the top of a working tree? Unlike
/// the discovering open the other verbs use, a directory that merely lies
/// inside some repository is not one.
pub fn is_repository(dir: &Path) -> bool {
    git_environment();
    git2::Repository::open(dir).is_ok()
}

/// The branch HEAD names in the repository at `dir`, also when it has no
/// commit yet (an empty repository's default branch).
pub fn head_branch(dir: &Path) -> Option<String> {
    git_environment();
    let repo = git2::Repository::open(dir).ok()?;
    let head = repo.find_reference("HEAD").ok()?;
    let target = head.symbolic_target().ok()??.to_string();
    target.strip_prefix("refs/heads/").map(String::from)
}

/// Every file path in the tree at `refname`, sorted; empty when the ref has
/// no commit (an empty repository). The local counterpart of a forge
/// plugin's `files` answer.
pub fn tree_paths(dir: &Path, refname: &str) -> Vec<String> {
    let Ok(repo) = open(dir) else {
        return Vec::new();
    };
    let Some(tree) = repo
        .find_reference(refname)
        .ok()
        .and_then(|r| r.peel_to_tree().ok())
    else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    let _ = tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        if entry.kind() == Some(git2::ObjectType::Blob) {
            if let Ok(name) = entry.name() {
                paths.push(format!("{root}{name}"));
            }
        }
        git2::TreeWalkResult::Ok
    });
    paths.sort();
    paths
}

/// Point an unborn HEAD at `branch`: a clone of an empty repository has no
/// branch yet, and its first commit must land on the one the forge names
/// as the default. A HEAD that already has a commit is left alone.
pub fn set_unborn_branch(dir: &Path, branch: &str) -> anyhow::Result<()> {
    let repo = open(dir).map_err(err)?;
    if repo.head().is_ok() {
        return Ok(());
    }
    repo.set_head(&format!("refs/heads/{branch}")).map_err(err)
}

/// Commit exactly what the index holds, as `author`: joy init stages the
/// files it wrote, and a host commits them without guessing which.
pub fn commit_index(
    repo_dir: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(err)?;
    let mut index = repo.index().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let signature = signature_now(repo_dir, author_name, author_email)?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(err)?;
    Ok(oid.to_string())
}

/// The commit joy writes for itself after a command
/// (`auto_git_post_command`), scoped to the paths joy wrote (D3.4 of the
/// forge connection NG design).
///
/// The tree is the parent's tree with the entries under `pathspecs`
/// replaced by what the index holds there, so nothing else in the index
/// reaches the commit: a person's half finished `git add -p` is neither
/// swept into a "joy: ..." commit nor able to hide joy's own change
/// behind a whole tree comparison. After the git2 only move there is no
/// pre-commit hook left to stand in the way, which is why the rule lives
/// in the commit path itself.
///
/// `Ok(None)` when that scoped tree is the parent's: the "nothing to
/// commit" the git binary used to answer.
///
/// A pathspec is a literal path relative to the repository root, and it
/// covers everything below it when it names a directory. joy's own paths
/// are literal (`.joy`, `SECURITY.md`, a version file a release bumped),
/// never globs.
pub fn commit_index_paths(
    repo_dir: &Path,
    pathspecs: &[String],
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<Option<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let index = repo.index().map_err(err)?;
    if index.has_conflicts() {
        anyhow::bail!("the index has unresolved conflicts");
    }
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());

    // Start from what is committed, so every path outside the scope is
    // exactly what the parent had, whatever the index says about it.
    let mut scoped = git2::Index::new().map_err(err)?;
    if let Some(parent) = &parent {
        scoped
            .read_tree(&parent.tree().map_err(err)?)
            .map_err(err)?;
    }
    let in_scope = |path: &str| {
        pathspecs
            .iter()
            .any(|spec| path == spec || path.starts_with(&format!("{spec}/")))
    };
    // Drop the scope from the snapshot (this is what commits a deletion),
    // then take it back from the index.
    let doomed: Vec<PathBuf> = scoped
        .iter()
        .filter_map(|entry| entry_path(&entry))
        .filter(|path| in_scope(&path.to_string_lossy()))
        .collect();
    for path in doomed {
        scoped.remove_path(&path).map_err(err)?;
    }
    for entry in index.iter() {
        let Some(path) = entry_path(&entry) else {
            continue;
        };
        if in_scope(&path.to_string_lossy()) {
            scoped.add(&entry).map_err(err)?;
        }
    }

    let tree_id = scoped.write_tree_to(&repo).map_err(err)?;
    if parent.as_ref().map(|p| p.tree_id()) == Some(tree_id) {
        return Ok(None);
    }
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let signature = signature_now(repo_dir, author_name, author_email)?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(err)?;
    Ok(Some(oid.to_string()))
}

/// The path of an index entry as a path, or `None` for a name no
/// filesystem on this machine could hold anyway.
fn entry_path(entry: &git2::IndexEntry) -> Option<PathBuf> {
    std::str::from_utf8(&entry.path).ok().map(PathBuf::from)
}

/// Configure a named remote.
pub fn add_remote(dir: &Path, name: &str, url: &str) -> anyhow::Result<()> {
    let repo = open(dir).map_err(err)?;
    repo.remote(name, url).map_err(err)?;
    Ok(())
}

/// The file's content at `refname` (harness verification).
pub fn blob_at(dir: &Path, refname: &str, path: &str) -> Option<String> {
    let repo = open(dir).ok()?;
    let commit = repo.find_reference(refname).ok()?.peel_to_commit().ok()?;
    let entry = commit.tree().ok()?.get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).to_string())
}

/// Every path `refname` changed relative to its merge-base with
/// `base_refname` (harness verification).
pub fn changed_paths_between(
    dir: &Path,
    base_refname: &str,
    refname: &str,
) -> anyhow::Result<Vec<String>> {
    let repo = open(dir).map_err(err)?;
    let base_tip = repo
        .find_reference(base_refname)
        .map_err(err)?
        .peel_to_commit()
        .map_err(err)?
        .id();
    let tip = repo
        .find_reference(refname)
        .map_err(err)?
        .peel_to_commit()
        .map_err(err)?
        .id();
    let base = repo.merge_base(base_tip, tip).map_err(err)?;
    let base_tree = repo.find_commit(base).map_err(err)?.tree().map_err(err)?;
    let tip_tree = repo.find_commit(tip).map_err(err)?.tree().map_err(err)?;
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&tip_tree), None)
        .map_err(err)?;
    let mut paths = Vec::new();
    for delta in diff.deltas() {
        for f in [delta.new_file().path(), delta.old_file().path()]
            .into_iter()
            .flatten()
        {
            let p = f.display().to_string();
            if !paths.contains(&p) {
                paths.push(p);
            }
        }
    }
    Ok(paths)
}

/// Stage EVERYTHING and commit it (seeding and harness use; product
/// writes go through [`commit_joy`] / [`commit_all`], which respect the
/// `.joy` boundary).
pub fn commit_everything(
    repo_dir: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(err)?;
    let mut index = repo.index().map_err(err)?;
    index
        .add_all(["."], git2::IndexAddOption::DEFAULT, None)
        .map_err(err)?;
    index.write().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let sig = signature_now(repo_dir, author_name, author_email)?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|o| repo.find_commit(o).ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .map_err(err)?;
    Ok(oid.to_string())
}

/// Whether ANY path (untracked included) differs from HEAD — the volume
/// GC's conservative dirt check. Unreadable answers dirty: never delete
/// on doubt.
pub fn worktree_dirty(repo_dir: &Path) -> bool {
    let Ok(repo) = open(repo_dir) else {
        return true;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    repo.statuses(Some(&mut opts))
        .map(|s| !s.is_empty())
        .unwrap_or(true)
}

// ---- checkout observation (status surfaces) ----------------------------

/// Changed paths under `.joy/` (worktree or index), untracked included.
pub fn joy_dirty_paths(repo_dir: &Path) -> anyhow::Result<Vec<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(".joy");
    let statuses = repo.statuses(Some(&mut opts)).map_err(err)?;
    Ok(statuses
        .iter()
        .filter_map(|entry| entry.path().ok().map(str::to_string))
        .collect())
}

/// The dirty `.joy/` paths WITH their status flags — the change
/// fingerprint the app's debounce uses (a status change without a path
/// change must still arm it).
pub fn joy_dirty_fingerprint(repo_dir: &Path) -> Vec<String> {
    let Ok(repo) = open(repo_dir) else {
        return Vec::new();
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(".joy");
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return Vec::new();
    };
    statuses
        .iter()
        .filter_map(|e| e.path().ok().map(|p| format!("{}:{:?}", p, e.status())))
        .collect()
}

/// The commit signature of the acting member (D4.5 of the forge
/// connection NG design): the ONE place that decides what git2 stamps on
/// a commit joy writes.
///
/// `project` is the project the commit lands in, and it is what decides
/// the rule, never the shape of `member`: the caller may hand this an
/// address (every auth and crypt path still holds one until J11 lands),
/// and in an anonymous project that address is resolved to its opaque id
/// before anything is signed. Deciding by shape would have signed the
/// address that was handed in, which is exactly what ADR-042 forbids.
///
/// * Open mode: the e-mail is the member id (the member's address) and
///   the name is `config_name` when the caller established that git
///   config maps to THIS member, else the member id. joy never signs with
///   a name it cannot attribute.
/// * Anonymous mode (ADR-042): the opaque `m-<id>` in BOTH fields, never
///   the address and never a person's name, so a git2 commit cannot undo
///   the privacy mode. An address the project cannot map to a member is
///   refused rather than signed with: in an anonymous project there is no
///   safe way to write it down.
/// * Both fields are guaranteed non-empty, because `git_signature_new`
///   refuses an empty name or e-mail; when no member is known at all the
///   caller gets the typed error instead of a commit signed by nobody.
pub fn member_signature(
    project: Option<&crate::model::project::Project>,
    member: &str,
    config_name: Option<&str>,
) -> Result<(String, String), crate::error::JoyError> {
    let member = member.trim();
    if member.is_empty() {
        return Err(crate::error::JoyError::UnknownActingMember);
    }
    let anonymous =
        project.is_some_and(|p| p.privacy_mode() == crate::model::project::PrivacyMode::Anonymous);
    // The at-rest key of this member in THIS project: the map key when the
    // caller already had one, else the key the address resolves to.
    let key = project.and_then(|p| {
        p.member_by_key(member)
            .is_some()
            .then(|| member.to_string())
            .or_else(|| crate::privacy::member_key_for_email(p, member))
    });
    let key = match key {
        Some(key) => key,
        // An anonymous project that cannot name this member must not fall
        // back to what it was handed: that string is an address.
        None if anonymous => return Err(crate::error::JoyError::UnknownActingMember),
        None => member.to_string(),
    };
    if anonymous || crate::member_id::is_opaque_member_id(&key) {
        return Ok((key.clone(), key));
    }
    let name = config_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&key);
    Ok((name.to_string(), key))
}

/// The ONE way joy builds a `git2::Signature` (D4.5): every commit, tag
/// and merge joy writes goes through here, so the rule cannot be true in
/// one function and false in the next.
///
/// It applies [`member_signature`] to what the caller carries, against
/// the project in `repo_dir`, which guarantees the three things libgit2
/// and ADR-042 need: both strings are non-empty (`git_signature_new`
/// refuses an empty name or e-mail, signature.c:68-99), an anonymous
/// project signs with the opaque member id whether the caller carried the
/// id or the address, and a caller with no member at all gets the typed
/// `UnknownActingMember` instead of libgit2's "failed to parse signature".
fn signature_now(
    repo_dir: &Path,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<git2::Signature<'static>> {
    let project = crate::store::load_project(repo_dir).ok();
    let (name, email) = member_signature(project.as_ref(), author_email, Some(author_name))?;
    git2::Signature::now(&name, &email).map_err(err)
}

/// PREFILL ONLY (D4.5): the identity the repository's git config carries.
/// It is a suggestion for a mask and the source of the DISPLAY NAME in
/// [`member_signature`], never the identity of a commit and never a member
/// key on its own. Demoted from "what the CLI commits as": a Joy commit is
/// signed for the acting member, which joy resolves through
/// `joy_core::identity`, and a project may have no git config at all.
pub fn repo_identity(repo_dir: &Path) -> anyhow::Result<(String, String)> {
    let repo = open(repo_dir).map_err(err)?;
    let sig = repo.signature().map_err(|e| {
        anyhow::anyhow!(
            "git identity missing (user.name/user.email): {}",
            e.message()
        )
    })?;
    Ok((
        sig.name().unwrap_or_default().to_string(),
        sig.email().unwrap_or_default().to_string(),
    ))
}

/// Local branch names, plus remote branches without a local counterpart
/// (shown checkout-able in a dropdown).
pub fn branch_names(repo_dir: &Path) -> Vec<String> {
    let Ok(repo) = open(repo_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        for (branch, _) in branches.flatten() {
            if let Ok(Some(name)) = branch.name() {
                names.push(name.to_string());
            }
        }
    }
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Remote)) {
        for (branch, _) in branches.flatten() {
            if let Ok(Some(full)) = branch.name() {
                let short = full.split_once('/').map(|(_, b)| b).unwrap_or(full);
                if short != "HEAD" && !names.iter().any(|n| n == short) {
                    names.push(short.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

/// The forge's branch names as the clone knows them: the remote-tracking
/// refs without HEAD (JP-0126-72). The list a member chooses a branch
/// from; local-only branches (a job's working branches) are not on it.
pub fn remote_branch_names(repo_dir: &Path) -> Vec<String> {
    let Ok(repo) = open(repo_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Remote)) {
        for (branch, _) in branches.flatten() {
            if let Ok(Some(full)) = branch.name() {
                let short = full.split_once('/').map(|(_, b)| b).unwrap_or(full);
                if short != "HEAD" && !names.iter().any(|n| n == short) {
                    names.push(short.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

/// The URL of the remote joy actually contacts: `origin`, or the
/// first configured one ([`origin_or_first`]). Forge detection lives
/// with the caller.
///
/// This used to read `remotes.get(0)` while every contact took
/// `origin` (design D1.1). In a checkout with more than one remote the
/// two disagreed, and with them the throttle key, the credential
/// shape, the transport memory and the ownership join key - all keyed
/// on a host joy was not talking to.
pub fn remote_url(repo_dir: &Path) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    let remote = origin_or_first(&repo).ok()?;
    remote.url().ok().map(|u| u.to_string())
}

/// The oid a ref points at, as hex; `None` when absent or unborn.
pub fn ref_oid(repo_dir: &Path, refname: &str) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    repo.refname_to_id(refname).ok().map(|o| o.to_string())
}

/// HEAD's commit oid; `None` on an unborn branch.
pub fn head_oid(repo_dir: &Path) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    let oid = repo.head().ok()?.target().map(|o| o.to_string());
    oid
}

/// The author e-mail of the checkout's HEAD commit. The fallback merge
/// author for leftover dirt whose writer is no longer known (commits
/// found AHEAD after a restart): the merge finishes THAT member's
/// delivery, so it rides under the same name (JP-00DE-11).
pub fn head_author(repo_dir: &Path) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    let head = repo.head().ok()?.peel_to_commit().ok()?;
    let email = head.author().email().ok().map(|e| e.to_string());
    email
}

/// Local commits not on the remote-tracking branch and vice versa, as of
/// the last fetch (the sync button's honest counters).
pub fn ahead_behind(repo_dir: &Path) -> anyhow::Result<(u32, u32)> {
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?;
    let branch = head
        .shorthand()
        .map_err(|_| anyhow::anyhow!("detached HEAD"))?
        .to_string();
    let local = head
        .target()
        .ok_or_else(|| anyhow::anyhow!("unborn HEAD"))?;
    let upstream = match tracking_ref_name(&repo, &branch)
        .ok()
        .and_then(|name| repo.refname_to_id(&name).ok())
    {
        Some(oid) => oid,
        None => return Ok((0, 0)),
    };
    let (ahead, behind) = repo.graph_ahead_behind(local, upstream).map_err(err)?;
    Ok((ahead as u32, behind as u32))
}

/// Stage the given pathspecs (relative to the repo root) and commit them
/// as the acting account; returns the commit id, or None when nothing
/// under the pathspecs changed. Unlike [`commit_joy`] this stages
/// arbitrary paths: `joy release bump` patches version files OUTSIDE
/// `.joy/`, and a shared server checkout must never stay dirty.
pub fn commit_paths(
    repo_dir: &Path,
    pathspecs: &[String],
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<Option<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let mut index = repo.index().map_err(err)?;
    index
        .add_all(pathspecs, git2::IndexAddOption::DEFAULT, None)
        .map_err(err)?;
    index.write().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    if parent.as_ref().map(|p| p.tree_id()) == Some(tree_id) {
        return Ok(None);
    }
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let signature = signature_now(repo_dir, author_name, author_email)?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(err)?;
    Ok(Some(oid.to_string()))
}

/// Create an annotated tag on HEAD (the release record's tag), replacing
/// an existing tag of the same name (re-record after a failed publish).
pub fn tag_annotated(
    repo_dir: &Path,
    name: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let signature = signature_now(repo_dir, author_name, author_email)?;
    repo.tag(name, head.as_object(), &signature, message, true)
        .map_err(err)?;
    Ok(())
}

/// Push one tag to the forge (joy release publish's tag push).
pub fn push_tag(repo_dir: &Path, auth: &Auth, tag: &str) -> anyhow::Result<()> {
    super::contact::run_for(repo_dir, "push", auth.credentialed(), || {
        push_tag_raw(repo_dir, auth, tag)
    })
}

fn push_tag_raw(repo_dir: &Path, auth: &Auth, tag: &str) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    // The address the ssh config really names, not the alias the
    // remote was written as (design D1.4).
    let mut remote = contact_remote(&repo)?;
    let url = remote_url_of(&remote);
    let proxy = proxy_for(&url, Some(&repo))?;
    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(auth.callbacks(cred_source(Some(&repo))));
    opts.proxy_options(proxy.options());
    let refspec = format!("refs/tags/{tag}:refs/tags/{tag}");
    remote
        .push(&[refspec.as_str()], Some(&mut opts))
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Push, e))?;
    Ok(())
}

/// The newest local `v*` version tag by semver order, or None (no tags,
/// or not a git repo). The git2 counterpart of the CLI's shelled
/// `latest_version_tag` — headless callers stay on the one git engine
/// (JP-0034).
pub fn latest_version_tag(repo_dir: &Path) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    let names = repo.tag_names(Some("v*")).ok()?;
    let mut versions: Vec<(Vec<u64>, String)> = names
        .iter()
        .flatten()
        .flatten()
        .filter_map(|name| {
            let nums: Vec<u64> = name
                .trim_start_matches('v')
                .split('.')
                .map(|part| {
                    part.chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0)
                })
                .collect();
            (nums.len() == 3).then(|| (nums, name.to_string()))
        })
        .collect();
    versions.sort();
    versions.pop().map(|(_, name)| name)
}

/// Create a linked git worktree of the project checkout on a fresh branch
/// (the Job Container works here; it shares the checkout's object database).
/// Returns the worktree directory.
pub fn create_worktree(
    repo_dir: &Path,
    worktree_name: &str,
    branch: &str,
    worktree_path: &Path,
) -> anyhow::Result<PathBuf> {
    let repo = open(repo_dir).map_err(err)?;
    // Branch off the current HEAD commit.
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    if repo.find_branch(branch, git2::BranchType::Local).is_err() {
        repo.branch(branch, &head, false).map_err(err)?;
    }
    let reference = repo
        .find_reference(&format!("refs/heads/{branch}"))
        .map_err(err)?;
    let mut opts = git2::WorktreeAddOptions::new();
    opts.reference(Some(&reference));
    repo.worktree(worktree_name, worktree_path, Some(&opts))
        .map_err(|e| anyhow::anyhow!("worktree add failed: {}", e.message()))?;
    Ok(worktree_path.to_path_buf())
}

/// Stage every change in the worktree EXCEPT `.joy/` and commit as the AI
/// member. Returns None when that leaves nothing to commit. Unlike
/// `commit_joy` this commits code, and it is the fallback for work an agent
/// left uncommitted — item state never rides a job branch (JP-006D-28), so
/// `.joy` paths are excluded from staging (and any `.joy` change the agent
/// staged itself is unstaged first).
pub fn commit_all(
    worktree_dir: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<Option<String>> {
    let repo = open(worktree_dir).map_err(err)?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    // Unstage anything the agent staged under .joy (git add without commit),
    // so the fallback tree below cannot carry it.
    if let Some(p) = &parent {
        repo.reset_default(Some(p.as_object()), [".joy"]).ok();
    }
    let mut index = repo.index().map_err(err)?;
    index
        .add_all(
            ["*"],
            git2::IndexAddOption::DEFAULT,
            Some(&mut |path: &Path, _spec: &[u8]| -> i32 {
                if path.starts_with(".joy") {
                    1 // skip: item state never rides the job branch
                } else {
                    0
                }
            }),
        )
        .map_err(err)?;
    index.write().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;
    if parent.as_ref().map(|p| p.tree_id()) == Some(tree_id) {
        return Ok(None); // nothing but (excluded) .joy noise changed
    }
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let signature = signature_now(worktree_dir, author_name, author_email)?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(err)?;
    Ok(Some(oid.to_string()))
}

/// Push the worktree's branch to the forge with the account token.
pub fn push_branch(worktree_dir: &Path, auth: &Auth) -> anyhow::Result<()> {
    push(worktree_dir, auth)
}

/// Remove a linked worktree and its registration in the checkout.
pub fn prune_worktree(repo_dir: &Path, worktree_name: &str, worktree_path: &Path) {
    std::fs::remove_dir_all(worktree_path).ok();
    if let Ok(repo) = open(repo_dir) {
        if let Ok(wt) = repo.find_worktree(worktree_name) {
            // best effort by contract — a stale registration only blocks
            // the next create_worktree, but the cause belongs in the log
            if let Err(e) = wt.prune(Some(git2::WorktreePruneOptions::new().valid(true))) {
                tracing::warn!(worktree = %worktree_name, error = %e.message(),
                    "worktree registration prune failed");
            }
        }
    }
}

// ---- job sandbox: the joywork checkout (JP-006D-28) ---------------------
//
// jobs/<job-id>/joywork is the agent's item-state surface: a git2 worktree
// of the project checkout at MAIN's tip. refs/heads/main is already checked
// out in the platform checkout and a branch cannot be checked out twice, so
// the joywork worktree sits on a per-job LOCAL branch
// `joy/jobwork/<job-id>` forked at main's tip (never pushed). After the run
// the platform commits joywork's `.joy` changes on that branch and lands
// them on main via [`land_branch_yaml`].

/// The local (never pushed) branch carrying a job's joywork checkout.
pub fn jobwork_branch(job_id: &str) -> String {
    format!("joy/jobwork/{job_id}")
}

fn jobwork_worktree_name(job_id: &str) -> String {
    format!("jobwork-{job_id}")
}

/// Create the joywork worktree for a job: fork `joy/jobwork/<job-id>` at
/// the checkout's current HEAD (main's tip) and check it out at
/// `joywork_path`. A stale registration or branch from a crashed run is
/// replaced.
pub fn create_joywork(
    repo_dir: &Path,
    job_id: &str,
    joywork_path: &Path,
) -> anyhow::Result<PathBuf> {
    let repo = open(repo_dir).map_err(err)?;
    let name = jobwork_worktree_name(job_id);
    if let Ok(wt) = repo.find_worktree(&name) {
        // the working tree is gone (caller checked); drop the registration
        if let Err(e) = wt.prune(Some(git2::WorktreePruneOptions::new().valid(true))) {
            tracing::warn!(worktree = %name, error = %e.message(),
                "stale joywork registration prune failed");
        }
    }
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let refname = format!("refs/heads/{}", jobwork_branch(job_id));
    repo.reference(&refname, head.id(), true, "joy-vcs: joywork fork")
        .map_err(err)?;
    let reference = repo.find_reference(&refname).map_err(err)?;
    let mut opts = git2::WorktreeAddOptions::new();
    opts.reference(Some(&reference));
    repo.worktree(&name, joywork_path, Some(&opts))
        .map_err(|e| anyhow::anyhow!("joywork add failed: {}", e.message()))?;
    Ok(joywork_path.to_path_buf())
}

/// Remove a job's joywork worktree and its local jobwork branch.
pub fn prune_joywork(repo_dir: &Path, job_id: &str, joywork_path: &Path) {
    prune_worktree(repo_dir, &jobwork_worktree_name(job_id), joywork_path);
    if let Ok(repo) = open(repo_dir) {
        if let Ok(mut b) = repo.find_branch(&jobwork_branch(job_id), git2::BranchType::Local) {
            if let Err(e) = b.delete() {
                tracing::warn!(job = %job_id, error = %e.message(),
                    "jobwork branch delete failed; a later round re-points it");
            }
        }
    }
}

/// Which branch every linked worktree of the clone has checked out
/// (JP-0126-72): (worktree name, branch). A worktree on a detached HEAD
/// or one that cannot be opened is left out.
pub fn worktree_branches(repo_dir: &Path) -> Vec<(String, String)> {
    let Ok(repo) = open(repo_dir) else {
        return Vec::new();
    };
    let Ok(names) = repo.worktrees() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for name in (0..names.len()).filter_map(|i| names.get(i).ok().flatten()) {
        let Ok(wt) = repo.find_worktree(name) else {
            continue;
        };
        let Ok(wt_repo) = git2::Repository::open_from_worktree(&wt) else {
            continue;
        };
        let Ok(head) = wt_repo.head() else {
            continue;
        };
        if !head.is_branch() {
            continue;
        }
        if let Ok(branch) = head.shorthand() {
            out.push((name.to_string(), branch.to_string()));
        }
    }
    out
}

/// The tip of `branch`: the local branch, else the remote-tracking ref.
fn any_branch_tip(repo: &git2::Repository, branch: &str) -> anyhow::Result<git2::Oid> {
    if let Ok(oid) = repo.refname_to_id(&format!("refs/heads/{branch}")) {
        return Ok(oid);
    }
    let remote = origin_or_first(repo)?;
    let name = remote
        .name()
        .map_err(err)?
        .ok_or_else(|| anyhow::anyhow!("remote name is not utf-8"))?;
    repo.refname_to_id(&format!("refs/remotes/{name}/{branch}"))
        .map_err(|_| anyhow::anyhow!("unknown branch: {branch}"))
}

/// A READ-ONLY view of `branch` in its own worktree (JP-0126-72): git
/// keeps a branch in one worktree, and a job holds its branch in its
/// own, so a member who wants to read that branch gets a worktree with a
/// DETACHED head standing at the branch's tip: no branch of its own,
/// nothing to push, nothing to list. Calling again moves the head to the
/// tip the branch has now. A view of an earlier make that stood on a
/// mirror branch is detached here and the mirror dropped.
pub fn ensure_view_worktree(
    repo_dir: &Path,
    name: &str,
    branch: &str,
    path: &Path,
) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    let tip = any_branch_tip(&repo, branch)?;
    if path.join(".git").exists() {
        let view = open(path).map_err(err)?;
        let head = view.head().ok();
        let on_branch = head
            .as_ref()
            .filter(|h| h.is_branch())
            .and_then(|h| h.shorthand().ok().map(str::to_string));
        if on_branch.is_none() && head.as_ref().and_then(|h| h.target()) == Some(tip) {
            return Ok(());
        }
        drop(head);
        view.set_head_detached(tip).map_err(err)?;
        view.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(|e| anyhow::anyhow!("view checkout: {}", e.message()))?;
        if let Some(mirror) = on_branch {
            if let Ok(mut b) = repo.find_branch(&mirror, git2::BranchType::Local) {
                let _ = b.delete();
            }
        }
        return Ok(());
    }
    if let Ok(wt) = repo.find_worktree(name) {
        // the working tree is gone; drop the registration
        let _ = wt.prune(Some(git2::WorktreePruneOptions::new().valid(true)));
    }
    // libgit2 adds a worktree on a branch: a transient one, detached from
    // and deleted the moment the worktree stands
    let transient = format!("refs/heads/joy/view-add/{name}");
    repo.reference(&transient, tip, true, "joy-vcs: view (transient)")
        .map_err(err)?;
    let reference = repo.find_reference(&transient).map_err(err)?;
    let mut opts = git2::WorktreeAddOptions::new();
    opts.reference(Some(&reference));
    repo.worktree(name, path, Some(&opts))
        .map_err(|e| anyhow::anyhow!("view worktree add failed: {}", e.message()))?;
    let view = open(path).map_err(err)?;
    view.set_head_detached(tip).map_err(err)?;
    if let Ok(mut b) = repo.find_branch(&format!("joy/view-add/{name}"), git2::BranchType::Local) {
        b.delete().map_err(err)?;
    }
    Ok(())
}

/// The commit id a local branch points at.
pub fn branch_tip(repo_dir: &Path, branch: &str) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(err)?;
    Ok(repo
        .refname_to_id(&format!("refs/heads/{branch}"))
        .map_err(err)?
        .to_string())
}

/// Validate a job branch's own commits (fork-point..HEAD of the worktree):
/// returns the first commit whose diff against its first parent touches a
/// path under `.joy/`, as `(commit-description, path)`. Item state never
/// rides a job branch — the agent's `.joy` writes belong in the joywork
/// checkout (JP-006D-28). `None` means the branch is clean.
pub fn joy_commit_on_branch(
    checkout_dir: &Path,
    worktree_dir: &Path,
) -> anyhow::Result<Option<(String, String)>> {
    let main_repo = open(checkout_dir).map_err(err)?;
    let main_oid = main_repo
        .head()
        .map_err(err)?
        .target()
        .ok_or_else(|| anyhow::anyhow!("unborn HEAD in checkout"))?;
    // the linked worktree shares the checkout's object database, so main's
    // oid resolves here too
    let repo = open(worktree_dir).map_err(err)?;
    let tip = repo
        .head()
        .map_err(err)?
        .target()
        .ok_or_else(|| anyhow::anyhow!("unborn HEAD in worktree"))?;
    let fork = repo.merge_base(tip, main_oid).map_err(err)?;
    let mut walk = repo.revwalk().map_err(err)?;
    walk.push(tip).map_err(err)?;
    walk.hide(fork).map_err(err)?;
    for oid in walk {
        let oid = oid.map_err(err)?;
        let commit = repo.find_commit(oid).map_err(err)?;
        let tree = commit.tree().map_err(err)?;
        let parent_tree = match commit.parents().next() {
            Some(p) => Some(p.tree().map_err(err)?),
            None => None,
        };
        let diff = repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .map_err(err)?;
        for delta in diff.deltas() {
            for f in [delta.new_file().path(), delta.old_file().path()]
                .into_iter()
                .flatten()
            {
                if f.starts_with(".joy") {
                    let describe = format!(
                        "{:.8} ({})",
                        oid.to_string(),
                        commit.summary().ok().flatten().unwrap_or_default()
                    );
                    return Ok(Some((describe, f.display().to_string())));
                }
            }
        }
    }
    Ok(None)
}

/// The first path under `.joy/` in the diff of `branch` against its merge
/// base with HEAD (main) — AcceptJob's second line of defense, which also
/// catches direct commits pushed to the branch on the forge.
pub fn branch_touches_joy(repo_dir: &Path, branch: &str) -> anyhow::Result<Option<String>> {
    let repo = open(repo_dir).map_err(err)?;
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let their = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|e| anyhow::anyhow!("branch {branch}: {}", e.message()))?
        .into_reference()
        .peel_to_commit()
        .map_err(err)?;
    let base_oid = repo
        .merge_base(head.id(), their.id())
        .unwrap_or_else(|_| head.id());
    let base_tree = repo
        .find_commit(base_oid)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let their_tree = their.tree().map_err(err)?;
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&their_tree), None)
        .map_err(err)?;
    for delta in diff.deltas() {
        for f in [delta.new_file().path(), delta.old_file().path()]
            .into_iter()
            .flatten()
        {
            if f.starts_with(".joy") {
                return Ok(Some(f.display().to_string()));
            }
        }
    }
    Ok(None)
}

/// Land a jobwork branch's `.joy` commit(s) onto the current branch (main)
/// of the checkout: fast-forward when main has not moved since the fork,
/// otherwise a three-way merge whose conflicting `.joy/*.yaml` files
/// resolve through joy-core's YAML engine (same policy as [`pull_merge`]).
/// Does NOT push — the caller lands this in the same gate-locked phase as
/// the attempt write and pushes main once.
pub fn land_branch_yaml(
    repo_dir: &Path,
    branch: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let span = tracing::info_span!("git.land_branch_yaml", repo = %repo_dir.display(), %branch);
    let _s = span.enter();
    let result = land_branch_yaml_inner(repo_dir, branch, message, author_name, author_email);
    if let Err(e) = &result {
        tracing::error!(repo = %repo_dir.display(), %branch, error = %e,
            "landing joywork changes on main failed");
    }
    result
}

fn land_branch_yaml_inner(
    repo_dir: &Path,
    branch: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(err)?;
    let head_refname = repo
        .head()
        .map_err(err)?
        .name()
        .map_err(|_| anyhow::anyhow!("unnamed HEAD"))?
        .to_string();
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let their = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|e| anyhow::anyhow!("branch {branch}: {}", e.message()))?
        .into_reference()
        .peel_to_commit()
        .map_err(err)?;
    let base = repo.merge_base(head.id(), their.id()).map_err(err)?;
    if their.id() == head.id() || base == their.id() {
        return Ok(head.id().to_string()); // nothing new on the branch
    }
    if base == head.id() {
        // main has not moved since the fork: fast-forward
        let mut reference = repo.find_reference(&head_refname).map_err(err)?;
        reference
            .set_target(their.id(), "joy-vcs: land joywork (ff)")
            .map_err(err)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(err)?;
        return Ok(their.id().to_string());
    }
    let mut index = repo.merge_commits(&head, &their, None).map_err(err)?;
    resolve_conflicts_yaml_aware(&repo, &mut index)?;
    let tree_id = index.write_tree_to(&repo).map_err(err)?;
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let sig = signature_now(repo_dir, author_name, author_email)?;
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head, &their])
        .map_err(err)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
        .map_err(err)?;
    repo.cleanup_state().ok();
    Ok(oid.to_string())
}

/// Best-effort refresh of a local branch from the forge before AcceptJob's
/// checks: fetch `refs/heads/<branch>` and point the local ref at the
/// remote tip (the forge is the source of truth — this is how direct
/// commits pushed to the branch become visible to the `.joy` freeze and
/// the merge). Errors (offline, unborn remote ref) leave the local state.
pub fn refresh_branch_from_forge(repo_dir: &Path, branch: &str, auth: &Auth) {
    let refresh = || -> anyhow::Result<()> {
        let repo = open(repo_dir).map_err(err)?;
        let tracking = format!("refs/joy/branch-refresh/{branch}");
        let src = format!("refs/heads/{branch}");
        // download_ref, like every fetch here: libgit2's update_tips (and
        // its unconditional FETCH_HEAD truncation) never runs
        let Some(tip) = download_ref(&repo, auth, &src, &tracking)? else {
            anyhow::bail!("branch {branch} not on the forge");
        };
        repo.reference(
            &format!("refs/heads/{branch}"),
            tip,
            true,
            "joy-vcs: refresh job branch from forge",
        )
        .map_err(err)?;
        if let Ok(mut done) = repo.find_reference(&tracking) {
            done.delete().ok();
        }
        Ok(())
    };
    if let Err(e) = refresh() {
        tracing::debug!(%branch, error = %e, "job branch refresh skipped; using local state");
    }
}

/// AcceptJob's merge (JP-006D-28): refuse when the branch diff (merge
/// base..branch) touches `.joy/` — job branches must not carry item state;
/// `.joy` changes ride main via the joywork landing — then merge the
/// branch into main.
pub fn merge_job_branch(
    repo_dir: &Path,
    branch: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let span = tracing::info_span!("git.merge_job_branch", repo = %repo_dir.display(), %branch);
    let _s = span.enter();
    let result = (|| -> anyhow::Result<String> {
        if let Some(path) = branch_touches_joy(repo_dir, branch)? {
            anyhow::bail!(
                "branch {branch} touches {path}: job branches must not carry item state; \
                 .joy changes ride main (JP-006D-28)"
            );
        }
        merge_branch(repo_dir, branch, message, author_name, author_email)
    })();
    if let Err(e) = &result {
        tracing::error!(repo = %repo_dir.display(), %branch, error = %e,
            "job branch merge refused or failed");
    }
    result
}

/// Merge `branch` into the current branch (main) with a merge commit and
/// return the new commit id. Code changes (branch) and record changes
/// (main) touch different files, so the merge is clean; a genuine conflict
/// is surfaced as an error rather than a fake success.
pub fn merge_branch(
    repo_dir: &std::path::Path,
    branch: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> anyhow::Result<String> {
    let repo = open(repo_dir).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let their = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|e| anyhow::anyhow!("branch {branch}: {}", e.message()))?
        .into_reference()
        .peel_to_commit()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let head = repo.head()?.peel_to_commit()?;
    let annotated = repo.find_annotated_commit(their.id())?;
    repo.merge(&[&annotated], None, None)
        .map_err(|e| anyhow::anyhow!("merge {branch}: {}", e.message()))?;
    if repo.index()?.has_conflicts() {
        repo.cleanup_state().ok();
        anyhow::bail!("merge of {branch} has conflicts");
    }
    let mut index = repo.index()?;
    let tree = repo.find_tree(index.write_tree()?)?;
    let sig = signature_now(repo_dir, author_name, author_email)?;
    let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head, &their])?;
    repo.cleanup_state().ok();
    // reset the working tree to the merged commit
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
    Ok(oid.to_string())
}

/// One changed file in a job branch vs its base.
pub struct DiffFile {
    pub path: String,
    pub patch: String,
    pub additions: u32,
    pub deletions: u32,
}

/// The diff of `branch` against its merge-base with HEAD (main): what the AI
/// proposes. Returns one entry per changed file with a unified patch.
pub fn branch_diff(repo_dir: &std::path::Path, branch: &str) -> anyhow::Result<Vec<DiffFile>> {
    let repo = open(repo_dir).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let head = repo.head()?.peel_to_commit()?;
    let their = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|e| anyhow::anyhow!("branch {branch}: {}", e.message()))?
        .into_reference()
        .peel_to_commit()?;
    let base_oid = repo
        .merge_base(head.id(), their.id())
        .unwrap_or_else(|_| head.id());
    let base_tree = repo.find_commit(base_oid)?.tree()?;
    let their_tree = their.tree()?;
    let mut opts = git2::DiffOptions::new();
    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&their_tree), Some(&mut opts))?;

    // Collect per-file patches by walking the diff.
    let mut files: Vec<DiffFile> = Vec::new();
    let num_deltas = diff.deltas().len();
    for i in 0..num_deltas {
        if let Some(mut patch) = git2::Patch::from_diff(&diff, i)? {
            let delta = patch
                .delta()
                .new_file()
                .path()
                .or_else(|| patch.delta().old_file().path())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let (_ctx, adds, dels) = patch.line_stats()?;
            let buf = patch.to_buf()?;
            files.push(DiffFile {
                path: delta,
                patch: String::from_utf8_lossy(&buf).to_string(),
                additions: adds as u32,
                deletions: dels as u32,
            });
        }
    }
    Ok(files)
}

/// Branch state of a checkout for the web header (JP-0062): current branch,
/// all local branches, ahead/behind vs the remote counterpart.
pub struct BranchState {
    pub branch: String,
    pub branches: Vec<String>,
    pub ahead: u32,
    pub behind: u32,
    pub has_remote: bool,
}

pub fn branch_state(repo_dir: &Path) -> anyhow::Result<BranchState> {
    let repo = open(repo_dir).map_err(err)?;
    // an unborn HEAD (fresh init, no commit yet) is a state, not an error
    let head = repo.head().ok();
    let branch = head
        .as_ref()
        .and_then(|h| h.shorthand().ok())
        .unwrap_or("HEAD")
        .to_string();
    let mut branches = Vec::new();
    for b in repo.branches(Some(git2::BranchType::Local)).map_err(err)? {
        let (b, _) = b.map_err(err)?;
        if let Some(name) = b.name().map_err(err)? {
            branches.push(name.to_string());
        }
    }
    branches.sort();
    let (mut ahead, mut behind, mut has_remote) = (0u32, 0u32, false);
    if let (Some(local), Ok(upstream)) = (
        head.as_ref().and_then(|h| h.target()),
        repo.find_branch(&branch, git2::BranchType::Local)
            .and_then(|b| b.upstream())
            .and_then(|u| {
                u.into_reference()
                    .target()
                    .ok_or_else(|| git2::Error::from_str("no upstream target"))
            }),
    ) {
        has_remote = true;
        if let Ok((a, b)) = repo.graph_ahead_behind(local, upstream) {
            ahead = a as u32;
            behind = b as u32;
        }
    }
    Ok(BranchState {
        branch,
        branches,
        ahead,
        behind,
        has_remote,
    })
}

/// Make sure `branch` exists as a LOCAL branch, creating it from the
/// remote-tracking ref when only the forge has it (JP-0126-72: a member's
/// worktree starts on the branch's forge tip, never on somebody's HEAD).
/// Errors when neither side knows the branch. Never moves HEAD.
pub fn ensure_local_branch(repo_dir: &Path, branch: &str) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    if repo.find_branch(branch, git2::BranchType::Local).is_ok() {
        return Ok(());
    }
    let remote_ref = repo
        .branches(Some(git2::BranchType::Remote))
        .map_err(err)?
        .flatten()
        .find(|(b, _)| {
            b.name()
                .ok()
                .flatten()
                .map(|full| full.split_once('/').map(|(_, s)| s).unwrap_or(full) == branch)
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow::anyhow!("unknown branch: {branch}"))?;
    let target = remote_ref.0.get().peel_to_commit().map_err(err)?;
    let mut local = repo.branch(branch, &target, false).map_err(err)?;
    // upstream set, so ahead/behind and the fast-forward know their ref
    if let Ok(Some(name)) = remote_ref.0.name() {
        local.set_upstream(Some(name)).map_err(err)?;
    }
    Ok(())
}

/// Bring EVERY branch of the forge into the remote-tracking refs, the way
/// [`fetch_branch`] does for the working branch alone (JP-0126-72: the
/// branch list a member chooses from is the forge's, not the clone's
/// memory of its first day). FETCH_HEAD is not touched. Returns the
/// branch names the forge advertises.
pub fn fetch_heads(repo_dir: &Path, auth: &Auth) -> anyhow::Result<Vec<String>> {
    super::contact::run_for(repo_dir, "fetch", auth.credentialed(), || {
        fetch_heads_raw(repo_dir, auth)
    })
}

fn fetch_heads_raw(repo_dir: &Path, auth: &Auth) -> anyhow::Result<Vec<String>> {
    let span = tracing::info_span!("git.fetch-heads", repo = %repo_dir.display());
    let _s = span.enter();
    let repo = open(repo_dir).map_err(err)?;
    // The tracking refs are named after the CONFIGURED remote, which
    // is read before the contact: the contact itself may run over an
    // anonymous remote when the ssh config renames the host, and an
    // anonymous remote has no name (see `contact_remote`).
    let remote_name = {
        let configured = origin_or_first(&repo)?;
        configured
            .name()
            .map_err(err)?
            .ok_or_else(|| anyhow::anyhow!("remote name is not utf-8"))?
            .to_string()
    };
    // The address the ssh config really names, not the alias the
    // remote was written as (design D1.4).
    let mut remote = contact_remote(&repo)?;
    let url = remote_url_of(&remote);
    let proxy = proxy_for(&url, Some(&repo))?;
    // one connection for the advertisement and the download, as
    // download_ref does (D1.9)
    let mut connection = remote
        .connect_auth(
            git2::Direction::Fetch,
            Some(auth.callbacks(cred_source(Some(&repo)))),
            Some(proxy.options()),
        )
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Fetch, e))?;
    let heads: Vec<(String, git2::Oid)> = connection
        .list()
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Fetch, e))?
        .iter()
        .filter_map(|r| {
            r.name()
                .strip_prefix("refs/heads/")
                .map(|b| (b.to_string(), r.oid()))
        })
        .collect();
    if heads.is_empty() {
        return Ok(Vec::new());
    }
    let refspecs: Vec<String> = heads
        .iter()
        .map(|(b, _)| format!("+refs/heads/{b}:refs/remotes/{remote_name}/{b}"))
        .collect();
    let refspec_strs: Vec<&str> = refspecs.iter().map(String::as_str).collect();
    let mut opts = git2::FetchOptions::new();
    opts.remote_callbacks(auth.callbacks(cred_source(Some(&repo))));
    opts.proxy_options(proxy.options());
    connection
        .remote()
        .download(&refspec_strs, Some(&mut opts))
        .map_err(|e| contact_failed(&url, auth, super::contact::ContactDirection::Fetch, e))?;
    drop(connection);
    // the tracking refs by hand, as download_ref does: update_tips would
    // write FETCH_HEAD
    for (b, tip) in &heads {
        repo.reference(
            &format!("refs/remotes/{remote_name}/{b}"),
            *tip,
            true,
            "joy-vcs: fetch heads",
        )
        .map_err(err)?;
    }
    Ok(heads.into_iter().map(|(b, _)| b).collect())
}

/// The forge's default branch as the clone recorded it (`origin/HEAD`,
/// JP-0126-72): `None` when the clone carries no such pointer.
pub fn remote_head_branch(repo_dir: &Path) -> Option<String> {
    let repo = open(repo_dir).ok()?;
    let remote = origin_or_first(&repo).ok()?;
    let name = remote.name().ok()??.to_string();
    let head = repo
        .find_reference(&format!("refs/remotes/{name}/HEAD"))
        .ok()?;
    let target = head.symbolic_target().ok()??;
    target
        .strip_prefix(&format!("refs/remotes/{name}/"))
        .map(str::to_string)
}

/// Switch the checkout to `branch` — creating a local branch from the
/// remote-tracking ref when only the remote has it. Tree first, HEAD
/// second: the reverse order leaves HEAD on the new branch with the old
/// worktree when checkout fails or is partial, which reads as dirty and
/// blocks every further switch (JAPP-0097).
pub fn checkout_branch(repo_dir: &Path, branch: &str) -> anyhow::Result<()> {
    let repo = open(repo_dir).map_err(err)?;
    let refname = format!("refs/heads/{branch}");
    if repo.find_reference(&refname).is_err() {
        let remote_ref = repo
            .branches(Some(git2::BranchType::Remote))
            .map_err(err)?
            .flatten()
            .find(|(b, _)| {
                b.name()
                    .ok()
                    .flatten()
                    .map(|full| full.split_once('/').map(|(_, s)| s).unwrap_or(full) == branch)
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow::anyhow!("unknown branch: {branch}"))?;
        let target = remote_ref.0.get().peel_to_commit().map_err(err)?;
        repo.branch(branch, &target, false).map_err(err)?;
    }
    let target = repo
        .find_reference(&refname)
        .map_err(err)?
        .peel_to_commit()
        .map_err(err)?;
    repo.checkout_tree(
        target.as_object(),
        Some(git2::build::CheckoutBuilder::new().safe()),
    )
    .map_err(|e| anyhow::anyhow!("checkout: {}", e.message()))?;
    repo.set_head(&refname).map_err(err)?;
    Ok(())
}
#[cfg(test)]
mod init_on_git2_tests {
    use super::*;

    #[test]
    fn a_project_created_in_an_empty_repository_lands_on_the_named_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        init_worktree(&repo_dir).unwrap();
        set_unborn_branch(&repo_dir, "trunk").unwrap();
        assert!(is_repository(&repo_dir));
        assert_eq!(head_branch(&repo_dir).as_deref(), Some("trunk"));
        // a directory inside the repository is not a repository of its own
        std::fs::create_dir_all(repo_dir.join("inner")).unwrap();
        assert!(!is_repository(&repo_dir.join("inner")));
        std::fs::write(repo_dir.join("staged.md"), "in").unwrap();
        std::fs::write(repo_dir.join("unstaged.md"), "out").unwrap();
        stage_paths(&repo_dir, &["staged.md"]).unwrap();

        commit_index(&repo_dir, "set up", "Founder", "founder@example.com").unwrap();

        let repo = git2::Repository::open(&repo_dir).unwrap();
        assert_eq!(repo.head().unwrap().name().ok(), Some("refs/heads/trunk"));
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        assert!(tree.get_name("staged.md").is_some());
        assert!(
            tree.get_name("unstaged.md").is_none(),
            "only what was staged"
        );
        assert_eq!(tree_paths(&repo_dir, "HEAD"), vec!["staged.md".to_string()]);
        // a branch that has a commit is not moved
        set_unborn_branch(&repo_dir, "other").unwrap();
        assert_eq!(repo.head().unwrap().name().ok(), Some("refs/heads/trunk"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The proxy of THIS contact reaches the failure (D1.11, D1.8c).
    /// joy's own decision travels in a cell, because the alternative is
    /// a parameter on fifteen call sites, and `contact_failed` puts it
    /// into the evidence: a 407 then names the machine in the middle
    /// and never the forge.
    #[test]
    fn a_407_behind_a_proxy_names_the_proxy_and_the_cell_is_per_contact() {
        let libgit2 = || {
            git2::Error::new(
                git2::ErrorCode::Auth,
                git2::ErrorClass::Http,
                "proxy authentication required but no callback set",
            )
        };
        super::super::proxy::note(Some("proxy.acme.example:8080"));
        let failure = contact_failed(
            "https://github.com/joyint/joy.git",
            &Auth::Local,
            super::super::contact::ContactDirection::Fetch,
            libgit2(),
        );
        let text = format!("{failure}");
        assert!(
            text.contains("proxy.acme.example:8080"),
            "the proxy is named: {text}"
        );
        assert!(!text.contains("github.com"), "and the forge is not: {text}");

        // The cell belongs to one contact: the boundary forgets it
        // before the next one runs, and a contact that took no proxy
        // names none.
        super::super::proxy::forget();
        let failure = contact_failed(
            "https://github.com/joyint/joy.git",
            &Auth::Local,
            super::super::contact::ContactDirection::Fetch,
            libgit2(),
        );
        let text = format!("{failure}");
        assert!(
            !text.contains("proxy.acme.example"),
            "the last contact's proxy is not this one's: {text}"
        );
        assert!(
            text.contains("A proxy in front of github.com"),
            "and an unnamed proxy is said to be in front of the forge: {text}"
        );
    }

    /// D4.5, open mode: the e-mail is the member, and the display name is
    /// the git config name only when that config maps to this member.
    #[test]
    fn an_open_mode_signature_carries_the_member_as_the_address() {
        assert_eq!(
            member_signature(None, "scotty@example.com", Some("Scotty")).unwrap(),
            ("Scotty".to_string(), "scotty@example.com".to_string())
        );
        // no name joy can attribute: the member id stands in for it
        assert_eq!(
            member_signature(None, "scotty@example.com", None).unwrap(),
            (
                "scotty@example.com".to_string(),
                "scotty@example.com".to_string()
            )
        );
        // an empty or blank configured name is not a name (git2 refuses it)
        assert_eq!(
            member_signature(None, "scotty@example.com", Some("   ")).unwrap(),
            (
                "scotty@example.com".to_string(),
                "scotty@example.com".to_string()
            )
        );
    }

    /// D4.5, anonymous mode (ADR-042): the opaque id in BOTH fields, and
    /// no git config name can smuggle a person's name into a commit.
    #[test]
    fn an_anonymous_signature_is_the_opaque_id_in_both_fields() {
        let id = crate::member_id::opaque_member_id(&"ab".repeat(32)).unwrap();
        assert!(crate::member_id::is_opaque_member_id(&id));
        assert_eq!(
            member_signature(None, &id, Some("Scotty")).unwrap(),
            (id.clone(), id.clone())
        );
    }

    /// D4.5, at the gate every commit goes through: `signature_now` is
    /// the only way joy builds a signature, so an empty display name can
    /// no longer reach libgit2 as "failed to parse signature", and an
    /// anonymous member id cannot pick up a name on the way in.
    #[test]
    fn every_commit_passes_the_signature_gate() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        stage_paths(dir.path(), &["a.txt"]).unwrap();

        // A member with no configured name: the member stands in for it,
        // where git2 would have refused the empty string outright.
        let oid = commit_index(dir.path(), "first", "", "scotty@example.com").unwrap();
        let commit = repo
            .find_commit(git2::Oid::from_str(&oid).unwrap())
            .unwrap();
        assert_eq!(commit.author().name().ok(), Some("scotty@example.com"));
        assert_eq!(commit.author().email().ok(), Some("scotty@example.com"));

        // An anonymous member id keeps both fields, whatever name a
        // caller carries beside it.
        let id = crate::member_id::opaque_member_id(&"cd".repeat(32)).unwrap();
        std::fs::write(dir.path().join("a.txt"), "b").unwrap();
        stage_paths(dir.path(), &["a.txt"]).unwrap();
        let oid = commit_index(dir.path(), "second", "Scotty", &id).unwrap();
        let commit = repo
            .find_commit(git2::Oid::from_str(&oid).unwrap())
            .unwrap();
        assert_eq!(commit.author().name().ok(), Some(id.as_str()));
        assert_eq!(commit.author().email().ok(), Some(id.as_str()));

        // Nobody at all: the typed error, not libgit2's parse failure.
        let err = commit_index(dir.path(), "third", "Scotty", "  ").unwrap_err();
        assert!(
            err.to_string()
                .starts_with("this project does not know who you are, pick your member"),
            "{err}"
        );
    }

    /// D3.4: a commit joy writes carries joy's own paths and nothing
    /// else, and a person's half staged work neither rides along nor
    /// hides joy's change behind a whole tree comparison.
    #[test]
    fn a_scoped_commit_leaves_the_persons_staged_work_alone() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join(".joy")).unwrap();
        std::fs::write(dir.path().join(".joy/project.yaml"), "name: first").unwrap();
        std::fs::write(dir.path().join("src.rs"), "half finished").unwrap();
        stage_paths(dir.path(), &[".joy", "src.rs"]).unwrap();

        let scope = vec![".joy".to_string()];
        let oid = commit_index_paths(dir.path(), &scope, "joy: first", "m", "m@e.c")
            .unwrap()
            .expect("joy's own path changed");
        let tree = repo
            .find_commit(git2::Oid::from_str(&oid).unwrap())
            .unwrap()
            .tree()
            .unwrap();
        assert!(tree.get_path(Path::new(".joy/project.yaml")).is_ok());
        assert!(
            tree.get_path(Path::new("src.rs")).is_err(),
            "the person's staged file is not joy's to commit"
        );

        // Nothing of joy's changed since: the staged `src.rs` must not
        // make joy believe there is something to commit.
        assert!(
            commit_index_paths(dir.path(), &scope, "joy: again", "m", "m@e.c")
                .unwrap()
                .is_none()
        );

        // A deletion under joy's own paths is a change like any other.
        std::fs::remove_file(dir.path().join(".joy/project.yaml")).unwrap();
        stage_paths(dir.path(), &[".joy"]).unwrap();
        let oid = commit_index_paths(dir.path(), &scope, "joy: gone", "m", "m@e.c")
            .unwrap()
            .expect("the deletion is a change");
        let tree = repo
            .find_commit(git2::Oid::from_str(&oid).unwrap())
            .unwrap()
            .tree()
            .unwrap();
        assert!(tree.get_path(Path::new(".joy/project.yaml")).is_err());
        assert!(tree.get_path(Path::new("src.rs")).is_err());
    }

    /// D4.5: no member, no commit, and the sentence says what to do.
    #[test]
    fn a_signature_without_a_member_is_the_typed_error() {
        let err = member_signature(None, "   ", Some("Scotty")).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("this project does not know who you are, pick your member"),
            "{err}"
        );
        // the command line has no picker, so the sentence names its remedy
        assert!(err.to_string().contains("--user <address>"), "{err}");
        assert!(matches!(err, crate::error::JoyError::UnknownActingMember));
    }

    /// A forge that accepts the connection and then says nothing, the way
    /// Codeberg's proxy did on 2026-09-03 before its own 504 at thirty
    /// seconds: the fetch gives up on our socket bound (JP-0115-EC), not
    /// on the forge's. The listener answers nothing for longer than the
    /// bound, so the fetch's return is the bound firing.
    #[test]
    fn a_silent_forge_is_given_up_on_within_the_bound() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            // accept and hold every connection without a byte in reply
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept() {
                held.push(stream);
                if held.len() > 8 {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                }
            }
        });
        let tmp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(tmp.path()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        repo.remote("origin", &format!("http://127.0.0.1:{port}/silent.git"))
            .unwrap();

        let started = std::time::Instant::now();
        let result = fetch_branch(tmp.path(), &Auth::token("x"));
        let took = started.elapsed();
        let error = result.expect_err("a silent forge cannot deliver a branch");
        // and the state is "nobody answered", not a certificate fault:
        // the bound surfaces as libgit2's raw EAGAIN under class Ssl
        // (JOY-0278-85), and the classifier reads it for what it is
        let sentence = error.to_string();
        assert_eq!(
            crate::vcs::contact::failure_of(&error),
            crate::vcs::contact::Failure::Offline,
            "the bound fired: {sentence}"
        );
        assert!(
            sentence.contains("No connection to 127.0.0.1"),
            "the sentence names the host: {sentence}"
        );
        // libgit2's own words stay in the detail line (D1.8b)
        let detail = crate::vcs::contact::detail_of(&error).unwrap_or_default();
        assert!(
            !sentence.contains("libgit2") && detail.starts_with("libgit2:"),
            "sentence {sentence:?}, detail {detail:?}"
        );
        assert!(
            took >= std::time::Duration::from_secs(10) && took < std::time::Duration::from_secs(25),
            "gave up after {took:?}: expected the 15s socket bound, not the forge's timing"
        );
        assert_eq!(
            unsafe { git2::opts::get_server_connect_timeout_in_milliseconds() }.unwrap(),
            10_000
        );
    }

    /// Local end-to-end against a bare "forge" repo: clone, commit, push,
    /// pull. No network, real git2 semantics.
    #[test]
    fn clone_commit_push_pull_roundtrip() {
        let base = std::env::temp_dir().join(format!("jp-git-test-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();

        // Seed the forge with an initial commit holding a .joy marker.
        let seed = base.join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/marker"), "hello").unwrap();
        let mut index = seed_repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = seed_repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        seed_repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        let head = seed_repo.head().unwrap();
        let branch = head.shorthand().unwrap().to_string();
        seed_repo
            .find_remote("origin")
            .unwrap()
            .push(
                &[format!("refs/heads/{branch}:refs/heads/{branch}").as_str()],
                None,
            )
            .unwrap();

        // Clone like the server does (file URLs ignore the token callback).
        let checkout = base.join("checkout");
        clone(
            forge.to_str().unwrap(),
            &Auth::token("irrelevant"),
            &checkout,
        )
        .expect("clone");
        assert!(checkout.join(".joy/marker").exists());

        // Write, commit, push, and see it arrive via a second pull.
        std::fs::write(checkout.join(".joy/item.yaml"), "id: X-1").unwrap();
        let committed =
            commit_joy(&checkout, "test: item", "Tester", "tester@example.com").expect("commit");
        assert!(committed.is_some());
        push(&checkout, &Auth::token("irrelevant")).expect("push");
        assert!(commit_joy(&checkout, "again", "T", "t@e.c")
            .unwrap()
            .is_none());

        let second = base.join("second");
        clone(forge.to_str().unwrap(), &Auth::token("irrelevant"), &second).expect("clone 2");
        assert!(second.join(".joy/item.yaml").exists());
        pull_ff(&checkout, &Auth::token("irrelevant")).expect("pull up-to-date");

        std::fs::remove_dir_all(&base).ok();
    }

    /// A job worktree: branch off the checkout, change code (not just .joy),
    /// commit all, push the branch to the bare forge, verify it landed —
    /// and verify `.joy` changes never reach the branch commit
    /// (JP-006D-28: item state rides main, not job branches).
    #[test]
    fn worktree_branch_commit_push_roundtrip() {
        let base = std::env::temp_dir().join(format!("jp-wt-test-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();
        let seed = base.join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::write(seed.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/marker.yaml"), "state: original\n").unwrap();
        let mut index = seed_repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = seed_repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        seed_repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        let branch0 = seed_repo.head().unwrap().shorthand().unwrap().to_string();
        seed_repo
            .find_remote("origin")
            .unwrap()
            .push(
                &[format!("refs/heads/{branch0}:refs/heads/{branch0}").as_str()],
                None,
            )
            .unwrap();

        let checkout = base.join("checkout");
        clone(forge.to_str().unwrap(), &Auth::token("x"), &checkout).expect("clone");

        // job worktree on a fresh branch
        let wt = base.join("wt");
        let job_branch = "joy/claude/X-1-abc";
        create_worktree(&checkout, "job-abc", job_branch, &wt).expect("worktree");
        assert!(wt.join("main.rs").exists());

        // the agent changes code AND (illegitimately) item state; the
        // fallback commit stages the code only
        std::fs::write(wt.join("main.rs"), "fn main() { println!(\"hi\"); }\n").unwrap();
        std::fs::write(wt.join("NEW.txt"), "added").unwrap();
        std::fs::write(wt.join(".joy/marker.yaml"), "state: tampered\n").unwrap();
        std::fs::write(wt.join(".joy/new-item.yaml"), "id: nope\n").unwrap();
        let committed = commit_all(&wt, "feat: work", "claude", "ai:claude@joy").expect("commit");
        assert!(committed.is_some());
        push_branch(&wt, &Auth::token("x")).expect("push");

        // the branch is on the forge with the code change, .joy untouched
        let forge_repo = git2::Repository::open_bare(&forge).unwrap();
        let branch_ref = forge_repo
            .find_reference(&format!("refs/heads/{job_branch}"))
            .expect("job branch on forge");
        let commit = branch_ref.peel_to_commit().unwrap();
        assert!(commit.message().unwrap().contains("feat: work"));
        let tree = commit.tree().unwrap();
        assert!(tree.get_name("NEW.txt").is_some());
        let joy = tree
            .get_name(".joy")
            .unwrap()
            .to_object(&forge_repo)
            .unwrap()
            .peel_to_tree()
            .unwrap();
        assert!(joy.get_name("new-item.yaml").is_none(), "no new .joy file");
        let marker = joy
            .get_name("marker.yaml")
            .unwrap()
            .to_object(&forge_repo)
            .unwrap()
            .peel_to_blob()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(marker.content()),
            "state: original\n",
            ".joy edit did not ride the fallback commit"
        );

        // a worktree with ONLY .joy noise yields no commit at all
        assert!(commit_all(&wt, "noise", "c", "c@e").unwrap().is_none());

        prune_worktree(&checkout, "job-abc", &wt);
        std::fs::remove_dir_all(&base).ok();
    }

    /// AcceptJob's freeze (JP-006D-28): a job branch whose diff touches
    /// `.joy/` is refused; a clean branch merges.
    #[test]
    fn merge_job_branch_refuses_item_state_on_the_branch() {
        let base = std::env::temp_dir().join(format!("jp-freeze-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let forge = base.join("forge.git");
        std::fs::create_dir_all(&forge).unwrap();
        git2::Repository::init_bare(&forge).unwrap();
        let seed = base.join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::write(seed.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/item.yaml"), "id: X-1\n").unwrap();
        let mut index = seed_repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = seed_repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        seed_repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        let b = seed_repo.head().unwrap().shorthand().unwrap().to_string();
        seed_repo
            .find_remote("origin")
            .unwrap()
            .push(&[format!("refs/heads/{b}:refs/heads/{b}").as_str()], None)
            .unwrap();
        let checkout = base.join("checkout");
        clone(forge.to_str().unwrap(), &Auth::token("x"), &checkout).unwrap();

        // a dirty branch: code plus a direct .joy commit
        let wt = base.join("wt-dirty");
        create_worktree(&checkout, "wt-dirty", "joy/claude/dirty", &wt).unwrap();
        std::fs::write(wt.join("ok.txt"), "fine\n").unwrap();
        std::fs::write(wt.join(".joy/item.yaml"), "id: X-1\nstatus: closed\n").unwrap();
        {
            let repo = open(&wt).unwrap();
            let mut idx = repo.index().unwrap();
            idx.add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            idx.write().unwrap();
            let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("agent", "a@e.c").unwrap();
            let parent = repo.head().unwrap().peel_to_commit().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "sneaky", &tree, &[&parent])
                .unwrap();
        }
        assert_eq!(
            branch_touches_joy(&checkout, "joy/claude/dirty").unwrap(),
            Some(".joy/item.yaml".to_string())
        );
        let err = merge_job_branch(&checkout, "joy/claude/dirty", "merge", "h", "h@e.c")
            .expect_err("dirty branch refused");
        assert!(
            err.to_string().contains("must not carry item state"),
            "clear refusal: {err}"
        );

        // a clean branch passes the freeze and merges
        let wt2 = base.join("wt-clean");
        create_worktree(&checkout, "wt-clean", "joy/claude/clean", &wt2).unwrap();
        std::fs::write(wt2.join("ok.txt"), "fine\n").unwrap();
        commit_all(&wt2, "feat: clean", "c", "c@e.c").unwrap();
        assert_eq!(
            branch_touches_joy(&checkout, "joy/claude/clean").unwrap(),
            None
        );
        merge_job_branch(&checkout, "joy/claude/clean", "merge", "h", "h@e.c")
            .expect("clean branch merges");
        assert!(checkout.join("ok.txt").is_file());

        std::fs::remove_dir_all(&base).ok();
    }
}

#[cfg(test)]
mod engine_invariant_tests {
    use super::*;

    struct Rig {
        _tmp: tempfile::TempDir,
        forge: std::path::PathBuf,
        clone_dir: std::path::PathBuf,
        seed: std::path::PathBuf,
    }

    /// A bare "forge" with one commit on main, plus a clone the way the
    /// product clones. No network, real git2 semantics.
    fn rig() -> Rig {
        let tmp = tempfile::tempdir().expect("tempdir");
        let forge = tmp.path().join("forge.git");
        git2::Repository::init_bare(&forge).unwrap();
        let seed = tmp.path().join("seed");
        let seed_repo = git2::Repository::init(&seed).unwrap();
        std::fs::create_dir_all(seed.join(".joy")).unwrap();
        std::fs::write(seed.join(".joy/item.yaml"), "id: X-1\ntitle: one\n").unwrap();
        commit_everything(&seed_repo, "seed");
        seed_repo.remote("origin", forge.to_str().unwrap()).unwrap();
        push_current_branch(&seed_repo);
        let clone_dir = tmp.path().join("clone");
        clone(forge.to_str().unwrap(), &Auth::token(""), &clone_dir).expect("clone");
        Rig {
            _tmp: tmp,
            forge,
            clone_dir,
            seed,
        }
    }

    fn commit_everything(repo: &git2::Repository, message: &str) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Seed", "seed@example.com").unwrap();
        let parent = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .and_then(|o| repo.find_commit(o).ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap();
    }

    fn push_current_branch(repo: &git2::Repository) {
        let branch = repo.head().unwrap().shorthand().unwrap().to_string();
        repo.find_remote("origin")
            .unwrap()
            .push(
                &[format!("refs/heads/{branch}:refs/heads/{branch}").as_str()],
                None,
            )
            .unwrap();
    }

    /// A branch born on the forge after the clone (JP-0126-72): fetch_heads
    /// learns it, ensure_local_branch stands it up at the forge's tip with
    /// its upstream, and neither writes FETCH_HEAD.
    #[test]
    fn a_forge_branch_the_clone_never_saw_becomes_a_local_branch_at_its_tip() {
        let rig = rig();
        let auth = Auth::token("");
        let seed_repo = git2::Repository::open(&rig.seed).unwrap();
        let base = seed_repo.head().unwrap().peel_to_commit().unwrap();
        seed_repo.branch("feat", &base, false).unwrap();
        seed_repo.set_head("refs/heads/feat").unwrap();
        std::fs::write(rig.seed.join("feature.txt"), "on feat\n").unwrap();
        commit_everything(&seed_repo, "feat work");
        push_current_branch(&seed_repo);
        let feat_tip = seed_repo.head().unwrap().target().unwrap();

        assert!(
            ensure_local_branch(&rig.clone_dir, "feat").is_err(),
            "before the fetch the clone cannot know feat"
        );
        let fetch_head = rig.clone_dir.join(".git/FETCH_HEAD");
        std::fs::remove_file(&fetch_head).ok();
        let heads = fetch_heads(&rig.clone_dir, &auth).unwrap();
        assert!(heads.contains(&"feat".to_string()), "{heads:?}");
        assert!(!fetch_head.exists(), "fetch_heads wrote FETCH_HEAD");
        assert!(branch_names(&rig.clone_dir).contains(&"feat".to_string()));

        ensure_local_branch(&rig.clone_dir, "feat").unwrap();
        let clone = git2::Repository::open(&rig.clone_dir).unwrap();
        let local = clone.find_branch("feat", git2::BranchType::Local).unwrap();
        assert_eq!(
            local.get().target().unwrap(),
            feat_tip,
            "the forge's tip, not HEAD"
        );
        assert!(local.upstream().is_ok(), "the upstream is set");
        assert_ne!(
            clone.head().unwrap().shorthand().unwrap(),
            "feat",
            "HEAD stays where it was"
        );
        // idempotent
        ensure_local_branch(&rig.clone_dir, "feat").unwrap();
    }

    /// The list a member chooses from is the forge's (JP-0126-72): a
    /// local-only branch stays off it, a forge branch is on it.
    #[test]
    fn the_choosable_branches_are_the_forges_not_the_clones_private_ones() {
        let rig = rig();
        let clone = git2::Repository::open(&rig.clone_dir).unwrap();
        let head = clone.head().unwrap().peel_to_commit().unwrap();
        clone.branch("joy/jobwork/X-1", &head, false).unwrap();
        let names = remote_branch_names(&rig.clone_dir);
        assert!(!names.iter().any(|n| n == "joy/jobwork/X-1"), "{names:?}");
        assert!(
            names.iter().any(|n| n == "main") || !names.is_empty(),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n == "HEAD"));
    }

    /// A branch a worktree holds (a job's) is listed with its worktree,
    /// and a member still gets a read-only view of it beside the job's
    /// tree that follows the branch as it moves (JP-0126-72).
    #[test]
    fn a_branch_held_by_a_worktree_is_readable_through_a_view_that_follows_it() {
        let rig = rig();
        let clone = git2::Repository::open(&rig.clone_dir).unwrap();
        let job_tree = rig.clone_dir.parent().unwrap().join("job-tree");
        create_worktree(&rig.clone_dir, "job-X-1", "joy/vibe/X-1", &job_tree).unwrap();
        let held = worktree_branches(&rig.clone_dir);
        assert!(
            held.contains(&("job-X-1".to_string(), "joy/vibe/X-1".to_string())),
            "{held:?}"
        );
        // a second worktree on the same branch is what git refuses
        let second = rig.clone_dir.parent().unwrap().join("second");
        assert!(create_worktree(&rig.clone_dir, "second", "joy/vibe/X-1", &second).is_err());

        let view = rig.clone_dir.parent().unwrap().join("view");
        ensure_view_worktree(&rig.clone_dir, "view-X-1", "joy/vibe/X-1", &view).unwrap();
        let tip = clone.refname_to_id("refs/heads/joy/vibe/X-1").unwrap();
        let view_repo = git2::Repository::open(&view).unwrap();
        assert_eq!(view_repo.head().unwrap().target(), Some(tip));
        assert!(
            view_repo.head_detached().unwrap(),
            "a view has no branch of its own"
        );
        let locals: Vec<String> = branch_names(&rig.clone_dir);
        assert!(
            !locals.iter().any(|b| b.starts_with("joy/view")),
            "no mirror left behind: {locals:?}"
        );
        assert!(
            !worktree_branches(&rig.clone_dir)
                .iter()
                .any(|(w, _)| w == "view-X-1"),
            "a detached view holds no branch"
        );

        // the job commits on its branch; the view follows on the next call
        let job_repo = git2::Repository::open(&job_tree).unwrap();
        std::fs::write(job_tree.join("work.txt"), "done\n").unwrap();
        commit_everything(&job_repo, "job work");
        let moved = clone.refname_to_id("refs/heads/joy/vibe/X-1").unwrap();
        assert_ne!(moved, tip);
        ensure_view_worktree(&rig.clone_dir, "view-X-1", "joy/vibe/X-1", &view).unwrap();
        let view_repo = git2::Repository::open(&view).unwrap();
        assert_eq!(view_repo.head().unwrap().target(), Some(moved));
        assert!(view_repo.head_detached().unwrap());
        assert!(view.join("work.txt").exists(), "the working tree followed");
        assert!(!rig.clone_dir.join(".git/FETCH_HEAD").exists());
    }

    /// A job forks from the member's branch (platform JP-0128-0F): its
    /// worktree is added FROM the branch's linked worktree, and the clone
    /// still registers and lists it like every other.
    #[test]
    fn a_worktree_added_from_a_linked_worktree_belongs_to_the_clone() {
        let rig = rig();
        let base = rig.clone_dir.parent().unwrap().join("feat");
        ensure_local_branch(&rig.clone_dir, "main").ok();
        let clone = git2::Repository::open(&rig.clone_dir).unwrap();
        let head = clone.head().unwrap().peel_to_commit().unwrap();
        clone.branch("feat", &head, false).unwrap();
        create_worktree(&rig.clone_dir, "branch-feat", "feat", &base).unwrap();
        std::fs::write(base.join("on-feat.txt"), "feat\n").unwrap();
        commit_everything(&git2::Repository::open(&base).unwrap(), "on feat");

        let job = rig.clone_dir.parent().unwrap().join("job");
        create_worktree(&base, "job-J-1", "joy/vibe/J-1", &job).unwrap();
        assert!(
            job.join("on-feat.txt").exists(),
            "forked from the branch's tip"
        );
        let held = worktree_branches(&rig.clone_dir);
        assert!(
            held.contains(&("job-J-1".to_string(), "joy/vibe/J-1".to_string())),
            "the clone lists it: {held:?}"
        );
        assert!(
            rig.clone_dir.join(".git/worktrees/job-J-1").is_dir(),
            "registered in the clone's common dir"
        );
        prune_worktree(&rig.clone_dir, "job-J-1", &job);
        assert!(!worktree_branches(&rig.clone_dir)
            .iter()
            .any(|(w, _)| w == "job-J-1"));
    }

    /// The clone remembers the forge's default branch (origin/HEAD).
    #[test]
    fn the_clone_knows_the_forges_default_branch() {
        let rig = rig();
        let seed_repo = git2::Repository::open(&rig.seed).unwrap();
        let default = seed_repo.head().unwrap().shorthand().unwrap().to_string();
        assert_eq!(
            remote_head_branch(&rig.clone_dir).as_deref(),
            Some(default.as_str())
        );
    }

    /// THE invariant this engine exists for: no verb ever writes
    /// FETCH_HEAD. It is the one file git updates without a lock, and
    /// sharing it tore syncs apart twice (JP-00DB-61, JAPP-0198-EA).
    #[test]
    fn no_verb_ever_writes_fetch_head() {
        let rig = rig();
        let auth = Auth::token("");
        let fetch_head = rig.clone_dir.join(".git/FETCH_HEAD");
        std::fs::remove_file(&fetch_head).ok();
        fetch_branch(&rig.clone_dir, &auth).unwrap();
        if fetch_head.exists() {
            panic!(
                "fetch_branch wrote FETCH_HEAD: {:?}",
                std::fs::read_to_string(&fetch_head)
            );
        }
        ff_from_tracking(&rig.clone_dir).unwrap();
        assert!(!fetch_head.exists(), "ff wrote FETCH_HEAD");
        // an absent side ref (chats of a project that never pushed them)
        assert!(!fetch_ref(
            &rig.clone_dir,
            &auth,
            "refs/joy/chats",
            "refs/joy/chats-tracking"
        )
        .unwrap());
        assert!(!fetch_head.exists(), "fetch_ref wrote FETCH_HEAD");
        pull_merge(&rig.clone_dir, &auth, "T", "t@example.com").unwrap();
        assert!(!fetch_head.exists(), "pull_merge wrote FETCH_HEAD");
        push(&rig.clone_dir, &auth).unwrap();
        assert!(!fetch_head.exists(), "push wrote FETCH_HEAD");
        let branch = open(&rig.clone_dir)
            .unwrap()
            .head()
            .unwrap()
            .shorthand()
            .unwrap()
            .to_string();
        refresh_branch_from_forge(&rig.clone_dir, &branch, &auth);
        assert!(
            !fetch_head.exists(),
            "refresh_branch_from_forge wrote FETCH_HEAD"
        );
    }

    /// ONE advertisement answers for several refs (D1.9): the working
    /// branch and a side ref come back from the same contact, and a ref
    /// the forge does not have is simply absent. This is what turns a
    /// poll tick that watches two refs into one contact.
    #[test]
    fn one_advertisement_answers_for_several_refs() {
        let rig = rig();
        let auth = Auth::token("");
        // a side ref on the forge, the way the chat store pushes one
        let forge_repo = git2::Repository::open_bare(&rig.forge).unwrap();
        let tip = forge_repo.head().unwrap().target().unwrap();
        forge_repo
            .reference("refs/joy/chats", tip, true, "chats")
            .unwrap();
        let branch = open(&rig.clone_dir)
            .unwrap()
            .head()
            .unwrap()
            .shorthand()
            .unwrap()
            .to_string();
        let head_ref = format!("refs/heads/{branch}");

        let found = ls_remote_refs(
            &rig.clone_dir,
            &auth,
            &[head_ref.as_str(), "refs/joy/chats", "refs/joy/absent"],
        )
        .unwrap();
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[&head_ref], tip.to_string());
        assert_eq!(found["refs/joy/chats"], tip.to_string());
        assert!(!found.contains_key("refs/joy/absent"));

        // the single ref verb is that same advertisement, one name wide
        assert_eq!(
            ls_remote_ref(&rig.clone_dir, &auth, "refs/joy/chats").unwrap(),
            Some(tip.to_string())
        );
        assert_eq!(
            ls_remote_ref(&rig.clone_dir, &auth, "refs/joy/absent").unwrap(),
            None
        );
    }

    /// The 30-second re-clone loop of 2026-08-25: a forge WITHOUT
    /// refs/joy/chats must not poison anything — every round lands, and
    /// a stale tracking ref from earlier days is cleaned up.
    #[test]
    fn a_forge_without_a_chats_ref_is_harmless() {
        let rig = rig();
        let auth = Auth::token("");
        let repo = git2::Repository::open(&rig.clone_dir).unwrap();
        // a stale tracking ref from an earlier sync
        let head = repo.head().unwrap().target().unwrap();
        repo.reference("refs/joy/chats-tracking", head, true, "stale")
            .unwrap();
        for _ in 0..2 {
            fetch_branch(&rig.clone_dir, &auth).unwrap();
            ff_from_tracking(&rig.clone_dir).unwrap();
            assert!(!fetch_ref(
                &rig.clone_dir,
                &auth,
                "refs/joy/chats",
                "refs/joy/chats-tracking"
            )
            .unwrap());
        }
        assert!(
            repo.find_reference("refs/joy/chats-tracking").is_err(),
            "the stale tracking ref must be gone"
        );
    }

    /// A branch renamed or deleted on the forge is said out loud
    /// (JP-00DB-61 follow-up de0d186), not reported as corruption.
    #[test]
    fn a_vanished_branch_is_named() {
        let rig = rig();
        let forge_repo = git2::Repository::open_bare(&rig.forge).unwrap();
        forge_repo
            .find_branch("main", git2::BranchType::Local)
            .or_else(|_| forge_repo.find_branch("master", git2::BranchType::Local))
            .unwrap()
            .rename("trunk", true)
            .unwrap();
        forge_repo.set_head("refs/heads/trunk").unwrap();
        let err = fetch_branch(&rig.clone_dir, &Auth::token("")).expect_err("branch is gone");
        let msg = err.to_string();
        assert!(msg.contains("not found on the forge"), "{msg}");
        assert!(!msg.contains("corrupted"), "{msg}");
    }

    /// Divergence is the NORMAL case under write-behind: the remote moves,
    /// the local side has its own `.joy` commit, and pull_merge produces a
    /// YAML-merged commit carrying the ACTING MEMBER as author
    /// (JP-00DE-11), never the server.
    #[test]
    fn diverged_histories_merge_yaml_aware_under_the_members_name() {
        let rig = rig();
        let auth = Auth::token("");
        // remote side: title changes
        std::fs::write(rig.seed.join(".joy/item.yaml"), "id: X-1\ntitle: forge\n").unwrap();
        let seed_repo = git2::Repository::open(&rig.seed).unwrap();
        commit_everything(&seed_repo, "remote change");
        push_current_branch(&seed_repo);
        // local side: a new field
        std::fs::write(
            rig.clone_dir.join(".joy/item.yaml"),
            "id: X-1\ntitle: one\npriority: high\n",
        )
        .unwrap();
        let committed = commit_joy(
            &rig.clone_dir,
            "local change",
            "Member",
            "member@example.com",
        )
        .unwrap();
        assert!(committed.is_some());
        pull_merge(&rig.clone_dir, &auth, "Member", "member@example.com").unwrap();
        let merged = std::fs::read_to_string(rig.clone_dir.join(".joy/item.yaml")).unwrap();
        assert!(merged.contains("title: forge"), "{merged}");
        assert!(merged.contains("priority: high"), "{merged}");
        let repo = git2::Repository::open(&rig.clone_dir).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.parents().len(), 2, "a real merge commit");
        assert_eq!(head.author().email().ok(), Some("member@example.com"));
        // and the result pushes: nothing left ahead or behind afterwards
        push(&rig.clone_dir, &auth).unwrap();
        let (ahead, behind) = ahead_behind(&rig.clone_dir).unwrap();
        assert_eq!((ahead, behind), (0, 0));
    }

    /// A hand-wired repo may call its remote anything: the tracking ref
    /// follows the RESOLVED remote's name, never a hardwired "origin".
    #[test]
    fn a_remote_by_any_other_name_works() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let forge = tmp.path().join("forge.git");
        git2::Repository::init_bare(&forge).unwrap();
        let work = tmp.path().join("work");
        init_repo(&work, "main").unwrap();
        std::fs::write(work.join("a.txt"), "a").unwrap();
        super::commit_everything(&work, "seed", "t", "t@example.com").unwrap();
        add_remote(&work, "upstream", forge.to_str().unwrap()).unwrap();
        let auth = Auth::token("");
        push(&work, &auth).unwrap();
        fetch_branch(&work, &auth).unwrap();
        ff_from_tracking(&work).unwrap();
        let repo = open(&work).unwrap();
        assert!(
            repo.find_reference("refs/remotes/upstream/main").is_ok(),
            "the tracking ref lives under the remote's real name"
        );
        assert!(repo.find_reference("refs/remotes/origin/main").is_err());
        assert_eq!(ahead_behind(&work).unwrap(), (0, 0));
    }

    /// The plain catch-up: remote moved, local clean, the fetch + ff pair
    /// brings the clone current without inventing commits.
    #[test]
    fn a_clean_clone_fast_forwards() {
        let rig = rig();
        let auth = Auth::token("");
        std::fs::write(rig.seed.join("code.txt"), "v2").unwrap();
        let seed_repo = git2::Repository::open(&rig.seed).unwrap();
        commit_everything(&seed_repo, "remote moves");
        push_current_branch(&seed_repo);
        fetch_branch(&rig.clone_dir, &auth).unwrap();
        ff_from_tracking(&rig.clone_dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(rig.clone_dir.join("code.txt")).unwrap(),
            "v2"
        );
        let (ahead, behind) = ahead_behind(&rig.clone_dir).unwrap();
        assert_eq!((ahead, behind), (0, 0));
    }
}

#[cfg(test)]
mod pull_merge_tests {
    use super::*;

    fn commit_all(repo: &git2::Repository, message: &str) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@example.com").unwrap();
        let parents: Vec<git2::Commit> = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .and_then(|o| repo.find_commit(o).ok())
            .into_iter()
            .collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .unwrap();
    }

    #[test]
    fn diverged_chat_writes_merge_and_push() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("forge.git");
        git2::Repository::init_bare(&bare).unwrap();
        let url = bare.to_str().unwrap().to_string();

        let a_dir = tmp.path().join("a");
        let a = git2::Repository::clone(&url, &a_dir).unwrap();
        std::fs::create_dir_all(a_dir.join(".joy/chats")).unwrap();
        std::fs::write(
            a_dir.join(".joy/chats/c.yaml"),
            "id: c\ntitle: T\ncreated: 2026-07-05T10:00:00Z\nupdated: 2026-07-05T10:00:00Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T10:00:01Z\n  author: a@x\n  text: hello\n",
        )
        .unwrap();
        commit_all(&a, "seed");
        push(&a_dir, &Auth::token("")).unwrap();

        let b_dir = tmp.path().join("b");
        let _b = git2::Repository::clone(&url, &b_dir).unwrap();

        // A appends m2 and pushes; B appends m3 without knowing about m2
        std::fs::write(
            a_dir.join(".joy/chats/c.yaml"),
            "id: c\ntitle: T\ncreated: 2026-07-05T10:00:00Z\nupdated: 2026-07-05T10:00:02Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T10:00:01Z\n  author: a@x\n  text: hello\n- id: m2\n  at: 2026-07-05T10:00:02Z\n  author: a@x\n  text: from A\n",
        )
        .unwrap();
        commit_all(&a, "a: m2");
        push(&a_dir, &Auth::token("")).unwrap();

        let b = open(&b_dir).unwrap();
        std::fs::write(
            b_dir.join(".joy/chats/c.yaml"),
            "id: c\ntitle: T\ncreated: 2026-07-05T10:00:00Z\nupdated: 2026-07-05T10:00:03Z\nparticipants:\n- a@x\nmessages:\n- id: m1\n  at: 2026-07-05T10:00:01Z\n  author: a@x\n  text: hello\n- id: m3\n  at: 2026-07-05T10:00:03Z\n  author: b@x\n  text: from B\n",
        )
        .unwrap();
        commit_all(&b, "b: m3");

        // push rejects (non-ff), merge unites, push succeeds
        assert!(push(&b_dir, &Auth::token("")).is_err());
        pull_merge(&b_dir, &Auth::token(""), "t", "t@example.com").unwrap();
        push(&b_dir, &Auth::token("")).unwrap();

        let merged = std::fs::read_to_string(b_dir.join(".joy/chats/c.yaml")).unwrap();
        assert!(merged.contains("from A"), "missing A's message: {merged}");
        assert!(merged.contains("from B"), "missing B's message: {merged}");
        let (ahead, behind) = ahead_behind(&b_dir).unwrap();
        assert_eq!((ahead, behind), (0, 0));
    }
}

#[cfg(test)]
mod credential_shape_tests {
    use super::*;

    /// The user name and the password inside a plaintext credential.
    ///
    /// git2 0.21 exposes no accessor for either (`Cred` holds one raw
    /// pointer and hands out only `credtype`), so the test reads
    /// libgit2's own public struct, `git_credential_userpass_plaintext`
    /// from include/git2/credential.h: the credential header, then the
    /// two strings. This is the only way to see what joy would send
    /// over the wire. The credential is leaked, which is what a test
    /// can afford.
    fn userpass(cred: git2::Cred) -> (String, String) {
        #[repr(C)]
        struct RawUserpass {
            credtype: u32,
            free: Option<extern "C" fn(*mut std::ffi::c_void)>,
            username: *const std::ffi::c_char,
            password: *const std::ffi::c_char,
        }
        assert_eq!(
            cred.credtype(),
            git2::Cred::userpass_plaintext("u", "p").unwrap().credtype(),
            "not a user-and-password credential"
        );
        unsafe {
            let raw = cred.unwrap().cast::<RawUserpass>();
            let user = std::ffi::CStr::from_ptr((*raw).username)
                .to_string_lossy()
                .into_owned();
            let password = std::ffi::CStr::from_ptr((*raw).password)
                .to_string_lossy()
                .into_owned();
            (user, password)
        }
    }

    fn is_userpass(cred: &git2::Cred) -> bool {
        cred.credtype() == git2::Cred::userpass_plaintext("u", "p").unwrap().credtype()
    }

    /// The claim decides the shape, and the FIRST attempt carries it.
    /// This is the acceptance sentence of J4a for a GitHub Enterprise
    /// Server host whose name says nothing about GitHub: without the
    /// claim there is no first attempt that can be right, because the
    /// engine's table knows three hosts and guesses at none.
    #[test]
    fn a_claimed_ghes_host_is_sent_x_access_token_on_the_first_attempt() {
        let url = "https://source.acme-internal.example/o/r.git";
        assert_eq!(known_forge_kind("source.acme-internal.example"), None);
        let mut credentials = Auth::token_for("ghp_token", ForgeKind::GitHubEnterprise)
            .credential_source(CredSource::none());
        let first = credentials(url, None, git2::CredentialType::USER_PASS_PLAINTEXT)
            .expect("a credential on the first attempt");
        assert_eq!(
            userpass(first),
            ("x-access-token".to_string(), "ghp_token".to_string())
        );
        // A claimed host never gets the other name on the second
        // attempt: the claim is not a guess to be corrected.
        let mut gitlab =
            Auth::token_for("glpat", ForgeKind::GitLab).credential_source(CredSource::none());
        let first = gitlab(url, None, git2::CredentialType::USER_PASS_PLAINTEXT).unwrap();
        assert_eq!(userpass(first), ("oauth2".to_string(), "glpat".to_string()));
        let second = gitlab(url, None, git2::CredentialType::USER_PASS_PLAINTEXT);
        assert!(
            second.is_err(),
            "a claimed host has one shape, not two, and then the chain"
        );
    }

    #[test]
    fn an_unclaimed_host_tries_the_second_shape_and_a_known_one_does_not() {
        let unknown = "https://forge.acme-internal.example/o/r.git";
        let mut credentials = Auth::token("tok").credential_source(CredSource::none());
        let first = credentials(unknown, None, git2::CredentialType::USER_PASS_PLAINTEXT).unwrap();
        assert_eq!(userpass(first), ("oauth2".to_string(), "tok".to_string()));
        let second = credentials(unknown, None, git2::CredentialType::USER_PASS_PLAINTEXT).unwrap();
        assert_eq!(
            userpass(second),
            ("x-access-token".to_string(), "tok".to_string()),
            "the empty-password shape is never one of the two (D1.6)"
        );
        // Then the chain, and its sentence names the token as well as
        // what came after it.
        let exhausted = match credentials(unknown, None, git2::CredentialType::USER_PASS_PLAINTEXT)
        {
            Ok(_) => panic!("nothing else to offer"),
            Err(e) => e,
        };
        assert!(
            exhausted.message().contains("refused the access token"),
            "{}",
            exhausted.message()
        );
        assert!(
            exhausted.message().contains("forge.acme-internal.example"),
            "{}",
            exhausted.message()
        );

        // github.com is in the table, so there is no second shape to
        // try: one attempt, then the chain.
        let known = "https://github.com/o/r.git";
        let mut credentials = Auth::token("tok").credential_source(CredSource::none());
        let first = credentials(known, None, git2::CredentialType::USER_PASS_PLAINTEXT).unwrap();
        assert_eq!(
            userpass(first),
            ("x-access-token".to_string(), "tok".to_string())
        );
        let exhausted = match credentials(known, None, git2::CredentialType::USER_PASS_PLAINTEXT) {
            Ok(_) => panic!("nothing else to offer"),
            Err(e) => e,
        };
        assert!(
            exhausted
                .message()
                .contains("refused the access token (sent as x-access-token)"),
            "{}",
            exhausted.message()
        );
    }

    /// Design D1.6: the callback honours the `allowed` mask. Without
    /// it, an insteadOf rewrite to ssh makes libgit2 answer
    /// "authentication callback returned unsupported credentials type"
    /// (ssh_libssh2.c:415-418) and the person is told the token is
    /// wrong.
    #[test]
    fn a_mask_without_user_and_password_never_gets_the_token() {
        let mut credentials =
            Auth::token_for("ghp_token", ForgeKind::GitHub).credential_source(CredSource::none());
        // An https URL with an ssh-only mask: the chain holds a helper
        // step, which this mask cannot take, so the answer is the
        // sentence and never the token.
        let refused = match credentials(
            "https://github.com/o/r.git",
            None,
            git2::CredentialType::SSH_KEY,
        ) {
            Ok(_) => panic!("a token is not an ssh key"),
            Err(e) => e,
        };
        assert!(refused.message().contains("github.com"), "{refused}");
        assert!(
            !refused.message().contains("refused the access token"),
            "the token was never offered, so it was never refused: {refused}"
        );

        // And the ssh URL an insteadOf rewrite produces: libgit2 asks
        // for the user name first, then for a key. Whatever this
        // machine's agent and key files hold, a user-and-password
        // credential is the one answer that must not come back.
        let ssh = "ssh://git@nothing.example.invalid/o/r.git";
        let user = credentials(ssh, None, git2::CredentialType::USERNAME)
            .expect("the one user name of this contact");
        assert_eq!(
            user.credtype(),
            git2::Cred::username("git").unwrap().credtype()
        );
        if let Ok(offered) = credentials(ssh, Some("git"), git2::CredentialType::SSH_KEY) {
            assert!(
                !is_userpass(&offered),
                "an ssh mask must never be answered with the token"
            );
        }
    }

    #[test]
    fn a_host_with_no_token_goes_straight_to_the_local_chain() {
        for auth in [
            Auth::Local,
            Auth::local(HostKind::Interactive),
            Auth::token(""),
        ] {
            let mut credentials = auth.credential_source(CredSource::none());
            let answer = credentials(
                "https://gitea.example.com/o/r.git",
                None,
                git2::CredentialType::USER_PASS_PLAINTEXT,
            );
            let error = answer.err().expect("no configuration, so no credential");
            assert!(
                error.message().contains("git configuration"),
                "{}",
                error.message()
            );
        }
    }

    /// Each forge wants its own name in front of the token. Sending
    /// GitHub's convention to Codeberg is what made a valid token look
    /// like an unsupported authentication method (JP-00D8-94).
    #[test]
    fn every_forge_gets_the_name_it_expects() {
        assert_eq!(token_user("github.com", None), "x-access-token");
        assert_eq!(token_user("gitlab.com", None), "oauth2");
        assert_eq!(token_user("gitlab.self-hosted.example", None), "oauth2");
        // Design D1.6: the Gitea family takes oauth2 as well, never
        // the token as the user name with an empty password.
        assert_eq!(token_user("codeberg.org", None), "oauth2");
        assert_eq!(token_user("gitea.int.joydev.com", None), "oauth2");
    }

    /// The table knows three hosts and guesses at no other, which is
    /// what D1.1 and D1.5 say it holds. A Gitea that answers at
    /// `github.internal.example` would otherwise be classified as
    /// GitHub Enterprise by its name alone, be sent `x-access-token`,
    /// and never be offered `oauth2` either, because the second shape
    /// is kept for hosts no table knows: a valid token would look
    /// refused.
    #[test]
    fn the_engine_table_holds_three_hosts_and_guesses_at_none() {
        assert_eq!(known_forge_kind("github.com"), Some(ForgeKind::GitHub));
        assert_eq!(known_forge_kind("ssh.github.com"), Some(ForgeKind::GitHub));
        assert_eq!(known_forge_kind("gitlab.com"), Some(ForgeKind::GitLab));
        assert_eq!(
            known_forge_kind("altssh.gitlab.com"),
            Some(ForgeKind::GitLab)
        );
        assert_eq!(known_forge_kind("codeberg.org"), Some(ForgeKind::Gitea));
        for guessable in [
            "github.internal.example",
            "gitlab.acme.example",
            "gitea.int.joydev.com",
            "forgejo.example.org",
            "source.acme-internal.example",
        ] {
            assert_eq!(known_forge_kind(guessable), None, "for {guessable}");
        }
        // A Gitea behind a name that says GitHub gets both shapes, one
        // per attempt, and the second one is the one it accepts.
        let url = "https://github.internal.example/o/r.git";
        let mut credentials = Auth::token("tok").credential_source(CredSource::none());
        let first = credentials(url, None, git2::CredentialType::USER_PASS_PLAINTEXT).unwrap();
        assert_eq!(userpass(first), ("oauth2".to_string(), "tok".to_string()));
        let second = credentials(url, None, git2::CredentialType::USER_PASS_PLAINTEXT).unwrap();
        assert_eq!(
            userpass(second),
            ("x-access-token".to_string(), "tok".to_string())
        );
    }

    #[test]
    fn a_claimed_host_decides_the_shape_whatever_its_name_says() {
        // A GitHub Enterprise Server host with nothing in its name:
        // only the plugin's claim gets the shape right on the first
        // attempt.
        let host = "source.acme-internal.example";
        assert_eq!(known_forge_kind(host), None);
        assert_eq!(
            token_user(host, Some(ForgeKind::GitHubEnterprise)),
            "x-access-token"
        );
        assert_eq!(token_user(host, Some(ForgeKind::Gitea)), "oauth2");
        // And the claim wins over a name that points elsewhere.
        assert_eq!(
            token_user("gitlab.acme.example", Some(ForgeKind::GitHubEnterprise)),
            "x-access-token"
        );
    }

    #[test]
    fn the_empty_password_shape_is_never_sent() {
        for host in [
            "github.com",
            "gitlab.com",
            "codeberg.org",
            "gitea.example.com",
            "anything.else.example",
            "",
        ] {
            assert!(!token_user(host, None).is_empty(), "for {host}");
            assert!(!other_token_user(host).is_empty(), "for {host}");
            assert_ne!(token_user(host, None), other_token_user(host));
        }
    }

    #[test]
    fn the_plugin_ids_map_onto_the_families() {
        assert_eq!(ForgeKind::from_plugin_id("github"), Some(ForgeKind::GitHub));
        assert_eq!(ForgeKind::from_plugin_id("GitLab"), Some(ForgeKind::GitLab));
        assert_eq!(ForgeKind::from_plugin_id("forgejo"), Some(ForgeKind::Gitea));
        assert_eq!(ForgeKind::from_plugin_id("sourcehut"), None);
    }

    #[test]
    fn a_host_that_never_named_its_kind_is_the_quiet_one() {
        assert_eq!(Auth::Local.host_kind(), HostKind::Background);
        assert!(!Auth::Local.host_kind().may_prompt());
        assert_eq!(
            Auth::local(HostKind::Interactive).host_kind(),
            HostKind::Interactive
        );
        assert!(Auth::local(HostKind::Interactive).host_kind().may_prompt());
        // A token host is the platform or a worker: never asked.
        assert_eq!(Auth::token("t").host_kind(), HostKind::Background);
        assert_eq!(
            Auth::token_for("t", ForgeKind::GitLab).claimed_kind(),
            Some(ForgeKind::GitLab)
        );
        assert_eq!(Auth::token("t").claimed_kind(), None);
    }

    /// The normal case stays the normal case: a remote the ssh config
    /// does not rename is handed to libgit2 exactly as configured, with
    /// its name and its refspecs. The rewrite itself, and the settings
    /// that keep coming from the alias's own `Host` block, are pinned
    /// in tests/ssh_config_alias.rs, which owns HOME.
    #[test]
    fn a_remote_no_ssh_config_renames_is_contacted_as_it_stands() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://github.com/joyint/joy.git")
            .unwrap();
        let remote = contact_remote(&repo).expect("the configured remote");
        assert_eq!(remote.url().ok(), Some("https://github.com/joyint/joy.git"));
        assert_eq!(
            remote.name().ok().flatten(),
            Some("origin"),
            "the named remote, so its refspecs and its tracking refs stand"
        );
    }

    /// git applies an `insteadOf` rule before ssh ever reads its
    /// config, and so does libgit2 (remote.c:254-255, :509-510). Where
    /// there is such a rule, joy's own `HostName` rewrite stands back,
    /// so the rule's target is what is dialled.
    #[test]
    fn an_insteadof_rule_keeps_joys_own_rewrite_out_of_the_way() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        {
            let mut config = repo.config().unwrap();
            config
                .set_str("url.git@github.com:.insteadOf", "work:")
                .unwrap();
            config
                .set_str("url.git@codeberg.org:.pushInsteadOf", "cb:")
                .unwrap();
        }
        assert!(rewritten_by_insteadof(&repo, "work:joyint/joy.git"));
        assert!(rewritten_by_insteadof(&repo, "cb:joyint/joy.git"));
        assert!(!rewritten_by_insteadof(&repo, "git@work:joyint/joy.git"));
        assert!(!rewritten_by_insteadof(
            &repo,
            "https://github.com/joyint/joy.git"
        ));
    }

    #[test]
    fn an_https_remote_offers_the_helper_and_says_so_when_it_has_none() {
        let mut state = ChainState::prepare(
            "https://gitea.example.com/o/r.git",
            None,
            HostKind::Background,
            None,
        );
        assert_eq!(state.host, "gitea.example.com");
        assert_eq!(state.steps.len(), 1);
        // With no git config in hand the chain is exhausted at once,
        // and the sentence says which host and why.
        let mut chain = LocalChain::new(HostKind::Background);
        let error = match chain.credential(
            "https://gitea.example.com/o/r.git",
            None,
            git2::CredentialType::USER_PASS_PLAINTEXT,
            &CredSource::none(),
        ) {
            Ok(_) => panic!("a chain with no configuration cannot answer"),
            Err(e) => e,
        };
        assert!(
            error.message().contains("gitea.example.com"),
            "{}",
            error.message()
        );
        assert!(
            error.message().contains("git configuration"),
            "{}",
            error.message()
        );
        let _ = state.take_next(git2::CredentialType::USER_PASS_PLAINTEXT);
    }

    #[test]
    fn the_user_name_stays_the_same_however_often_it_is_asked() {
        let mut state = ChainState::prepare(
            "ssh://deploy@git.example.invalid/o/r.git",
            None,
            HostKind::Background,
            None,
        );
        let first = state.user.clone();
        assert_eq!(first, "deploy");
        for _ in 0..3 {
            assert!(state.user_name().is_ok());
            assert_eq!(state.user, first);
        }
        // And it does not loop for ever when a forge keeps asking.
        assert!(state.user_name().is_err());
    }

    #[test]
    fn a_candidate_the_forge_does_not_accept_is_skipped_not_offered() {
        let mut state = ChainState::prepare(
            "https://gitea.example.com/o/r.git",
            None,
            HostKind::Background,
            None,
        );
        // An https chain holds a helper step, which an ssh-only mask
        // cannot take: it is skipped, and the chain is then empty.
        assert!(state.take_next(git2::CredentialType::SSH_KEY).is_none());
        assert!(state.exhausted().contains("gitea.example.com"));
    }

    #[test]
    fn a_remote_with_nothing_in_the_way_is_not_guarded_against() {
        assert!(guard_transport(None).is_ok());
        assert!(guard_transport(Some("https://github.com/o/r.git")).is_ok());
        assert!(guard_transport(Some("/srv/git/local.git")).is_ok());
    }

    /// D1.4a's pre-validation, at the one place it acts: an ssh contact
    /// stops before libgit2 opens a socket and reads the line number
    /// instead of "error reading known_hosts", and no other transport
    /// even looks at the file. The host is a name no ssh config can
    /// hold an opinion about, so the answer is the guard's and not the
    /// machine's.
    #[test]
    fn a_broken_known_hosts_file_stops_an_ssh_contact_with_the_line_number() {
        let sentence = || Some("/home/troi/.ssh/known_hosts line 7: ... ".to_string());
        let ssh = "git@known-hosts.invalid:owner/repo.git";
        let refused = guard_transport_with(Some(ssh), sentence).expect_err("the file is broken");
        assert!(refused.to_string().contains("line 7"), "{refused}");
        // a file libssh2 can read lets the same contact through
        assert!(guard_transport_with(Some(ssh), || None).is_ok());
        // and nothing else reads known_hosts at all
        for other in ["https://github.com/o/r.git", "/srv/git/local.git"] {
            assert!(
                guard_transport_with(Some(other), || { panic!("{other} read known_hosts") })
                    .is_ok()
            );
        }
        assert!(guard_transport_with(None, || panic!("no remote read known_hosts")).is_ok());
    }
}
