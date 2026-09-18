// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The connector runner against real child processes (JOY-0293-12,
//! package J1 of the forge connection NG design).
//!
//! Every case here is one of J1's acceptance sentences, and each needs a
//! process: a 1 MB answer, a connector that refuses with text on stderr,
//! a connector that never answers and leaves a grandchild behind, a
//! protocol 1 binary beside a protocol 2 one, and an event stream a
//! caller reads while the child is still running. They live in their own
//! test binary because the resolution contract has process-wide state
//! (the registered directories of `set_plugin_dirs` and the test hook
//! `JOY_PLUGIN_DIR`), and they take one lock so that state belongs to
//! one case at a time.
//!
//! Unix only: the stubs are shell scripts. The rules they prove are not
//! unix rules, but a Windows stub is a different file format and a
//! different kill mechanism (the job object), and neither can be
//! exercised from here.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use joy_core::forge_plugins::{
    self, CallContext, CallerFacts, CancelToken, EventSink, FilesAnswer, ForgePluginSpec, FoundIn,
    PluginError, StreamBounds, Target, COMBINED_BINARY, PLUGIN_DIR_ENV,
};
use joy_core::host::HostKind;

// ---------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------

/// One case at a time: the search order and the test hook are process
/// state, so two cases sharing them would test each other.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The registry row every case asks with. `github` is the first row, so
/// its binary names are `joy-forge` then `joy-github`, the order D2.2
/// fixes.
fn github() -> &'static ForgePluginSpec {
    forge_plugins::by_id("github").expect("the registry carries github")
}

/// Point the resolution at one directory and nothing else.
fn only(dir: &Path) {
    std::env::remove_var(PLUGIN_DIR_ENV);
    forge_plugins::set_plugin_dirs(vec![dir.to_path_buf()]);
}

/// Write one executable stub and hand back its path.
fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    {
        let mut file = std::fs::File::create(&path).expect("write the stub");
        file.write_all(body.as_bytes()).expect("write the stub");
        file.flush().expect("flush the stub");
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make the stub executable");
    path
}

/// A protocol 2 connector that answers the verbs each case needs. It
/// records its own argv, so a case can read what actually went over the
/// wire (`--host`, `--host-kind`, `--login`).
const PROTOCOL_2: &str = r#"#!/bin/sh
if [ -n "$JOY_STUB_ARGV" ]; then
  echo "$@" >> "$JOY_STUB_ARGV"
fi
# The handshake asks the BINARY what protocol it speaks, never a forge
# inside it (D2.2a), so `version` arrives as the first argument with
# nothing before it and answers for every forge this file carries.
if [ "$1" = "version" ]; then
  echo '{"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}'
  exit 0
fi
forge="$1"
shift
verb="$1"
shift
case "$verb" in
  claims) echo '{"claims":true}' ;;
  identity) echo '{"known":true,"login":"alice","user_id":"12345","emails":["a@example.com"]}' ;;
  unknown-identity) echo '{"known":false}' ;;
  store) echo '{"state":"missing","may_create":true,"size_bytes":4096}' ;;
  files)
    printf '{"state":"files","truncated":false,"paths":["'
    head -c 1048576 /dev/zero | tr '\0' 'a'
    printf '"]}\n'
    ;;
  refuse)
    echo "the token was refused by github.com" >&2
    exit 3
    ;;
  slow)
    sleep 60 &
    echo "$!" > "$JOY_STUB_PIDFILE"
    echo "waiting for github.com to answer" >&2
    sleep 60
    ;;
  answer-and-leave)
    sleep 60 &
    echo "$!" > "$JOY_STUB_PIDFILE"
    echo '{"claims":true}'
    ;;
  garbage) echo 'not json at all' ;;
  noted-claims)
    echo "the login keychain refused the ca_bundle, the system trust store was used" >&2
    echo '{"claims":true}'
    ;;
  login)
    echo '{"event":"verification","host":"github.com","url":"https://github.com/login/device","url_complete":null,"code":"WDJB-MJHT","expires_in":900,"interval":5}'
    sleep 1
    echo '{"event":"result","known":true,"login":"scotty","stored":"keychain"}'
    ;;
  login-and-leave)
    sleep 60 &
    echo "$!" > "$JOY_STUB_PIDFILE"
    echo '{"event":"verification","host":"github.com","url":"https://github.com/login/device","url_complete":null,"code":"WDJB-MJHT","expires_in":900,"interval":5}'
    echo '{"event":"result","known":true,"login":"scotty","stored":"keychain"}'
    ;;
  login-hangs)
    sleep 60 &
    echo "$!" > "$JOY_STUB_PIDFILE"
    echo '{"event":"verification","host":"github.com","code":"WDJB-MJHT","expires_in":900,"interval":5}'
    sleep 60
    ;;
  *)
    echo "error: unrecognized subcommand '$verb'" >&2
    exit 2
    ;;
