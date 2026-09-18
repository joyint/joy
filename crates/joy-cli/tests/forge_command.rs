// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! `joy forge`, driven as the person and the agent drive it
//! (JOY-029D-3E, package J10 of the forge connection NG design: D3.10
//! and D3.11).
//!
//! Every case runs the shipped `joy` binary as a process against a FAKE
//! connector: a shell script that answers the protocol 2 verbs, plus a
//! second one that answers like a binary from before the handshake
//! existed. No forge is contacted, no forge CLI is on the PATH (it is
//! emptied for every child), and every child gets a HOME and an XDG
//! configuration directory of its own, so nothing on the developer's
//! machine can answer for it and nothing it writes leaves the sandbox.
//!
//! Unix only: the stubs are shell scripts and the terminal cases need
//! `openpty`. What they prove is not a unix rule; the host decision
//! itself is `joy_core::host::HostKind::detect`, which joy-core tests on
//! every platform.

#![cfg(unix)]

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;

mod process_list;

// The two cases that read the process list are unix gated (see the
// module); on Windows the import would be the one unused name.
#[cfg(unix)]
use process_list::argv_of;

/// A connector that speaks protocol 2 and answers every verb this
/// package's cases need. It records its own argv, so a case can prove
/// what was NOT run as well as what was.
const CONNECTOR: &str = r#"#!/bin/sh
if [ -n "$JOY_STUB_ARGV" ]; then
  echo "$@" >> "$JOY_STUB_ARGV"
fi
if [ "$1" = "version" ]; then
  echo '{"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}'
  exit 0
fi
forge="$1"
shift
verb="$1"
shift
host=""
host_kind=""
while [ $# -gt 0 ]; do
  case "$1" in
    --host) host="$2"; shift 2 ;;
    --host-kind) host_kind="$2"; shift 2 ;;
    *) shift ;;
  esac
done
source="${JOY_STUB_SOURCE:-keychain}"
case "$verb" in
  claims)
    # JOY_STUB_CLAIMS=0 is the machine on which NO connector knows the
    # host: `forges.yaml` has no entry for it and no builtin matches.
    if [ "$forge" = "github" ] && [ "${JOY_STUB_CLAIMS:-1}" = "1" ]; then
      echo '{"claims":true}'
    else
      echo '{"claims":false}'
    fi
    ;;
  token)
    # A connector that answers and still has something to say says it
    # on stderr, on a ZERO exit (JOY-02A8-F4).
    if [ -n "$JOY_STUB_NOTE" ]; then echo "$JOY_STUB_NOTE" >&2; fi
    if [ "${JOY_STUB_SIGNED_IN:-1}" = "1" ]; then
      printf '{"known":true,"host":"%s","login":"scotty","token":"x","username":"x-access-token","source":"%s","scopes":"repo user:email","expires_at":null,"chose_by":"only"}\n' "$host" "$source"
    else
      echo '{"known":false,"reason":"no-login"}'
    fi
    ;;
  login)
    printf '{"event":"verification","host":"%s","url":"https://github.test/login/device","url_complete":null,"code":"WDJB-MJHT","expires_in":900,"interval":5}\n' "$host"
    echo '{"event":"result","known":true,"login":"scotty","user_id":"12345","emails":["s@example.test"],"scopes":"repo user:email","stored":"keychain","source":"device"}'
    ;;
  token-store)
    read -r token
    # A case can ask this connector to HOLD the token for a moment, so
    # that the window in which joy has it can really be sampled. The
    # PATH of every child here is EMPTY on purpose, so `sleep` is
    # called by its path and a machine without one busy waits instead.
    if [ -n "$JOY_STUB_SLEEP" ]; then
      if [ -x /bin/sleep ]; then /bin/sleep "$JOY_STUB_SLEEP"
      elif [ -x /usr/bin/sleep ]; then /usr/bin/sleep "$JOY_STUB_SLEEP"
      else i=0; while [ $i -lt 300000 ]; do i=$((i+1)); done
      fi
    fi
    case "$token" in
      ghp_*)
        printf '{"known":true,"host":"%s","login":"scotty","source":"file","scopes":"repo"}\n' "$host"
        ;;
      *)
        echo '{"known":false,"reason":"no-login","message":"the forge did not accept this token"}'
        ;;
    esac
    ;;
  logout)
    # What joy-forge-net answers a delegated call (auth/verbs.rs,
    # NO_LOGOUT_HERE): a delegated session reads the person's vault and
    # never writes it, so nothing is removed, nothing is revoked, and
    # the sentence says why.
    if [ "$host_kind" = "delegated" ]; then
      echo '{"removed":false,"revoked":false,"source":null,"reason":"unsupported","message":"this process runs under a delegation session, which may use the credential this machine holds and may never sign it out. Sign out on the machine that owns the session with joy forge logout"}'
    else
      printf '{"removed":true,"revoked":true,"source":"%s","login":"scotty"}\n' "$source"
    fi
    ;;
  *)
    echo "error: unrecognized subcommand '$verb'" >&2
    exit 2
    ;;
esac
"#;

