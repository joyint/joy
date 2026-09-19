// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The contact budget of D1.9 (JOY-0295-36), measured in HTTP REQUESTS.
//!
//! The acceptance criterion of package J5 is written in requests per
//! host, not in contacts, because a contact is not a unit: the first
//! request of every new connection to a private repository carries no
//! `Authorization` header and is answered 401, and the credential
//! callback then replays it (libgit2 httpclient.c:566-568). One fetch
//! used to cost FIVE requests, because joy dropped the `RemoteConnection`
//! before `remote.download` and libgit2 reconnected and paid the 401
//! challenge a second time.
//!
//! This test is the counting proxy the criterion names, moved into the
//! process: a minimal git smart-HTTP server on the loopback interface
//! that demands Basic authentication and counts every request it
//! answers. It needs no network and no forge.
//!
//! Two notes on how literally to read it. The criterion says "a private
//! https remote" and the server below speaks plain http on 127.0.0.1:
//! the request count is what is measured and TLS adds no HTTP request,
//! but the handshake is not exercised here. And the gap table is
//! process-wide state, so both tests take [`SERIAL`] before they touch
//! it; cargo runs them on two threads in one binary, and without the
//! lock one test could silently pay the other's gap.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use joy_core::vcs::contact;
use joy_core::vcs::forge::{fetch_ref, Auth};
use joy_core::vcs::resolver;

const CHATS_REF: &str = "refs/joy/chats";

/// The gap table joy-core keeps is process-wide: the tests that set it
/// run one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// joy's own state file, inside this case's temporary directory. A
/// contact that authenticates writes down that a credential worked on
/// the host (D1.8b, JOY-02A9-48), and a test must never write that into
/// the person's own state directory.
fn state_file_in(tmp: &std::path::Path) {
    resolver::set_state_file(Some(tmp.join("forge-state.json")));
}

/// One HTTP request the server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    verb: String,
    path: String,
    authenticated: bool,
    status: u16,
}

struct Server {
    port: u16,
    seen: Arc<std::sync::Mutex<Vec<Request>>>,
    requests: Arc<AtomicUsize>,
}

/// A pkt-line: four hex digits of total length, then the payload.
fn pkt(line: &str) -> Vec<u8> {
    format!("{:04x}{line}", line.len() + 4).into_bytes()
}

/// The advertisement `GET /info/refs?service=git-upload-pack` returns.
fn advertisement(refs: &[(String, git2::Oid)]) -> Vec<u8> {
    let mut body = pkt("# service=git-upload-pack\n");
    body.extend_from_slice(b"0000");
    for (i, (name, oid)) in refs.iter().enumerate() {
        let line = if i == 0 {
            format!("{oid} {name}\0agent=joy-test\n")
        } else {
            format!("{oid} {name}\n")
        };
        body.extend_from_slice(&pkt(&line));
    }
    body.extend_from_slice(b"0000");
    body
}

/// Every object of the bare repository in one packfile: the test serves
/// the whole repository whatever the client asked for, which is enough
/// for a fetch of its only history.
fn packfile(forge: &std::path::Path, tip: git2::Oid) -> Vec<u8> {
    let repo = git2::Repository::open_bare(forge).expect("bare forge");
    let mut builder = repo.packbuilder().expect("packbuilder");
    builder.insert_commit(tip).expect("insert commit");
    let mut buf = git2::Buf::new();
    builder.write_buf(&mut buf).expect("write pack");
    let mut body = pkt("NAK\n");
    body.extend_from_slice(&buf);
    body
}