esac
"#;

/// A connector from before the handshake existed: clap rejects the
/// unknown subcommand, prints usage on stderr and exits 2 with nothing
/// on stdout (D2.2a). It still answers the six old verbs.
const PROTOCOL_1: &str = r#"#!/bin/sh
if [ -n "$JOY_STUB_ARGV" ]; then
  echo "$@" >> "$JOY_STUB_ARGV"
fi
case "$1" in
  claims) echo '{"claims":true}' ;;
  identity) echo '{"known":true,"login":"legacy"}' ;;
  resolve) echo '{"known":false}' ;;
  store) echo '{"state":"gone"}' ;;
  files) echo '{"state":"unknown"}' ;;
  release) echo '{"url":"https://github.com/o/r/releases/tag/v1"}' ;;
  *)
    echo "error: unrecognized subcommand '$1'" >&2
    echo "Usage: joy-github <COMMAND>" >&2
    exit 2
    ;;
esac
"#;

/// Where a stub writes its argv. It goes into THIS process's
/// environment because the children of the verb helpers inherit it and
/// take no per call environment of their own.
fn argv_log(dir: &Path) -> PathBuf {
    let path = dir.join("argv.log");
    std::env::set_var("JOY_STUB_ARGV", &path);
    path
}

fn argv_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------
// Resolution and the handshake (D2.2, D2.2a)
// ---------------------------------------------------------------------

/// J1 acceptance: a protocol 1 `joy-github` in `~/.cargo/bin` is
/// reported as protocol 1, and a protocol 2 binary named `joy-forge` in
/// the same directory wins the name order.
#[test]
fn joy_forge_wins_the_name_order_over_a_stale_legacy_binary() {
    let _guard = lock();
    let cargo_bin = tempfile::tempdir().expect("a temp ~/.cargo/bin");
    stub(cargo_bin.path(), "joy-github", PROTOCOL_1);
    only(cargo_bin.path());

    // Alone in the directory, the legacy binary is what answers, and
    // the handshake calls it by its protocol.
    let legacy = forge_plugins::resolve_plugin(github()).expect("the legacy binary is there");
    assert_eq!(legacy.protocol, 1);
    assert_eq!(legacy.plugin_version, None);
    assert_eq!(legacy.resolved_path, cargo_bin.path().join("joy-github"));
    assert_eq!(legacy.found_in, FoundIn::Registered);
    assert!(!legacy.is_combined());
    assert_eq!(
        legacy.removal_line(),
        format!("rm {}", cargo_bin.path().join("joy-github").display())
    );

    // The new connector lands beside it and nothing is deleted: the
    // name order alone makes the stale binary harmless (D3.12).
    stub(cargo_bin.path(), COMBINED_BINARY, PROTOCOL_2);
    forge_plugins::set_plugin_dirs(vec![cargo_bin.path().to_path_buf()]);
    let fresh = forge_plugins::resolve_plugin(github()).expect("the fresh binary is there");
    assert_eq!(fresh.protocol, 2);
    assert_eq!(fresh.plugin_version.as_deref(), Some("joy-forge 0.21.0"));
    assert_eq!(fresh.resolved_path, cargo_bin.path().join(COMBINED_BINARY));
    assert!(fresh.is_combined());

    // The stale binary is still on disk and still visible, which is
    // what `joy forge plugins` reports with its `rm` line.
    let candidates = forge_plugins::candidates(github());
    assert!(candidates.len() >= 2, "{candidates:?}");
    assert_eq!(candidates[0].0, cargo_bin.path().join(COMBINED_BINARY));
    assert_eq!(candidates[1].0, cargo_bin.path().join("joy-github"));
}

/// J1 acceptance: a binary placed next to the CALLING executable is
/// found on macOS and Linux without PATH. This test binary is the
/// caller, so the directory is the one its own executable sits in, and
/// no directory is registered and no test hook is set.
#[test]
fn a_connector_beside_the_calling_executable_is_found_without_path() {
    let _guard = lock();
    std::env::remove_var(PLUGIN_DIR_ENV);
    forge_plugins::set_plugin_dirs(Vec::new());
    let exe = std::env::current_exe().expect("this test binary");
    let exe_dir = exe.parent().expect("its directory").to_path_buf();
    assert!(
        !std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .any(|dir| dir == exe_dir),
        "the point of the case is that {exe_dir:?} is NOT on PATH"
    );
    let placed = stub(&exe_dir, COMBINED_BINARY, PROTOCOL_2);
    let resolved = forge_plugins::resolve_plugin(github());
    let _ = std::fs::remove_file(&placed);
    forge_plugins::set_plugin_dirs(Vec::new());
    let resolved = resolved.expect("a connector beside the caller is found");
    assert_eq!(resolved.resolved_path, placed);
    assert_eq!(resolved.found_in, FoundIn::ExecutableDir);
    assert_eq!(resolved.protocol, 2);
}

