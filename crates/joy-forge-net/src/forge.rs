// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! What every forge connector is, and what every call carries.
//!
//! One binary carries all three forges (D2.1), so the forge is data the
//! dispatcher picks, adapter registry style (JI-017A-85): the binary
//! owns the protocol, the implementation owns the forge knowledge, and
//! neither knows the other's business.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::config::{Instance, Instances};
use crate::gitconfig::GitConfig;
use crate::http::{Http, HttpError};
use crate::trust;

/// Who is behind the process that asked (D1.10). The word arrives as
/// `--host-kind` on every protocol 2 call.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// A person typed this and is watching.
    Interactive,
    /// No person: a hook, a worker, a server. The careful default.
    #[default]
    Background,
    /// An agent under a live delegation session.
    Delegated,
}

impl HostKind {
    pub fn as_str(self) -> &'static str {
        match self {
            HostKind::Interactive => "interactive",
            HostKind::Background => "background",
            HostKind::Delegated => "delegated",
        }
    }

    /// Whether a step that could raise an operating system dialog may
    /// be taken at all.
    pub fn may_ask(self) -> bool {
        matches!(self, HostKind::Interactive)
    }
}

impl std::str::FromStr for HostKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "interactive" => Ok(HostKind::Interactive),
            "background" => Ok(HostKind::Background),
            "delegated" => Ok(HostKind::Delegated),
            other => Err(format!(
                "unknown host kind '{other}'; expected interactive, background or delegated"
            )),
        }
    }
}

/// What a verb is asked about: a remote URL, or a bare host for the
/// calls that have no repository (D2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Remote(String),
    Host(String),
}

impl Target {
    /// The host this target names.
    pub fn host(&self) -> Option<String> {
        match self {
            Target::Remote(url) => crate::url::host_of(url),
            Target::Host(host) => {
                let host = host.trim().to_ascii_lowercase();
                (!host.is_empty()).then_some(host)
            }
        }
    }

    /// The `owner/repo` path, for the verbs that need a repository.
    pub fn repo_path(&self) -> Option<String> {
        match self {
            Target::Remote(url) => crate::url::repo_path_of(url),
            Target::Host(_) => None,
        }
    }

    /// The remote URL, where the target is one.
    pub fn url(&self) -> Option<&str> {
        match self {
            Target::Remote(url) => Some(url),
            Target::Host(_) => None,
        }
    }
}

/// What a `repositories` call asks for (D2.4).
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub query: Option<String>,
    pub limit: usize,
    pub page: Option<String>,
}

/// The default of D2.4: 200 repositories, paginated so one answer stays
/// well under the 64 KiB a pipe buffer holds.
pub const DEFAULT_LIMIT: usize = 200;

/// What a `create-repository` call asks for (D2.4).
#[derive(Debug, Clone, Default)]
pub struct NewRepository {
    pub name: String,
    pub owner: Option<String>,
    pub private: bool,
}

/// What a `release` call asks for.
#[derive(Debug, Clone)]
pub struct ReleaseRequest {
    pub tag: String,
    pub title: String,
    pub notes: String,
}

/// Who a token speaks for, asked of the instance's own API. This is
/// what `token-store` validates a pasted token with before it stores
/// it (D2.4), and what a finished `login` reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub login: String,
    pub user_id: Option<String>,
    pub emails: Vec<String>,
    /// The granted set the forge reported, space separated. `None`
    /// means "not known", and an unknown set is never reported as a
    /// missing one (D2.7c).
    pub scopes: Option<String>,
}

/// What one probe of D4.1c found: whether this login sees the
/// repository at all, and whether it may push to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reach {
    pub read: bool,
    pub push: bool,
}

/// One forge, as the dispatcher sees it.
pub trait Forge: Sync {
    /// The id `project.yaml`'s `forge:` uses and the combined binary
    /// takes as its first argument.
    fn id(&self) -> &'static str;

    /// The name a person writes.
    fn display(&self) -> &'static str;

