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

/// J3's acceptance, on the store D2.6 calls the normal case: after a
/// successful login `token` answers `"source":"keychain"` with the
/// granted scopes, and the 0600 file is not written at all.
///
/// The store here is an in process one with an operating system's
/// semantics (it persists across `Entry` objects). A test must never
/// write into the person's own keychain, and on a machine with a real
/// Secret Service `Vault::real` would.
#[test]
fn a_login_stores_its_token_in_the_credential_store_and_token_reads_it_back() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/login/device/code" => Reply::json(
            200,
            r#"{"device_code":"dev-1","user_code":"WDJB-MJHT",
                "verification_uri":"https://forge.test/login/device",
                "expires_in":900,"interval":5}"#,
        ),
        "/login/oauth/access_token" => Reply::json(
            200,
            r#"{"access_token":"gho_in_the_keychain","token_type":"bearer",
                "scope":"repo,user:email"}"#,
        ),
        "/user" => user_reply(),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let vault = Vault::fake_keychain_at(dir.path().join("config"));
    let ctx = interactive(
        Ctx::bare(dir.path().join("project"))
            .with_vault(vault.clone())
            .with_state_dir(dir.path().join("state")),
    );
    let mut events: Vec<Value> = Vec::new();
    assert_eq!(
        verbs::login(
            &forge,
            &Target::Host("forge.test".into()),
            Purpose::Write,
            &ctx,
            &mut events,
            &NoWait::default(),
        ),
        0
    );
    let result = &events_of(&events, "result")[0];
    assert_eq!(result["stored"], "keychain", "{result}");
    assert_eq!(result["login"], "scotty");
    assert_eq!(result["scopes"], "repo user:email");

    let answer = verbs::token(&forge, &Target::Host("forge.test".into()), None, &ctx);
    assert_eq!(answer["source"], "keychain", "{answer}");
    assert_eq!(answer["token"], "gho_in_the_keychain");
    assert_eq!(answer["login"], "scotty");
    assert_eq!(answer["scopes"], "repo user:email");
    // Step 3 of D4.1c: the only login the host holds, and the list it
    // reads comes from the index entry, because `Entry::new` cannot
    // enumerate.
    assert_eq!(answer["chose_by"], "only");
    assert!(
        !vault.file().exists(),
        "a store that answered means no 0600 file at all"
    );

    // And `logout` takes it out of that same store.
    let out = verbs::logout(&forge, &Target::Host("forge.test".into()), &ctx);
    assert_eq!(out["removed"], true, "{out}");
    assert_eq!(out["source"], "keychain");
    assert!(vault.logins("forge.test").is_empty());
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
    let answer = verbs::token(&forge, &Target::Host("forge.test".into()), None, &ctx);
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

/// D2.4: "newline delimited JSON on stdout, one object per line, each
/// flushed", and that is why the `waiting` ticks have to leave the
/// connector WHILE it waits. A Gitea, Forgejo or Codeberg sign in waits
/// up to fifteen minutes for the person; a connector that collected the
/// ticks and printed them afterwards would be silent for all of it and
/// then replay nine hundred stale lines at once.
#[test]
fn a_pkce_login_reports_the_seconds_left_while_it_is_still_waiting() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        _ if call.path.starts_with("/login/oauth/access_token") => {
            Reply::json(200, r#"{"access_token":"gitea_token","expires_in":3600}"#)
        }
        "/user" => Reply::json(200, r#"{"login":"scotty","id":12345}"#),
        _ => Reply::not_found(),
    });
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
    let url = verification["url"].as_str().unwrap().to_string();
    let query = url.split_once('?').unwrap().1;
    let state = field(query, "state");
    let redirect = field(query, "redirect_uri");
    let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();

    // Nobody has opened the browser yet, and the connector is still
    // waiting. The next event has to arrive anyway: the tick is about
    // one a second.
    let ticked = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("a waiting event while the sign in is still running");
    assert_eq!(
        ticked["event"], "waiting",
        "the seconds left are reported during the wait, not after it"
    );
    assert!(ticked["seconds_left"].as_i64().unwrap() > 0, "{ticked:?}");

    knock(port, &format!("/?code=the-code&state={state}"));
    assert_eq!(handle.join().unwrap(), 0);
    let events: Vec<Value> = rx.try_iter().collect();
    assert_eq!(events_of(&events, "result").len(), 1);
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
        verbs::token(&forge, &host, None, &ctx)["known"] == false,
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
    let again = verbs::token(&forge, &host, None, &ctx);
    assert_eq!(again["token"], "the-right-token");
    assert_eq!(again["login"], "scotty");

    // The token travelled in a header and nowhere else.
    for call in fake.calls() {
        assert!(!call.path.contains("the-right-token"), "{}", call.path);
        assert!(!call.body.contains("the-right-token"), "{}", call.body);
    }
}