/// A connector from before the handshake existed: its parser rejects
/// `version` and exits 2 with nothing on stdout, which is the detector
/// of D2.2a. It still answers the six legacy verbs, `release` among
/// them.
const LEGACY_CONNECTOR: &str = r#"#!/bin/sh
case "$1" in
  claims) echo '{"claims":true}' ;;
  identity) echo '{"known":true,"login":"legacy"}' ;;
  resolve) echo '{"known":false}' ;;
  store) echo '{"state":"gone"}' ;;
  files) echo '{"state":"unknown"}' ;;
  release) echo '{"url":"https://github.test/o/r/releases/tag/v1"}' ;;
  *)
    echo "error: unrecognized subcommand '$1'" >&2
    echo "Usage: joy-github <COMMAND>" >&2
    exit 2
    ;;
esac
"#;

/// One machine per case: its own HOME, its own configuration and state
/// directories, and its own connector directory.
struct Machine {
    dir: tempfile::TempDir,
}

impl Machine {
    fn new() -> Machine {
        let dir = tempfile::tempdir().expect("a sandbox");
        for sub in ["home", "config/joy", "state", "plugins"] {
            std::fs::create_dir_all(dir.path().join(sub)).expect("the sandbox directories");
        }
        Machine { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn plugins(&self) -> PathBuf {
        self.dir.path().join("plugins")
    }

    /// Put one connector script in this machine's plugin directory.
    fn connector(&self, name: &str, body: &str) -> PathBuf {
        let path = self.plugins().join(name);
        std::fs::write(&path, body).expect("write the connector");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make the connector executable");
        path
    }

    /// An operator's instance list (D2.5), which is one of the three
    /// sources of `joy forge status`'s host set.
    fn forges_yaml(&self, host: &str, kind: &str) {
        std::fs::write(
            self.path().join("config/joy/forges.yaml"),
            format!("- host: {host}\n  kind: {kind}\n"),
        )
        .expect("write forges.yaml");
    }

    /// `joy`, with nothing of the developer's machine in reach: no
    /// PATH, no forge CLI, no session, and the connector of this
    /// machine as the only one findable.
    fn joy(&self, args: &[&str]) -> std::process::Command {
        let mut command = joy_process::command(env!("CARGO_BIN_EXE_joy"));
        command
            .args(args)
            .current_dir(self.path())
            .env_clear()
            .env("PATH", "")
            .env("HOME", self.path().join("home"))
            .env("XDG_CONFIG_HOME", self.path().join("config"))
            .env("XDG_STATE_HOME", self.path().join("state"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            // The documented test hook of D2.2, and the reason it
            // exists: a case has to be able to say which binary
            // answers.
            .env("JOY_PLUGIN_DIR", self.plugins());
        command
    }

    fn run(&self, args: &[&str]) -> Answer {
        let output = self.joy(args).output().expect("joy runs");
        Answer::of(output)
    }
}

/// What a `joy` run said and how it ended.
struct Answer {
    ok: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Answer {
    fn of(output: std::process::Output) -> Answer {
        Answer {
            ok: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// The one envelope a `--json` run prints on stdout, unwrapped.
    fn data(&self) -> Value {
        let envelope: Value = serde_json::from_str(self.stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "stdout is not one JSON envelope ({e}): {}\nstderr: {}",
                self.stdout, self.stderr
            )
        });
        assert_eq!(envelope["version"], 1, "{}", self.stdout);
        envelope["data"].clone()
    }
}

// ---------------------------------------------------------------------
// A terminal, for the cases that need a person at one
// ---------------------------------------------------------------------

/// A pty pair: what the case holds, and what the child gets as its
/// three standard streams. Without one the host kind is `Background`
/// whatever else is true, and the interactive half of D3.11 is never
/// entered.
struct Terminal {
    person: std::fs::File,
    child: OwnedFd,
}

impl Terminal {
    fn open() -> Terminal {
        let mut controller = 0;
        let mut follower = 0;
        // SAFETY: openpty writes two valid descriptors or returns -1,
        // and both are taken over by owning types below.
        let rc = unsafe {
            libc::openpty(
                &mut controller,
                &mut follower,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty");
        // SAFETY: both descriptors come from openpty and are owned here.
        unsafe {
            Terminal {
                person: std::fs::File::from_raw_fd(controller),
                child: OwnedFd::from_raw_fd(follower),
            }
        }
    }

    fn stdio(&self) -> [Stdio; 3] {
        [
            Stdio::from(self.child.try_clone().unwrap()),
            Stdio::from(self.child.try_clone().unwrap()),
            Stdio::from(self.child.try_clone().unwrap()),
        ]
    }
}

/// Run `joy` on a terminal and return everything the person would have
/// seen, plus whether it succeeded.
fn on_a_terminal(machine: &Machine, args: &[&str], typed: &str) -> (bool, String) {
    let terminal = Terminal::open();
    let [stdin, stdout, stderr] = terminal.stdio();
    let mut command = machine.joy(args);
    command.stdin(stdin).stdout(stdout).stderr(stderr);
    let mut child = command.spawn().expect("joy runs");
    // Every copy on this side has to go, or the read below never sees
    // the end of the child's output.
    drop(command);
    drop(terminal.child);

    let mut person = terminal.person;
    person.write_all(typed.as_bytes()).unwrap();
    person.flush().unwrap();

    let mut reader = person.try_clone().unwrap();
    let drain = std::thread::spawn(move || {
        let mut seen = Vec::new();
        let mut buffer = [0u8; 4096];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            seen.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8_lossy(&seen).to_string()
    });

    let ok = wait_with_a_bound(&mut child);
    drop(person);
    let seen = drain.join().unwrap_or_default();
    (ok, seen)
}

fn wait_with_a_bound(child: &mut std::process::Child) -> bool {
    for _ in 0..600 {
        match child.try_wait().unwrap() {
            Some(status) => return status.success(),
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    let _ = child.kill();
    panic!("joy did not finish: it is waiting for an answer nobody scripted");
}

// ---------------------------------------------------------------------
// login (D3.10)
// ---------------------------------------------------------------------

/// The acceptance of J10, first sentence: `joy forge login --host
/// github.com` on a machine with no gh prints a URL and a code and ends
/// with a stored credential.
///
/// It runs on a terminal, because that is what makes this host
/// `Interactive`; with a pipe the host is `Background` and D3.11's
/// refusal would answer instead, which is the case below.
#[test]
fn login_on_a_terminal_prints_the_url_and_the_code_and_signs_in() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let (ok, seen) = on_a_terminal(&machine, &["forge", "login", "--host", "github.test"], "");

    assert!(ok, "{seen}");
    assert!(seen.contains("https://github.test/login/device"), "{seen}");
    assert!(seen.contains("WDJB-MJHT"), "{seen}");
    assert!(
        seen.contains("Signed in to github.test as scotty"),
        "{seen}"
    );
    assert!(
        seen.contains("15 minutes"),
        "the countdown is shown: {seen}"
    );
}

/// The acceptance of J10, last sentence: `JOY_SESSION=... joy forge
/// login --host github.com` refuses IMMEDIATELY with the delegation
/// sentence. "Immediately" is proved by the connector's own argv log:
/// no `login` ever reached it.
#[test]
fn login_under_a_delegation_session_refuses_by_name_and_spawns_no_login() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);
    let session = a_live_session(&machine);
    let argv = machine.path().join("argv.log");

    let output = machine
        .joy(&["forge", "login", "--host", "github.test", "--json"])
        .env("JOY_SESSION", &session)
        .env("JOY_STUB_ARGV", &argv)
        .output()
        .expect("joy runs");
    let answer = Answer::of(output);

    assert_eq!(answer.code, Some(1), "{}", answer.stderr);
    let data = answer.data();
    assert_eq!(data["host"], "github.test");
    assert_eq!(data["state"], "needs_sign_in");
    let message = data["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("needs a person at this machine")
            && message.contains("delegation session")
            && message.contains("--token-stdin"),
        "{message}"
    );
    let log = std::fs::read_to_string(&argv).unwrap_or_default();
    assert!(
        !log.contains(" login "),
        "the refusal happened before any sign in was started: {log}"
    );
}

/// The acceptance of J10, second sentence: `joy forge login
/// --token-stdin --host codeberg.org < token` stores a validated token,
/// and `ps` during the run shows no token.
///
/// The token path is NOT what D3.11 refuses: it is the headless door of
/// D2.4, written for exactly the machines that have no person at them,
/// and this case runs with pipes, which is a `Background` host.
#[test]
#[cfg(unix)]
fn a_token_from_stdin_is_stored_and_never_in_the_process_list() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let mut child = machine
        .joy(&[
            "forge",
            "login",
            "--host",
            "codeberg.test",
            "--token-stdin",
            "--json",
        ])
        // The connector holds the token for a second, so the window in
        // which joy really has the secret is long enough to sample.
        .env("JOY_STUB_SLEEP", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("joy runs");
    let watch = watch_the_process_list(child.id());
    let before_the_token = watch.until_the_child_has_its_own("--token-stdin");
    {
        let mut stdin = child.stdin.take().expect("the child's stdin");
        stdin.write_all(b"ghp_the_pasted_secret\n").unwrap();
    }
    let answer = Answer::of(child.wait_with_output().expect("joy ends"));
    let cmdlines = watch.stop();
    assert!(
        cmdlines.len() > before_the_token,
        "the process list was not read once while joy held the token: {cmdlines:?}"
    );
    for cmdline in &cmdlines {
        assert!(
            !cmdline.contains("ghp_"),
            "a process list carried the token: {cmdline}"
        );
    }

    assert!(answer.ok, "{}", answer.stderr);
    let data = answer.data();
    assert_eq!(data["state"], "signed-in");
    assert_eq!(data["host"], "codeberg.test");
    assert_eq!(data["login"], "scotty");
    assert_eq!(data["source"], "token");
    assert_eq!(data["stored"], "file");
    assert!(
        !answer.stdout.contains("ghp_") && !answer.stderr.contains("ghp_"),
        "the token was printed: {}{}",
        answer.stdout,
        answer.stderr
    );
}

/// A host with no terminal is refused too (D3.11), and the sentence is
/// TRUE for it: a hook or a piped run is not a delegation session, and
/// telling it that it is sends a person looking for an agent that does
/// not exist. Both sentences name the headless door.
#[test]
fn login_without_a_terminal_refuses_without_inventing_a_session() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);
    let argv = machine.path().join("argv.log");

    let output = machine
        .joy(&["forge", "login", "--host", "github.test", "--json"])
        .env("JOY_STUB_ARGV", &argv)
        .output()
        .expect("joy runs");
    let answer = Answer::of(output);

    assert_eq!(answer.code, Some(1), "{}", answer.stderr);
    let message = answer.data()["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(message.contains("no terminal to ask at"), "{message}");
    assert!(!message.contains("delegation session"), "{message}");
    assert!(message.contains("--token-stdin"), "{message}");
    // The message says what to do, so no second help line is added
    // under it: "run `joy forge login --host github.test`" is the
    // command that has just refused.
    assert_eq!(
        answer.data()["action"],
        "",
        "the refusal contradicted itself: {}",
        answer.stderr
    );
    let log = std::fs::read_to_string(&argv).unwrap_or_default();
    assert!(!log.contains(" login "), "nothing was started: {log}");
}

/// An empty line is not a token, and the refusal says so without
/// pretending anything was stored.
#[test]
fn an_empty_line_is_refused_instead_of_stored() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let mut child = machine
        .joy(&["forge", "login", "--host", "github.test", "--token-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("joy runs");
    child
        .stdin
        .take()
        .expect("the child's stdin")
        .write_all(b"\n")
        .unwrap();
    let answer = Answer::of(child.wait_with_output().expect("joy ends"));

    assert!(!answer.ok, "{}{}", answer.stdout, answer.stderr);
    assert!(answer.stderr.contains("empty input"), "{}", answer.stderr);
    assert!(
        answer.stderr.contains("= note: state error"),
        "the state word is there for a person too: {}",
        answer.stderr
    );
}

/// EVERY answer of this command is one envelope, and that includes the
/// ways out that have nothing to do with a forge (D3.10). Before this
/// case the three below left through `bail!`: stdout stayed empty, and
/// an agent in `--json` mode got no `state` and no `action` at all.
#[test]
fn every_refusal_answers_with_one_envelope_in_json_mode() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    // 1. an empty line on stdin
    let mut child = machine
        .joy(&[
            "forge",
            "login",
            "--host",
            "github.test",
            "--token-stdin",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("joy runs");
    child
        .stdin
        .take()
        .expect("the child's stdin")
        .write_all(b"\n")
        .unwrap();
    let answer = Answer::of(child.wait_with_output().expect("joy ends"));
    assert_eq!(answer.code, Some(1), "{}", answer.stderr);
    let data = answer.data();
    assert_eq!(data["state"], "error");
    assert_eq!(data["host"], "github.test");
    assert!(
        data["message"]
            .as_str()
            .unwrap_or_default()
            .contains("empty input"),
        "{data}"
    );

    // 2. a stdin that was closed before a token arrived
    let mut child = machine
        .joy(&[
            "forge",
            "login",
            "--host",
            "github.test",
            "--token-stdin",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("joy runs");
    drop(child.stdin.take().expect("the child's stdin"));
    let answer = Answer::of(child.wait_with_output().expect("joy ends"));
    assert_eq!(answer.code, Some(1), "{}", answer.stderr);
    let data = answer.data();
    assert_eq!(data["state"], "error");
    assert!(
        data["message"]
            .as_str()
            .unwrap_or_default()
            .contains("stdin closed"),
        "{data}"
    );

    // 3. a logout that was told neither a host nor --all
    let answer = machine.run(&["forge", "logout", "--json"]);
    assert_eq!(answer.code, Some(1), "{}", answer.stderr);
    let data = answer.data();
    assert_eq!(data["state"], "error");
    assert!(
        data["message"]
            .as_str()
            .unwrap_or_default()
            .contains("needs a host"),
        "{data}"
    );
    // and none of the three is told to run the command that refused
    assert_eq!(data["action"], "", "{data}");
}

/// A token the forge refuses ends the command with exit 1 and a state
/// word, not with a success nobody can tell from a real sign in.
#[test]
fn a_token_the_forge_refuses_exits_one_with_a_state() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let mut child = machine
        .joy(&[
            "forge",
            "login",
            "--host",
            "github.test",
            "--token-stdin",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("joy runs");
    child
        .stdin
        .take()
        .expect("the child's stdin")
        .write_all(b"not-a-token\n")
        .unwrap();
    let answer = Answer::of(child.wait_with_output().expect("joy ends"));

    assert_eq!(answer.code, Some(1));
    let data = answer.data();
    assert_eq!(data["state"], "needs_sign_in");
    assert_eq!(
        data["action"], "run `joy forge login --host github.test`",
        "every failure names the next step"
    );
}

// ---------------------------------------------------------------------
// status (D3.10)
// ---------------------------------------------------------------------

/// The acceptance of J10, third sentence: `joy forge status --json`
/// lists the host with source, scopes and expiry, and exits 1 when
/// nothing is signed in.
#[test]
fn status_lists_the_host_with_its_source_and_scopes() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);
    machine.forges_yaml("github.test", "github");

    let answer = machine.run(&["forge", "status", "--json"]);

    assert!(answer.ok, "{}", answer.stderr);
    let data = answer.data();
    // The answer says what it is as a whole, not only per host, and a
    // signed in machine needs no next step.
    assert_eq!(data["state"], "signed-in");
    assert!(data["help"].is_null(), "{data}");
    let host = &data["hosts"][0];
    assert_eq!(host["host"], "github.test");
    assert_eq!(host["forge"], "github");
    assert_eq!(host["login"], "scotty");
    assert_eq!(host["state"], "signed-in");
    assert_eq!(host["source"], "keychain");
    assert_eq!(host["scopes"], "repo user:email");
    assert!(host["expires_at"].is_null(), "{host}");
    // and which binary answered, with its path and its protocol
    assert_eq!(host["plugin"]["id"], "github");
    assert_eq!(host["plugin"]["protocol"], 2);
    assert_eq!(
        host["plugin"]["path"],
        machine.plugins().join("joy-forge").display().to_string()
    );
}

#[test]
fn status_exits_one_when_nothing_is_signed_in() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);
    machine.forges_yaml("github.test", "github");

    let output = machine
        .joy(&["forge", "status", "--json"])
        .env("JOY_STUB_SIGNED_IN", "0")
        .output()
        .expect("joy runs");
    let answer = Answer::of(output);

    assert_eq!(answer.code, Some(1), "{}", answer.stdout);
    // The envelope is printed FIRST and the process then exits with the
    // code, the way `joy auth status` does it.
    let data = answer.data();
    assert_eq!(data["hosts"][0]["state"], "none");
    assert_eq!(data["hosts"][0]["source"], "none");
    // and the envelope says WHY the exit code is 1 and what to do,
    // which is the same sentence the human answer prints
    assert_eq!(data["state"], "none");
    assert_eq!(data["help"], "run `joy forge login --host github.test`");
}

/// A machine that knows no forge host at all: the empty list is an
/// answer, so it carries the state and the next step rather than being
/// `{"hosts":[]}` and exit 1 with nothing to read (JOY-02A7-A2).
#[test]
fn status_with_no_host_at_all_still_says_what_to_do() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let output = machine
        .joy(&["forge", "status", "--json"])
        .output()
        .expect("joy runs");
    let answer = Answer::of(output);

    assert_eq!(answer.code, Some(1), "{}", answer.stdout);
    let data = answer.data();
    assert!(
        data["hosts"].as_array().is_some_and(|rows| rows.is_empty()),
        "{data}"
    );
    assert_eq!(data["state"], "none");
    assert_eq!(data["help"], "run `joy forge login --host <host>`");
}

/// JOY-02A8-F4: a host no connector claims is not a host to sign in to.
/// The help line used to be `joy forge login --host <that host>` in
/// every case, which sends a person at a command that refuses them for
/// exactly the reason the row above already states.
#[test]
fn status_for_a_host_nobody_claims_points_at_forges_yaml_and_not_at_a_login() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let output = machine
        .joy(&["forge", "status", "--host", "nowhere.example"])
        .env("JOY_STUB_CLAIMS", "0")
        .output()
        .expect("joy runs");
    let answer = Answer::of(output);

    assert_eq!(answer.code, Some(1), "{}", answer.stdout);
    assert!(
        answer.stdout.contains("nowhere.example"),
        "the row names the host: {}",
        answer.stdout
    );
    assert!(
        answer
            .stdout
            .contains("connector: none answered for this host"),
        "{}",
        answer.stdout
    );
    assert!(
        answer
            .stderr
            .contains("add the instance to forges.yaml, or name a host a connector knows"),
        "the next step is the configuration: {}",
        answer.stderr
    );
    assert!(
        !answer.stderr.contains("joy forge login"),
        "no door exists to point at yet: {}",
        answer.stderr
    );
}

/// JOY-02A8-F4: what the connector said while it answered reaches the
/// person. Its stderr used to be read off the pipe and dropped at the
/// first zero exit, so a keychain that refused and was fallen back from
/// was invisible on the surface that exists to report exactly that.
#[test]
fn status_prints_what_the_connector_said_while_it_answered() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);
    machine.forges_yaml("github.test", "github");
    const SAID: &str = "the login keychain refused the ca_bundle, the system trust store was used";

    let output = machine
        .joy(&["forge", "status"])
        .env("JOY_STUB_NOTE", SAID)
        .output()
        .expect("joy runs");
    let answer = Answer::of(output);

    assert!(answer.ok, "{}", answer.stderr);
    assert!(
        answer.stderr.contains(&format!("= note: {SAID}")),
        "the sentence reaches the person, on stderr: {}",
        answer.stderr
    );
    // and a quiet connector produces no empty note line
    let quiet = Answer::of(
        machine
            .joy(&["forge", "status"])
            .output()
            .expect("joy runs"),
    );
    assert!(!quiet.stderr.contains("= note:"), "{}", quiet.stderr);
}

// ---------------------------------------------------------------------
// logout (D3.10, D2.6)
// ---------------------------------------------------------------------

#[test]
fn logout_removes_joys_own_credential() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let answer = machine.run(&["forge", "logout", "--host", "github.test", "--json"]);

    assert!(answer.ok, "{}", answer.stderr);
    let data = answer.data();
    assert_eq!(data["host"], "github.test");
    assert_eq!(data["removed"], true);
    assert_eq!(data["revoked"], true);
    assert_eq!(data["source"], "keychain");
}

/// A credential that came from gh is removed by gh and by nobody else,
/// so joy removes nothing and names the foreign command (D2.6, D3.10).
#[test]
fn logout_names_the_foreign_command_for_a_foreign_credential() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let output = machine
        .joy(&["forge", "logout", "--host", "github.test"])
        .env("JOY_STUB_SOURCE", "gh")
        .output()
        .expect("joy runs");
    let answer = Answer::of(output);

