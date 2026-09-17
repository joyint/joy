// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The sign in verbs against in process fakes (JOY-029B-B0, package
//! J3).
//!
//! Every OAuth and REST behaviour of D2.7 is proved here, and nothing
//! in this file contacts a real forge: the device endpoints, the token
//! endpoint, the PKCE authorize step and the REST calls are all a
//! `FakeForge` on `127.0.0.1`, and the "browser" is a TCP connection
//! this test opens to the loopback listener itself.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use joy_forge_net::auth::oauth::{Events, NoWait};
use joy_forge_net::auth::store::{Record, Vault};
use joy_forge_net::auth::{verbs, Purpose};
use joy_forge_net::fake::{FakeForge, Reply, TestForge};
use joy_forge_net::forge::{Ctx, HostKind, Target};
use serde_json::{json, Value};

/// A context whose credentials, locks and login memory all live under
/// one temporary directory: a test must never touch the person's own
/// keychain or their app state.
fn sandbox(dir: &std::path::Path) -> Ctx {
    Ctx::bare(dir.join("project"))
        .with_vault(Vault::file_at(dir.join("config")))
        .with_state_dir(dir.join("state"))
}

fn interactive(ctx: Ctx) -> Ctx {
    let mut ctx = ctx;
    ctx.host_kind = HostKind::Interactive;
    ctx
}

fn events_of(events: &[Value], name: &str) -> Vec<Value> {
    events
        .iter()
        .filter(|event| event["event"] == name)
        .cloned()
        .collect()
}

/// The fake's account endpoint: the same answer every test needs.
fn user_reply() -> Reply {
    Reply::json(
        200,
        r#"{"login":"scotty","id":12345,"emails":["s@example.test"]}"#,
    )
    .with_header("X-OAuth-Scopes", "repo, user:email")
}

/// D2.4 and D2.7: the device grant, end to end. The verification event
/// carries the code and the URL, `slow_down` adds five seconds, the
/// `result` names the login and the store, and the token never appears
/// in an event.
#[test]
fn a_device_login_streams_its_events_and_stores_the_token() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let fake = FakeForge::start(move |call| match call.path.as_str() {
        "/login/device/code" => Reply::json(
            200,
            r#"{"device_code":"dev-1","user_code":"WDJB-MJHT",
                "verification_uri":"https://forge.test/login/device",
                "expires_in":900,"interval":5}"#,
        ),
        "/login/oauth/access_token" => match counter.fetch_add(1, Ordering::SeqCst) {
            0 => Reply::json(200, r#"{"error":"authorization_pending"}"#),
            1 => Reply::json(200, r#"{"error":"slow_down"}"#),
            _ => Reply::json(
                200,
                r#"{"access_token":"gho_from_the_fake","token_type":"bearer",
                    "scope":"repo,user:email","expires_in":28800,
                    "refresh_token":"rt-1"}"#,
            ),
        },
        "/user" => user_reply(),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let ctx = interactive(sandbox(dir.path()));
    let mut events: Vec<Value> = Vec::new();
    let clock = NoWait::default();
    let code = verbs::login(
        &forge,
        &Target::Host("forge.test".into()),
        Purpose::Write,
        &ctx,
        &mut events,
        &clock,
    );
    assert_eq!(code, 0);

    let verification = &events_of(&events, "verification")[0];
    assert_eq!(verification["code"], "WDJB-MJHT");
    assert_eq!(verification["url"], "https://forge.test/login/device");
    assert_eq!(verification["expires_in"], 900);
    assert_eq!(verification["interval"], 5);
    assert_eq!(verification["host"], "forge.test");
    // it is the FIRST event, so a caller sees the code before anything
    // else happens (D2.3's first event bound)
    assert_eq!(events[0]["event"], "verification");

    assert_eq!(events_of(&events, "waiting").len(), 1);
    let slow = &events_of(&events, "slow_down")[0];
    assert_eq!(slow["interval"], 10, "slow_down adds five seconds (D2.7)");
    assert_eq!(
        clock.waits(),
        vec![
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(10)
        ],
        "the forge's interval is honoured, and slow_down widens it"
    );

    let result = &events_of(&events, "result")[0];
    assert_eq!(result["known"], true);
    assert_eq!(result["login"], "scotty");
    assert_eq!(result["user_id"], "12345");
    assert_eq!(result["emails"], json!(["s@example.test"]));
    assert_eq!(result["scopes"], "repo user:email");
    assert_eq!(result["stored"], "file");
    assert!(result["expires_at"].is_string());

    // The token is never printed during login (D2.4).
    let printed = serde_json::to_string(&events).unwrap();
    assert!(
        !printed.contains("gho_from_the_fake"),
        "no event may carry the token"
    );

    // And afterwards `token` answers with the stored credential, the
    // granted set and the login it belongs to (D2.4, D4.1c).
    let answer = verbs::token(&forge, &Target::Host("forge.test".into()), &ctx);
    assert_eq!(answer["known"], true);
    assert_eq!(answer["token"], "gho_from_the_fake");
    assert_eq!(answer["login"], "scotty");
    assert_eq!(answer["source"], "file");
    assert_eq!(answer["scopes"], "repo user:email");
    assert_eq!(answer["chose_by"], "only");
    assert_eq!(answer["username"], "x-access-token");
}

/// D2.7: every named OAuth error becomes one `error` event and the
/// verb still exits 0, because a refusal is an answer.
#[test]
fn a_refused_device_login_ends_in_one_error_event() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/login/device/code" => Reply::json(
            200,
            r#"{"device_code":"d","user_code":"C-1","verification_uri":"https://forge.test/d",
                "expires_in":60,"interval":1}"#,
        ),
        "/login/oauth/access_token" => Reply::json(
            200,
            r#"{"error":"access_denied","error_description":"the person said no"}"#,
        ),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let mut events: Vec<Value> = Vec::new();
    let code = verbs::login(
        &forge,
        &Target::Host("forge.test".into()),
        Purpose::Write,
        &interactive(sandbox(dir.path())),
        &mut events,
        &NoWait::default(),
    );
    assert_eq!(code, 0, "a refusal is an answer, not a failure");
    let error = &events_of(&events, "error")[0];
    assert_eq!(error["code"], "access_denied");
    assert_eq!(error["message"], "the person said no");
}

