// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The sign in verbs, driven as PROCESSES (JOY-029B-B0, package J3).
//!
//! The rest of J3's proof runs inside the connector crate; two things
//! cannot be proved there and are proved here, with the shipped
//! `joy-forge` binary:
//!
//! - **two processes refreshing one entry at once** (D2.6a). Two
//!   threads contend on the same flock, but "two joy processes" is what
//!   the acceptance says, so two joy processes is what runs.
//! - **`ps` during a `token-store` shows no token** (D2.4, D5). The
//!   token goes in on stdin; here the child's own `/proc/<pid>/cmdline`
//!   is read while it runs, which is exactly what `ps` reads.
//!
//! No real network and no forge CLI: PATH is emptied for every child,
//! the forge is an in process fake on the loopback interface, and the
//! credential store is the 0600 file of D2.6, because every child gets
//! a `DBUS_SESSION_BUS_ADDRESS` that answers nothing. That is not a
//! trick: it is the machine D2.6 wrote the fallback for, a host whose
//! credential store cannot be reached.

#![cfg(unix)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use joy_forge_net::fake::{FakeForge, Reply};
use serde_json::Value;

/// The connector as cargo built it for this test run.
const CONNECTOR: &str = env!("CARGO_BIN_EXE_joy-forge");

/// One sandbox per case: its own config directory (the credential file
/// of D2.6 lives there), its own state directory (the refresh locks of
/// D2.6a and the login memory of D4.1c live there) and its own
/// `forges.yaml`.
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new(host: &str, kind: &str, api_base: &str) -> Sandbox {
        let dir = tempfile::tempdir().expect("a sandbox");
        std::fs::create_dir_all(dir.path().join("config/joy")).unwrap();
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        std::fs::write(
            dir.path().join("config/joy/forges.yaml"),
            format!("- host: {host}\n  kind: {kind}\n  api_base: {api_base}\n"),
        )
        .unwrap();
        Sandbox { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn tokens_file(&self) -> PathBuf {
        self.dir.path().join("config/joy/forge-tokens.json")
    }

    /// The connector, with nothing of the developer's machine in reach.
    fn connector(&self) -> std::process::Command {
        // joy_process::command, like every other spawn in joy: a GUI
        // host on Windows must open no console window.
        let mut command = joy_process::command(CONNECTOR);
        command
            .env_clear()
            .env("PATH", "")
            .env("HOME", self.path())
            .env("XDG_CONFIG_HOME", self.path().join("config"))
            .env("XDG_STATE_HOME", self.path().join("state"))
            // No credential store answers here, which is the case the
            // 0600 file of D2.6 exists for.
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/joy-test-no-bus",
            )
            .env("XDG_RUNTIME_DIR", self.path().join("run"));
        command
    }

    /// Put one credential in the file store, the way a finished `login`
    /// would have.
    fn seed(&self, host: &str, login: &str, record: Value) {
        let text = serde_json::json!({ "hosts": { host: { login: record } } });
        std::fs::write(
            self.tokens_file(),
            serde_json::to_string_pretty(&text).unwrap(),
        )
        .unwrap();
    }
}