    assert!(answer.ok, "{}", answer.stderr);
    assert!(
        answer
            .stdout
            .contains("the token for github.test comes from gh")
            && answer
                .stdout
                .contains("gh auth logout --hostname github.test"),
        "{}",
        answer.stdout
    );
}

/// `--all` works on the whole host set and answers one object per host.
#[test]
fn logout_all_answers_for_every_host_of_the_set() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);
    machine.forges_yaml("github.test", "github");

    let answer = machine.run(&["forge", "logout", "--all", "--json"]);

    assert!(answer.ok, "{}", answer.stderr);
    let data = answer.data();
    assert_eq!(data["hosts"][0]["host"], "github.test");
    assert_eq!(data["hosts"][0]["removed"], true);
}

/// Under a delegation session the connector removes nothing and says
/// why (G2, D3.8): a delegated process may use the credential this
/// machine holds and may never sign it out. The CLI prints THAT
/// sentence and the state word, in both modes. A bare `removed: false`
/// would leave an agent with nothing to read and a call to retry for
/// ever.
#[test]
fn logout_under_a_delegation_session_prints_the_connectors_refusal() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);
    let session = a_live_session(&machine);

    let seen = Answer::of(
        machine
            .joy(&["forge", "logout", "--host", "github.test"])
            .env("JOY_SESSION", &session)
            .output()
            .expect("joy runs"),
    );

    assert_eq!(seen.code, Some(1), "{}{}", seen.stdout, seen.stderr);
    assert!(
        seen.stderr.contains("delegation session") && seen.stderr.contains("may never sign it out"),
        "the connector's sentence is missing: {}{}",
        seen.stdout,
        seen.stderr
    );
    assert!(
        seen.stderr.contains("state unsupported"),
        "the state word is missing: {}",
        seen.stderr
    );
    assert!(
        !seen.stdout.contains("No credential for"),
        "the refusal was printed as an answer: {}",
        seen.stdout
    );

    let answer = Answer::of(
        machine
            .joy(&["forge", "logout", "--host", "github.test", "--json"])
            .env("JOY_SESSION", &session)
            .output()
            .expect("joy runs"),
    );

    assert_eq!(answer.code, Some(1), "{}{}", answer.stdout, answer.stderr);
    let data = answer.data();
    assert_eq!(data["host"], "github.test");
    assert_eq!(data["state"], "unsupported");
    let message = data["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("delegation session")
            && message.contains("may never sign it out")
            && message.contains("joy forge logout"),
        "{message}"
    );
    assert!(
        data["removed"].is_null(),
        "a refusal is not an answer with two false flags: {data}"
    );
}