/// D2.7: device flow disabled on the registration is its own code, so
/// a host can say what to do about it.
#[test]
fn a_forge_with_the_device_flow_switched_off_says_so_by_name() {
    let fake = FakeForge::start(|_| {
        Reply::json(
            200,
            r#"{"error":"device_flow_disabled","error_description":"not enabled"}"#,
        )
    });
    let dir = tempfile::tempdir().unwrap();
    let mut events: Vec<Value> = Vec::new();
    verbs::login(
        &TestForge::device(fake.base()),
        &Target::Host("forge.test".into()),
        Purpose::Write,
        &interactive(sandbox(dir.path())),
        &mut events,
        &NoWait::default(),
    );
    assert_eq!(
        events_of(&events, "error")[0]["code"],
        "device_flow_disabled"
    );
}

/// D3.11, layer 3: the agent image carries `joy forge login`, so the
/// connector refuses it too, instantly, and names the headless door.
#[test]
fn a_delegated_session_is_refused_before_anything_is_contacted() {
    let fake = FakeForge::start(|_| Reply::json(500, "{}"));
    let dir = tempfile::tempdir().unwrap();
    for kind in [HostKind::Background, HostKind::Delegated] {
        let mut ctx = sandbox(dir.path());
        ctx.host_kind = kind;
        let mut events: Vec<Value> = Vec::new();
        let started = std::time::Instant::now();
        verbs::login(
            &TestForge::device(fake.base()),
            &Target::Host("forge.test".into()),
            Purpose::Write,
            &ctx,
            &mut events,
            &NoWait::default(),
        );
        assert_eq!(events.len(), 1, "{kind:?}");
        assert_eq!(events[0]["event"], "error");
        let message = events[0]["message"].as_str().unwrap();
        assert!(message.contains("--token-stdin"), "{message}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
    assert!(fake.calls().is_empty(), "nothing is contacted at all");
}

/// D2.7: the Gitea family has no device grant in any released version,
/// so the door is the authorization code flow with PKCE S256 on a
/// loopback listener. The connector never opens a browser: this test IS
/// the browser.
#[test]
fn a_pkce_login_answers_the_loopback_redirect_and_exchanges_the_code() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let recorder = seen.clone();
    let fake = FakeForge::start(move |call| {
        if call.path.starts_with("/login/oauth/access_token") {
            recorder.lock().unwrap().push(call.body.clone());
            return Reply::json(
                200,
                r#"{"access_token":"gitea_token","expires_in":3600,"refresh_token":"rt"}"#,
            );
        }
        match call.path.as_str() {
            // No `X-OAuth-Scopes` here: the Gitea family's token answer
            // carries no scope field and its API does not introspect
            // one, which is the case D2.7c writes the rule for.
            "/user" => Reply::json(200, r#"{"login":"scotty","id":12345}"#),
            _ => Reply::not_found(),
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let base = fake.base();
    let root = dir.path().to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel::<Value>();

    // The login runs on its own thread: the "browser" has to answer the
    // loopback listener while the verb is still waiting for it.
    let handle = std::thread::spawn(move || {
        let forge = TestForge::pkce(base);
        let ctx = interactive(
            Ctx::bare(root.join("project"))
                .with_vault(Vault::file_at(root.join("config")))
                .with_state_dir(root.join("state")),
        );
        let mut sink = Channel(tx);
        verbs::login(
            &forge,
            &Target::Host("forge.test".into()),
            Purpose::Write,
            &ctx,
            &mut sink,
            &NoWait::default(),
        )
    });

    // The verification event carries the whole authorize URL, because
    // there is no user code to type in this flow.
    let verification = rx
        .recv_timeout(std::time::Duration::from_secs(15))
        .expect("the verification event");
    assert_eq!(verification["event"], "verification");
    assert!(verification["code"].is_null(), "PKCE has no user code");
    let url = verification["url"].as_str().unwrap().to_string();
    let query = url.split_once('?').unwrap().1;
    assert!(query.contains("code_challenge_method=S256"));
    assert!(query.contains("response_type=code"));
    let state = field(query, "state");
    let redirect = field(query, "redirect_uri");
    assert!(redirect.starts_with("http://127.0.0.1:"), "{redirect}");
    let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
    knock(port, &format!("/?code=the-code&state={state}"));

    assert_eq!(handle.join().unwrap(), 0);
    let events: Vec<Value> = rx.try_iter().collect();
    let result = &events_of(&events, "result")[0];
    assert_eq!(result["login"], "scotty");
    assert_eq!(result["stored"], "file");
    // Gitea's token answer has no scope field, so the set STORED is the
    // set requested (D2.7c).
    assert_eq!(result["scopes"], "repo user:email");

    let body = seen.lock().unwrap().join("\n");
    assert!(body.contains("code=the-code"), "{body}");
    assert!(body.contains("code_verifier="), "{body}");
    assert!(body.contains("grant_type=authorization_code"), "{body}");
    assert!(
        body.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"),
        "the exchange repeats the redirect URI: {body}"
    );
}

/// A redirect that does not carry this attempt's own state is a cross
/// site request, and the code is never spent on it.
#[test]
fn a_pkce_redirect_with_a_strange_state_is_refused() {
    let fake = FakeForge::start(|_| Reply::json(500, "{}"));
    let dir = tempfile::tempdir().unwrap();
    let base = fake.base();
    let root = dir.path().to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    let handle = std::thread::spawn(move || {
        let forge = TestForge::pkce(base);
        let ctx = interactive(
            Ctx::bare(root.join("project"))
                .with_vault(Vault::file_at(root.join("config")))
                .with_state_dir(root.join("state")),
        );
        let mut sink = Channel(tx);
        verbs::login(
            &forge,
            &Target::Host("forge.test".into()),
            Purpose::Write,
            &ctx,
            &mut sink,
            &NoWait::default(),
        )
    });
    let verification = rx
        .recv_timeout(std::time::Duration::from_secs(15))
        .expect("the verification event");
    let url = verification["url"].as_str().unwrap();
    let redirect = field(url.split_once('?').unwrap().1, "redirect_uri");
    let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
    knock(port, "/?code=the-code&state=somebody-elses");
    handle.join().unwrap();
    let events: Vec<Value> = rx.try_iter().collect();
    assert_eq!(events_of(&events, "error").len(), 1);
    assert!(events_of(&events, "result").is_empty());
    assert!(
        fake.calls().is_empty(),
        "a refused redirect never reaches the token endpoint"
    );
}

/// A sink that hands every event to the test as it is flushed, which is
/// what makes the loopback half of the PKCE flow drivable at all.
struct Channel(std::sync::mpsc::Sender<Value>);

impl Events for Channel {
    fn emit(&mut self, event: Value) {
        let _ = self.0.send(event);
    }
}

fn field(query: &str, name: &str) -> String {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .map(percent_decode)
        .unwrap_or_default()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Knock on the loopback listener the way a browser's redirect does.
fn knock(port: u16, path: &str) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("the listener");
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
}

// -- token-store, logout, the refresh lock and the probe -----------------------

/// D2.4: `token-store` reads ONE token, validates it with `identity`
/// and stores it, and then answers the same object as `token`. The
/// token is never an argument in either direction: it arrives as a
/// parameter here and on the child's stdin in the CLI.
#[test]
fn token_store_validates_the_token_with_the_forge_before_it_stores_it() {
    let fake = FakeForge::start(|call| match (call.method.as_str(), call.path.as_str()) {
        ("GET", "/user") => {
            if call.authorization() == Some("Bearer the-right-token") {
                user_reply()
            } else {
                Reply::json(401, r#"{"message":"Bad credentials"}"#)
            }
        }
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let ctx = sandbox(dir.path());
    let host = Target::Host("forge.test".into());

    let refused = verbs::store_token(&forge, &host, &ctx, "the-wrong-token");
    assert_eq!(refused["known"], false);
    assert_eq!(refused["reason"], "no-login");
    assert!(
        verbs::token(&forge, &host, &ctx)["known"] == false,
        "a token the forge refused is never stored"
    );

    // The trailing newline of a paste is not part of the token.
    let stored = verbs::store_token(&forge, &host, &ctx, "  the-right-token\n");
    assert_eq!(stored["known"], true);
    assert_eq!(stored["login"], "scotty");
    assert_eq!(stored["token"], "the-right-token");
    assert_eq!(stored["source"], "file");
    // The granted set the forge reported, beside the token in the same
    // entry and SPACE separated, whatever the forge's own spelling was
    // (D2.7c).
    assert_eq!(stored["scopes"], "repo user:email");
    assert_eq!(stored["chose_by"], "only");

    // and it is there for the next call
    let again = verbs::token(&forge, &host, &ctx);
    assert_eq!(again["token"], "the-right-token");
    assert_eq!(again["login"], "scotty");

    // The token travelled in a header and nowhere else.
    for call in fake.calls() {
        assert!(!call.path.contains("the-right-token"), "{}", call.path);
        assert!(!call.body.contains("the-right-token"), "{}", call.body);
    }
}

/// D2.4: `logout` removes the entry and revokes the token at the forge
/// where the forge offers it, with `DELETE /applications/{client_id}/token`
/// and never `.../grant`.
#[test]
fn logout_removes_the_entry_and_revokes_the_token_at_the_forge() {
    let fake = FakeForge::start(|call| {
        if call.method == "DELETE" && call.path.starts_with("/applications/") {
            return Reply::text(204, "");
        }
        Reply::not_found()
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let ctx = sandbox(dir.path());
    let host = Target::Host("forge.test".into());
    ctx.vault()
        .put(
            "forge.test",
            &Record {
                token: "gho_stored".into(),
                login: Some("scotty".into()),
                scopes: "repo user:email".into(),
                client_id: Some("test-client".into()),
                ..Record::default()
            },
        )
        .unwrap();

    let answer = verbs::logout(&forge, &host, &ctx);
    assert_eq!(answer["removed"], true);
    assert_eq!(answer["revoked"], true);
    assert_eq!(answer["source"], "file");
    assert_eq!(answer["login"], "scotty");
    assert!(
        fake.saw("DELETE", "/applications/test-client/token"),
        "the TOKEN is revoked, never the grant: {:?}",
        fake.calls()
            .iter()
            .map(|c| c.path.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        !fake
            .calls()
            .iter()
            .any(|call| call.path.ends_with("/grant")),
        "deleting the grant would kill every token of this app for the person"
    );
    assert_eq!(verbs::token(&forge, &host, &ctx)["known"], false);
}

/// D2.4, D2.6: a credential joy did not write is not joy's to remove.
/// `logout` names the foreign command instead.
#[test]
fn logout_names_the_foreign_command_for_a_foreign_credential() {
    let fake = FakeForge::start(|_| Reply::not_found());
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base()).with_foreign(&["scotty"]);
    let answer = verbs::logout(
        &forge,
        &Target::Host("forge.test".into()),
        &sandbox(dir.path()),
    );
    assert_eq!(answer["removed"], false);
    assert_eq!(answer["revoked"], false);
    assert_eq!(answer["source"], "gh");
    assert_eq!(answer["command"], "gh auth logout --hostname forge.test");
}

/// D2.6a: an expired token is refreshed exactly once, the whole answer
/// is written back (a forge that ROTATES its refresh token leaves no
/// stale one behind), and the second reader sees the new token without
/// a second refresh.
#[test]
fn an_expired_token_is_refreshed_once_and_the_rotation_is_written_back() {
    let refreshes = Arc::new(AtomicUsize::new(0));
    let counter = refreshes.clone();
    let fake = FakeForge::start(move |call| {
        if call.path == "/login/oauth/access_token" {
            let round = counter.fetch_add(1, Ordering::SeqCst);
            assert!(
                call.body.contains("grant_type=refresh_token"),
                "{}",
                call.body
            );
            assert!(call.body.contains("refresh_token=rt-old"), "{}", call.body);
            return Reply::json(
                200,
                format!(
                    r#"{{"access_token":"fresh-{round}","refresh_token":"rt-new-{round}",
                        "expires_in":3600,"scope":"repo,user:email"}}"#
                ),
            );
        }
        Reply::not_found()
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let ctx = sandbox(dir.path());
    let host = Target::Host("forge.test".into());
    ctx.vault()
        .put(
            "forge.test",
            &Record {
                token: "stale".into(),
                login: Some("scotty".into()),
                scopes: "repo".into(),
                expires_at: Some((chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339()),
                refresh_token: Some("rt-old".into()),
                token_endpoint: Some(format!("{}/login/oauth/access_token", fake.base())),
                client_id: Some("test-client".into()),
                ..Record::default()
            },
        )
        .unwrap();

    let first = verbs::token(&forge, &host, &ctx);
    assert_eq!(first["token"], "fresh-0");
    assert_eq!(first["scopes"], "repo user:email");
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);

    // The rotated refresh token replaced the old one, so a second
    // refresh would spend the NEW one and never the retired one.
    let (record, _) = ctx.vault().get("forge.test", Some("scotty")).unwrap();
    assert_eq!(record.refresh_token.as_deref(), Some("rt-new-0"));
    assert_eq!(record.token, "fresh-0");

    // A fresh context reads the stored token and refreshes nothing.
    let second = sandbox(dir.path());
    assert_eq!(verbs::token(&forge, &host, &second)["token"], "fresh-0");
    assert_eq!(
        refreshes.load(Ordering::SeqCst),
        1,
        "a token that is not expired is never refreshed"
    );
}

/// D2.6a: while another process holds the refresh lock, this one does
/// NOT refresh. It looks again and, finding nothing usable, answers
/// `busy`.
#[test]
fn a_held_refresh_lock_answers_busy_and_refreshes_nothing() {
    let refreshes = Arc::new(AtomicUsize::new(0));
    let counter = refreshes.clone();
    let fake = FakeForge::start(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Reply::json(200, r#"{"access_token":"should-not-happen"}"#)
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let ctx = sandbox(dir.path());
    let host = Target::Host("forge.test".into());
    ctx.vault()
        .put(
            "forge.test",
            &Record {
                token: "stale".into(),
                login: Some("scotty".into()),
                expires_at: Some((chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339()),
                refresh_token: Some("rt-old".into()),
                token_endpoint: Some(format!("{}/login/oauth/access_token", fake.base())),
                client_id: Some("test-client".into()),
                ..Record::default()
            },
        )
        .unwrap();

    // Somebody else holds it. flock keys on the open file description,
    // so a second handle in this process contends exactly as a second
    // process does.
    let path = joy_forge_net::auth::lock::lock_path(
        Some(&dir.path().join("state")),
        "forge.test",
        Some("scotty"),
    )
    .unwrap();
    let held = joy_forge_net::auth::lock::take_at(&path).expect("the first lock");

    let answer = verbs::token(&forge, &host, &ctx);
    assert_eq!(answer["known"], false);
    assert_eq!(answer["reason"], "busy");
    assert_eq!(
        refreshes.load(Ordering::SeqCst),
        0,
        "never refresh anyway (D2.6a)"
    );
    drop(held);
}

/// D4.1c: a host with two logins and a repository only the second can
/// reach. One probe per candidate, the answer says `"chose_by":"probe"`,
/// and the winner is remembered so the next call spends nothing.
#[test]
fn a_repository_only_the_second_login_reaches_is_chosen_by_the_probe() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/widgets" => {
            if call.authorization() == Some("Bearer token-of-work") {
                Reply::json(200, r#"{"permissions":{"push":true}}"#)
            } else {
                // GitHub answers 404 rather than 403 for a private
                // repository the caller may not see.
                Reply::not_found()
            }
        }
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let base = sandbox(dir.path());
    for (login, token) in [("scotty", "token-of-scotty"), ("work", "token-of-work")] {
        base.vault()
            .put(
                "forge.test",
                &Record {
                    token: token.into(),
                    login: Some(login.into()),
                    scopes: "repo user:email".into(),
                    ..Record::default()
                },
            )
            .unwrap();
    }
    let remote = "https://forge.test/acme/widgets.git";
    let ctx = sandbox(dir.path()).with_remote(remote);
    let answer = verbs::token(&forge, &Target::Remote(remote.into()), &ctx);
    assert_eq!(answer["known"], true);
    assert_eq!(answer["login"], "work");
    assert_eq!(answer["token"], "token-of-work");
    assert_eq!(answer["chose_by"], "probe");
    assert_eq!(fake.calls().len(), 2, "one request per candidate, no more");

    // The winner is remembered per normalised remote, so the next call
    // spends nothing at all (D4.1c).
    let next = sandbox(dir.path()).with_remote(remote);
    let again = verbs::token(&forge, &Target::Remote(remote.into()), &next);
    assert_eq!(again["login"], "work");
    assert_eq!(again["chose_by"], "memory");
    assert_eq!(fake.calls().len(), 2, "the memory spends no request");
}

/// D4.1c, step 5: when no login reaches the repository, the answer says
/// so and names the logins that were tried.
#[test]
fn no_login_that_reaches_the_repository_is_its_own_answer() {
    let fake = FakeForge::start(|_| Reply::not_found());
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let ctx = sandbox(dir.path());
    for login in ["scotty", "work"] {
        ctx.vault()
            .put(
                "forge.test",
                &Record {
                    token: format!("token-of-{login}"),
                    login: Some(login.into()),
                    ..Record::default()
                },
            )
            .unwrap();
    }
    let answer = verbs::token(
        &forge,
        &Target::Remote("https://forge.test/acme/widgets.git".into()),
        &ctx,
    );
    assert_eq!(answer["known"], false);
    assert_eq!(answer["reason"], "no-login-for-repo");
    let message = answer["message"].as_str().unwrap();
    assert!(message.contains("scotty"), "{message}");
    assert!(message.contains("acme/widgets"), "{message}");
}

/// D4.1c, step 1: the device local pin wins over everything and spends
/// no request.
#[test]
fn the_device_local_pin_decides_without_a_single_request() {
    let fake = FakeForge::start(|_| Reply::not_found());
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let ctx = sandbox(dir.path());
    for login in ["scotty", "work"] {
        ctx.vault()
            .put(
                "forge.test",
                &Record {
                    token: format!("token-of-{login}"),
                    login: Some(login.into()),
                    ..Record::default()
                },
            )
            .unwrap();
    }
    // The `--login` of the call is a pin the caller states directly.
    let pinned = ctx.with_login("scotty");
    let answer = verbs::token(&forge, &Target::Host("forge.test".into()), &pinned);
    assert_eq!(answer["login"], "scotty");
    assert_eq!(answer["token"], "token-of-scotty");
    assert_eq!(answer["chose_by"], "pin");
    assert!(fake.calls().is_empty(), "a pin costs no request");
}

/// D2.4: `web-url` is the twin source of D1.5, and only the connector
/// knows the web base of a self hosted instance.
#[test]
fn web_url_answers_the_https_twin_of_a_remote() {
    let fake = FakeForge::start(|_| Reply::not_found());
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let answer = verbs::web_url(
        &forge,
        &Target::Remote("git@forge.test:team/sub/repo.git".into()),
        &sandbox(dir.path()),
    );
    assert_eq!(answer["known"], true);
    assert_eq!(answer["https_url"], "https://forge.test/team/sub/repo.git");
    assert!(fake.calls().is_empty(), "the twin costs no request");
}
