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

    /// Put a fake `gh` on this sandbox's PATH, with a `hosts.yml` that
    /// names these logins, the active one first.
    ///
    /// Spawning a forge CLI by name is what decision 19 asks for, so a
    /// test of that path has to have one to spawn. This one prints a
    /// token per `--user` and nothing else, exactly as
    /// `gh auth token` does.
    fn with_gh(&self, host: &str, logins: &[&str]) {
        let bin = self.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = "#!/bin/sh\n\
             user=\"\"\n\
             while [ $# -gt 0 ]; do\n\
             case \"$1\" in\n\
             --user) user=\"$2\"; shift 2;;\n\
             *) shift;;\n\
             esac\n\
             done\n\
             if [ -z \"$user\" ]; then user=\"$GH_ACTIVE\"; fi\n\
             echo \"gh-token-of-$user\"\n";
        let path = bin.join("gh");
        std::fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = self.path().join("gh-config");
        std::fs::create_dir_all(&config).unwrap();
        let mut hosts = format!("{host}:\n    users:\n");
        for login in logins {
            hosts.push_str(&format!("        {login}:\n            oauth_token: x\n"));
        }
        hosts.push_str(&format!(
            "    user: {}\n    git_protocol: https\n",
            logins.first().copied().unwrap_or_default()
        ));
        std::fs::write(config.join("hosts.yml"), hosts).unwrap();
    }

    /// The connector with the fake `gh` reachable.
    fn connector_with_gh(&self, active: &str) -> std::process::Command {
        let mut command = self.connector();
        command
            .env("PATH", self.path().join("bin"))
            .env("GH_CONFIG_DIR", self.path().join("gh-config"))
            .env("GH_ACTIVE", active);
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

/// Everything `ps` would have shown for a running child, sampled for
/// as long as the case lets it run.
///
/// Sampling once is not enough and stopping at the child's own argv is
/// worse than not sampling at all: that window ENDS before the token
/// has even been written, so a connector that put the secret in an
/// argv afterwards would pass. The watcher runs beside the child from
/// the spawn until the case stops it, which is after the child has
/// read the token and answered.
#[cfg(target_os = "linux")]
struct ProcessWatch {
    stop: Arc<std::sync::atomic::AtomicBool>,
    samples: Arc<std::sync::Mutex<Vec<String>>>,
    thread: std::thread::JoinHandle<()>,
}

#[cfg(target_os = "linux")]
fn watch_the_process_list(pid: u32) -> ProcessWatch {
    use std::sync::atomic::AtomicBool;
    let stop = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(std::sync::Mutex::new(Vec::new()));
    let thread = {
        let stop = stop.clone();
        let samples = samples.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let raw = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                let text = String::from_utf8_lossy(&raw).replace('\0', " ");
                if !text.trim().is_empty() {
                    samples.lock().expect("the samples").push(text);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })
    };
    ProcessWatch {
        stop,
        samples,
        thread,
    }
}

#[cfg(target_os = "linux")]
impl ProcessWatch {
    fn seen(&self) -> Vec<String> {
        self.samples.lock().expect("the samples").clone()
    }