/// The test hook of D2.2 is a hook: in a DEVELOPMENT build it is
/// searched before everything the host registered, which is what makes
/// it useful to a test, and a shipped joy does not read it at all,
/// which is what keeps it from being a product switch. The case is
/// compiled for the build that has it, for the same reason.
#[cfg(debug_assertions)]
#[test]
fn the_test_hook_is_searched_first() {
    let _guard = lock();
    let registered = tempfile::tempdir().expect("a registered directory");
    let hook = tempfile::tempdir().expect("a hooked directory");
    stub(registered.path(), COMBINED_BINARY, PROTOCOL_2);
    stub(hook.path(), COMBINED_BINARY, PROTOCOL_2);
    forge_plugins::set_plugin_dirs(vec![registered.path().to_path_buf()]);
    std::env::set_var(PLUGIN_DIR_ENV, hook.path());
    let resolved = forge_plugins::resolve_plugin(github()).expect("the hooked binary");
    std::env::remove_var(PLUGIN_DIR_ENV);
    forge_plugins::set_plugin_dirs(Vec::new());
    assert_eq!(resolved.resolved_path, hook.path().join(COMBINED_BINARY));
    assert_eq!(resolved.found_in, FoundIn::TestHook);
}

/// D2.2a: the handshake is asked of the FILE, once per path and mtime.
/// `joy-forge` answers for every forge it carries, which is why its
/// answer lists them, so a machine with one connector and three
/// registry rows spawns ONE `version` process, and the question that
/// goes over the wire is `version` with no forge id before it.
#[test]
fn one_file_is_asked_its_protocol_once_for_every_forge_it_carries() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let log = argv_log(dir.path());
    for id in ["github", "gitlab", "gitea"] {
        let spec = forge_plugins::by_id(id).expect("a registry row");
        let resolved = forge_plugins::resolve_plugin(spec).expect("the one connector answers");
        assert_eq!(resolved.id, id);
        assert_eq!(resolved.protocol, 2);
        assert_eq!(resolved.plugin_version.as_deref(), Some("joy-forge 0.21.0"));
        assert_eq!(resolved.resolved_path, dir.path().join(COMBINED_BINARY));
    }
    let seen = argv_lines(&log);
    std::env::remove_var("JOY_STUB_ARGV");
    assert_eq!(
        seen,
        vec!["version".to_string()],
        "one file, one handshake, and no forge id in front of it"
    );
}

/// Nothing installed is its own state, with the names that were looked
/// for (D2.2a: `plugin_missing`).
#[test]
fn nothing_installed_is_plugin_missing() {
    let _guard = lock();
    let empty = tempfile::tempdir().expect("an empty directory");
    only(empty.path());
    // The executable directory and PATH are still searched, so the case
    // asks for a forge whose names cannot exist anywhere.
    let ghost = ForgePluginSpec {
        id: "ghost",
        display: "Ghost",
        binary_names: &["joy-does-not-exist-anywhere"],
    };
    let error = forge_plugins::resolve_plugin(&ghost).expect_err("nothing is installed");
    assert_eq!(error.state(), "plugin_missing");
    assert!(error.resolved_path().is_none());
    let text = error.to_string();
    assert!(text.contains("joy-does-not-exist-anywhere"), "{text}");
    assert!(
        forge_plugins::query::<serde_json::Value>(
            &ghost,
            "claims",
            Some(&Target::host("github.com")),
            &[],
            &CallContext::rootless()
        )
        .is_err(),
        "every verb of a connector nobody installed is the same state"
    );
}

/// D2.2a: a protocol 1 connector still answers the six old verbs with
/// `--remote`, and every verb of D2.4 is `plugin_outdated` with the
/// resolved path and the `rm` line.
#[test]
fn a_protocol_one_connector_answers_the_old_verbs_and_refuses_the_new_ones() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), "joy-github", PROTOCOL_1);
    only(dir.path());
    let log = argv_log(dir.path());
    let ctx = CallContext::rootless()
        .with_host_kind(HostKind::Interactive)
        .with_facts(CallerFacts {
            login: Some("scotty".into()),
            ..CallerFacts::default()
        });

    assert!(forge_plugins::claims(
        github(),
        &Target::remote("git@github.com:o/r.git"),
        &ctx
    ));
    // The old binary never saw a flag it does not know.
    let seen = argv_lines(&log);
    std::env::remove_var("JOY_STUB_ARGV");
    assert!(
        seen.iter().any(|line| line.starts_with("claims --remote")),
        "{seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.contains("--host-kind")),
        "the old binary never saw a flag it does not know: {seen:?}"
    );

    for (verb, target) in [
        ("token", Target::host("github.com")),
        ("web-url", Target::remote("git@github.com:o/r.git")),
        ("repositories", Target::host("github.com")),
    ] {
        let error =
            forge_plugins::query::<serde_json::Value>(github(), verb, Some(&target), &[], &ctx)
                .expect_err("a protocol 1 connector cannot answer a protocol 2 verb");
        assert_eq!(error.state(), "plugin_outdated");
        let text = error.to_string();
        let path = dir.path().join("joy-github");
        assert!(text.contains(&path.display().to_string()), "{text}");
        assert!(text.contains("speaks protocol 1"), "{text}");
        assert!(text.contains(&format!("rm {}", path.display())), "{text}");
    }
    // A host-only target is the other half of the rule: protocol 1
    // knows `--remote` and nothing else.
    let error = forge_plugins::claims_full(github(), &Target::host("github.com"), &ctx)
        .expect_err("protocol 1 has no --host");
    assert_eq!(error.state(), "plugin_outdated");
}