/// JOY-02A8-F4: "the forge refused this token" and "the forge could not
/// be reached" are two facts and get two answers. Collapsing them into
/// `no-login` told a person on a train that their credential had been
/// rejected and sent them, through `needs_sign_in`, at a door that
/// could not open either.
#[test]
fn a_forge_that_cannot_be_reached_is_not_a_forge_that_refused_the_token() {
    // A base nobody listens on: the fake is started for its address and
    // stopped again, so the connection is REFUSED rather than hanging
    // and the case stays as fast as every other one here.
    let base = {
        let fake = FakeForge::start(|_| Reply::not_found());
        fake.base()
    };
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(base);
    let ctx = sandbox(dir.path());
    let host = Target::Host("forge.test".into());

    let answer = verbs::store_token(&forge, &host, &ctx, "the-right-token");
    assert_eq!(answer["known"], false);
    assert_eq!(
        answer["reason"], "offline",
        "an unreachable forge is not a refused token: {answer}"
    );
    let message = answer["message"]
        .as_str()
        .expect("the person gets a sentence, not a bare reason");
    assert!(message.contains("forge.test"), "{message}");
    assert!(message.contains("could not be reached"), "{message}");
    assert!(
        !message.contains("did not accept"),
        "nothing accepted or refused anything: {message}"
    );
    // The token is not stored either: a credential that was never
    // checked is not a credential this machine may keep.
    assert_eq!(verbs::token(&forge, &host, None, &ctx)["known"], false);
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
    assert_eq!(verbs::token(&forge, &host, None, &ctx)["known"], false);
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

/// D2.6a, verbatim: "the lock lives in the plugin and is taken by every
/// `token`, `login`, `token-store` and `logout` call that may write".
///
/// `logout` writes: it deletes the entry. Without the lock a refresh
/// running beside it re reads the entry under its own lock, sees the
/// record this call is about to delete, renews it and writes it back,
/// and the credential the person just revoked is alive again.
#[test]
fn logout_takes_the_refresh_lock_and_never_deletes_beside_a_refresh() {
    let fake = FakeForge::start(|_| Reply::text(204, ""));
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
                client_id: Some("test-client".into()),
                ..Record::default()
            },
        )
        .unwrap();

    // Somebody else holds the lock of exactly this host and login.
    // flock keys on the open file description, so a second handle in
    // this process contends exactly as a second process does.
    let path = joy_forge_net::auth::lock::lock_path(
        Some(&dir.path().join("state")),
        "forge.test",
        Some("scotty"),
    )
    .unwrap();
    let held = joy_forge_net::auth::lock::take_at(&path).expect("the first lock");

    let answer = verbs::logout(&forge, &host, &ctx);
    assert_eq!(answer["removed"], false);
    assert_eq!(answer["revoked"], false);
    assert_eq!(answer["reason"], "busy");
    assert!(
        fake.calls().is_empty(),
        "nothing is revoked either: the entry was never read"
    );
    // and the credential is still exactly where it was
    assert_eq!(
        verbs::token(&forge, &host, None, &ctx)["token"],
        "gho_stored"
    );
    drop(held);

    // With the lock free the same call removes it.
    let answer = verbs::logout(&forge, &host, &ctx);
    assert_eq!(answer["removed"], true);
    assert_eq!(answer["login"], "scotty");
    assert!(path.exists(), "the lock file is never unlinked");
}

/// D2.4: `logout` on a host that holds two of joy's OWN logins removes
/// nothing and names them. Naming gh's command there would be a lie
/// twice over: joy holds these credentials, and which one to sign out
/// is not a thing this call may guess at.
#[test]
fn logout_on_a_host_with_two_logins_asks_which_one() {
    let fake = FakeForge::start(|_| Reply::text(204, ""));
    let dir = tempfile::tempdir().unwrap();
    // gh is signed in here too, so the foreign branch is reachable and
    // this proves the answer is not it.
    let forge = TestForge::device(fake.base()).with_foreign(&["ci"]);
    let ctx = sandbox(dir.path());
    let host = Target::Host("forge.test".into());
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

    let answer = verbs::logout(&forge, &host, &ctx);
    assert_eq!(answer["removed"], false);
    assert_eq!(answer["revoked"], false);
    assert!(answer["source"].is_null(), "{answer}");
    assert!(answer.get("command").is_none(), "this is not gh's business");
    let message = answer["message"].as_str().unwrap();
    assert!(message.contains("scotty, work"), "{message}");
    assert!(message.contains("--login"), "{message}");
    assert_eq!(answer["logins"], json!(["scotty", "work"]));
    // and nothing was removed
    assert_eq!(ctx.vault().logins("forge.test").len(), 2);

    // Named, it removes exactly the one named.
    let named = sandbox(dir.path()).with_login("work");
    let answer = verbs::logout(&forge, &host, &named);
    assert_eq!(answer["removed"], true);
    assert_eq!(answer["login"], "work");
    assert_eq!(ctx.vault().logins("forge.test"), vec!["scotty".to_string()]);
}