    /// Wait until the child carries its OWN argv: between the fork and
    /// the exec it still carries the parent's, so a case that asserted
    /// on the first sample would be asserting about this test binary.
    fn until_the_child_has_its_own(&self, needle: &str) -> usize {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let seen = self.seen();
            if seen.iter().any(|line| line.contains(needle)) {
                return seen.len();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the child's own argument list never appeared: {seen:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn stop(self) -> Vec<String> {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.thread.join();
        self.samples.lock().expect("the samples").clone()
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
            // The forge answers slowly on purpose: the window in which
            // the connector really HOLDS the pasted token is the window
            // this case has to sample, and an instant answer would
            // close it before one sample fell into it.
            std::thread::sleep(std::time::Duration::from_millis(500));
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
    // `/proc/<pid>/cmdline` is sampled from the spawn until the child
    // has read the token and answered, and every sample is asserted
    // over.
    let watch = watch_the_process_list(child.id());
    let before_the_token = watch.until_the_child_has_its_own("token-store");
    {
        let mut stdin = child.stdin.take().expect("the child's stdin");
        stdin.write_all(b"ghp_the_pasted_secret\n").unwrap();
    }
    let output = child.wait_with_output().expect("the connector's end");
    let cmdlines = watch.stop();
    assert!(
        cmdlines.len() > before_the_token,
        "the process list was not read once while the connector held the token: {cmdlines:?}"
    );
    for cmdline in &cmdlines {
        assert!(
            !cmdline.contains("ghp_"),
            "the process list carried a token: {cmdline}"
        );
    }
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

/// J3's acceptance, through the shipped binary: `login` prints the
/// verification line within fifteen seconds and the CALLER SEES IT
/// BEFORE THE PROCESS EXITS. The line is read off the child's stdout
/// while it is still polling the forge.
#[test]
fn a_login_prints_its_verification_line_while_it_is_still_running() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/login/device/code" => Reply::json(
            200,
            r#"{"device_code":"dev-1","user_code":"WDJB-MJHT",
                "verification_uri":"https://forge.test/login/device",
                "expires_in":900,"interval":5}"#,
        ),
        // Nobody ever finishes the sign in: the point is the FIRST
        // line, and the child is ended once it has been read.
        "/login/oauth/access_token" => Reply::json(200, r#"{"error":"authorization_pending"}"#),
        _ => Reply::not_found(),
    });
    let sandbox = Sandbox::new("forge.test", "github", &format!("{}/api/v3", fake.base()));
    std::fs::write(
        sandbox.path().join("config/joy/forges.yaml"),
        format!(
            "- host: forge.test\n  kind: github\n  api_base: {base}/api/v3\n  \
             client_id: test-client\n  device_endpoint: {base}/login/device/code\n  \
             token_endpoint: {base}/login/oauth/access_token\n",
            base = fake.base()
        ),
    )
    .unwrap();
    let mut child = sandbox
        .connector()
        .args([
            "github",
            "login",
            "--host",
            "forge.test",
            "--host-kind",
            "interactive",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the connector");
    let stdout = child.stdout.take().expect("the child's stdout");
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let mut line = String::new();
        if std::io::BufReader::new(stdout).read_line(&mut line).is_ok() {
            let _ = tx.send(line);
        }
    });
    let line = rx
        .recv_timeout(std::time::Duration::from_secs(15))
        .expect("the verification line inside fifteen seconds");
    // The child is still polling: it has not exited, and this line
    // reached the caller anyway.
    assert!(
        matches!(child.try_wait(), Ok(None)),
        "the caller sees the line BEFORE the process exits"
    );
    let event: Value = serde_json::from_str(line.trim()).expect("one JSON object per line");
    assert_eq!(event["event"], "verification");
    assert_eq!(event["code"], "WDJB-MJHT");
    assert_eq!(event["url"], "https://forge.test/login/device");
    assert_eq!(event["host"], "forge.test");
    assert_eq!(event["interval"], 5);
    let _ = child.kill();
    let _ = child.wait();
}

/// J3's acceptance: on a machine with a signed in gh, `token` answers
/// `"source":"gh"` with zero clicks and zero dialogs. There is no
/// credential of joy's own here, and no credential store is reachable
/// either, so the only way to the token is spawning gh (decision 19).
#[test]
fn a_signed_in_gh_answers_the_token_verb_by_being_spawned() {
    let fake = FakeForge::start(|_| Reply::not_found());
    let sandbox = Sandbox::new("forge.test", "github", &format!("{}/api/v3", fake.base()));
    sandbox.with_gh("forge.test", &["scotty"]);
    let output = sandbox
        .connector_with_gh("scotty")
        .args(["github", "token", "--host", "forge.test"])
        .output()
        .expect("the connector");
    let answer = answer_of(&output);
    assert_eq!(answer["known"], true, "{answer}");
    assert_eq!(answer["source"], "gh");
    assert_eq!(answer["token"], "gh-token-of-scotty");
    assert!(
        fake.calls().is_empty(),
        "reading gh's token costs no forge request"
    );
    // joy never writes, refreshes or revokes what gh owns: `logout`
    // names gh's own command and removes nothing (D2.6).
    let out = sandbox
        .connector_with_gh("scotty")
        .args(["github", "logout", "--host", "forge.test"])
        .output()
        .expect("the connector");
    let answer = answer_of(&out);
    assert_eq!(answer["removed"], false);
    assert_eq!(answer["source"], "gh");
    assert_eq!(answer["command"], "gh auth logout --hostname forge.test");
}

/// D1.10, mechanism 5: "every plugin call carries the host kind as a
/// protocol field, and the plugin uses it to skip any step that can
/// raise an operating system dialog".
///
/// A DELEGATED session is not this machine's person. D3.11 already
/// refuses `login`, `logout` and the token paste there, its credential
/// travels in the variable the caller named, and the person's own
/// credential store is none of its business: it is never opened at all.
/// A BACKGROUND host is this person's own machine and keeps its entry,
/// because the desktop's sync poll is a background host and needs it.
#[test]
fn a_delegated_session_never_opens_this_persons_credential_store() {
    let fake = FakeForge::start(|_| Reply::not_found());
    let sandbox = Sandbox::new("forge.test", "github", &format!("{}/api/v3", fake.base()));
    sandbox.seed(
        "forge.test",
        "scotty",
        serde_json::json!({ "token": "gho_of_this_person", "login": "scotty" }),
    );

    let delegated = sandbox
        .connector()
        .args([
            "github",
            "token",
            "--host",
            "forge.test",
            "--host-kind",
            "delegated",
        ])
        .output()
        .expect("the connector");
    let answer = answer_of(&delegated);
    assert_eq!(answer["known"], false, "{answer}");
    assert_eq!(answer["reason"], "no-keychain");
    assert!(
        !String::from_utf8_lossy(&delegated.stdout).contains("gho_of_this_person"),
        "a delegated session never reads this person's entry"
    );

    for kind in ["background", "interactive"] {
        let output = sandbox
            .connector()
            .args([
                "github",
                "token",
                "--host",
                "forge.test",
                "--host-kind",
                kind,
            ])
            .output()
            .expect("the connector");
        let answer = answer_of(&output);
        assert_eq!(answer["token"], "gho_of_this_person", "{kind}: {answer}");
        assert_eq!(answer["source"], "file");
    }
}