// ---------------------------------------------------------------------
// The runner (D2.3)
// ---------------------------------------------------------------------

/// J1 acceptance: a connector answer of 1 MB is returned intact instead
/// of timing out. Reading stdout only after the child exits deadlocks at
/// the pipe buffer (64 KiB on Linux), which is well under what a
/// repository listing produces.
#[test]
fn a_one_megabyte_answer_comes_back_whole() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let started = Instant::now();
    let answer = forge_plugins::files(
        github(),
        &Target::host("github.com"),
        &CallContext::rootless(),
    )
    .expect("the big answer arrives");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "it must not run into the deadline"
    );
    match answer {
        FilesAnswer::Files { paths, truncated } => {
            assert!(!truncated);
            assert_eq!(paths.len(), 1);
            assert_eq!(paths[0].len(), 1024 * 1024, "the answer is intact");
            assert!(paths[0].bytes().all(|b| b == b'a'));
        }
        other => panic!("expected a file listing, got {other:?}"),
    }
}

/// J1 acceptance: a connector that exits 3 with text on stderr produces
/// an error naming the connector and the text, not `None`. Stderr is
/// piped and captured now, so the message survives a host with no
/// terminal.
#[test]
fn a_refusal_with_stderr_becomes_an_error_that_carries_both() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    let path = stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let error = forge_plugins::query::<serde_json::Value>(
        github(),
        "refuse",
        Some(&Target::host("github.com")),
        &[],
        &CallContext::rootless(),
    )
    .expect_err("exit 3 is a refusal");
    assert_eq!(error.state(), "plugin_failed");
    assert_eq!(error.resolved_path(), Some(path.as_path()));
    let text = error.to_string();
    assert!(text.contains(&path.display().to_string()), "{text}");
    assert!(text.contains("exit 3"), "{text}");
    assert!(
        text.contains("the token was refused by github.com"),
        "the connector's own words reach the caller: {text}"
    );
    match error {
        PluginError::Failed {
            exit_code, stderr, ..
        } => {
            assert_eq!(exit_code, Some(3));
            assert!(stderr.contains("refused"));
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// JOY-02A8-F4: a connector that exits 0 and still had something to say
/// is heard. Its stderr used to be read off the pipe and thrown away at
/// the first zero exit, so the macOS refusal of a `ca_bundle` that
/// names the system trust store reached nobody: the answer looked like
/// a plain success and the sentence was gone.
#[test]
fn a_clean_answer_carries_what_the_connector_said_while_it_answered() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let noted = forge_plugins::query_noted::<serde_json::Value>(
        github(),
        "noted-claims",
        Some(&Target::host("github.com")),
        &[],
        &CallContext::rootless(),
    )
    .expect("exit 0 is an answer");
    assert_eq!(noted.answer["claims"], true);
    let note = noted.note.expect("the connector said something");
    assert!(
        note.contains("the login keychain refused the ca_bundle"),
        "{note}"
    );
    assert!(note.contains("the system trust store was used"), "{note}");
    // and a connector that said nothing gets no note, so a caller
    // cannot print an empty line under every row.
    let quiet = forge_plugins::query_noted::<serde_json::Value>(
        github(),
        "claims",
        Some(&Target::host("github.com")),
        &[],
        &CallContext::rootless(),
    )
    .expect("exit 0 is an answer");
    assert_eq!(quiet.note, None, "silence is not a note");
}

/// D2.3: the deadline ends the call, and the process GROUP is killed,
/// so a `gh` or `curl` grandchild does not outlive it (and does not
/// hold the pipe open, which would hang the runner just as badly as the
/// old sequential read did).
#[test]
fn a_connector_that_never_answers_is_stopped_with_its_grandchildren() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let resolved = forge_plugins::resolve_plugin(github()).expect("the stub is there");
    let pidfile = dir.path().join("grandchild.pid");
    let started = Instant::now();
    let outcome = forge_plugins::run_once(
        &resolved,
        &["github".to_string(), "slow".to_string()],
        &[(
            "JOY_STUB_PIDFILE".to_string(),
            pidfile.display().to_string(),
        )],
        Duration::from_millis(500),
    );
    let elapsed = started.elapsed();
    assert!(outcome.timed_out, "{outcome:?}");
    assert!(
        elapsed < Duration::from_secs(10),
        "the runner returned after {elapsed:?}, so the grandchild still held the pipe"
    );
    assert!(
        outcome.stderr_text.contains("waiting for github.com"),
        "what the connector said before it hung is captured: {outcome:?}"
    );
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .expect("the stub wrote its grandchild's pid")
        .trim()
        .parse()
        .expect("a pid");
    // The grandchild had 60 s to live; after the group kill it is gone.
    let mut alive = true;
    for _ in 0..50 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            alive = false;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !alive,
        "the grandchild {pid} outlived the process group kill"
    );
}

