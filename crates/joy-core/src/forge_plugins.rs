// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! THE forge-plugin registry and its query client (JOY-0252-1A, epic
//! JOY-0251-AA).
//!
//! All forge knowledge (host names, alias address formats, API access)
//! lives in the `joy-<forge>` plugin binaries; this module knows only the
//! plugin NAMES and the JSON query protocol (docs/plugins.md, "Forge
//! plugins"). Everything here is BEST EFFORT by design: a missing binary,
//! a timeout or a garbled answer degrades to "no claim / unknown" —
//! identity resolution must never fail because a plugin is absent, and a
//! project without remotes or plugins behaves as if this module did not
//! exist.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// One row per known forge plugin, adapter-registry style (JI-017A-85):
/// data, not behavior. Adding a forge is one row here plus installing its
/// binary; nothing else in joy-core changes.
pub struct ForgePluginSpec {
    /// The forge id as `project.yaml`'s `forge:` override names it.
    pub id: &'static str,
    /// The plugin binary on the PATH.
    pub binary: &'static str,
}

/// The registry. Order matters only for the claims round-robin.
pub const FORGE_PLUGINS: &[ForgePluginSpec] = &[
    ForgePluginSpec {
        id: "github",
        binary: "joy-github",
    },
    ForgePluginSpec {
        id: "gitlab",
        binary: "joy-gitlab",
    },
    ForgePluginSpec {
        id: "gitea",
        binary: "joy-gitea",
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

/// Caller facts a multi-account host hands to the plugin: the platform's
/// session knows who acts (forge login, user id, a token in an env var),
/// while a single-person device passes none and the plugin finds its own
/// facts (e.g. the forge CLI's config).
#[derive(Debug, Clone, Default)]
pub struct CallerFacts {
    pub login: Option<String>,
    pub user_id: Option<String>,
    /// Name of an environment variable holding a forge token — the token
    /// itself must never appear in a process list. When `token_value` is
    /// set too, the variable is injected into the PLUGIN's environment
    /// only (a multi-account host must never widen its own process env).
    pub token_env: Option<String>,
    pub token_value: Option<String>,
}

/// A forge plugin's identity answer (docs/plugins.md).
#[derive(Debug, Clone, Deserialize)]
pub struct ForgeIdentity {
    pub known: bool,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    /// Verified addresses the plugin vouches for; possibly empty when its
    /// source cannot list them.
    #[serde(default)]
    pub emails: Vec<String>,
}

#[derive(Deserialize)]
struct ClaimsAnswer {
    claims: bool,
}

/// Whether this plugin claims the remote. False on every failure.
pub fn claims(spec: &ForgePluginSpec, root: &Path, remote_url: &str) -> bool {
    run_query(spec.binary, root, &["claims", "--remote", remote_url])
        .and_then(|out| serde_json::from_str::<ClaimsAnswer>(&out).ok())
        .map(|a| a.claims)
        .unwrap_or(false)
}

/// Who is ACTING on the forge; `None` on every failure or `known:false`.
pub fn identity(spec: &ForgePluginSpec, root: &Path, facts: &CallerFacts) -> Option<ForgeIdentity> {
    let mut args: Vec<&str> = vec!["identity"];
    if let Some(login) = facts.login.as_deref() {
        args.extend(["--login", login]);
    }
    if let Some(id) = facts.user_id.as_deref() {
        args.extend(["--user-id", id]);
    }
    if let Some(var) = facts.token_env.as_deref() {
        args.extend(["--token-env", var]);
    }
    let env = match (facts.token_env.as_deref(), facts.token_value.as_deref()) {
        (Some(var), Some(value)) => Some((var, value)),
        _ => None,
    };
    run_query_env(spec.binary, root, &args, env)
        .and_then(|out| serde_json::from_str::<ForgeIdentity>(&out).ok())
        .filter(|identity| identity.known)
}

/// Whose address is this? PURE by contract: the plugin answers from the
/// address alone, never from ambient state. `None` on every failure or
/// `known:false`.
pub fn resolve(spec: &ForgePluginSpec, root: &Path, email: &str) -> Option<ForgeIdentity> {
    run_query(spec.binary, root, &["resolve", "--email", email])
        .and_then(|out| serde_json::from_str::<ForgeIdentity>(&out).ok())
        .filter(|identity| identity.known)
}

/// What a forge says about a repository's joy store (JP-013C-11), the
/// answer of the `store` query. A multi-account host asks it instead of
/// cloning the repository to find out.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum StoreAnswer {
    /// The store is there and readable: the content of its project.yaml.
    Store { project_yaml: String },
    /// The repository is there but holds no store; whether the caller may
    /// push one, and the branch the forge names as the default (the first
    /// push of an empty repository goes there).
    Missing {
        may_create: bool,
        #[serde(default)]
        default_branch: Option<String>,
    },
    /// The forge does not show the caller the repository: deleted, or no
    /// access. Forges answer both the same way on purpose.
    Gone,
    /// The forge could not be asked (network, a refused token, an answer
    /// the plugin did not expect). Never a verdict on the repository.
    Unknown,
}

/// Does the repository at `remote_url` hold a joy store, and may the
/// caller create one? `None` on every plugin failure and on `unknown`:
/// the question stayed unanswered.
pub fn store(
    spec: &ForgePluginSpec,
    root: &Path,
    remote_url: &str,
    facts: &CallerFacts,
) -> Option<StoreAnswer> {
    let mut args: Vec<&str> = vec!["store", "--remote", remote_url];
    if let Some(var) = facts.token_env.as_deref() {
        args.extend(["--token-env", var]);
    }
    let env = match (facts.token_env.as_deref(), facts.token_value.as_deref()) {
        (Some(var), Some(value)) => Some((var, value)),
        _ => None,
    };
    run_query_full(spec.binary, root, &args, env, STORE_TIMEOUT)
        .and_then(|out| serde_json::from_str::<StoreAnswer>(&out).ok())
        .filter(|answer| *answer != StoreAnswer::Unknown)
}

/// The files a repository's default branch carries (JAPP-0293-A7), the
/// answer of the `files` query. A listing the forge cut off, or the plugin
/// bounded, says so.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum FilesAnswer {
    Files { paths: Vec<String>, truncated: bool },
    Unknown,
}