fn respond(stream: &mut TcpStream, status: u16, reason: &str, headers: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n{headers}\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// A git smart-HTTP server that demands Basic authentication, answers an
/// advertisement and a packfile, and counts what it answered.
fn serve(forge: std::path::PathBuf, refs: Vec<(String, git2::Oid)>, tip: git2::Oid) -> Server {
    serve_kind(forge, refs, tip, true)
}

/// The same server for a PUBLIC repository: it never challenges, so
/// libgit2 never asks joy for a credential and joy presents nothing.
/// That is the remote of D1.9's no anonymous polling rule.
fn serve_public(
    forge: std::path::PathBuf,
    refs: Vec<(String, git2::Oid)>,
    tip: git2::Oid,
) -> Server {
    serve_kind(forge, refs, tip, false)
}

fn serve_kind(
    forge: std::path::PathBuf,
    refs: Vec<(String, git2::Oid)>,
    tip: git2::Oid,
    private: bool,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = Arc::new(AtomicUsize::new(0));
    let (seen_thread, count_thread) = (seen.clone(), requests.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let refs = refs.clone();
            let forge = forge.clone();
            let seen = seen_thread.clone();
            let count = count_thread.clone();
            std::thread::spawn(move || {
                // one connection may carry several requests
                loop {
                    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = request_line.split_whitespace();
                    let verb = parts.next().unwrap_or_default().to_string();
                    let path = parts.next().unwrap_or_default().to_string();
                    let mut authenticated = false;
                    let mut length = 0usize;
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 {
                            return;
                        }
                        let header = header.trim_end().to_string();
                        if header.is_empty() {
                            break;
                        }
                        let lower = header.to_ascii_lowercase();
                        if let Some(value) = lower.strip_prefix("authorization:") {
                            authenticated = value.trim().starts_with("basic ");
                        }
                        if let Some(value) = lower.strip_prefix("content-length:") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                    }
                    if length > 0 {
                        let mut body = vec![0u8; length];
                        if reader.read_exact(&mut body).is_err() {
                            return;
                        }
                    }
                    count.fetch_add(1, Ordering::SeqCst);
                    let status = if private && !authenticated {
                        // the 401 every private repository answers first
                        respond(
                            &mut stream,
                            401,
                            "Unauthorized",
                            "WWW-Authenticate: Basic realm=\"joy\"\r\n",
                            b"",
                        );
                        401
                    } else if path.contains("/info/refs") {
                        respond(
                            &mut stream,
                            200,
                            "OK",
                            "Content-Type: application/x-git-upload-pack-advertisement\r\nCache-Control: no-cache\r\n",
                            &advertisement(&refs),
                        );
                        200
                    } else if path.ends_with("/git-upload-pack") {
                        respond(
                            &mut stream,
                            200,
                            "OK",
                            "Content-Type: application/x-git-upload-pack-result\r\nCache-Control: no-cache\r\n",
                            &packfile(&forge, tip),
                        );
                        200
                    } else {
                        respond(&mut stream, 404, "Not Found", "", b"");
                        404
                    };
                    seen.lock().unwrap().push(Request {
                        verb: verb.clone(),
                        path: path.clone(),
                        authenticated,
                        status,
                    });
                }
            });
        }
    });
    Server {
        port,
        seen,
        requests,
    }
}

/// A bare repository with one commit on `refs/heads/main` and the same
/// tip on `refs/joy/chats`, the side ref the chat store watches.
fn forge_repository(dir: &std::path::Path) -> (Vec<(String, git2::Oid)>, git2::Oid) {
    let repo = git2::Repository::init_bare(dir).expect("init bare");
    let tree = {
        let mut index = repo.index().expect("index");
        let oid = index.write_tree().expect("write tree");
        repo.find_tree(oid).expect("tree")
    };
    let sig = git2::Signature::now("Seed", "seed@example.com").expect("signature");
    let tip = repo
        .commit(Some("refs/heads/main"), &sig, &sig, "seed", &tree, &[])
        .expect("commit");
    repo.reference(CHATS_REF, tip, true, "chats").expect("ref");
    (
        vec![
            ("refs/heads/main".to_string(), tip),
            (CHATS_REF.to_string(), tip),
        ],
        tip,
    )
}

