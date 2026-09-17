// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The three verbs that need a person at the machine: `login`,
//! `logout` and the token paste (D2.4, D3.11).
//!
//! They live in a module of their own because D3.11 compiles them out
//! of every build that must not perform them. The platform sets
//! `Background` everywhere and its binary is not supposed to carry
//! these at all: package J10 puts `interactive = []` into joy-core's
//! manifest, marks this module `#[cfg(feature = "interactive")]` and
//! adds the CI guard that keeps the platform's manifest from drifting
//! back. Nothing outside this file has to move when it does, which is
//! the whole reason the three verbs are here and not beside the read
//! verbs.
//!
//! Two of D3.11's three layers are already real here:
//!
//! - **No silent call path.** [`login`] takes a progress sink and a
//!   cancel token and has no argument free wrapper and no default sink.
//!   A caller cannot start a fifteen minute browser flow by accident.
//! - **Runtime refusal.** The agent image builds joy-cli from source,
//!   so a delegated agent has `joy forge login` on its PATH. [`login`]
//!   refuses on a `Background` or `Delegated` host before it spawns
//!   anything, with the sentence D3.11 writes, and the refusal is
//!   instant rather than a fifteen minute wait. The connector refuses
//!   the same call for itself, so neither side depends on the other.

use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;

use super::{
    run_stream, timeout_for, CallContext, CancelToken, EventSink, ForgePluginSpec, ForgeToken,
    PluginError, ResolvedPlugin, StreamBounds, Target,
};
use crate::host::HostKind;

/// The word list of `--for`. It lives beside the read verbs, because
/// `token` takes the same flag and stays in every build; this module is
/// the one D3.11 compiles out.
pub use super::Access;

/// The sentence of D3.11 for a host that has no person at it. It names
/// the headless door, because refusing without one would leave a CI
/// runner with nothing to do.
pub const NO_PERSON_HERE: &str =
    "joy forge login needs a person at this machine; this process runs under a delegation \
     session. Sign in on the machine that owns the session, or store a token there with \
     joy forge login --token-stdin";

/// What a finished `login` reported (the `result` event of D2.4).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LoginOutcome {
    pub known: bool,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub scopes: Option<String>,
    /// `keychain` or `file`: which store took the token (D2.6).
    #[serde(default)]
    pub stored: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// What `logout` answered.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LogoutOutcome {
    #[serde(default)]
    pub removed: bool,
    #[serde(default)]
    pub revoked: bool,
    /// Where the credential was: `keychain`, `file`, `gh`, `glab` or
    /// `tea`.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub login: Option<String>,
    /// The foreign command that removes a foreign credential, because
    /// joy never removes one itself (D2.6).
    #[serde(default)]
    pub command: Option<String>,
    /// `busy` where another joy process is writing this credential:
    /// `logout` writes, so it takes the refresh lock of D2.6a and never
    /// deletes an entry beside a refresh.
    #[serde(default)]
    pub reason: Option<String>,
    /// The logins this host holds, when there are several of joy's own
    /// and the call named none. Nothing is removed then, and the caller
    /// asks which one.
    #[serde(default)]
    pub logins: Vec<String>,
    /// Why nothing was removed, where the connector wrote a sentence.
    #[serde(default)]
    pub message: Option<String>,
}

/// Sign in to a forge.
///
/// The signature is the one D3.11 requires: a progress sink and a
/// cancel token, both mandatory. Every event the connector flushes
/// reaches `progress` while the connector is still polling, which is
/// what lets the caller show the verification code inside fifteen
/// seconds and cancel the wait afterwards.
///
/// The connector never opens a browser. What the caller does with the
/// `verification` event is the caller's decision: the desktop opens the
/// URL, the CLI prints it, a delegated session never gets this far.
pub fn login(
    spec: &ResolvedPlugin,
    target: &Target,
    access: Access,
    progress: &mut dyn EventSink,
    cancel: &CancelToken,
    ctx: &CallContext,
) -> Result<LoginOutcome, PluginError> {
    if matches!(ctx.host_kind, HostKind::Background | HostKind::Delegated) {
        return Err(PluginError::Failed {
            display: spec.display,
            path: spec.resolved_path.clone(),
            verb: "login".to_string(),
            exit_code: None,
            stderr: NO_PERSON_HERE.to_string(),
        });
    }
    let args = super::call_args(
        spec,
        "login",
        Some(target),
        &["--for", access.as_str()],
        ctx,
    );
    let outcome = run_stream(
        spec,
        &args,
        &ctx.env(),
        progress,
        cancel,
        StreamBounds::for_verb("login"),
        ctx.root(),
    );
    if let Some(error) = outcome.spawn_error {
        return Err(PluginError::Spawn {
            display: spec.display,
            path: spec.resolved_path.clone(),
            error,
        });
    }
    if outcome.timed_out {
        return Err(PluginError::TimedOut {
            display: spec.display,
            path: spec.resolved_path.clone(),
            verb: "login".to_string(),
            timeout: StreamBounds::for_verb("login").total,
            stderr: outcome.stderr_text,
        });
    }
    // The last event that parsed is the `result` or the `error` of the
    // stream (D2.3).
    let last = outcome.stdout_json.ok_or_else(|| PluginError::Unparsable {
        display: spec.display,
        path: spec.resolved_path.clone(),
        verb: "login".to_string(),
        answer: outcome.stdout_text.clone(),
    })?;
    match last.get("event").and_then(|event| event.as_str()) {
        Some("result") => serde_json::from_value(last).map_err(|e| PluginError::Unparsable {
            display: spec.display,
            path: spec.resolved_path.clone(),
            verb: "login".to_string(),
            answer: e.to_string(),
        }),
        _ => Err(PluginError::Failed {
            display: spec.display,
            path: spec.resolved_path.clone(),
            verb: "login".to_string(),
            exit_code: outcome.exit_code,
            stderr: last
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("the sign in did not finish")
                .to_string(),
        }),
    }
}