/// The files of the repository at `remote_url`; `None` on every plugin
/// failure and on `unknown`.
pub fn files(
    spec: &ForgePluginSpec,
    root: &Path,
    remote_url: &str,
    facts: &CallerFacts,
) -> Option<FilesAnswer> {
    let mut args: Vec<&str> = vec!["files", "--remote", remote_url];
    if let Some(var) = facts.token_env.as_deref() {
        args.extend(["--token-env", var]);
    }
    let env = match (facts.token_env.as_deref(), facts.token_value.as_deref()) {
        (Some(var), Some(value)) => Some((var, value)),
        _ => None,
    };
    run_query_full(spec.binary, root, &args, env, STORE_TIMEOUT)
        .and_then(|out| serde_json::from_str::<FilesAnswer>(&out).ok())
        .filter(|answer| *answer != FilesAnswer::Unknown)
}

/// The plugin responsible for this project: the `forge:` override when it
/// names a registered plugin, else the first registry row that claims one
/// of the remotes. `None` = nobody is responsible (a local-only project,
/// or no plugin installed) and every caller proceeds exactly as before.
pub fn responsible_plugin(
    forge_override: Option<&str>,
    root: &Path,
    remotes: &[(String, String)],
) -> Option<&'static ForgePluginSpec> {
    if let Some(id) = forge_override {
        // An explicit override is the operator's word: no claims round.
        return by_id(id);
    }
    if remotes.is_empty() {
        return None;
    }
    FORGE_PLUGINS
        .iter()
        .find(|spec| remotes.iter().any(|(_, url)| claims(spec, root, url)))
}

/// How long a plugin may take per query. Queries are local parses or one
/// forge API call; anything slower must not stall a `joy` command.
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the store and files queries may take: several forge API calls
/// (the file, the repository, GitLab's branch protection; the pages of a
/// tree), each bounded by the plugin itself.
const STORE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the release verb may take: it talks to the forge's release
/// API (possibly view + edit/create), so it gets network patience the
/// read queries must not have.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(120);

/// What the release verb answered (JOY-0256-64). `unsupported` is the
/// plugin saying "my forge has no release backend yet" — the caller then
/// keeps the tag-only publish instead of failing the whole release.
#[derive(Debug, Deserialize)]
pub struct ReleaseOutcome {
    /// The release URL when the forge reports one.
    pub url: Option<String>,
    /// The plugin has no release capability for its forge (yet).
    #[serde(default)]
    pub unsupported: bool,
}

/// Create (or complete) the release for `tag` through the plugin.
/// `None` = the plugin failed (not installed, non-zero exit, timeout);
/// its own message reached stderr already, the caller adds the verdict.
pub fn release(
    spec: &ForgePluginSpec,
    root: &Path,
    tag: &str,
    title: &str,
    notes_file: &Path,
) -> Option<ReleaseOutcome> {
    let notes_path = notes_file.to_string_lossy();
    run_query_timeout(
        spec.binary,
        root,
        &[
            "release",
            "--tag",
            tag,
            "--title",
            title,
            "--notes-file",
            &notes_path,
        ],
        RELEASE_TIMEOUT,
    )
    .and_then(|out| serde_json::from_str::<ReleaseOutcome>(&out).ok())
}