/// Neither a host nor `--all`: joy refuses rather than guessing which
/// credential to delete.
#[test]
fn logout_without_a_host_refuses_instead_of_guessing() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);

    let answer = machine.run(&["forge", "logout"]);

    assert!(!answer.ok);
    assert!(answer.stderr.contains("needs a host"), "{}", answer.stderr);
}

// ---------------------------------------------------------------------
// plugins, and the stale binary (D2.2a, D3.12)
// ---------------------------------------------------------------------

/// `joy forge plugins` is the diagnostic: which file answered, where it
/// was found, what it speaks, and what is wrong with the picture.
#[test]
fn plugins_names_the_file_that_answers_and_the_stale_one_beside_it() {
    let machine = Machine::new();
    machine.connector("joy-forge", CONNECTOR);
    let stale = machine.connector("joy-github", LEGACY_CONNECTOR);

    let answer = machine.run(&["forge", "plugins", "--json"]);

    assert!(answer.ok, "{}", answer.stderr);
    let data = answer.data();
    let github = data["plugins"]
        .as_array()
        .expect("a row per registry id")
        .iter()
        .find(|row| row["id"] == "github")
        .expect("the github row")
        .clone();
    assert_eq!(
        github["protocol"], 2,
        "the fresh binary wins the name order"
    );
    assert_eq!(github["version"], "joy-forge 0.21.0");
    assert_eq!(
        github["path"],
        machine.plugins().join("joy-forge").display().to_string()
    );
    assert_eq!(github["problem"], "shadowed-legacy");
    assert_eq!(
        github["found_in"], "JOY_PLUGIN_DIR",
        "the row says which step of the search order found the file"
    );
    assert_eq!(
        github["shadowed"][0],
        format!("rm {}", stale.display()),
        "the exact line that removes the stale binary"
    );
}

