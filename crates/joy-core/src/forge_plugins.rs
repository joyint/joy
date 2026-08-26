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
use std::process::{Command, Stdio};
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

fn run_query_full(
    binary: &str,
    root: &Path,
    args: &[&str],
    env: Option<(&str, &str)>,
    timeout: Duration,
) -> Option<String> {
    let mut command = Command::new(binary);
    if let Some((var, value)) = env {
        command.env(var, value);
    }
    let mut child = command
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return Some(out);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
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
        // garbage output and unknown subcommands degrade to nothing
        assert!(run_query(spec.binary, root, &["garbage"])
            .and_then(|o| serde_json::from_str::<ClaimsAnswer>(&o).ok())
            .is_none());
        assert!(run_query(spec.binary, root, &["nope"]).is_none());
        // a missing binary is silently no answer
        let missing = ForgePluginSpec {
            id: "ghost",
            binary: "joy-does-not-exist-anywhere",
        };
        assert!(!claims(&missing, root, "url"));
        assert!(identity(&missing, root, &CallerFacts::default()).is_none());
        assert!(resolve(&missing, root, "x@y").is_none());
    }
}