/// Remove the credential the connector holds for a host, and revoke it
/// at the forge where the forge offers that.
pub fn logout(
    spec: &ForgePluginSpec,
    target: &Target,
    ctx: &CallContext,
) -> Result<LogoutOutcome, PluginError> {
    super::query(spec, "logout", Some(target), &[], ctx)
}

/// Store ONE token the person pasted (`--token-stdin`, D2.4).
///
/// The token goes in on the child's stdin and is never an argument, so
/// no process list can carry it, and it is never logged: the answer
/// names the login, not the secret.
pub fn token_store(
    spec: &ResolvedPlugin,
    target: &Target,
    token: &str,
    ctx: &CallContext,
) -> Result<ForgeToken, PluginError> {
    let args = super::call_args(spec, "token-store", Some(target), &[], ctx);
    let outcome = run_with_stdin(
        spec,
        &args,
        token,
        ctx.root(),
        &ctx.env(),
        timeout_for("token-store"),
    )?;
    serde_json::from_value(outcome).map_err(|e| PluginError::Unparsable {
        display: spec.display,
        path: spec.resolved_path.clone(),
        verb: "token-store".to_string(),
        answer: e.to_string(),
    })
}

/// Run one verb with a secret on its stdin. A cousin of `run_once`, and
/// separate on purpose: every other verb closes stdin so a connector
/// that would ask something fails instead of waiting for ever, and this
/// is the one that must not.
fn run_with_stdin(
    spec: &ResolvedPlugin,
    args: &[String],
    stdin_text: &str,
    root: Option<&Path>,
    env: &[(String, String)],
    timeout: Duration,
) -> Result<serde_json::Value, PluginError> {
    let mut command = joy_process::command(&spec.resolved_path);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = root {
        command.current_dir(root);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().map_err(|e| PluginError::Spawn {
        display: spec.display,
        path: spec.resolved_path.clone(),
        error: e.to_string(),
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        // A write that fails is a connector that closed stdin, which
        // the wait below reports with its own exit code; the secret is
        // not repeated into an error here.
        let _ = stdin.write_all(stdin_text.as_bytes());
        let _ = stdin.write_all(b"\n");
        let _ = stdin.flush();
    }
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PluginError::TimedOut {
                    display: spec.display,
                    path: spec.resolved_path.clone(),
                    verb: "token-store".to_string(),
                    timeout,
                    stderr: String::new(),
                });
            }
            Err(e) => {
                return Err(PluginError::Failed {
                    display: spec.display,
                    path: spec.resolved_path.clone(),
                    verb: "token-store".to_string(),
                    exit_code: None,
                    stderr: e.to_string(),
                })
            }
        }
    }
    let output = child.wait_with_output().map_err(|e| PluginError::Failed {
        display: spec.display,
        path: spec.resolved_path.clone(),
        verb: "token-store".to_string(),
        exit_code: None,
        stderr: e.to_string(),
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        return Err(PluginError::Failed {
            display: spec.display,
            path: spec.resolved_path.clone(),
            verb: "token-store".to_string(),
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    serde_json::from_str(stdout.trim()).map_err(|_| PluginError::Unparsable {
        display: spec.display,
        path: spec.resolved_path.clone(),
        verb: "token-store".to_string(),
        answer: stdout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_access_words_are_the_ones_the_connector_takes() {
        assert_eq!(Access::default().as_str(), "write");
        assert_eq!(Access::Create.as_str(), "create");
        assert_eq!(Access::Release.as_str(), "release");
        assert_eq!(Access::Read.as_str(), "read");
    }

    /// D3.11's refusal is instant and names the headless door.
    #[test]
    fn the_refusal_sentence_names_the_headless_door() {
        assert!(NO_PERSON_HERE.contains("--token-stdin"));
        assert!(NO_PERSON_HERE.contains("delegation session"));
    }
}