/// The acceptance of J10, fourth sentence: a protocol 1 `joy-github`
/// alone on the machine makes `joy forge login` print the binary's path
/// and the `rm` line, while the legacy `release` verb that binary DOES
/// answer keeps working.
#[test]
fn a_protocol_1_connector_refuses_login_with_the_path_and_the_rm_line() {
    let machine = Machine::new();
    let stale = machine.connector("joy-github", LEGACY_CONNECTOR);

    let answer = machine.run(&["forge", "login", "--host", "github.test", "--json"]);

    assert_eq!(answer.code, Some(1), "{}", answer.stdout);
    let data = answer.data();
    assert_eq!(data["state"], "plugin_outdated");
    let message = data["message"].as_str().unwrap_or_default();
    assert!(message.contains(&stale.display().to_string()), "{message}");
    assert!(
        message.contains(&format!("rm {}", stale.display())),
        "{message}"
    );
    assert_eq!(
        data["action"], "run `joy forge plugins` to see which binary answered",
        "the state decides the next step"
    );

    // And the verb a protocol 1 connector still answers keeps working,
    // which is what "joy release publish still works" rests on (D2.2a).
    let notes = machine.path().join("notes.md");
    std::fs::write(&notes, "the notes").unwrap();
    joy_core::forge_plugins::set_plugin_dirs(vec![machine.plugins()]);
    let outcome = joy_core::forge_plugins::release(
        joy_core::forge_plugins::by_id("github").expect("the registry row"),
        Some(&joy_core::forge_plugins::Target::remote(
            "https://github.test/o/r.git",
        )),
        "v1",
        "v1",
        &notes,
        &joy_core::forge_plugins::CallContext::rootless(),
    )
    .expect("the legacy connector still publishes");
    assert_eq!(
        outcome.url.as_deref(),
        Some("https://github.test/o/r/releases/tag/v1")
    );
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// Everything `ps` would have shown for a running child, sampled for
/// as long as the case lets it run.
///
/// Sampling once is not enough and stopping at the child's own argv is
/// worse than not sampling at all: that window ENDS before the token
/// has even been written, so a joy that put the secret in an argv
/// afterwards would pass. The watcher runs beside the child from the
/// spawn until the case stops it, which is after the child has read
/// the token and answered.
#[cfg(unix)]
struct ProcessWatch {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    samples: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    thread: std::thread::JoinHandle<()>,
}

#[cfg(unix)]
fn watch_the_process_list(pid: u32) -> ProcessWatch {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let stop = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(Mutex::new(Vec::new()));
    let thread = {
        let stop = stop.clone();
        let samples = samples.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if let Some(text) = argv_of(pid) {
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

#[cfg(unix)]
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
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.thread.join();
        self.samples.lock().expect("the samples").clone()
    }
}

/// A `JOY_SESSION` that names a delegation joy can really load, minted
/// the way an agent gets one: a project with an AI member, a delegation
/// token, and the redemption that prints the handle. A leftover or
/// malformed value would make the process no less interactive than it
/// already was, so the refusal it proves needs a real one.
fn a_live_session(machine: &Machine) -> String {
    let root = machine.path().join("home/delegator");
    std::fs::create_dir_all(&root).unwrap();
    let joy = |args: &[&str]| -> (bool, String) {
        let output = machine
            .joy(args)
            .current_dir(&root)
            .output()
            .expect("joy runs");
        (
            output.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    };
    let passphrase = "correct horse battery staple";
    let (ok, seen) = joy(&["init", "--name", "Delegator", "--user", "human@example.com"]);
    assert!(ok, "{seen}");
    // The commands that mint a delegation still read a git identity
    // (their call sites are package J11); this checkout gets one of its
    // own, repository local, so the machine under test keeps none.
    let repo = git2::Repository::open(&root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "The Human").unwrap();
    config.set_str("user.email", "human@example.com").unwrap();
    for args in [
        vec!["auth", "init", "--passphrase", passphrase],
        vec![
            "project",
            "member",
            "add",
            "ai:claude@joy",
            "--passphrase",
            passphrase,
        ],
    ] {
        let (ok, seen) = joy(&args);
        assert!(ok, "{args:?}: {seen}");
    }
    let (ok, token) = joy(&[
        "auth",
        "token",
        "add",
        "ai:claude@joy",
        "--passphrase",
        passphrase,
    ]);
    assert!(ok, "{token}");
    let token = token
        .lines()
        .find_map(|line| line.trim().strip_prefix('"')?.strip_suffix('"'))
        .expect("the token is printed")
        .to_string();
    let (ok, handle) = joy(&["auth", "--token", &token]);
    assert!(ok, "{handle}");
    handle
        .lines()
        .find_map(|line| line.strip_prefix("export JOY_SESSION="))
        .expect("the redemption prints the handle")
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------
// What the person sees while a sign in runs (JOY-02A9-48, D3.10)
// ---------------------------------------------------------------------

/// A connector whose sign in waits twice and then fails: the shape of
/// the run that produced this item. The second wait carries the reason
/// of D2.4, which is what a poll that is riding out a transport fault
/// reports.
const FAILING_LOGIN: &str = r#"#!/bin/sh
if [ "$1" = "version" ]; then
  echo '{"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}'
  exit 0
fi
forge="$1"
shift
verb="$1"
shift
host=""
while [ $# -gt 0 ]; do
  case "$1" in
    --host) host="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$verb" in
  claims) echo '{"claims":true}' ;;
  login)
    printf '{"event":"verification","host":"%s","url":"https://github.test/login/device","url_complete":null,"code":"WDJB-MJHT","expires_in":900,"interval":5}\n' "$host"
    echo '{"event":"waiting","seconds_left":895}'
    echo '{"event":"waiting","seconds_left":890,"reason":"github.test could not be reached: Temporary failure in name resolution"}'
    echo '{"event":"error","code":"network","message":"the sign in to github.test could not be finished: Temporary failure in name resolution"}'
    ;;
  *) echo '{"known":false}' ;;
esac
"#;

/// A connector that says the code is good for one second and then stops
/// answering: the "did not answer in time" path, with nothing of its
/// own to say about it.
const SILENT_AFTER_THE_CODE: &str = r#"#!/bin/sh
if [ "$1" = "version" ]; then
  echo '{"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}'
  exit 0
fi
forge="$1"
shift
verb="$1"
shift
host=""
while [ $# -gt 0 ]; do
  case "$1" in
    --host) host="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$verb" in
  claims) echo '{"claims":true}' ;;
  login)
    printf '{"event":"verification","host":"%s","url":"https://github.test/login/device","url_complete":null,"code":"WDJB-MJHT","expires_in":1,"interval":5}\n' "$host"
    if [ -x /bin/sleep ]; then /bin/sleep 60
    elif [ -x /usr/bin/sleep ]; then /usr/bin/sleep 60
    else i=0; while [ $i -lt 100000000 ]; do i=$((i+1)); done
    fi
    ;;
  *) echo '{"known":false}' ;;