/// J3's acceptance is "`logout` removes the entry". A state directory
/// that could not be written is a refusal and never a reported success:
/// answering `"removed": true` while the token is still in the file is
/// the one thing this verb must not do.
#[test]
fn logout_that_could_not_write_the_file_does_not_report_success() {
    let fake = FakeForge::start(|_| Reply::text(204, ""));
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
                client_id: Some("test-client".into()),
                ..Record::default()
            },
        )
        .unwrap();
    // The document is written through a staging file and a rename;
    // something else in the staging file's place makes the write fail
    // the way a full or read only directory does.
    let mut staging = ctx.vault().file().to_path_buf().into_os_string();
    staging.push(".new");
    std::fs::create_dir(&staging).unwrap();

    let answer = verbs::logout(&forge, &host, &ctx);
    assert_eq!(
        answer["removed"], false,
        "the entry is still there, so the answer says so: {answer}"
    );
    assert_eq!(answer["revoked"], true, "the forge revocation did happen");
    let message = answer["message"].as_str().unwrap();
    assert!(message.contains("forge-tokens.json"), "{message}");
    assert!(!message.contains("gho_stored"), "no secret in a message");
    assert_eq!(
        verbs::token(&forge, &host, None, &ctx)["token"],
        "gho_stored"
    );
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

    let first = verbs::token(&forge, &host, None, &ctx);
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
    assert_eq!(
        verbs::token(&forge, &host, None, &second)["token"],
        "fresh-0"
    );
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

    let answer = verbs::token(&forge, &host, None, &ctx);
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
    let answer = verbs::token(&forge, &Target::Remote(remote.into()), None, &ctx);
    assert_eq!(answer["known"], true);
    assert_eq!(answer["login"], "work");
    assert_eq!(answer["token"], "token-of-work");
    assert_eq!(answer["chose_by"], "probe");
    assert_eq!(fake.calls().len(), 2, "one request per candidate, no more");

    // The winner is remembered per normalised remote, so the next call
    // spends nothing at all (D4.1c).
    let next = sandbox(dir.path()).with_remote(remote);
    let again = verbs::token(&forge, &Target::Remote(remote.into()), None, &next);
    assert_eq!(again["login"], "work");
    assert_eq!(again["chose_by"], "memory");
    assert_eq!(fake.calls().len(), 2, "the memory spends no request");
}

