// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The two arguments a clone grew for the desktop (D4.3, package A4):
//! how much history it downloads, and who watches it arrive.
//!
//! In process, against a git smart-HTTP server of this test binary's own
//! (the rule: no default test contacts a real forge). The LOCAL
//! transport is no use for either half: it refuses any depth at all
//! ("shallow fetch is not supported by the local transport", libgit2
//! transports/local.c:310) and a clone from a path takes libgit2's local
//! shortcut, which copies the object database and counts nothing. So the
//! server here speaks the protocol: it advertises `shallow`, answers a
//! `deepen 1` request with the cut point and a pack of the tip alone,
//! and the clone that reads it writes the `shallow` file the app records
//! (A4) and the banner of A5 reads.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use joy_core::vcs::contact;
use joy_core::vcs::forge::{self, Auth, CloneProgress};

/// A bare repository with TWO commits on `refs/heads/main`, so that a
/// depth 1 clone really leaves something behind.
fn forge_repository(dir: &std::path::Path) -> (Vec<(String, git2::Oid)>, git2::Oid) {
    let repo = git2::Repository::init_bare(dir).expect("init bare");
    let tree = {
        let mut index = repo.index().expect("index");
        let oid = index.write_tree().expect("write tree");
        repo.find_tree(oid).expect("tree")
    };
    let sig = git2::Signature::now("Seed", "seed@example.com").expect("signature");
    let first = repo
        .commit(Some("refs/heads/main"), &sig, &sig, "first", &tree, &[])
        .expect("commit");
    let parent = repo.find_commit(first).expect("parent");
    let tip = repo
        .commit(
            Some("refs/heads/main"),
            &sig,
            &sig,
            "second",
            &tree,
            &[&parent],
        )
        .expect("commit");
    (vec![("refs/heads/main".to_string(), tip)], tip)
}

fn pkt(line: &str) -> Vec<u8> {
    format!("{:04x}{line}", line.len() + 4).into_bytes()
}

/// The advertisement, with `shallow` among the capabilities: without it
/// the client sends no `deepen` line at all (libgit2 transports/smart.h).
fn advertisement(refs: &[(String, git2::Oid)]) -> Vec<u8> {
    let mut body = pkt("# service=git-upload-pack\n");
    body.extend_from_slice(b"0000");
    for (i, (name, oid)) in refs.iter().enumerate() {
        let line = if i == 0 {
            format!("{oid} {name}\0shallow agent=joy-test\n")
        } else {
            format!("{oid} {name}\n")
        };
        body.extend_from_slice(&pkt(&line));
    }
    body.extend_from_slice(b"0000");
    body
}

/// The shallow-info section a `deepen` request is answered with FIRST,
/// on its own: over HTTP the client sends its wants and its `deepen`
/// line, reads the cut points, and only then asks a second time with
/// `done`. Answering the pack straight away is what a client reads as a
/// broken connection.
fn shallow_info(tip: git2::Oid) -> Vec<u8> {
    // No trailing newline: libgit2 measures the line and refuses
    // anything longer than "shallow " plus the hex oid (smart_pkt.c:469).
    let mut body = pkt(&format!("shallow {tip}"));
    body.extend_from_slice(b"0000");
    body
}

/// The pack the request asked for: the TIP ALONE for a `deepen 1`
/// request, and the WHOLE HISTORY for a request that sent no depth.
///
/// The difference is the point of both tests: a depth 1 clone must come
/// out with one commit and a `shallow` file, and a depth 0 clone of the
/// same repository must come out with every commit and none.
fn packfile(forge: &std::path::Path, tip: git2::Oid, deepened: bool) -> Vec<u8> {
    let repo = git2::Repository::open_bare(forge).expect("bare forge");
    let mut builder = repo.packbuilder().expect("packbuilder");
    if deepened {
        builder.insert_commit(tip).expect("insert commit");
    } else {
        let mut walk = repo.revwalk().expect("revwalk");
        walk.push(tip).expect("push tip");
        builder.insert_walk(&mut walk).expect("insert walk");
    }
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

/// A git smart-HTTP server that needs no credential and answers what the
/// client asked for. Returns its port.
fn serve(forge: std::path::PathBuf, refs: Vec<(String, git2::Oid)>, tip: git2::Oid) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let refs = refs.clone();
            let forge = forge.clone();
            std::thread::spawn(move || loop {
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                    return;
                }
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                let mut length = 0usize;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 {
                        return;
                    }
                    let header = header.trim_end().to_ascii_lowercase();
                    if header.is_empty() {
                        break;
                    }
                    if let Some(value) = header.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                if length > 0 && reader.read_exact(&mut body).is_err() {
                    return;
                }
                let asked = String::from_utf8_lossy(&body).to_string();
                if path.contains("/info/refs") {
                    respond(
                        &mut stream,
                        200,
                        "OK",
                        "Content-Type: application/x-git-upload-pack-advertisement\r\nCache-Control: no-cache\r\n",
                        &advertisement(&refs),
                    );
                } else if path.ends_with("/git-upload-pack") {
                    let deepened = asked.contains("deepen ");
                    let answer = if deepened && !asked.contains("done\n") {
                        shallow_info(tip)
                    } else {
                        packfile(&forge, tip, deepened)
                    };
                    respond(
                        &mut stream,
                        200,
                        "OK",
                        "Content-Type: application/x-git-upload-pack-result\r\nCache-Control: no-cache\r\n",
                        &answer,
                    );
                } else {
                    respond(&mut stream, 404, "Not Found", "", b"");
                }
            });
        }
    });
    port
}