esac
"#;

/// The last thing written on the terminal line that carries `text`:
/// everything after the carriage return or newline before it. It is
/// what a person really READS there, which is the whole point of
/// wiping a progress line before printing over it.
fn line_showing<'a>(seen: &'a str, text: &str) -> &'a str {
    seen.split(['\r', '\n'])
        .find(|part| part.contains(text))
        .unwrap_or_default()
}

/// JOY-02A9-48, finding 4: after the connector's `error` event the CLI
/// prints that error and stops.
///
/// Two things went wrong in the run that found this, and both are here:
/// the countdown was left standing on the terminal, so the refusal was
/// printed INTO "Still waiting, 14 minutes left." and the person read
/// both at once; and the run had to be waited out rather than ending
/// when the connector did.
#[test]
fn a_login_that_fails_prints_the_error_and_stops() {
    let machine = Machine::new();
    machine.connector("joy-forge", FAILING_LOGIN);

    let started = std::time::Instant::now();
    let (ok, seen) = on_a_terminal(&machine, &["forge", "login", "--host", "github.test"], "");
    let took = started.elapsed();

    assert!(!ok, "a sign in that failed exits 1: {seen}");
    assert!(
        seen.contains("the sign in to github.test could not be finished"),
        "the connector's own sentence reaches the person: {seen}"
    );
    assert!(
        seen.contains("= note: state offline"),
        "under the state its code names: {seen}"
    );
    // The wait was shown while it ran, with the reason the connector
    // gave for it.
    assert!(seen.contains("Still waiting"), "{seen}");
    assert!(
        seen.contains("Temporary failure in name resolution"),
        "the reason of D2.4 is shown, not swallowed: {seen}"
    );
    // And it is not standing under the refusal any more.
    let line = line_showing(&seen, "could not be finished");
    assert!(
        !line.contains("Still waiting"),
        "the refusal is printed over a wiped line, not into the countdown: {line:?}"
    );
    assert!(
        took < std::time::Duration::from_secs(30),
        "the connector's exit ends the loop; the code's expiry is not waited out: {took:?}"
    );
}