/// D2.3: the deadline bounds the CALL, not the connector's own
/// lifetime, and the process group is killed on EVERY path.
///
/// This is the shape a connector that shells out to `gh`, `glab`, `tea`
/// or `curl` takes when it works: it starts the foreign CLI, prints its
/// answer and exits 0. The grandchild inherited stdout, and a pipe
/// reaches end of file only when every write end is closed, so a runner
/// that reaps the connector and then reads stdout to its end waits for
/// the grandchild, with `timed_out == false` and nothing killed.
#[test]
fn a_connector_that_leaves_a_grandchild_behind_does_not_hold_the_call() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let resolved = forge_plugins::resolve_plugin(github()).expect("the stub is there");
    let pidfile = dir.path().join("grandchild.pid");
    let started = Instant::now();
    let outcome = forge_plugins::run_once(
        &resolved,
        &["github".to_string(), "answer-and-leave".to_string()],
        &[(
            "JOY_STUB_PIDFILE".to_string(),
            pidfile.display().to_string(),
        )],
        Duration::from_secs(5),
    );
    let elapsed = started.elapsed();
    assert!(
        !outcome.timed_out,
        "the connector answered and exited 0: {outcome:?}"
    );
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(
        outcome
            .stdout_json
            .as_ref()
            .and_then(|value| value.get("claims"))
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "the answer the connector did print is intact: {outcome:?}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "the grandchild held the call for {elapsed:?}, so the deadline bounded nothing"
    );
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .expect("the stub wrote its grandchild's pid")
        .trim()
        .parse()
        .expect("a pid");
    let mut alive = true;
    for _ in 0..50 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            alive = false;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !alive,
        "the grandchild {pid} outlived the connector's normal exit"
    );
}

/// D2.2a scopes "exit 2 with empty stdout" to the HANDSHAKE. A protocol
/// 2 connector that rejects an unknown subcommand or an unknown flag (a
/// joy-forge built before `web-url`, say) exits 2 in the same way, and
/// telling its owner to `rm` it would contradict the protocol 2 this
/// joy just heard it claim.
#[test]
fn an_unknown_verb_on_a_protocol_two_connector_is_a_refusal_not_a_stale_binary() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    let path = stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let error = forge_plugins::query::<serde_json::Value>(
        github(),
        "web-url",
        Some(&Target::remote("git@github.com:o/r.git")),
        &[],
        &CallContext::rootless(),
    )
    .expect_err("this stub knows no web-url");
    assert_eq!(error.state(), "plugin_failed");
    assert_eq!(error.resolved_path(), Some(path.as_path()));
    let text = error.to_string();
    assert!(!text.contains("speaks protocol 1"), "{text}");
    assert!(!text.contains("rm "), "{text}");
    assert!(text.contains("unrecognized subcommand"), "{text}");
}

/// D2.3: `claims --host github.com` works with no project on disk, and
/// every call carries the host kind (D1.10) and the login pin (D4.1c).
///
/// "No project on disk" is observed and not assumed: this test binary
/// runs inside the joy repository, which IS a project, so the case
/// moves the working directory into an empty one for the duration of
/// the call. Without that it would only show that no `current_dir` is
/// passed, which is a different sentence.
#[test]
fn a_rootless_call_carries_the_host_the_host_kind_and_the_pin() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let log = argv_log(dir.path());
    let ctx = CallContext::rootless()
        .with_host_kind(HostKind::Delegated)
        .with_facts(CallerFacts {
            login: Some("scotty".into()),
            token_env: Some("JOY_FORGE_TOKEN".into()),
            token_value: Some("gho_secret".into()),
            ..CallerFacts::default()
        });
    assert!(ctx.root.is_none(), "there is no project on disk");
    // The handshake and the verb both run with nothing but an empty
    // directory around them.
    let nowhere = tempfile::tempdir().expect("a directory that is no project");
    let back = std::env::current_dir().expect("a working directory");
    std::env::set_current_dir(nowhere.path()).expect("move out of every project");
    let claimed = forge_plugins::claims(github(), &Target::host("github.com"), &ctx);
    std::env::set_current_dir(&back).expect("move back");
    assert!(claimed, "a rootless claims call is answered");
    let seen = argv_lines(&log);
    std::env::remove_var("JOY_STUB_ARGV");
    let call = seen
        .iter()
        .find(|line| line.contains("claims") && line.contains("--host-kind"))
        .unwrap_or_else(|| panic!("no claims call in {seen:?}"));
    assert!(
        seen.iter().any(|line| line == "version"),
        "the handshake ran without a working directory too: {seen:?}"
    );
    assert!(
        call.starts_with("github claims --host github.com"),
        "{call}"
    );
    assert!(call.contains("--host-kind delegated"), "{call}");
    assert!(call.contains("--login scotty"), "{call}");
    assert!(call.contains("--token-env JOY_FORGE_TOKEN"), "{call}");
    assert!(
        !call.contains("gho_secret"),
        "the token travels in the environment, never in argv: {call}"
    );
}