struct Forge {
    _dir: tempfile::TempDir,
    url: String,
    tip: git2::Oid,
    checkout: std::path::PathBuf,
}

fn a_forge() -> Forge {
    contact::set_gaps("127.0.0.1=0,default=0");
    let dir = tempfile::tempdir().expect("tempdir");
    let bare = dir.path().join("forge.git");
    let (refs, tip) = forge_repository(&bare);
    let port = serve(bare, refs, tip);
    Forge {
        url: format!("http://127.0.0.1:{port}/forge.git"),
        tip,
        checkout: dir.path().join("checkout"),
        _dir: dir,
    }
}

/// How many commits the checkout can reach from the tip it cloned. Under
/// a depth 1 clone this is 1, because the revwalk applies the shallow
/// grafts; under a full clone it is the whole history. The tip is pushed
/// by oid, because this test's server advertises no `HEAD` symref and the
/// clone therefore writes no local branch to walk from.
fn history_length(checkout: &std::path::Path, tip: git2::Oid) -> usize {
    let repo = git2::Repository::open(checkout).expect("open");
    let mut walk = repo.revwalk().expect("revwalk");
    walk.push(tip).expect("push the tip");
    walk.count()
}

/// The lean shape of D4.3: one snapshot, and the repository says so.
#[test]
fn a_depth_one_clone_lands_shallow_and_counts_its_objects() {
    let forge = a_forge();
    let mut seen: Vec<CloneProgress> = Vec::new();
    forge::clone(
        &forge.url,
        &Auth::token("x"),
        &forge.checkout,
        1,
        &mut |step| {
            seen.push(step);
            true
        },
    )
    .expect("the clone lands");

    let cut = std::fs::read_to_string(forge.checkout.join(".git/shallow"))
        .expect("a depth 1 clone writes its cut points");
    assert_eq!(
        cut.trim(),
        forge.tip.to_string(),
        "the tip is where the history is cut"
    );
    let repo = git2::Repository::open(&forge.checkout).expect("open");
    assert!(repo.is_shallow(), "and libgit2 agrees it is shallow");
    assert_eq!(
        history_length(&forge.checkout, forge.tip),
        1,
        "one snapshot: the first commit of the two stayed on the forge"
    );

    let last = seen.last().expect("the callback was called");
    assert!(
        last.total_objects > 0 && last.received_objects > 0 && last.received_bytes > 0,
        "the bytes AND the objects of D4.3 are counted, not invented: {last:?}"
    );
}

/// The whole history, for the CLI and the platform: the same call with
/// [`forge::CLONE_DEPTH_FULL`] writes no cut points at all.
#[test]
fn depth_full_clones_the_whole_history() {
    let forge = a_forge();
    forge::clone(
        &forge.url,
        &Auth::token("x"),
        &forge.checkout,
        forge::CLONE_DEPTH_FULL,
        &mut |_| true,
    )
    .expect("the clone lands");
    assert!(
        !forge.checkout.join(".git/shallow").exists(),
        "depth 0 is libgit2's own 'the whole history'"
    );
    let repo = git2::Repository::open(&forge.checkout).expect("open");
    assert!(!repo.is_shallow());
    assert_eq!(
        history_length(&forge.checkout, forge.tip),
        2,
        "BOTH commits of the forge arrived, not just the tip"
    );
}

/// The cancel of D4.3: a callback that says stop stops the DOWNLOAD,
/// which is the difference between a person waiting ten seconds and a
/// person waiting for a 2 GB pack nobody wants any more.
#[test]
fn a_callback_that_says_stop_stops_the_download() {
    let forge = a_forge();
    let mut calls = 0usize;
    let failed = forge::clone(
        &forge.url,
        &Auth::token("x"),
        &forge.checkout,
        forge::CLONE_DEPTH_FULL,
        &mut |_| {
            calls += 1;
            false
        },
    )
    .expect_err("a stopped transfer is not a clone");
    assert!(calls > 0, "the callback decided, nothing else did");
    assert!(
        !forge.checkout.join("HEAD").exists() && !forge.checkout.join(".git/HEAD").exists(),
        "no checkout came out of it: {failed:#}"
    );
}