/// D4.1c, step 4, verbatim: "the first that answers 200, and for a push
/// direction reports write, wins".
///
/// This is the section's own "This is not academic" case: a private
/// repository where the login the probe reaches first holds Reporter
/// rights and the other holds Developer rights. The first one answers
/// 200, so a probe that reads nothing but the status hands the push to
/// the account that cannot push, and the push then fails 403 under the
/// wrong login.
#[test]
fn a_push_direction_refuses_a_login_that_can_only_read() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/widgets" => match call.authorization() {
            // Reporter: sees the repository, may not push.
            Some("Bearer token-of-scotty") => Reply::json(200, r#"{"permissions":{"push":false}}"#),
            // Developer: both.
            Some("Bearer token-of-work") => Reply::json(200, r#"{"permissions":{"push":true}}"#),
            _ => Reply::not_found(),
        },
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let base = sandbox(dir.path());
    for login in ["scotty", "work"] {
        base.vault()
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
    // `scotty` is asked first, so a probe that stops at the first 200
    // picks the login that cannot push.
    let remote = Target::Remote("https://forge.test/acme/widgets.git".into());

    let pushing = verbs::token(&forge, &remote, Some(Purpose::Write), &sandbox(dir.path()));
    assert_eq!(pushing["login"], "work", "{pushing}");
    assert_eq!(pushing["token"], "token-of-work");
    assert_eq!(pushing["chose_by"], "probe");

    // `--for create` and `--for release` push too.
    let creating = verbs::token(&forge, &remote, Some(Purpose::Create), &sandbox(dir.path()));
    assert_eq!(creating["login"], "work", "{creating}");

    // Reading is a different question, and `scotty` answers it first.
    let reading = verbs::token(&forge, &remote, Some(Purpose::Read), &sandbox(dir.path()));
    assert_eq!(reading["login"], "scotty", "{reading}");

    // No direction stated: a login that can push outranks one that can
    // only read, and a caller that asked for nothing is never refused a
    // credential that would have worked.
    let neither = verbs::token(&forge, &remote, None, &sandbox(dir.path()));
    assert_eq!(neither["login"], "work", "{neither}");
}

/// D4.1c is the order for a REMOTE, and `token` is not the only verb
/// that needs one: `store`, `files`, `release`, `repositories` and
/// `create-repository` all reach their credential through `Ctx::token`.
/// A host with two logins, no pin and no memory is decided by the probe
/// there too, or `joy release publish` publishes under whichever
/// account the forge CLI last switched to.
#[test]
fn every_verb_reaches_its_credential_through_the_whole_login_order() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/widgets" => match call.authorization() {
            Some("Bearer token-of-scotty") => Reply::json(200, r#"{"permissions":{"push":false}}"#),
            Some("Bearer token-of-work") => Reply::json(200, r#"{"permissions":{"push":true}}"#),
            _ => Reply::not_found(),
        },
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    // A forge lives as long as the binary does, which is what lets the
    // context carry the one that answers.
    let forge: &'static TestForge = Box::leak(Box::new(TestForge::device(fake.base())));
    let filling = sandbox(dir.path());
    for login in ["scotty", "work"] {
        filling
            .vault()
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
    let remote = "https://forge.test/acme/widgets.git";
    let ctx = sandbox(dir.path()).with_remote(remote).with_forge(forge);

    // This is the call every verb makes.
    assert_eq!(
        ctx.token("github", "forge.test").as_deref(),
        Some("token-of-work"),
        "the verb takes the login that can push, not the first one stored"
    );
    let spent = fake.calls().len();
    assert_eq!(spent, 2, "one request per candidate");
    // One probe per remote, never one per contact (D4.1c): the same
    // question inside the same call spends nothing.
    assert_eq!(
        ctx.token("github", "forge.test").as_deref(),
        Some("token-of-work")
    );
    assert_eq!(fake.calls().len(), spent);
    assert_eq!(
        joy_forge_net::auth::pin::remembered(Some(&dir.path().join("state")), remote).as_deref(),
        Some("work"),
        "and the winner is remembered, so the next call spends nothing either"
    );
}

/// D4.1c, step 5, for a push: a repository every login can read and
/// none can push to is not "cannot reach", and the sentence says which
/// it is.
#[test]
fn a_repository_nobody_may_push_to_says_that_and_not_something_else() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/widgets" => Reply::json(200, r#"{"permissions":{"push":false}}"#),
        _ => Reply::not_found(),
    });
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
        Some(Purpose::Write),
        &ctx,
    );
    assert_eq!(answer["reason"], "no-login-for-repo");
    let message = answer["message"].as_str().unwrap();
    assert!(message.contains("can push to acme/widgets"), "{message}");
}