/// D2.3: `{"known":false}` is an ANSWER, and the five ways a call can
/// fail are five other things. Every caller can tell all of them apart.
#[test]
fn known_false_is_not_a_failure_and_the_failures_differ() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let ctx = CallContext::rootless();

    // an answer
    let known_false: forge_plugins::ForgeIdentity = forge_plugins::query(
        github(),
        "unknown-identity",
        Some(&Target::host("github.com")),
        &[],
        &ctx,
    )
    .expect("known:false is an answer, not a failure");
    assert!(!known_false.known);

    // an answer this joy cannot read
    let garbled = forge_plugins::query::<serde_json::Value>(
        github(),
        "garbage",
        Some(&Target::host("github.com")),
        &[],
        &ctx,
    )
    .expect_err("garbage is not an answer");
    assert_eq!(garbled.state(), "plugin_failed");
    assert!(matches!(garbled, PluginError::Unparsable { .. }));

    // a refusal
    let refused = forge_plugins::query::<serde_json::Value>(
        github(),
        "refuse",
        Some(&Target::host("github.com")),
        &[],
        &ctx,
    )
    .expect_err("exit 3 is not an answer");
    assert_eq!(refused.state(), "plugin_failed");
    assert!(matches!(refused, PluginError::Failed { .. }));

    // and the best-effort surface still degrades to nothing
    assert!(forge_plugins::identity(github(), None, &ctx).is_some());
}

/// The old promise, kept (D5 and package P1a): best effort is not the
/// same as silent. Every failed call leaves one warn line naming the
/// connector and the verb, because that pair is what a reader needs to
/// act on.
#[test]
fn every_failed_call_is_warned_with_the_connector_and_the_verb() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let log = WarnLog::default();
    tracing::subscriber::with_default(log.clone(), || {
        let resolved = forge_plugins::resolve_plugin(github()).expect("the stub is there");
        let _ = forge_plugins::query::<serde_json::Value>(
            github(),
            "refuse",
            Some(&Target::host("github.com")),
            &[],
            &CallContext::rootless(),
        );
        let gone = resolved
            .resolved_path
            .parent()
            .expect("a directory")
            .join("removed")
            .join(COMBINED_BINARY);
        let missing = forge_plugins::ResolvedPlugin {
            resolved_path: gone,
            ..resolved
        };
        let outcome = forge_plugins::run_once(
            &missing,
            &["github".to_string(), "claims".to_string()],
            &[],
            Duration::from_secs(5),
        );
        assert!(outcome.spawn_error.is_some());
    });
    let lines = log.lines();
    let refused = lines
        .iter()
        .find(|line| line.contains("refused the verb"))
        .unwrap_or_else(|| panic!("no line about the refusing connector in {lines:?}"));
    assert!(refused.contains("github"), "{refused}");
    assert!(refused.contains("refuse"), "{refused}");
    assert!(refused.contains("code=3"), "{refused}");
    assert!(
        refused.contains("the token was refused"),
        "the connector's own words reach the operator too: {refused}"
    );
    let unstartable = lines
        .iter()
        .find(|line| line.contains("could not be started"))
        .unwrap_or_else(|| panic!("no line about the missing connector in {lines:?}"));
    assert!(unstartable.contains("removed"), "{unstartable}");
    assert!(unstartable.contains("claims"), "{unstartable}");
}