/// Run one query, capture stdout. `None` on spawn failure (plugin not
/// installed), non-zero exit, timeout, or non-UTF8 output. Stderr is
/// inherited so a plugin's diagnostics reach the person unfiltered.
fn run_query(binary: &str, root: &Path, args: &[&str]) -> Option<String> {
    run_query_env(binary, root, args, None)
}

/// [`run_query`] with one variable injected into the CHILD's environment
/// only (a secret handed to the plugin, never widened onto this process).
fn run_query_env(
    binary: &str,
    root: &Path,
    args: &[&str],
    env: Option<(&str, &str)>,
) -> Option<String> {
    run_query_full(binary, root, args, env, QUERY_TIMEOUT)
}

/// [`run_query`] with its own deadline (the release verb needs network
/// patience the read queries must not have).
fn run_query_timeout(
    binary: &str,
    root: &Path,
    args: &[&str],
    timeout: Duration,
) -> Option<String> {
    run_query_full(binary, root, args, None, timeout)
}

/// The one place a plugin call can fail silently, so the one place that
/// says so (forge connection NG, D5 and D2.3, packages J1 and P1a).
///
/// The ANSWER stays best effort: every failure is still `None` and every
/// caller still degrades to "unknown". What changes is that the failure
/// leaves a trace. Without it, a connector that is missing, shadowed by
/// a stale binary, refused or slow turns into "unknown" for the person
/// and into nothing at all for the operator, which is exactly how a
/// broken server image survived a working day.
///
/// Every line carries the plugin and the verb, because both are what a
/// reader needs to act: the binary to look for and the question that was
/// asked.
fn run_query_full(
    binary: &str,
    root: &Path,
    args: &[&str],
    env: Option<(&str, &str)>,
    timeout: Duration,
) -> Option<String> {
    let verb = args.first().copied().unwrap_or("");
    let mut command = joy_process::command(binary);
    if let Some((var, value)) = env {
        command.env(var, value);
    }
    let mut child = match command
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            tracing::warn!(
                plugin = binary,
                verb,
                error = %e,
                "the forge plugin could not be started"
            );
            return None;
        }
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let code = status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "a signal".to_string());
                    tracing::warn!(
                        plugin = binary,
                        verb,
                        code = %code,
                        "the forge plugin refused the verb"
                    );
                    return None;
                }
                let mut out = String::new();
                match child.stdout.take() {
                    Some(mut pipe) => {
                        if let Err(e) = pipe.read_to_string(&mut out) {
                            tracing::warn!(
                                plugin = binary,
                                verb,
                                error = %e,
                                "the forge plugin's answer could not be read"
                            );
                            return None;
                        }
                    }
                    None => return None,
                }
                return Some(out);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    tracing::warn!(
                        plugin = binary,
                        verb,
                        timeout_secs = timeout.as_secs(),
                        "the forge plugin did not answer in time and was stopped"
                    );
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                tracing::warn!(
                    plugin = binary,
                    verb,
                    error = %e,
                    "the forge plugin could not be waited for"
                );
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;

    #[test]
    fn the_registry_answers_by_id_case_insensitively() {
        assert_eq!(by_id("github").map(|s| s.binary), Some("joy-github"));
        assert_eq!(by_id(" GitLab ").map(|s| s.binary), Some("joy-gitlab"));
        assert_eq!(by_id("sourcehut").map(|s| s.binary), None);
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
                project_yaml: "name: x\n".into()
            }
        );
        assert_eq!(
            parse(r#"{"state":"missing","may_create":true,"default_branch":"main"}"#),
            StoreAnswer::Missing {
                may_create: true,
                default_branch: Some("main".into())
            }
        );
        // a plugin that names no default branch still parses
        assert_eq!(
            parse(r#"{"state":"missing","may_create":false}"#),
            StoreAnswer::Missing {
                may_create: false,
                default_branch: None
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

    /// A stub plugin on a private PATH proves the subprocess round trip
    /// AND the best-effort rules (missing binary, garbage, timeout are
    /// all "no answer"). Unix only: the stub is a shell script.
    #[cfg(unix)]
    #[test]
    fn queries_run_the_binary_and_degrade_on_every_failure() {
        let dir = tempfile::tempdir().unwrap();
        let stub = dir.path().join("joy-stubforge");
        {
            let mut f = std::fs::File::create(&stub).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(f, "case \"$1\" in").unwrap();
            writeln!(f, "claims) echo '{{\"claims\": true}}' ;;").unwrap();
            writeln!(
                f,
                "identity) echo '{{\"known\": true, \"login\": \"alice\", \"emails\": [\"a@example.com\"]}}' ;;"
            )
            .unwrap();
            writeln!(
                f,
                "store) echo '{{\"state\": \"missing\", \"may_create\": true}}' ;;"
            )
            .unwrap();
            writeln!(f, "garbage) echo 'not json' ;;").unwrap();
            writeln!(f, "*) exit 1 ;;").unwrap();
            writeln!(f, "esac").unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let spec = ForgePluginSpec {
            id: "stubforge",
            binary: Box::leak(stub.display().to_string().into_boxed_str()),
        };
        let root = dir.path();
        // Retry loop: a freshly written executable can hit ETXTBSY when a
        // parallel test forks while our fd was open (test-only race).
        let mut claimed = false;
        for _ in 0..20 {
            if claims(&spec, root, "git@stub:owner/repo.git") {
                claimed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(claimed, "the stub claims its remote");
        let id = identity(&spec, root, &CallerFacts::default()).expect("the stub answers");
        assert_eq!(id.login.as_deref(), Some("alice"));
        assert_eq!(
            store(
                &spec,
                root,
                "git@stub:owner/repo.git",
                &CallerFacts::default()
            ),
            Some(StoreAnswer::Missing {
                may_create: true,
                default_branch: None
            })
        );
        // garbage output and unknown subcommands degrade to nothing
        assert!(run_query(spec.binary, root, &["garbage"])
            .and_then(|o| serde_json::from_str::<ClaimsAnswer>(&o).ok())
            .is_none());
        assert!(run_query(spec.binary, root, &["nope"]).is_none());
        // a missing binary is no answer to the CALLER (and a warn line
        // to the operator, see below)
        let missing = ForgePluginSpec {
            id: "ghost",
            binary: "joy-does-not-exist-anywhere",
        };
        assert!(!claims(&missing, root, "url"));
        assert!(identity(&missing, root, &CallerFacts::default()).is_none());
        assert!(resolve(&missing, root, "x@y").is_none());
        assert!(store(&missing, root, "url", &CallerFacts::default()).is_none());
    }

    /// Best effort is not the same as silent (forge connection NG, D5):
    /// the answer degrades to "unknown", and the operator gets one warn
    /// line per failed call naming the plugin and the verb. Without it a
    /// connector that is missing, refused or slow is invisible until a
    /// person complains about a result nobody can explain.
    #[cfg(unix)]
    #[test]
    fn a_failing_plugin_call_is_warned_with_the_plugin_and_the_verb() {
        let dir = tempfile::tempdir().unwrap();
        let stub = dir.path().join("joy-refusing");
        {
            let mut f = std::fs::File::create(&stub).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(f, "exit 3").unwrap();
        }
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let log = WarnLog::default();
        tracing::subscriber::with_default(log.clone(), || {
            // not installed at all
            assert!(run_query("joy-does-not-exist-anywhere", dir.path(), &["claims"]).is_none());
            // installed and refusing (retried: a fresh executable can
            // still be ETXTBSY while a parallel test forks)
            for _ in 0..20 {
                if log.lines().iter().any(|l| l.contains("refused the verb")) {
                    break;
                }
                assert!(run_query(
                    &stub.display().to_string(),
                    dir.path(),
                    &["identity", "--login", "alice"]
                )
                .is_none());
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        let lines = log.lines();
        let missing = lines
            .iter()
            .find(|line| line.contains("could not be started"))
            .unwrap_or_else(|| panic!("no line about the missing plugin in {lines:?}"));
        assert!(missing.contains("joy-does-not-exist-anywhere"), "{missing}");
        assert!(missing.contains("claims"), "{missing}");
        let refused = lines
            .iter()
            .find(|line| line.contains("refused the verb"))
            .unwrap_or_else(|| panic!("no line about the refusing plugin in {lines:?}"));
        assert!(refused.contains("joy-refusing"), "{refused}");
        assert!(refused.contains("identity"), "{refused}");
        // `code` is recorded with Display, so it carries no quotes
        assert!(refused.contains("code=3"), "{refused}");
    }

    /// Keeps the fields of every warn event, so a test can read the line
    /// an operator would read. Hand written on purpose: joy-core carries
    /// no subscriber crate, not even for tests.
    #[cfg(unix)]
    #[derive(Clone, Default)]
    struct WarnLog(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    #[cfg(unix)]
    impl WarnLog {
        fn lines(&self) -> Vec<String> {
            self.0.lock().expect("the warn log").clone()
        }
    }

    #[cfg(unix)]
    impl tracing::Subscriber for WarnLog {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() <= tracing::Level::WARN
        }
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Fields<'a>(&'a mut String);
            impl tracing::field::Visit for Fields<'_> {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    use std::fmt::Write;
                    let _ = write!(self.0, " {}={value:?}", field.name());
                }
            }
            let mut line = String::new();
            event.record(&mut Fields(&mut line));
            self.0.lock().expect("the warn log").push(line);
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }
}
