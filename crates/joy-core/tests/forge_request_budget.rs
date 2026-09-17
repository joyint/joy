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

const CHATS_REF: &str = "refs/joy/chats";

/// The gap table joy-core keeps is process-wide: the tests that set it
/// run one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
                    let status = if !authenticated {
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

    contact::set_gaps("");
}

/// One `ls_remote_refs` for two refs is ONE contact and two requests:
/// the advertisement carries every ref anyway, so a poll tick that
/// watches a branch and a chat ref asks once (D1.9).
#[test]
fn one_poll_tick_makes_one_contact_for_two_refs() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
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
}