/// D1.8a: the classifier reads the evidence, and joy does not destroy
/// it. A forge that could not be ASKED has told joy nothing about any
/// login, so "none of your logins can reach this repository" is a claim
/// this call has no ground for, and the login memory of the remote is
/// the answer the next online call would get for free.
#[test]
fn a_forge_that_cannot_be_reached_is_not_a_login_that_cannot_reach_it() {
    let dir = tempfile::tempdir().unwrap();
    // Port 1 on the loopback interface refuses at once: this is the
    // laptop off the network, with no real network touched.
    let forge = TestForge::device("http://127.0.0.1:1");
    let state = dir.path().join("state");
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
    let remote = "https://forge.test/acme/widgets.git";
    // What this machine learned when it was online, for a login whose
    // entry it cannot read right now.
    joy_forge_net::auth::pin::remember(Some(&state), remote, "ci");

    let answer = verbs::token(&forge, &Target::Remote(remote.into()), None, &ctx);
    assert_ne!(
        answer["reason"], "no-login-for-repo",
        "a forge that did not answer is not a login that cannot reach it: {answer}"
    );
    if answer["reason"] == "no-login" {
        let message = answer["message"].as_str().unwrap_or_default();
        assert!(message.contains("could not ask"), "{message}");
        assert!(message.contains("scotty, work"), "{message}");
    }
    assert_eq!(
        joy_forge_net::auth::pin::remembered(Some(&state), remote).as_deref(),
        Some("ci"),
        "the memory is evidence, and nothing here learned it was wrong"
    );
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
        None,
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
    let answer = verbs::token(&forge, &Target::Host("forge.test".into()), None, &pinned);
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

/// G2 and D3.8: a delegated agent inherits everything through the joy
/// CLI, so it READS the credential the person stored and it changes
/// nothing of it. It stores nothing, it signs nothing out, and where a
/// refresh would be needed it answers with the sentence that names who
/// has to sign in (D1.10: a delegated host never prompts).
///
/// The vault here is the person's own file with the flag `Ctx::new`
/// sets for a `Delegated` host; the wiring from the host kind to the
/// flag is `a_delegated_call_gets_the_persons_vault_read_only` in the
/// crate's own tests, because it needs the real store.
#[test]
fn a_delegated_session_reads_the_stored_credential_and_never_changes_it() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let fake = FakeForge::start(move |call| {
        counter.fetch_add(1, Ordering::SeqCst);
        match call.path.as_str() {
            "/user" => user_reply(),
            _ => Reply::json(200, r#"{"access_token":"renewed","expires_in":3600}"#),
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let host = Target::Host("forge.test".into());

    // what the person stored on this machine, on their own session
    let person = sandbox(dir.path());
    person
        .vault()
        .put(
            "forge.test",
            &Record {
                token: "gho_the_persons_token".into(),
                login: Some("scotty".into()),
                scopes: "repo".into(),
                refresh_token: Some("rt-the-persons".into()),
                token_endpoint: Some(format!("{}/login/oauth/access_token", fake.base())),
                client_id: Some("test-client".into()),
                ..Record::default()
            },
        )
        .unwrap();

    let mut agent = sandbox(dir.path());
    agent.host_kind = HostKind::Delegated;
    let agent = agent.with_vault(Vault::file_at(dir.path().join("config")).read_only());

    // it READS: this is the whole point, and refusing it here would
    // refuse the agent every contact the person can make
    let answer = verbs::token(&forge, &host, None, &agent);
    assert_eq!(answer["known"], true, "{answer}");
    assert_eq!(answer["token"], "gho_the_persons_token");
    assert_eq!(answer["login"], "scotty");

    // it stores nothing, and the token it was handed is never spent on
    // a request either
    let before = std::fs::read_to_string(person.vault().file()).unwrap();
    let stored = verbs::store_token(&forge, &host, &agent, "gho_a_token_of_its_own");
    assert_eq!(stored["known"], false, "{stored}");
    assert!(
        stored["message"]
            .as_str()
            .unwrap()
            .contains("delegation session"),
        "{stored}"
    );
    assert_eq!(
        std::fs::read_to_string(person.vault().file()).unwrap(),
        before
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing was contacted");

    // it signs nothing out: this verb revokes at the forge first, so a
    // delegated session that reached it would sign the person out of
    // their own machine
    let out = verbs::logout(&forge, &host, &agent);
    assert_eq!(out["removed"], false, "{out}");
    assert_eq!(out["revoked"], false, "{out}");
    assert!(
        out["message"]
            .as_str()
            .unwrap()
            .contains("joy forge logout"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(person.vault().file()).unwrap(),
        before
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing was contacted");

    // and it never refreshes: the person's refresh token is rotated by
    // the forge on use, so renewing it here would retire the one the
    // person holds. The answer says so instead.
    let expired = Record {
        token: "gho_the_persons_token".into(),
        login: Some("scotty".into()),
        scopes: "repo".into(),
        expires_at: Some((chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339()),
        refresh_token: Some("rt-the-persons".into()),
        token_endpoint: Some(format!("{}/login/oauth/access_token", fake.base())),
        client_id: Some("test-client".into()),
        ..Record::default()
    };
    person.vault().put("forge.test", &expired).unwrap();
    let stale = verbs::token(&forge, &host, None, &agent);
    assert_eq!(stale["known"], false, "{stale}");
    assert_eq!(stale["reason"], "expired", "{stale}");
    let message = stale["message"].as_str().unwrap();
    assert!(
        message.contains("joy forge login --host forge.test"),
        "{message}"
    );
    assert!(message.contains("sign in again"), "{message}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no refresh was spent");
    // the record the person holds is untouched, refresh token included
    let (still, _) = person.vault().get("forge.test", Some("scotty")).unwrap();
    assert_eq!(still.token, "gho_the_persons_token");
    assert_eq!(still.refresh_token.as_deref(), Some("rt-the-persons"));

    // the person's own session still refreshes, so the rule is the
    // delegated host's and not everybody's
    assert_eq!(
        verbs::token(&forge, &host, None, &person)["token"],
        "renewed",
        "the person's own session renews what it holds"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// -- the organisation wall of D2.7c (JOY-02A9-48) ------------------------------

/// The body GitHub writes when an OAuth application is not approved for
/// an organisation. It is the cheap reading of D2.7c: the forge names
/// the restriction itself and joy spends no second request on it.
const RESTRICTED: &str = r#"{"message":"Although you appear to have the correct authorization credentials, the `acme` organization has enabled OAuth App access restrictions, meaning that data access to third-parties is limited."}"#;

fn two_logins(ctx: &Ctx) {
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
}

/// JOY-02A9-48, finding 2: a 403 that names OAuth App access
/// restrictions is the ORGANISATION's wall and not "this login is not
/// the one". Telling a person to "sign in with the login that can"
/// sends them round a door that refuses every account they have, while
/// the one action that helps belongs to an owner of the organisation,
/// on one page (D2.7c).
#[test]
fn a_403_that_names_the_restriction_answers_needs_org_approval() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/widgets" => Reply::json(403, RESTRICTED),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base()).with_org_walls();
    let ctx = sandbox(dir.path());
    two_logins(&ctx);

    let answer = verbs::token(
        &forge,
        &Target::Remote("https://forge.test/acme/widgets.git".into()),
        None,
        &ctx,
    );

    assert_eq!(answer["known"], false, "{answer}");
    assert_eq!(answer["reason"], "needs_org_approval", "{answer}");
    let message = answer["message"].as_str().unwrap();
    assert!(
        message.starts_with("Your organisation must approve Joy for this repository."),
        "{message}"
    );
    assert_eq!(
        answer["action"], "https://forge.test/organizations/acme/settings/oauth_application_policy",
        "the page an owner acts on: {answer}"
    );
    assert_eq!(
        fake.calls().len(),
        2,
        "the forge named the restriction itself, so nothing else is asked: {:#?}",
        fake.calls()
    );
}

/// The second reading of D2.7c, and the one the operator's own probe
/// met: GitHub answers 404 rather than 403 for a private repository a
/// token may not see. A 404 for a repository owned by an organisation
/// the token's user BELONGS to is the same wall, and the organisation
/// list is what says so.
#[test]
fn a_404_on_a_repository_of_my_own_organisation_is_the_wall() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/user/orgs" => Reply::json(200, r#"[{"login":"acme"},{"login":"other"}]"#),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base()).with_org_walls();
    let ctx = sandbox(dir.path());
    two_logins(&ctx);

    let answer = verbs::token(
        &forge,
        &Target::Remote("https://forge.test/acme/widgets.git".into()),
        None,
        &ctx,
    );

    assert_eq!(answer["reason"], "needs_org_approval", "{answer}");
    assert_eq!(
        answer["action"],
        "https://forge.test/organizations/acme/settings/oauth_application_policy"
    );
    let asked: Vec<String> = fake.calls().iter().map(|call| call.path.clone()).collect();
    assert_eq!(
        asked,
        vec!["/repos/acme/widgets", "/repos/acme/widgets", "/user/orgs"],
        "one request per candidate, and the wall question asked ONCE, at the end"
    );
}

/// And joy invents no wall: a 404 for a repository of an organisation
/// the token's user does not belong to is a repository that was
/// renamed, deleted or never visible to these logins, which is D4.1c's
/// step 5 and says so.
#[test]
fn a_404_outside_my_organisations_is_still_no_login_for_repo() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/user/orgs" => Reply::json(200, r#"[{"login":"other"}]"#),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base()).with_org_walls();
    let ctx = sandbox(dir.path());
    two_logins(&ctx);

    let answer = verbs::token(
        &forge,
        &Target::Remote("https://forge.test/acme/widgets.git".into()),
        None,
        &ctx,
    );

    assert_eq!(answer["reason"], "no-login-for-repo", "{answer}");
    assert!(answer.get("action").is_none(), "{answer}");
}

/// JOY-02A9-48, finding 2, second half: whether a person is told about
/// the wall may not depend on a memory row. The pin and the memory of
/// D4.1c choose the LOGIN without spending a request, which is what
/// they are for; they know nothing about the repository, and a call
/// that skipped the question entirely answered "here is your token" for
/// a repository the token cannot reach.
#[test]
fn the_wall_is_answered_whether_a_pin_a_memory_or_the_probe_chose_the_login() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/widgets" => Reply::json(403, RESTRICTED),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base()).with_org_walls();
    let remote = "https://forge.test/acme/widgets.git";
    let state = dir.path().join("state");

    // The memory: this machine pushed to this remote as `work` before.
    let by_memory = sandbox(dir.path()).with_remote(remote);
    two_logins(&by_memory);
    joy_forge_net::auth::pin::remember(Some(&state), remote, "work");
    let answer = verbs::token(&forge, &Target::Remote(remote.into()), None, &by_memory);
    assert_eq!(answer["reason"], "needs_org_approval", "{answer}");
    assert!(
        answer["token"].is_null(),
        "no token is handed out: {answer}"
    );
    assert_eq!(
        fake.calls().len(),
        1,
        "the memory chose the login and one question settled the repository: {:#?}",
        fake.calls()
    );

    // The pin: the caller named the login itself.
    let by_pin = sandbox(dir.path()).with_remote(remote).with_login("scotty");
    let answer = verbs::token(&forge, &Target::Remote(remote.into()), None, &by_pin);
    assert_eq!(answer["reason"], "needs_org_approval", "{answer}");
    assert_eq!(answer["chose_by"], serde_json::Value::Null);

    // The only login this machine holds, which is step 3 and also spends
    // no request on the choice.
    let alone = tempfile::tempdir().unwrap();
    let only = sandbox(alone.path());
    only.vault()
        .put(
            "forge.test",
            &Record {
                token: "token-of-scotty".into(),
                login: Some("scotty".into()),
                ..Record::default()
            },
        )
        .unwrap();
    let answer = verbs::token(&forge, &Target::Remote(remote.into()), None, &only);
    assert_eq!(answer["reason"], "needs_org_approval", "{answer}");
}

/// A login that reaches the repository is not asked anything else: the
/// memory keeps its promise of D4.1c and the call spends one request,
/// not two.
#[test]
fn a_remembered_login_that_reaches_the_repository_still_answers_at_once() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/repos/acme/widgets" => Reply::json(200, r#"{"permissions":{"push":true}}"#),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base()).with_org_walls();
    let remote = "https://forge.test/acme/widgets.git";
    let ctx = sandbox(dir.path()).with_remote(remote);
    two_logins(&ctx);
    joy_forge_net::auth::pin::remember(Some(&dir.path().join("state")), remote, "work");

    let answer = verbs::token(&forge, &Target::Remote(remote.into()), None, &ctx);

    assert_eq!(answer["known"], true, "{answer}");
    assert_eq!(answer["login"], "work");
    assert_eq!(answer["chose_by"], "memory");
    assert_eq!(fake.calls().len(), 1, "{:#?}", fake.calls());
}

// -- transient transport failures while polling (JOY-02A9-48) ------------------

/// The device code answer every case below starts from.
fn device_code(expires_in: i64, interval: i64) -> Reply {
    Reply::json(
        200,
        format!(
            r#"{{"device_code":"dev-1","user_code":"WDJB-MJHT",
                "verification_uri":"https://forge.test/login/device",
                "expires_in":{expires_in},"interval":{interval}}}"#
        ),
    )
}

/// JOY-02A9-48, finding 3: a DNS hiccup while the person is still on
/// the forge's page is a wait, not an end. The poll that got no answer
/// says what it is waiting out and is tried again until the code
/// expires; the sign in then finishes as if nothing had happened.
#[test]
fn a_device_poll_rides_out_a_transport_failure_and_still_signs_in() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let fake = FakeForge::start(move |call| match call.path.as_str() {
        "/login/device/code" => device_code(900, 5),
        "/login/oauth/access_token" => match counter.fetch_add(1, Ordering::SeqCst) {
            // the name did not resolve, twice
            0 | 1 => Reply::hang_up(),
            2 => Reply::json(200, r#"{"error":"authorization_pending"}"#),
            _ => Reply::json(
                200,
                r#"{"access_token":"gho_after_the_hiccup","token_type":"bearer","scope":"repo"}"#,
            ),
        },
        "/user" => user_reply(),
        _ => Reply::not_found(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let ctx = interactive(sandbox(dir.path()));
    let mut events: Vec<Value> = Vec::new();

    let code = verbs::login(
        &forge,
        &Target::Host("forge.test".into()),
        Purpose::Write,
        &ctx,
        &mut events,
        &NoWait::default(),
    );

    assert_eq!(code, 0);
    assert!(
        events_of(&events, "error").is_empty(),
        "nothing failed here: {events:#?}"
    );
    let result = &events_of(&events, "result")[0];
    assert_eq!(result["login"], "scotty");
    assert_eq!(polls.load(Ordering::SeqCst), 4, "every poll was made");
    let waiting = events_of(&events, "waiting");
    assert_eq!(waiting.len(), 3, "two rode out the fault, one was pending");
    let reasons: Vec<&str> = waiting
        .iter()
        .filter_map(|event| event["reason"].as_str())
        .collect();
    assert_eq!(reasons.len(), 2, "the two faults said what they waited out");
    assert!(
        reasons[0].contains("could not be reached"),
        "and the reason is the client's own sentence: {reasons:?}"
    );
    // The countdown is the code's real life and not a count of
    // intervals: 900 seconds, five at a time.
    assert_eq!(waiting[0]["seconds_left"], 895);
    assert_eq!(waiting[1]["seconds_left"], 890);
    assert_eq!(waiting[2]["seconds_left"], 885);
}

/// The other half of the rule: `network` is reported only where the
/// code ran out without ONE answer from the forge. Then it is the
/// truth, and it names the host.
#[test]
fn a_device_poll_that_never_reached_the_forge_reports_network_when_the_code_expires() {
    let fake = FakeForge::start(|call| match call.path.as_str() {
        "/login/device/code" => device_code(30, 5),
        _ => Reply::hang_up(),
    });
    let dir = tempfile::tempdir().unwrap();
    let forge = TestForge::device(fake.base());
    let mut events: Vec<Value> = Vec::new();
    let clock = NoWait::default();

    verbs::login(
        &forge,
        &Target::Host("forge.test".into()),
        Purpose::Write,
        &interactive(sandbox(tempfile::tempdir().unwrap().path())),
        &mut events,
        &clock,
    );

    let error = &events_of(&events, "error")[0];
    assert_eq!(error["code"], "network", "{error}");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("forge.test"), "{message}");
    assert!(message.contains("waited for the sign in"), "{message}");
    assert_eq!(
        clock.waits().len(),
        6,
        "it kept trying for the whole life of the code"
    );
    let _ = fake.calls();
    let _ = dir;
}

/// And a code that really expired says so, even when the last polls
/// found nothing: the forge answered once, so the person's code running
/// out is the story and "no connection" would be a second, wrong one.
#[test]
fn a_code_that_expired_after_one_answer_is_expired_and_not_a_network_fault() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let fake = FakeForge::start(move |call| match call.path.as_str() {
        "/login/device/code" => device_code(20, 5),
        "/login/oauth/access_token" => match counter.fetch_add(1, Ordering::SeqCst) {
            0 => Reply::json(200, r#"{"error":"authorization_pending"}"#),
            _ => Reply::hang_up(),
        },
        _ => Reply::not_found(),
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

    let error = &events_of(&events, "error")[0];
    assert_eq!(error["code"], "expired_token", "{error}");
    assert_eq!(polls.load(Ordering::SeqCst), 4);
}

/// The same rule at the PKCE exchange (JOY-02A9-48, finding 3): the
/// person has approved joy in their browser by then, and throwing the
/// authorization code away for a DNS hiccup makes them do the whole
/// flow again. The exchange is retried inside the same window, and the
/// wait says what it is waiting out.
#[test]
fn a_pkce_exchange_rides_out_a_transport_failure_and_still_signs_in() {
    let tries = Arc::new(AtomicUsize::new(0));
    let counter = tries.clone();
    let fake = FakeForge::start(move |call| {
        if call.path.starts_with("/login/oauth/access_token") {
            return match counter.fetch_add(1, Ordering::SeqCst) {
                0 | 1 => Reply::hang_up(),
                _ => Reply::json(200, r#"{"access_token":"gitea_token","expires_in":3600}"#),
            };
        }
        match call.path.as_str() {
            "/user" => Reply::json(200, r#"{"login":"scotty","id":12345}"#),
            _ => Reply::not_found(),
        }
    });
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
    let url = verification["url"].as_str().unwrap().to_string();
    let query = url.split_once('?').unwrap().1;
    let state = field(query, "state");
    let redirect = field(query, "redirect_uri");
    let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
    knock(port, &format!("/?code=the-code&state={state}"));

    assert_eq!(handle.join().unwrap(), 0);
    let events: Vec<Value> = rx.try_iter().collect();
    assert!(
        events_of(&events, "error").is_empty(),
        "the code survived the two faults: {events:#?}"
    );
    assert_eq!(events_of(&events, "result")[0]["login"], "scotty");
    assert_eq!(
        tries.load(Ordering::SeqCst),
        3,
        "two faults, then the token"
    );
    let reasons: Vec<String> = events_of(&events, "waiting")
        .iter()
        .filter_map(|event| event["reason"].as_str().map(str::to_string))
        .collect();
    assert_eq!(reasons.len(), 2, "each attempt said what it waited out");
    assert!(reasons[0].contains("could not be reached"), "{reasons:?}");
}
