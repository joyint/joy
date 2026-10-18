// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! A `joy-forge` a test starts never reaches the operating system's
//! credential store, on any system (JOY-02DB-25).
//!
//! The connector here is the shipped binary, started the way every
//! test starts a process ([`joy_test_env::command`]) and with the
//! environment it inherits from this binary's [`joy_test_env::isolate!`]
//! and nothing taken away: the person's session bus is still in reach.
//! It stores a pasted token, which on a machine with a working store
//! lands in the store, and it answers that the 0600 file took it,
//! because the door to the store is closed (`joy_test_env::NO_KEYCHAIN`).
//!
//! On Linux the same case is the proof that nothing was asked: run
//! under a private session bus with a Secret Service on it and a
//! monitor beside it, it leaves no method call to
//! `org.freedesktop.secrets`, and with the door open the same run
//! leaves the calls of a store and fails here.

// Nothing of the developer's shell and session reaches this binary
// (JOY-02BB-C7).
joy_test_env::isolate!();

use std::io::Write;
use std::process::Stdio;

use joy_forge_net::fake::{FakeForge, Reply};
use serde_json::Value;

/// The connector as cargo built it for this test run.
const CONNECTOR: &str = env!("CARGO_BIN_EXE_joy-forge");

#[test]
fn a_token_store_of_the_real_connector_takes_the_file_and_never_the_keychain() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/api/v3/user" => {
            if call.authorization() == Some("Bearer ghp_kept_off_the_keychain") {
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
    let dir = tempfile::tempdir().expect("a sandbox");
    let config = dir.path().join("config");
    std::fs::create_dir_all(config.join("joy")).unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    std::fs::write(
        config.join("joy/forges.yaml"),
        format!(
            "- host: forge.test\n  kind: github\n  api_base: {}/api/v3\n",
            fake.base()
        ),
    )
    .unwrap();

    let mut child = joy_test_env::command(CONNECTOR)
        .args(["github", "token-store", "--host", "forge.test"])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the connector");
    child
        .stdin
        .take()
        .expect("the connector's stdin")
        .write_all(b"ghp_kept_off_the_keychain\n")
        .unwrap();
    let output = child.wait_with_output().expect("the connector's end");
    let text = String::from_utf8_lossy(&output.stdout);
    let answer: Value = serde_json::from_str(text.trim()).unwrap_or_else(|e| {
        panic!(
            "the connector answered no JSON ({e}): {text}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(answer["known"], true, "{answer}");
    assert_eq!(answer["login"], "scotty");
    assert_eq!(
        answer["source"], "file",
        "the store did not answer, so the 0600 file took the token: {answer}"
    );
    let tokens = std::fs::read_to_string(config.join("joy/forge-tokens.json"))
        .expect("the 0600 file was written");
    assert!(
        tokens.contains("ghp_kept_off_the_keychain"),
        "the file holds the token"
    );
}
