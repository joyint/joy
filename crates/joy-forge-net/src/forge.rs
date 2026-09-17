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
    git: GitConfig,
    clients: Mutex<HashMap<String, Arc<Http>>>,
    tokens: Mutex<HashMap<String, Option<String>>>,
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

    /// The token for this host, from the sources wave 1 has (D2.4, J2):
    /// the variable the caller named, or gh, spawned (decision 19).
    ///
    /// The connector grows its own store in J3; until then a token that
    /// is nowhere here means the forge is asked anonymously, which sees
    /// public repositories only.
    pub fn token(&self, forge: &str, host: &str) -> Option<String> {
        let key = format!("{forge}@{host}");
        let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(token) = tokens.get(&key) {
            return token.clone();
        }
        let token = self.find_token(forge, host);
        tokens.insert(key, token.clone());
        token
    }

    fn find_token(&self, forge: &str, host: &str) -> Option<String> {
        if let Some(var) = self.token_env.as_deref() {
            if let Ok(value) = std::env::var(var) {
                if !value.trim().is_empty() {
                    return Some(value);
                }
            }
        }
        // D2.4 names `glab auth credential-helper` and `tea login
        // helper get` as the other two device side sources; both belong
        // to the `token` verb J3 builds, together with the connector's
        // own keychain entry.
        if forge == "github" {
            return crate::foreign::gh_token(host, self.login.as_deref());
        }
        None
    }

    /// The git configuration this call reads (the proxy sources of
    /// D1.11 and the Linux CA keys of D1.12).
    pub fn git_config(&self) -> &GitConfig {
        &self.git
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