/// P1a's acceptance sentence: "a deliberately removed plugin produces a
/// warn log line and a failing startup probe instead of a silent
/// unknown". The two states that reach no process, `plugin_missing` and
/// `plugin_outdated`, are the ones a caller turns into `false` and
/// `None`, so they are the two an operator most needs in the log.
#[test]
fn a_removed_and_a_stale_connector_are_warned_about_on_the_normal_call_path() {
    let _guard = lock();

    // Removed: nothing with any of the names exists anywhere.
    let empty = tempfile::tempdir().expect("an empty directory");
    only(empty.path());
    let ghost = ForgePluginSpec {
        id: "ghost",
        display: "Ghost",
        binary_names: &["joy-does-not-exist-anywhere"],
    };
    let log = WarnLog::default();
    let claimed = tracing::subscriber::with_default(log.clone(), || {
        forge_plugins::claims(
            &ghost,
            &Target::host("github.com"),
            &CallContext::rootless(),
        )
    });
    assert!(!claimed, "the answer still degrades to no claim");
    let lines = log.lines();
    let missing = lines
        .iter()
        .find(|line| line.contains("not installed"))
        .unwrap_or_else(|| panic!("the removed connector was silent: {lines:?}"));
    assert!(missing.contains("ghost"), "{missing}");
    assert!(missing.contains("claims"), "{missing}");

    // Stale: the file that answers speaks protocol 1 and the verb is
    // not one it knows.
    let old = tempfile::tempdir().expect("a temp directory");
    stub(old.path(), "joy-github", PROTOCOL_1);
    only(old.path());
    let log = WarnLog::default();
    tracing::subscriber::with_default(log.clone(), || {
        assert!(forge_plugins::query::<serde_json::Value>(
            github(),
            "token",
            Some(&Target::host("github.com")),
            &[],
            &CallContext::rootless(),
        )
        .is_err());
    });
    let lines = log.lines();
    let stale = lines
        .iter()
        .find(|line| line.contains("older protocol"))
        .unwrap_or_else(|| panic!("the stale connector was silent: {lines:?}"));
    assert!(stale.contains("github"), "{stale}");
    assert!(stale.contains("token"), "{stale}");
    assert!(stale.contains("joy-github"), "{stale}");
}

// ---------------------------------------------------------------------
// The streaming runner (D2.3, D2.4's login event shapes)
// ---------------------------------------------------------------------

/// What a caller does with the event stream: keep the events, and
/// answer the first one with the deadline the forge granted.
#[derive(Default)]
struct Events {
    seen: Vec<serde_json::Value>,
    first_at: Option<Duration>,
    started: Option<Instant>,
    grant: Option<Duration>,
    cancel_on_first: Option<CancelToken>,
}

impl EventSink for Events {
    fn event(&mut self, event: &serde_json::Value) -> Option<Duration> {
        if self.first_at.is_none() {
            self.first_at = self.started.map(|start| start.elapsed());
        }
        self.seen.push(event.clone());
        if let Some(cancel) = &self.cancel_on_first {
            cancel.cancel();
        }
        // The second bound of D2.3: the verification event's own
        // `expires_in`, which the runner caps at `bounds.total`.
        if event.get("event").and_then(|e| e.as_str()) == Some("verification") {
            return self.grant.or_else(|| {
                event
                    .get("expires_in")
                    .and_then(serde_json::Value::as_u64)
                    .map(Duration::from_secs)
            });
        }
        None
    }
}

/// D2.3: `run_stream` hands every line to the caller WHILE the child
/// runs. That is what makes `login` usable: the verification code is on
/// screen a second before the connector is done polling the forge.
#[test]
fn events_reach_the_caller_before_the_connector_exits() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let resolved = forge_plugins::resolve_plugin(github()).expect("the stub is there");
    let mut sink = Events {
        started: Some(Instant::now()),
        ..Events::default()
    };
    let started = Instant::now();
    let outcome = forge_plugins::run_stream(
        &resolved,
        &["github".to_string(), "login".to_string()],
        &[],
        &mut sink,
        &CancelToken::new(),
        StreamBounds::for_verb("login"),
        None,
    );
    let elapsed = started.elapsed();
    assert!(!outcome.timed_out, "{outcome:?}");
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(sink.seen.len(), 2, "{:?}", sink.seen);
    assert_eq!(
        sink.seen[0].get("event").and_then(|e| e.as_str()),
        Some("verification")
    );
    assert_eq!(
        sink.seen[0].get("code").and_then(|e| e.as_str()),
        Some("WDJB-MJHT")
    );
    assert_eq!(
        sink.seen[1].get("event").and_then(|e| e.as_str()),
        Some("result")
    );
    // the last event that parsed is the one a caller reads as the answer
    assert_eq!(
        outcome
            .stdout_json
            .as_ref()
            .and_then(|v| v.get("login"))
            .and_then(|v| v.as_str()),
        Some("scotty")
    );
    let first_at = sink.first_at.expect("a first event");
    assert!(
        first_at < Duration::from_millis(900),
        "the first event arrived after {first_at:?}"
    );
    assert!(
        elapsed >= Duration::from_secs(1),
        "the child was still running when the first event arrived"
    );
}