/// The other half of finding 4: where joy itself stops the call, the
/// sentence says which sign in it stopped and how much of the code was
/// left. "the forge plugin did not answer in time and was stopped"
/// tells a person standing at the forge's page nothing they can act on.
#[test]
fn a_login_joy_stopped_names_the_sign_in_and_what_was_left_of_the_code() {
    let machine = Machine::new();
    machine.connector("joy-forge", SILENT_AFTER_THE_CODE);

    let started = std::time::Instant::now();
    let (ok, seen) = on_a_terminal(&machine, &["forge", "login", "--host", "github.test"], "");
    let took = started.elapsed();

    assert!(!ok, "{seen}");
    assert!(
        seen.contains("joy stopped the sign in to github.test"),
        "the sentence names the sign in: {seen}"
    );
    // And what the code had left when joy stopped, which after joy's
    // own wait on a one second code is nothing. Reporting the last
    // number the connector wrote would read "the code had 1 second
    // left" twenty seconds after that second ran out (the review of
    // JOY-02A9-48).
    assert!(
        seen.contains("joy stopped the sign in to github.test before it finished."),
        "the code was long gone, so no number is invented: {seen}"
    );
    assert!(
        !seen.contains("1 second left"),
        "the stale countdown is not the sentence: {seen}"
    );
    assert!(
        seen.contains("= note: state expired"),
        "under a state an agent reads: {seen}"
    );
    assert!(
        seen.contains("= help: run `joy forge login --host github.test`"),
        "with the one next step, which is this command's own door: {seen}"
    );
    // The bound is the code's own life plus the moment the connector
    // needs to say its last word, and it starts when the code is
    // issued, not when the process was spawned.
    assert!(
        took < std::time::Duration::from_secs(60),
        "the call is bounded by the code, not by the fifteen minute cap: {took:?}"
    );
}