    /// Does this forge own the host by its own knowledge? The
    /// configured instances of `forges.yaml` and the project override
    /// are consulted by the dispatcher, not here.
    fn claims(&self, host: &str, ctx: &Ctx) -> bool;

    fn identity(&self, target: &Target, ctx: &Ctx) -> Value;

    fn resolve(&self, email: &str) -> Value;

    fn store(&self, target: &Target, ctx: &Ctx) -> Value;

    fn files(&self, target: &Target, ctx: &Ctx) -> Value;

    fn repositories(&self, target: &Target, listing: &Listing, ctx: &Ctx) -> Value;

    fn create_repository(&self, target: &Target, new: &NewRepository, ctx: &Ctx) -> Value;

    /// The one write verb, and the one that reports its failure instead
    /// of degrading to "unknown".
    fn release(
        &self,
        target: &Target,
        request: &ReleaseRequest,
        ctx: &Ctx,
    ) -> anyhow::Result<Value>;

    // -- the sign in half (D2.4, D2.7, package J3) ---------------------

    /// The scope set this forge asks for at this level (D2.7a). The
    /// `--for` flag picks the level; the forge owns the words.
    fn scopes(&self, purpose: crate::auth::Purpose) -> &'static str;

    /// The OAuth application and endpoints for this host (D2.7), or
    /// `None` where no door exists: a self hosted instance whose
    /// operator registered no client and put none in `forges.yaml`.
    fn oauth(
        &self,
        host: &str,
        purpose: crate::auth::Purpose,
        ctx: &Ctx,
    ) -> Option<crate::auth::oauth::OAuth>;

    /// Who this token speaks for, asked of the instance's own API.
    /// `None` when the forge does not accept it, which is what makes
    /// `token-store` a validation and not a paste.
    fn account(&self, host: &str, token: &str, ctx: &Ctx) -> Option<Account>;

    /// Whether this token reaches `owner/repo` (the probe of D4.1c).
    /// One request per candidate, never per contact. `None` means the
    /// forge did not answer at all.
    fn reaches(&self, host: &str, repo_path: &str, token: &str, ctx: &Ctx) -> Option<Reach>;

    /// The https twin of a remote (D1.5, the `web-url` verb). Answered
    /// from the address and the instance configuration alone: only the
    /// forge knows the web base of a self hosted instance, which may
    /// sit under a nested sub path or behind a different SSH domain.
    fn web_url(&self, target: &Target, ctx: &Ctx) -> Value;

    /// Revoke a token at the forge, where the forge offers it (D2.4).
    /// `false` is the honest answer of a forge that offers nothing, and
    /// the local entry is removed either way.
    fn revoke(&self, _host: &str, _record: &crate::auth::store::Record, _ctx: &Ctx) -> bool {
        false
    }

    /// The user name the https twin presents beside the token in basic
    /// authentication (the `username` field of D2.4's `token` answer).
    /// It is forge knowledge: GitHub takes `x-access-token`, GitLab
    /// takes `oauth2`, the Gitea family takes the login.
    fn https_username(&self) -> &'static str;

    /// The forge CLI a credential may also come from: `gh`, `glab` or
    /// `tea`. joy reads it by spawning that CLI and never writes,
    /// refreshes or revokes it (D2.6, decision 19).
    fn foreign_cli(&self) -> &'static str;

    /// The command a person runs to remove the FOREIGN credential,
    /// which `logout` names instead of removing anything itself.
    fn foreign_logout_command(&self, host: &str) -> String;