/// D2.3's second bound: after the first event the call lives for what
/// that event granted, not for the 15 s that bounded the first one and
/// not for ever.
#[test]
fn the_first_event_sets_the_deadline_for_the_rest() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let resolved = forge_plugins::resolve_plugin(github()).expect("the stub is there");
    let pidfile = dir.path().join("grandchild.pid");
    let mut sink = Events {
        started: Some(Instant::now()),
        grant: Some(Duration::from_millis(400)),
        ..Events::default()
    };
    let started = Instant::now();
    let outcome = forge_plugins::run_stream(
        &resolved,
        &["github".to_string(), "login-hangs".to_string()],
        &[(
            "JOY_STUB_PIDFILE".to_string(),
            pidfile.display().to_string(),
        )],
        &mut sink,
        &CancelToken::new(),
        StreamBounds::for_verb("login"),
        None,
    );
    let elapsed = started.elapsed();
    assert!(outcome.timed_out, "{outcome:?}");
    assert_eq!(sink.seen.len(), 1, "{:?}", sink.seen);
    // The granted deadline plus the last word grace, which is longer
    // than one request of a connector on purpose (JOY-02A9-48): a poll
    // started with a second of the code left may cost fifteen, and joy
    // must not kill the connector before it can say so. Either way it
    // is nowhere near the 900 s cap.
    assert!(
        elapsed < Duration::from_secs(30),
        "the granted deadline decided, not the 900 s cap: {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(400),
        "and it was not cut short either: {elapsed:?}"
    );
}

/// D2.3, the streaming half of the same rule: a connector that answered
/// and exited must end the call, even when something it started still
/// holds stdout open. End of file is the only other way out of the
/// event loop, and a `gh` grandchild postpones it past every bound the
/// caller set.
#[test]
fn a_stream_ends_at_the_connector_and_not_at_its_grandchild() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let resolved = forge_plugins::resolve_plugin(github()).expect("the stub is there");
    let pidfile = dir.path().join("grandchild.pid");
    let mut sink = Events {
        started: Some(Instant::now()),
        // What the forge granted, shortened so the case is quick: the
        // call must end at the connector's exit, well inside it.
        grant: Some(Duration::from_millis(400)),
        ..Events::default()
    };
    let started = Instant::now();
    let outcome = forge_plugins::run_stream(
        &resolved,
        &["github".to_string(), "login-and-leave".to_string()],
        &[(
            "JOY_STUB_PIDFILE".to_string(),
            pidfile.display().to_string(),
        )],
        &mut sink,
        &CancelToken::new(),
        StreamBounds::for_verb("login"),
        None,
    );
    let elapsed = started.elapsed();
    assert!(
        !outcome.timed_out,
        "the connector answered and exited 0: {outcome:?}"
    );
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(
        sink.seen.len(),
        2,
        "every line it wrote arrived: {:?}",
        sink.seen
    );
    assert_eq!(
        sink.seen[1].get("event").and_then(|e| e.as_str()),
        Some("result")
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "the grandchild held the stream for {elapsed:?}"
    );
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .expect("the stub wrote its grandchild's pid")
        .trim()
        .parse()
        .expect("a pid");
    let mut alive = true;
    for _ in 0..50 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            alive = false;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive, "the grandchild {pid} outlived the stream");
}

/// D2.3: cancelling ends the call at once, and the process group goes
/// with it.
#[test]
fn a_cancelled_stream_ends_the_connector_and_its_grandchildren() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("a temp directory");
    stub(dir.path(), COMBINED_BINARY, PROTOCOL_2);
    only(dir.path());
    let resolved = forge_plugins::resolve_plugin(github()).expect("the stub is there");
    let pidfile = dir.path().join("grandchild.pid");
    let cancel = CancelToken::new();
    let mut sink = Events {
        started: Some(Instant::now()),
        cancel_on_first: Some(cancel.clone()),
        ..Events::default()
    };
    let started = Instant::now();
    let outcome = forge_plugins::run_stream(
        &resolved,
        &["github".to_string(), "login-hangs".to_string()],
        &[(
            "JOY_STUB_PIDFILE".to_string(),
            pidfile.display().to_string(),
        )],
        &mut sink,
        &cancel,
        StreamBounds::for_verb("login"),
        None,
    );
    let elapsed = started.elapsed();
    assert!(!outcome.timed_out, "a cancel is not a timeout: {outcome:?}");
    assert!(
        elapsed < Duration::from_secs(5),
        "the cancel took {elapsed:?}"
    );
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .expect("the stub wrote its grandchild's pid")
        .trim()
        .parse()
        .expect("a pid");
    let mut alive = true;
    for _ in 0..50 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            alive = false;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive, "the grandchild {pid} outlived the cancel");
}

// ---------------------------------------------------------------------

/// Keeps the fields of every warn event, so a test can read the line an
/// operator would read. Hand written on purpose: joy-core carries no
/// subscriber crate, not even for tests.
#[derive(Clone, Default)]
struct WarnLog(std::sync::Arc<Mutex<Vec<String>>>);

impl WarnLog {
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

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
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                let _ = write!(self.0, " {}={value:?}", field.name());
            }
        }
        let mut line = String::new();
        event.record(&mut Fields(&mut line));
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(line);
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}