/// One `fetch_ref` against a private https remote costs THREE requests,
/// not five: the 401 and its replay on the one connection, and the
/// upload-pack POST that rides the same connection.
#[test]
fn one_fetch_of_a_private_remote_costs_three_requests() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    state_file_in(tmp.path());
    let forge = tmp.path().join("forge.git");
    let (refs, tip) = forge_repository(&forge);
    let server = serve(forge.clone(), refs, tip);

    // the throttle is not what this test measures
    contact::set_gaps("127.0.0.1=0,default=0");

    let checkout = tmp.path().join("checkout");
    let repo = git2::Repository::init(&checkout).expect("init");
    repo.remote(
        "origin",
        &format!("http://127.0.0.1:{}/forge.git", server.port),
    )
    .expect("remote");
    drop(repo);

    let fetched = fetch_ref(
        &checkout,
        &Auth::token("a-token"),
        CHATS_REF,
        "refs/joy/chats-tracking",
    )
    .expect("the forge has the ref");
    assert!(fetched, "the ref was advertised");

    let seen = server.seen.lock().unwrap().clone();
    let counted = server.requests.load(Ordering::SeqCst);
    assert_eq!(
        counted, 3,
        "one fetch is one 401, one replayed advertisement and one upload-pack: {seen:#?}"
    );
    assert_eq!(
        seen.iter().filter(|r| r.status == 401).count(),
        1,
        "exactly ONE challenge, because the connection is held: {seen:#?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|r| r.path.contains("/info/refs"))
            .count(),
        2,
        "the advertisement is asked for once, not twice: {seen:#?}"
    );
    assert_eq!(
        seen.iter().filter(|r| r.verb == "POST").count(),
        1,
        "one upload-pack: {seen:#?}"
    );

    // and the objects really arrived
    let repo = git2::Repository::open(&checkout).expect("open");
    assert_eq!(
        repo.find_reference("refs/joy/chats-tracking")
            .expect("tracking ref")
            .target(),
        Some(tip)
    );
    assert!(repo.find_commit(tip).is_ok(), "the commit was downloaded");

    // The other half of the credential memory the test above reads:
    // a token that really went over the wire IS remembered. The
    // resolver of D1.1 hands every candidate over inside its own chain
    // (D1.2), so the note of D1.8b has to sit on each of the chain's
    // hand-over points; the sibling test proves the negative case, and
    // without this one an engine that noted nothing at all would pass
    // both.
    assert!(
        contact::credential_answers("127.0.0.1", contact::Transport::Https, true),
        "a token the forge accepted is what joy has for this host (D1.9)"
    );
    assert!(
        contact::token_worked_before("127.0.0.1"),
        "and a 404 on this host is no longer read as 'never signed in' (D1.8b)"
    );

    contact::set_gaps("");
    resolver::set_state_file(None);
}

/// One `ls_remote_refs` for two refs is ONE contact and two requests:
/// the advertisement carries every ref anyway, so a poll tick that
/// watches a branch and a chat ref asks once (D1.9).
#[test]
fn one_poll_tick_makes_one_contact_for_two_refs() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    state_file_in(tmp.path());
    let forge = tmp.path().join("forge.git");
    let (refs, tip) = forge_repository(&forge);
    let server = serve(forge.clone(), refs, tip);
    contact::set_gaps("127.0.0.1=0,default=0");

    let checkout = tmp.path().join("checkout");
    let repo = git2::Repository::init(&checkout).expect("init");
    repo.remote(
        "origin",
        &format!("http://127.0.0.1:{}/forge.git", server.port),
    )
    .expect("remote");
    drop(repo);

    let found = joy_core::vcs::forge::ls_remote_refs(
        &checkout,
        &Auth::token("a-token"),
        &["refs/heads/main", CHATS_REF, "refs/joy/absent"],
    )
    .expect("the advertisement");
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found["refs/heads/main"], tip.to_string());
    assert_eq!(found[CHATS_REF], tip.to_string());

    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        2,
        "the 401 and its replay, and nothing else: {seen:#?}"
    );
    contact::set_gaps("");
    resolver::set_state_file(None);
}