    /// The logins that CLI is signed in as on this host, for the probe
    /// candidate order of D4.1c.
    fn foreign_logins(&self, _host: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Everything one call carries besides its verb and its target.
pub struct Ctx {
    pub host_kind: HostKind,
    pub login: Option<String>,
    pub user_id: Option<String>,
    pub token_env: Option<String>,
    pub instances: Instances,
    /// The project the call runs in, when there is one. A connector
    /// call needs no project root (D2.3).
    pub root: PathBuf,
    /// The `forge:` override of that project (D2.5).
    pub project_forge: Option<String>,
    /// The remote this call is about, when the target is one. The login
    /// order of D4.1c is per remote, so every `token` lookup inside a
    /// verb needs it without every verb having to pass it along.
    pub remote: Option<String>,
    git: GitConfig,
    /// Where this call keeps credentials (D2.6). A library caller gets
    /// [`crate::auth::store::Vault::none`], so nothing touches a
    /// person's credential store by accident.
    vault: crate::auth::store::Vault,
    /// Where the refresh locks of D2.6a and the login memory of D4.1c
    /// live. `None` is the person's own app state directory; a test
    /// names a temporary one so it takes no lock a person shares.
    state_dir: Option<PathBuf>,
    clients: Mutex<HashMap<String, Arc<Http>>>,
    tokens: Mutex<HashMap<String, Option<crate::auth::Resolved>>>,
}

impl Ctx {
    /// Build the context of one call. `root` is the working directory
    /// the caller started the connector in.
    pub fn new(
        host_kind: HostKind,
        login: Option<String>,
        user_id: Option<String>,
        token_env: Option<String>,
        root: PathBuf,
    ) -> Self {
        let project_forge = crate::config::project_forge(&root);
        Ctx {
            host_kind,
            login,
            user_id,
            token_env,
            instances: Instances::load(),
            git: GitConfig::load(Some(&root)),
            root,
            project_forge,
            remote: None,
            vault: crate::auth::store::Vault::real(),
            state_dir: None,
            clients: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// A context with nothing configured, for the pure verbs and the
    /// tests.
    pub fn bare(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Ctx {
            host_kind: HostKind::default(),
            login: None,
            user_id: None,
            token_env: None,
            instances: Instances::empty(),
            git: GitConfig::default(),
            project_forge: crate::config::project_forge(&root),
            root,
            remote: None,
            vault: crate::auth::store::Vault::none(),
            state_dir: None,
            clients: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Replace the configured instances (the tests and `bare` callers).
    pub fn with_instances(mut self, instances: Instances) -> Self {
        self.instances = instances;
        self
    }

    /// Name the variable a token travels in (the tests and a caller
    /// that drives a forge as a library).
    pub fn with_token_env(mut self, variable: impl Into<String>) -> Self {
        self.token_env = Some(variable.into());
        self
    }

    /// Pin the login this call speaks for (D4.1c).
    pub fn with_login(mut self, login: impl Into<String>) -> Self {
        self.login = Some(login.into());
        self
    }

    /// Name the remote this call is about, so the login order of D4.1c
    /// has its key.
    pub fn with_remote(mut self, remote: impl Into<String>) -> Self {
        self.remote = Some(remote.into());
        self
    }

    /// Give this call a credential store (the connector's own real one,
    /// or a file under a temporary directory in the tests).
    pub fn with_vault(mut self, vault: crate::auth::store::Vault) -> Self {
        self.vault = vault;
        self
    }

    /// Where this call keeps credentials.
    pub fn vault(&self) -> &crate::auth::store::Vault {
        &self.vault
    }

    /// Name the directory the refresh locks and the login memory live
    /// in, instead of the person's own app state directory.
    pub fn with_state_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.state_dir = Some(dir.into());
        self
    }

    /// Where the refresh locks and the login memory live, when the
    /// caller named a directory.
    pub fn state_dir(&self) -> Option<&Path> {
        self.state_dir.as_deref()
    }

    /// The project root of this call.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The instance an operator configured for this host (D2.5).
    pub fn instance(&self, host: &str) -> Option<&Instance> {
        self.instances.for_host(host)
    }

    /// The HTTP client for this host: one per host, because the trust
    /// of D1.12 is per instance.
    pub fn http(&self, host: &str) -> Result<Arc<Http>, HttpError> {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(client) = clients.get(host) {
            return Ok(client.clone());
        }
        let trust = trust::decide(self.instance(host), &self.git, &mut |sentence| {
            eprintln!("joy: {sentence}");
        });
        let client = Arc::new(Http::new(
            trust,
            self.git.clone(),
            format!("joy-forge/{}", env!("CARGO_PKG_VERSION")),
        ));
        clients.insert(host.to_string(), client.clone());
        Ok(client)
    }

    /// The token for this host: the whole source order of D2.4, cached
    /// per host for this process because the answer may cost a spawn.
    pub fn token(&self, forge: &str, host: &str) -> Option<String> {
        self.resolved_token(forge, host)
            .map(|resolved| resolved.token)
    }

    /// [`Ctx::token`] with everything D2.4's answer needs beside the
    /// secret: which login it belongs to, which source held it, the
    /// granted set and the lifetime.
    pub fn resolved_token(&self, forge: &str, host: &str) -> Option<crate::auth::Resolved> {
        let key = format!("{forge}@{host}");
        let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(resolved) = tokens.get(&key) {
            return resolved.clone();
        }
        let resolved = self.find_token(forge, host);
        tokens.insert(key, resolved.clone());
        resolved
    }

    /// The source order of D2.4: the connector's own entry, then the
    /// forge CLI, spawned. Two sources sit outside that list and before
    /// it, for reasons the design names:
    ///
    /// - the variable the CALLER named (`--token-env`) is the
    ///   platform's per call hand over and the whole answer when it is
    ///   given, because a multi account host whose variable happens to
    ///   be empty must never end up acting as the machine's own
    ///   account;
    /// - the forge's own variables (`GH_TOKEN` and its siblings) come
    ///   after the connector's own entry and before the forge CLI: they
    ///   are what a CI runner has, and `joy release publish` names no
    ///   variable of its own.
    fn find_token(&self, forge: &str, host: &str) -> Option<crate::auth::Resolved> {
        use crate::auth::Source;
        if let Some(var) = self.token_env.as_deref() {
            return read_variable(var).map(|token| crate::auth::Resolved {
                token,
                login: self.login.clone(),
                source: Source::Env,
                scopes: None,
                expires_at: None,
                chose_by: None,
            });
        }
        // The connector's own entry, refreshed under the lock of D2.6a
        // where it is past its lifetime.
        if let Some(resolved) = crate::auth::verbs::own_token(self, host) {
            return Some(resolved);
        }
        if let Some(value) = token_variables(forge, host)
            .iter()
            .find_map(|name| read_variable(name))
        {
            return Some(crate::auth::Resolved {
                token: value,
                login: self.login.clone(),
                source: Source::Env,
                scopes: None,
                expires_at: None,
                chose_by: None,
            });
        }
        // Decision 19: a foreign credential is obtained by SPAWNING the
        // CLI, never by reading its store, and that is also the only
        // way the CLI's own refresh runs.
        let (token, source) = match forge {
            "github" => (
                crate::foreign::gh_token(host, self.login.as_deref()),
                Source::Gh,
            ),
            "gitlab" => (crate::foreign::glab_token(host), Source::Glab),
            "gitea" => (crate::foreign::tea_token(host), Source::Tea),
            _ => (None, Source::Env),
        };
        token.map(|token| crate::auth::Resolved {
            token,
            login: self.login.clone(),
            source,
            scopes: None,
            expires_at: None,
            chose_by: None,
        })
    }

    /// The granted scope set of the credential this call will use,
    /// where the source knows it. Since J3 the connector's own entry
    /// carries it beside the token (D2.7c), so the local pre check
    /// costs nothing: no request is spent to learn a set joy already
    /// wrote down. `None` means "not known", and an unknown set is
    /// never reported as a missing one.
    pub fn granted_scopes(&self, forge: &str, host: &str) -> Option<Vec<String>> {
        let resolved = self.resolved_token(forge, host)?;
        let scopes = resolved.scopes?;
        let granted = crate::scope::parse_granted(&scopes);
        (!granted.is_empty()).then_some(granted)
    }

    /// The git configuration this call reads (the proxy sources of
    /// D1.11 and the Linux CA keys of D1.12).
    pub fn git_config(&self) -> &GitConfig {
        &self.git
    }
}

/// One environment variable, where it holds something.
fn read_variable(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The environment variables a forge's own tooling puts a token in, in
/// the order that tooling reads them.
///
/// GitHub's pair is split by host the way gh splits it, so a github.com
/// token is never sent to somebody's Enterprise Server and the other
/// way round. A variable is read only for the host it belongs to.
pub fn token_variables(forge: &str, host: &str) -> &'static [&'static str] {
    match forge {
        "github" => {
            if host == "github.com" || host.ends_with(".github.com") {
                &["GH_TOKEN", "GITHUB_TOKEN"]
            } else {
                &["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]
            }
        }
        "gitlab" => &["GITLAB_TOKEN"],
        "gitea" => &["GITEA_TOKEN"],
        _ => &[],
    }
}

/// The answer of a verb that could not be answered at all.
pub fn unknown_state() -> Value {
    json!({ "state": "unknown" })
}

/// The answer of a verb that knows nobody.
pub fn unknown() -> Value {
    json!({ "known": false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_reads_its_host_and_its_repository_path() {
        let remote = Target::Remote("git@github.com:joyint/app.git".into());
        assert_eq!(remote.host().as_deref(), Some("github.com"));
        assert_eq!(remote.repo_path().as_deref(), Some("joyint/app"));
        let host = Target::Host("GitHub.com".into());
        assert_eq!(host.host().as_deref(), Some("github.com"));
        assert_eq!(host.repo_path(), None);
    }

    /// The `env` source of D2.4, split by host the way gh splits it: a
    /// github.com token never travels to an Enterprise Server.
    #[test]
    fn the_forges_own_token_variables_are_read_per_host() {
        assert_eq!(
            token_variables("github", "github.com"),
            ["GH_TOKEN", "GITHUB_TOKEN"]
        );
        assert_eq!(
            token_variables("github", "ghe.acme.test"),
            ["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]
        );
        assert_eq!(token_variables("gitlab", "gitlab.com"), ["GITLAB_TOKEN"]);
        assert_eq!(token_variables("gitea", "git.acme.test"), ["GITEA_TOKEN"]);
        assert!(token_variables("sourcehut", "sr.ht").is_empty());
    }

    /// Two rules in one case: the forge's own variable answers a call
    /// that named none (J2's `GH_TOKEN` acceptance), and a call that
    /// DID name one gets that variable and nothing else, even when it
    /// holds nothing.
    #[test]
    fn a_named_variable_is_the_whole_answer_and_the_forges_own_fills_the_gap() {
        std::env::set_var("GH_TOKEN", "gho_from_the_environment");
        std::env::set_var("JOY_TEST_EMPTY_TOKEN", "");
        let ambient = Ctx::bare(std::env::temp_dir());
        assert_eq!(
            ambient.token("github", "github.com").as_deref(),
            Some("gho_from_the_environment")
        );
        // which variable belongs to which host is the case above; no
        // forge CLI is spawned here, because both answers are found
        // before that step
        let named = Ctx::bare(std::env::temp_dir()).with_token_env("JOY_TEST_EMPTY_TOKEN");
        assert_eq!(named.token("github", "github.com"), None);
        std::env::remove_var("GH_TOKEN");
        std::env::remove_var("JOY_TEST_EMPTY_TOKEN");
    }

    #[test]
    fn a_host_kind_parses_the_three_words_and_nothing_else() {
        assert_eq!(
            "delegated".parse::<HostKind>().unwrap(),
            HostKind::Delegated
        );
        assert_eq!(HostKind::Interactive.as_str(), "interactive");
        assert!(HostKind::Interactive.may_ask());
        assert!(!HostKind::Delegated.may_ask());
        assert!("nobody".parse::<HostKind>().is_err());
    }
}