/// What `ps` would show for a running child: its argument list.
#[cfg(target_os = "linux")]
fn process_list_of(pid: u32) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let text = String::from_utf8_lossy(&raw).replace('\0', " ");
        if !text.trim().is_empty() || std::time::Instant::now() >= deadline {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn answer_of(output: &std::process::Output) -> Value {
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(text.trim()).unwrap_or_else(|e| {
        panic!(
            "the connector answered no JSON ({e}): {text}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// D2.6a, the acceptance verbatim: two joy processes refreshing the
/// same entry at once produce ONE refresh and one `busy`, and the token
/// still works afterwards.
///
/// The fake holds the first refresh past the ten second bound of D2.6a
/// on purpose: that bound is what turns the second process from "a
/// second refresh" into "an answer that says busy", and a refresh that
/// answered instantly would never reach it.
#[test]
fn two_processes_refreshing_one_entry_produce_one_refresh_and_one_busy() {
    let refreshes = Arc::new(AtomicUsize::new(0));
    let counter = refreshes.clone();
    let fake = FakeForge::start(move |call| {
        if call.path == "/login/oauth/access_token" {
            let round = counter.fetch_add(1, Ordering::SeqCst);
            // Only the first caller is slow; a second one would answer
            // at once, and the test would not notice it had happened.
            if round == 0 {
                std::thread::sleep(std::time::Duration::from_millis(11_500));
            }
            return Reply::json(
                200,
                format!(
                    r#"{{"access_token":"fresh-{round}","refresh_token":"rt-{round}",
                        "expires_in":3600,"scope":"repo,user:email"}}"#
                ),
            );
        }
        Reply::not_found()
    });
    let sandbox = Sandbox::new("forge.test", "github", &format!("{}/api/v3", fake.base()));
    sandbox.seed(
        "forge.test",
        "scotty",
        serde_json::json!({
            "token": "stale",
            "login": "scotty",
            "scopes": "repo user:email",
            "expires_at": (chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339(),
            "refresh_token": "rt-old",
            "token_endpoint": format!("{}/login/oauth/access_token", fake.base()),
            "client_id": "test-client",
        }),
    );

    let first = sandbox
        .connector()
        .args(["github", "token", "--host", "forge.test"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the first connector");
    // Long enough that the first process is inside the lock and the
    // slow refresh, short enough that the second still waits its own
    // ten seconds against it.
    std::thread::sleep(std::time::Duration::from_millis(700));
    let second = sandbox
        .connector()
        .args(["github", "token", "--host", "forge.test"])
        .output()
        .expect("the second connector");
    let first = first.wait_with_output().expect("the first connector's end");

    let winner = answer_of(&first);
    let loser = answer_of(&second);
    assert_eq!(winner["known"], true, "the holder refreshed: {winner}");
    assert_eq!(winner["token"], "fresh-0");
    assert_eq!(winner["login"], "scotty");
    assert_eq!(
        loser["known"], false,
        "the waiter must not refresh: {loser}"
    );
    assert_eq!(loser["reason"], "busy");
    assert_eq!(
        refreshes.load(Ordering::SeqCst),
        1,
        "exactly one refresh reached the forge"
    );

    // And the token still works afterwards: a third call reads the
    // stored one and refreshes nothing.
    let third = sandbox
        .connector()
        .args(["github", "token", "--host", "forge.test"])
        .output()
        .expect("the third connector");
    let after = answer_of(&third);
    assert_eq!(after["token"], "fresh-0");
    assert_eq!(after["source"], "file");
    assert_eq!(after["chose_by"], "only");
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
}

/// D2.4 and D5: `token-store` reads the token from STDIN, and `ps`
/// during the run shows no token. `/proc/<pid>/cmdline` is what `ps`
/// reads, so that is what this asserts, on the running child.
#[test]
#[cfg(target_os = "linux")]
fn the_token_of_a_token_store_is_never_in_the_process_list() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/api/v3/user" => {
            if call.authorization() == Some("Bearer ghp_the_pasted_secret") {
                Reply::json(200, r#"{"login":"scotty","id":7}"#)
                    .with_header("X-OAuth-Scopes", "repo, user:email")
            } else {
                Reply::json(401, r#"{"message":"Bad credentials"}"#)
            }
        }
        "/api/v3/user/emails" => {
            Reply::json(200, r#"[{"email":"s@example.test","verified":true}]"#)
        }
        _ => Reply::not_found(),
    });
    let sandbox = Sandbox::new("forge.test", "github", &format!("{}/api/v3", fake.base()));
    let mut child = sandbox
        .connector()
        .args(["github", "token-store", "--host", "forge.test"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the connector");
    // `/proc/<pid>/cmdline` is empty for the moment between the fork
    // and the exec, so it is read until the child has its own.
    let cmdline = process_list_of(child.id());
    assert!(
        !cmdline.contains("ghp_"),
        "the process list carried a token: {cmdline}"
    );
    assert!(cmdline.contains("token-store"), "{cmdline}");
    {
        let mut stdin = child.stdin.take().expect("the child's stdin");
        stdin.write_all(b"ghp_the_pasted_secret\n").unwrap();
    }
    let output = child.wait_with_output().expect("the connector's end");
    let answer = answer_of(&output);
    assert_eq!(answer["known"], true, "{answer}");
    assert_eq!(answer["login"], "scotty");
    assert_eq!(answer["source"], "file", "the 0600 file of D2.6");
    assert_eq!(answer["scopes"], "repo user:email");

    // It is stored, so `token` finds it without asking the forge again.
    let before = fake.calls().len();
    let read = sandbox
        .connector()
        .args(["github", "token", "--host", "forge.test"])
        .output()
        .expect("the connector");
    let read = answer_of(&read);
    assert_eq!(read["token"], "ghp_the_pasted_secret");
    assert_eq!(read["username"], "x-access-token");
    assert_eq!(
        fake.calls().len(),
        before,
        "a stored token costs no request"
    );

    // The file joy wrote is readable by its owner alone (D2.6).
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(sandbox.tokens_file())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

/// D2.4: `web-url` is answered from the address and the instance
/// configuration alone, with no credential and no request.
#[test]
fn web_url_answers_without_a_credential_and_without_a_request() {
    let fake = FakeForge::start(|_| Reply::not_found());
    let sandbox = Sandbox::new("git.acme.test", "gitea", &format!("{}/api/v1", fake.base()));
    std::fs::write(
        sandbox.path().join("config/joy/forges.yaml"),
        format!(
            "- host: git.acme.test\n  kind: gitea\n  api_base: {}/api/v1\n  web_base: https://git.acme.test/code\n",
            fake.base()
        ),
    )
    .unwrap();
    let output = sandbox
        .connector()
        .args([
            "gitea",
            "web-url",
            "--remote",
            "git@git.acme.test:team/sub/repo.git",
        ])
        .output()
        .expect("the connector");
    let answer = answer_of(&output);
    assert_eq!(answer["known"], true);
    assert_eq!(
        answer["https_url"],
        "https://git.acme.test/code/team/sub/repo.git"
    );
    assert!(fake.calls().is_empty());
}

/// D3.11: a delegated session is refused instantly, and the refusal
/// names the headless door. The connector refuses this for itself, so
/// an agent image that carries the binary cannot start a browser flow
/// nobody can finish.
#[test]
fn a_delegated_login_is_refused_by_the_connector_itself() {
    let fake = FakeForge::start(|_| Reply::json(500, "{}"));
    let sandbox = Sandbox::new("forge.test", "github", &format!("{}/api/v3", fake.base()));
    let started = std::time::Instant::now();
    let output = sandbox
        .connector()
        .args([
            "github",
            "login",
            "--host",
            "forge.test",
            "--host-kind",
            "delegated",
        ])
        .output()
        .expect("the connector");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    let answer = answer_of(&output);
    assert_eq!(answer["event"], "error");
    assert_eq!(answer["code"], "unsupported");
    let message = answer["message"].as_str().unwrap();
    assert!(message.contains("--token-stdin"), "{message}");
    assert!(fake.calls().is_empty(), "nothing was contacted");
}