/// J5's acceptance, measured from OUTSIDE the crate with the public
/// API: "after a 429 the next contact to that host waits at least twice
/// the gap". The next one, not the one after it: the slot for the next
/// contact is reserved while the failing contact is still running, so
/// the strike has to widen a reservation that already exists.
#[test]
fn after_a_429_the_next_contact_to_that_host_waits_twice_the_gap() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // 100 ms per request, and a credentialed ls-remote is two requests
    contact::set_gaps("limited.example=100,default=0");
    let url = "https://limited.example/o/r.git";
    let limited = || {
        contact::failed(&contact::ContactEvidence::new(
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Http,
                "unexpected http status code: 429",
            ),
            url,
            contact::ContactDirection::Fetch,
            contact::CredentialSource::TokenPresented,
        ))
    };

    let refused = contact::run(url, "ls-remote", true, || Err::<(), _>(limited()))
        .expect_err("the forge said 429");
    assert_eq!(
        contact::failure_of(&refused),
        contact::Failure::RateLimited,
        "{refused}"
    );

    let started = std::time::Instant::now();
    contact::run(url, "ls-remote", true, || Ok(())).expect("every contact still goes out");
    let waited = started.elapsed();
    assert!(
        waited >= std::time::Duration::from_millis(360),
        "the next contact waits twice the 200 ms gap, waited {waited:?}"
    );

    contact::set_gaps("");
    resolver::set_state_file(None);
}

/// D1.9, no anonymous polling, on the desktop's own shape: `Auth::Local`
/// says "the machine's own credentials" for every host, and on a host
/// where the machine has none it hands over nothing at all. The engine
/// learns that from the contact itself and polls the host once every
/// fifteen minutes, with the sentence that says why.
#[test]
fn a_public_remote_nobody_is_signed_in_for_is_polled_every_fifteen_minutes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    state_file_in(tmp.path());
    let forge = tmp.path().join("forge.git");
    let (refs, tip) = forge_repository(&forge);
    let server = serve_public(forge.clone(), refs, tip);
    contact::set_gaps("localhost=0,default=0");

    let checkout = tmp.path().join("checkout");
    let repo = git2::Repository::init(&checkout).expect("init");
    // the same server under its OTHER name, so this test's host memory
    // is its own and the order of the tests in this binary cannot
    // change what it measures
    repo.remote(
        "origin",
        &format!("http://localhost:{}/forge.git", server.port),
    )
    .expect("remote");
    drop(repo);

    // nothing is known yet, so the first poll goes out on the claim
    assert_eq!(
        contact::poll_period("localhost", "ls-remote", contact::Transport::Https, true),
        std::time::Duration::from_secs(1)
    );
    let found = joy_core::vcs::forge::ls_remote_refs_poll(
        &checkout,
        &Auth::Local,
        &[CHATS_REF, "refs/heads/main"],
    )
    .expect("a public advertisement needs no credential");
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        1,
        "a public repository answers the first request: no challenge, no credential"
    );

    // and now the engine knows: nothing was ever presented here
    assert!(!contact::credential_answers(
        "localhost",
        contact::Transport::Https,
        true
    ));
    assert_eq!(
        contact::poll_period("localhost", "ls-remote", contact::Transport::Https, true),
        contact::ANONYMOUS_POLL_INTERVAL,
        "an https remote with no credential is polled once every 15 minutes (D1.9)"
    );

    // the door holds the next tick of the window, and the surface says why
    let held = joy_core::vcs::forge::ls_remote_ref_poll(&checkout, &Auth::Local, CHATS_REF)
        .expect_err("the second poll inside the window is held");
    assert_eq!(contact::failure_of(&held), contact::Failure::RateLimited);
    assert!(
        held.to_string().contains("15 minutes") && held.to_string().contains("Sign in"),
        "the surface says why: {held}"
    );
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        1,
        "and the forge was not contacted again"
    );

    // a person's own command is not a poll and is never held
    let asked = joy_core::vcs::forge::ls_remote_ref(&checkout, &Auth::Local, CHATS_REF)
        .expect("a person's own command goes out");
    assert_eq!(asked, Some(tip.to_string()));
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);

    contact::set_gaps("");
    resolver::set_state_file(None);
}