/// D4.1c, step 4, through the shipped binary: `--for` is the direction
/// of the probe, and "the first that answers 200, and for a push
/// direction reports write, wins".
#[test]
fn the_for_flag_decides_which_login_the_probe_accepts() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/api/v3/repos/acme/widgets" => match call.authorization() {
            // The ACTIVE account reads the repository and may not push.
            Some("Bearer gh-token-of-scotty") => {
                Reply::json(200, r#"{"permissions":{"push":false}}"#)
            }
            Some("Bearer gh-token-of-work") => Reply::json(200, r#"{"permissions":{"push":true}}"#),
            _ => Reply::not_found(),
        },
        _ => Reply::not_found(),
    });
    let sandbox = Sandbox::new("forge.test", "github", &format!("{}/api/v3", fake.base()));
    sandbox.with_gh("forge.test", &["scotty", "work"]);
    let ask = |purpose: &str| {
        let output = sandbox
            .connector_with_gh("scotty")
            .args([
                "github",
                "token",
                "--remote",
                "https://forge.test/acme/widgets.git",
                "--for",
                purpose,
            ])
            .output()
            .expect("the connector");
        answer_of(&output)
    };
    let reading = ask("read");
    assert_eq!(reading["login"], "scotty", "{reading}");
    assert_eq!(reading["chose_by"], "probe");
    // The memory now names scotty, and a push must not take it: the
    // probe for a push direction asks again and finds the login that
    // can push.
    let pushing = ask("write");
    assert_eq!(pushing["login"], "work", "{pushing}");
    assert_eq!(pushing["token"], "gh-token-of-work");

    // A word that is not one of the four is a usage error, not a guess.
    let wrong = sandbox
        .connector()
        .args([
            "github",
            "token",
            "--host",
            "forge.test",
            "--for",
            "everything",
        ])
        .output()
        .expect("the connector");
    assert_eq!(wrong.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("read, write, create or release"));
}

/// J3's acceptance: a host with TWO gh accounts answers `token` for a
/// repository only the second account can reach, and reports
/// `"chose_by":"probe"`.
///
/// This is the trap gh documents itself: "Without the --user flag, the
/// active account for the host is chosen", and the active account here
/// is the one that cannot reach the repository.
#[test]
fn a_host_with_two_gh_accounts_probes_for_the_one_that_reaches_the_repository() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/api/v3/repos/acme/widgets" => {
            if call.authorization() == Some("Bearer gh-token-of-work") {
                Reply::json(200, r#"{"permissions":{"push":true}}"#)
            } else {
                // GitHub answers 404, not 403, for a private repository
                // the caller may not see.
                Reply::not_found()
            }
        }
        _ => Reply::not_found(),
    });
    let sandbox = Sandbox::new("forge.test", "github", &format!("{}/api/v3", fake.base()));
    // scotty is the ACTIVE account and cannot reach it; work can.
    sandbox.with_gh("forge.test", &["scotty", "work"]);
    let output = sandbox
        .connector_with_gh("scotty")
        .args([
            "github",
            "token",
            "--remote",
            "https://forge.test/acme/widgets.git",
        ])
        .output()
        .expect("the connector");
    let answer = answer_of(&output);
    assert_eq!(answer["known"], true, "{answer}");
    assert_eq!(answer["login"], "work");
    assert_eq!(answer["token"], "gh-token-of-work");
    assert_eq!(answer["source"], "gh");
    assert_eq!(answer["chose_by"], "probe");
    assert_eq!(
        fake.calls().len(),
        2,
        "one request per candidate, never per contact"
    );

    // The winner is remembered per remote, so the second call spends
    // nothing (D4.1c).
    let again = sandbox
        .connector_with_gh("scotty")
        .args([
            "github",
            "token",
            "--remote",
            "https://forge.test/acme/widgets.git",
        ])
        .output()
        .expect("the connector");
    let again = answer_of(&again);
    assert_eq!(again["login"], "work");
    assert_eq!(again["chose_by"], "memory");
    assert_eq!(fake.calls().len(), 2, "the memory spends no request");
}
