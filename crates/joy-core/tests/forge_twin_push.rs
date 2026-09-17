// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The https twin, pushed over for real (package J4b, design D1.2 and
//! D1.5).
//!
//! The counting server of J5 only ever answered `git-upload-pack`; the
//! rules this file proves are all on the push side, so it grows a
//! `git-receive-pack` half: it advertises `report-status`, reads the
//! command list and the packfile, indexes the pack into the bare
//! repository with libgit2's own writer, moves the refs and answers
//! `ok` or `ng` per ref. It needs no network and no forge.
//!
//! Its own test binary, because everything it arranges is process
//! state: HOME and `SSH_AUTH_SOCK` (so the ssh probe of D1.2 finds
//! nothing, which is the normal state of a Windows desktop), the
//! connector search path, joy's own state file and the throttle's gap
//! table.
//!
//! Unix only: the connector stub is a shell script, exactly as the
//! runner's own tests are. Nothing it proves is a unix rule.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use joy_core::forge_plugins;
use joy_core::host::HostKind;
use joy_core::vcs::contact;
use joy_core::vcs::forge::{self, Auth};
use joy_core::vcs::resolver::{self, HostMemory, TransportState};

/// Everything below is process state; one case at a time.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------
// A git smart-HTTP server that can be pushed to
// ---------------------------------------------------------------------

/// What one ref of a push was answered with.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// The forge took it.
    Ok,
    /// The forge refused it, in its own words.
    Rejected(&'static str),
}

struct Server {
    port: u16,
    requests: Arc<AtomicUsize>,
    pushes: Arc<AtomicUsize>,
    /// A status the forge answers every authenticated request with,
    /// instead of serving it. 0 is "serve it".
    refuse: Arc<AtomicUsize>,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/{path}", self.port)
    }
}

/// A pkt-line: four hex digits of total length, then the payload.
fn pkt(line: &str) -> Vec<u8> {
    format!("{:04x}{line}", line.len() + 4).into_bytes()
}

/// The advertisement of one service. On the push side the capability
/// list carries `report-status`, which is what makes the per-ref
/// answers below reach `push_update_reference` at all
/// (remote.c:3034-3038).
fn advertisement(service: &str, refs: &[(String, git2::Oid)]) -> Vec<u8> {
    let caps = if service == "git-receive-pack" {
        "report-status delete-refs agent=joy-test"
    } else {
        "agent=joy-test"
    };
    let mut body = pkt(&format!("# service={service}\n"));
    body.extend_from_slice(b"0000");
    if refs.is_empty() {
        body.extend_from_slice(&pkt(&format!(
            "{} capabilities^{{}}\0{caps}\n",
            git2::Oid::ZERO_SHA1
        )));
    }
    for (i, (name, oid)) in refs.iter().enumerate() {
        let line = if i == 0 {
            format!("{oid} {name}\0{caps}\n")
        } else {
            format!("{oid} {name}\n")
        };
        body.extend_from_slice(&pkt(&line));
    }
    body.extend_from_slice(b"0000");
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

/// Read a request body that may arrive with a length or in chunks.
/// libgit2 streams `git-receive-pack` chunked (http.c:89-95), which the
/// fetch-side server of J5 never had to read.
fn read_body(reader: &mut impl BufRead, length: Option<usize>, chunked: bool) -> Option<Vec<u8>> {
    if let Some(length) = length {
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).ok()?;
        return Some(body);
    }
    if !chunked {
        return Some(Vec::new());
    }
    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        if reader.read_line(&mut size).ok()? == 0 {
            return None;
        }
        let size = usize::from_str_radix(size.trim(), 16).ok()?;
        let mut chunk = vec![0u8; size];
        reader.read_exact(&mut chunk).ok()?;
        // the CRLF after every chunk, the empty one included
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).ok()?;
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(&chunk);
    }
}

/// The command list and the packfile of one `git-receive-pack` request.
/// The commands are pkt-lines up to the flush; everything after it is
/// the pack.
fn commands_of(body: &[u8]) -> (Vec<(git2::Oid, git2::Oid, String)>, &[u8]) {
    let mut commands = Vec::new();
    let mut at = 0usize;
    while at + 4 <= body.len() {
        let header = std::str::from_utf8(&body[at..at + 4]).unwrap_or("0000");
        let size = usize::from_str_radix(header, 16).unwrap_or(0);
        if size == 0 {
            at += 4;
            break;
        }
        let line = String::from_utf8_lossy(&body[at + 4..at + size]).into_owned();
        let line = line.split('\0').next().unwrap_or("").trim().to_string();
        let mut parts = line.split(' ');
        if let (Some(old), Some(new), Some(name)) = (parts.next(), parts.next(), parts.next()) {
            if let (Ok(old), Ok(new)) = (git2::Oid::from_str(old), git2::Oid::from_str(new)) {
                commands.push((old, new, name.to_string()));
            }
        }
        at += size;
    }
    (commands, &body[at.min(body.len())..])
}

/// Take the pack into the bare repository and move the refs, which is
/// what makes the push a real one: the forge really holds the objects
/// afterwards.
fn receive(forge: &Path, commands: &[(git2::Oid, git2::Oid, String)], pack: &[u8]) -> bool {
    let Ok(repo) = git2::Repository::open_bare(forge) else {
        return false;
    };
    if pack.starts_with(b"PACK") {
        let Ok(odb) = repo.odb() else { return false };
        let Ok(mut writer) = odb.packwriter() else {
            return false;
        };
        if writer.write_all(pack).is_err() || writer.commit().is_err() {
            return false;
        }
    }
    for (_, new, name) in commands {
        if repo.reference(name, *new, true, "receive-pack").is_err() {
            return false;
        }
    }
    true
}

/// A git smart-HTTP server that demands Basic authentication, serves
/// the `git-receive-pack` advertisement, takes a push and answers every
/// ref with `answer`.
fn serve(forge: PathBuf, answer: Answer) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(AtomicUsize::new(0));
    let pushes = Arc::new(AtomicUsize::new(0));
    let refuse = Arc::new(AtomicUsize::new(0));
    let (count, pushed, refusing) = (requests.clone(), pushes.clone(), refuse.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let forge = forge.clone();
            let count = count.clone();
            let pushed = pushed.clone();
            let refusing = refusing.clone();
            std::thread::spawn(move || loop {
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                    return;
                }
                let mut parts = request_line.split_whitespace();
                let _verb = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let mut authenticated = false;
                let mut length: Option<usize> = None;
                let mut chunked = false;
                let mut expects_continue = false;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 {
                        return;
                    }
                    let header = header.trim_end().to_ascii_lowercase();
                    if header.is_empty() {
                        break;
                    }
                    if let Some(value) = header.strip_prefix("authorization:") {
                        authenticated = value.trim().starts_with("basic ");
                    }
                    if let Some(value) = header.strip_prefix("content-length:") {
                        length = value.trim().parse().ok();
                    }
                    if let Some(value) = header.strip_prefix("transfer-encoding:") {
                        chunked = value.trim().contains("chunked");
                    }
                    if let Some(value) = header.strip_prefix("expect:") {
                        expects_continue = value.trim().contains("100-continue");
                    }
                    if header.starts_with("content-type:") && header.contains("receive-pack") {
                        // nothing to record; the path already says it
                    }
                }
                count.fetch_add(1, Ordering::SeqCst);
                if !authenticated {
                    // the 401 every private repository answers first
                    respond(
                        &mut stream,
                        401,
                        "Unauthorized",
                        "WWW-Authenticate: Basic realm=\"joy\"\r\n",
                        b"",
                    );
                    continue;
                }
                if expects_continue {
                    let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                }
                match refusing.load(Ordering::SeqCst) {
                    0 => {}
                    403 => {
                        respond(&mut stream, 403, "Forbidden", "", b"");
                        continue;
                    }
                    _ => {
                        respond(&mut stream, 404, "Not Found", "", b"");
                        continue;
                    }
                }
                if path.contains("/info/refs") {
                    // The service the client asked for: a fetch and a
                    // push read the same file under two names, and a
                    // client that is handed the other one's service
                    // line stops there.
                    let service = if path.contains("service=git-receive-pack") {
                        "git-receive-pack"
                    } else {
                        "git-upload-pack"
                    };
                    let refs = refs_of(&forge);
                    respond(
                        &mut stream,
                        200,
                        "OK",
                        &format!(
                            "Content-Type: application/x-{service}-advertisement\r\nCache-Control: no-cache\r\n"
                        ),
                        &advertisement(service, &refs),
                    );
                    continue;
                }
                if path.ends_with("/git-receive-pack") {
                    let Some(body) = read_body(&mut reader, length, chunked) else {
                        return;
                    };
                    pushed.fetch_add(1, Ordering::SeqCst);
                    let (commands, pack) = commands_of(&body);
                    let mut report = Vec::new();
                    let taken = match answer {
                        Answer::Ok => receive(&forge, &commands, pack),
                        Answer::Rejected(_) => false,
                    };
                    report.extend_from_slice(&pkt("unpack ok\n"));
                    for (_, _, name) in &commands {
                        report.extend_from_slice(&pkt(&match answer {
                            Answer::Ok if taken => format!("ok {name}\n"),
                            Answer::Ok => format!("ng {name} the forge could not store it\n"),
                            Answer::Rejected(reason) => format!("ng {name} {reason}\n"),
                        }));
                    }
                    report.extend_from_slice(b"0000");
                    respond(
                        &mut stream,
                        200,
                        "OK",
                        "Content-Type: application/x-git-receive-pack-result\r\nCache-Control: no-cache\r\n",
                        &report,
                    );
                    continue;
                }
                respond(&mut stream, 404, "Not Found", "", b"");
            });
        }
    });
    Server {
        port,
        requests,
        pushes,
        refuse,
    }
}

fn refs_of(forge: &Path) -> Vec<(String, git2::Oid)> {
    let Ok(repo) = git2::Repository::open_bare(forge) else {
        return Vec::new();
    };
    let Ok(refs) = repo.references() else {
        return Vec::new();
    };
    refs.filter_map(|r| {
        let r = r.ok()?;
        let name = r.name().ok()?.to_string();
        Some((name, r.target()?))
    })
    .collect()
}

// ---------------------------------------------------------------------
// The machine the test runs on
// ---------------------------------------------------------------------

/// A home with no agent and no key file, joy's own state file inside
/// it, and a connector that claims the test host. This is exactly the
/// machine of D1.2 trigger (a).
struct Machine {
    _home: tempfile::TempDir,
    root: PathBuf,
    /// Every connector call, one argv per line, so a case can read what
    /// really went over the wire and how often.
    argv: PathBuf,
    /// The PATH this process had before the case emptied it.
    path: Option<std::ffi::OsString>,
}

impl Machine {
    /// The calls of one verb the connector really answered. The first
    /// word of an argv is the forge id of the combined binary, the
    /// second is the verb.
    fn calls(&self, verb: &str) -> Vec<String> {
        std::fs::read_to_string(&self.argv)
            .unwrap_or_default()
            .lines()
            .filter(|line| {
                line.split_whitespace()
                    .nth(1)
                    .is_some_and(|second| second == verb)
            })
            .map(str::to_string)
            .collect()
    }
}

fn machine(twin: &str, token: &str) -> Machine {
    machine_with(Some((twin, token)))
}

/// The same machine with no connector installed anywhere: no claim, no
/// token, and therefore no twin. Together with the empty home above
/// this is a desktop that is signed in to nothing at all.
fn machine_without_a_connector() -> Machine {
    machine_with(None)
}

fn machine_with(connector: Option<(&str, &str)>) -> Machine {
    let home = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", home.path());
    std::env::set_var("USERPROFILE", home.path());
    std::env::set_var("XDG_STATE_HOME", home.path().join("state"));
    std::env::remove_var("SSH_AUTH_SOCK");
    std::env::remove_var(forge_plugins::PLUGIN_DIR_ENV);
    resolver::set_state_file(Some(home.path().join("forge-state.json")));
    resolver::invalidate_all_facts();
    contact::set_gaps("127.0.0.1=0,joy-test.invalid=0,default=0");

    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let argv = home.path().join("argv.log");
    std::env::set_var("JOY_STUB_ARGV", &argv);
    let path = std::env::var_os("PATH");
    let Some((twin, token)) = connector else {
        // No connector on any of the three search paths of D2.2: the
        // registered directories, the executable's own and PATH.
        std::env::set_var("PATH", &bin);
        forge_plugins::set_plugin_dirs(vec![bin]);
        return Machine {
            root: home.path().to_path_buf(),
            argv,
            path,
            _home: home,
        };
    };
    let stub = bin.join(forge_plugins::COMBINED_BINARY);
    std::fs::write(
        &stub,
        format!(
            r#"#!/bin/sh
if [ -n "$JOY_STUB_ARGV" ]; then
  echo "$@" >> "$JOY_STUB_ARGV"
fi
if [ "$1" = "version" ]; then
  echo '{{"protocol":2,"plugin":"joy-forge 0.21.0","forges":["github","gitlab","gitea"]}}'
  exit 0
fi
shift
verb="$1"
case "$verb" in
  claims) echo '{{"claims":true}}' ;;
  web-url) echo '{{"known":true,"https_url":"{twin}"}}' ;;
  token) echo '{{"known":true,"host":"127.0.0.1","login":"scotty-work","token":"{token}","username":"x-access-token","source":"keychain","chose_by":"only"}}' ;;
  *) echo '{{"known":false}}' ;;
esac
"#
        ),
    )
    .expect("write the stub");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    forge_plugins::set_plugin_dirs(vec![bin]);

    Machine {
        root: home.path().to_path_buf(),
        argv,
        path,
        _home: home,
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        forge_plugins::set_plugin_dirs(Vec::new());
        resolver::set_state_file(None);
        resolver::invalidate_all_facts();
        contact::set_gaps("");
        // Everything a case taught this process, taken back: libtest
        // guarantees no order, and a case that ran after one which left
        // 127.0.0.1 taught as a GitHub host with a fixed oracle would
        // read another case's forge.
        contact::clear_host_families();
        contact::clear_oracle();
        std::env::remove_var("JOY_STUB_ARGV");
        match self.path.take() {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        let _ = &self.root;
    }
}

/// Put one readable, unencrypted ssh key into this machine's home, so
/// the probe of D1.2 trigger (a) finds a candidate and the plan keeps
/// an ssh leg behind the twin.
///
/// The blob is the smallest thing `examine` reads as an openssh-key-v1
/// file that is not encrypted: the magic, then the cipher name `none`.
/// Nothing ever hands it to libssh2 in these cases; what is under test
/// is the SHAPE of the plan, which is decided before any socket.
fn with_a_readable_key(home: &Path) {
    let dir = home.join(".ssh");
    std::fs::create_dir_all(&dir).expect("ssh dir");
    std::fs::write(
        dir.join("id_ed25519"),
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU=\n-----END OPENSSH PRIVATE KEY-----\n",
    )
    .expect("write the key");
}

/// A bare forge with one commit on `refs/heads/main`.
fn forge_repository(dir: &Path) -> git2::Oid {
    let repo = git2::Repository::init_bare(dir).expect("init bare");
    let tree = {
        let mut index = repo.index().expect("index");
        let oid = index.write_tree().expect("write tree");
        repo.find_tree(oid).expect("tree")
    };
    let sig = git2::Signature::now("Seed", "seed@example.com").expect("signature");
    repo.commit(Some("refs/heads/main"), &sig, &sig, "seed", &tree, &[])
        .expect("commit")
}

/// A checkout of that forge whose CONFIGURED remote is ssh, with one
/// commit of its own on top: one ahead, nothing behind.
fn checkout_ahead(dir: &Path, forge_dir: &Path, remote: &str, base: git2::Oid) -> git2::Oid {
    let repo = git2::Repository::init(dir).expect("init");
    repo.remote("origin", remote).expect("remote");
    // The base commit, fetched the short way: this test is about the
    // push, and a local clone of the bare repository is the same
    // history without a contact.
    {
        let odb = repo.odb().expect("odb");
        let mut pack = odb.packwriter().expect("packwriter");
        let source = git2::Repository::open_bare(forge_dir).expect("bare");
        let mut builder = source.packbuilder().expect("packbuilder");
        builder.insert_commit(base).expect("insert");
        let mut buf = git2::Buf::new();
        builder.write_buf(&mut buf).expect("write pack");
        pack.write_all(&buf).expect("write");
        pack.commit().expect("commit pack");
    }
    repo.reference("refs/heads/main", base, true, "base")
        .expect("branch");
    repo.reference("refs/remotes/origin/main", base, true, "base")
        .expect("tracking");
    repo.set_head("refs/heads/main").expect("head");
    let sig = git2::Signature::now("Scotty", "scotty@example.com").expect("signature");
    let parent = repo.find_commit(base).expect("base commit");
    let tree = parent.tree().expect("tree");
    repo.commit(
        Some("refs/heads/main"),
        &sig,
        &sig,
        "one of my own",
        &tree,
        &[&parent],
    )
    .expect("commit")
}

fn git_config_bytes(checkout: &Path) -> Vec<u8> {
    std::fs::read(checkout.join(".git").join("config")).expect("the checkout's own config")
}

// ---------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------

/// D1.2 trigger (a) and D1.5, end to end: a machine with no agent and
/// no readable key pushes an ssh remote over the https twin, the twin
/// address never touches `.git/config`, and the ahead and behind
/// counter reads 0 afterwards because the engine wrote the tracking ref
/// itself.
#[test]
fn a_push_over_the_twin_updates_the_tracking_ref_and_leaves_git_config_alone() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");

    let checkout = tmp.path().join("checkout");
    let tip = checkout_ahead(&checkout, &forge_dir, "ssh://git@127.0.0.1/forge.git", base);
    let before = git_config_bytes(&checkout);
    assert_eq!(
        forge::ahead_behind(&checkout).expect("ahead behind"),
        (1, 0),
        "one commit of our own, nothing from the forge"
    );

    forge::push(&checkout, &Auth::local(HostKind::Background)).expect("the twin carried the push");

    assert_eq!(
        forge::ahead_behind(&checkout).expect("ahead behind"),
        (0, 0),
        "the twin carries zero refspecs, so the engine writes the tracking ref itself (D1.5)"
    );
    assert_eq!(
        git_config_bytes(&checkout),
        before,
        "the twin is computed for a contact; the configured remote is never rewritten (D1.2)"
    );
    assert_eq!(
        server.pushes.load(Ordering::SeqCst),
        1,
        "one receive-pack, and it went to the twin"
    );
    // and the forge really holds it
    let bare = git2::Repository::open_bare(&forge_dir).expect("bare");
    assert_eq!(
        bare.refname_to_id("refs/heads/main").expect("main"),
        tip,
        "the objects arrived, not only the report"
    );
    // "says which credential it used" (J4b acceptance)
    let sentence = resolver::used("127.0.0.1").expect("the transport memory names it");
    assert!(
        sentence.contains("over https with the access token"),
        "{sentence}"
    );
    let memory = resolver::recall("127.0.0.1").expect("a row");
    assert_eq!(
        memory.state,
        TransportState::NoSshCredential,
        "the ssh side is remembered as what it was, not as a failure of the forge"
    );
    drop(machine);
}

/// D1.5: "joy collects every `Some(reason)`, fails the operation, and
/// puts the server's sentence into the detail line." Without
/// `push_update_reference` this push returns `Ok(())`, because
/// `git_push_finish` fails only when the pack could not be unpacked.
#[test]
fn a_ref_the_forge_rejected_fails_the_push_with_the_forge_s_own_sentence() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Rejected("non-fast-forward"));
    let machine = machine(&server.url("forge.git"), "a-token");

    let checkout = tmp.path().join("checkout");
    checkout_ahead(&checkout, &forge_dir, "ssh://git@127.0.0.1/forge.git", base);

    let refused = forge::push(&checkout, &Auth::local(HostKind::Background))
        .expect_err("a rejected ref is not a successful push");
    assert_eq!(
        contact::failure_of(&refused),
        contact::Failure::Error,
        "the forge answered, so this is not offline and not a refusal of the login (D1.8b)"
    );
    assert!(
        refused
            .to_string()
            .contains("refused to update refs/heads/main"),
        "the sentence a person reads is joy's own: {refused}"
    );
    let detail = contact::detail_of(&refused).expect("a detail line");
    assert!(
        detail.contains("non-fast-forward"),
        "the forge's own sentence goes to the detail line and nowhere else: {detail}"
    );
    assert_eq!(
        forge::ahead_behind(&checkout).expect("ahead behind"),
        (1, 0),
        "a rejected ref writes no tracking ref"
    );
    drop(machine);
}

/// D1.2 trigger (b): a host the transport memory remembers as
/// `ssh-failed` starts at the twin, and the configured ssh remote is
/// still the one in `.git/config` afterwards. The failure is injected
/// as the row an ssh authentication failure writes, which is what the
/// engine's own unit tests pin to `class == Ssh` with `code == Auth`.
#[test]
fn a_remembered_ssh_failure_sends_the_next_push_to_the_twin() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");
    // A machine that DOES hold an ssh credential would stay on ssh
    // without this row; the row is the whole difference.
    resolver::remember("127.0.0.1", HostMemory::new(TransportState::SshFailed));

    let checkout = tmp.path().join("checkout");
    checkout_ahead(&checkout, &forge_dir, "ssh://git@127.0.0.1/forge.git", base);
    let before = git_config_bytes(&checkout);

    forge::push(&checkout, &Auth::local(HostKind::Background)).expect("the twin carried the push");

    assert_eq!(server.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(git_config_bytes(&checkout), before);
    assert_eq!(
        forge::ahead_behind(&checkout).expect("ahead behind"),
        (0, 0)
    );
    drop(machine);
}

/// D1.2 rule 3: "A host whose memory says `ssh-worked` never goes to
/// the twin, whatever tokens exist." The connector here holds a token
/// and names the twin, and the twin is never dialled: the counting
/// server answers nothing at all.
#[test]
fn a_host_that_ssh_worked_for_is_never_taken_to_the_twin() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");
    resolver::remember(
        "joy-test.invalid",
        HostMemory::new(TransportState::SshWorked),
    );

    let checkout = tmp.path().join("checkout");
    let repo = git2::Repository::init(&checkout).expect("init");
    repo.remote("origin", "ssh://git@joy-test.invalid/acme/widgets.git")
        .expect("remote");
    drop(repo);

    // The contact itself cannot succeed: `.invalid` resolves nowhere
    // (RFC 2606) and a lean build carries no ssh transport either. What
    // is under test is where it was ATTEMPTED.
    let _ = forge::ls_remote_refs(
        &checkout,
        &Auth::local(HostKind::Background),
        &["refs/heads/main"],
    );
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        0,
        "the twin was never dialled, and a token existed the whole time"
    );
    assert!(
        machine.calls("token").is_empty(),
        "and no connector was asked for a credential the contact would not use"
    );
    drop(machine);
}

/// The oracle of D2.10, as far as this test needs one: it says what the
/// case installs it to say.
struct FixedOracle(contact::OracleAnswer);

impl contact::RateLimitOracle for FixedOracle {
    fn ask(&self, _host: &str, _status: u16) -> Option<contact::OracleAnswer> {
        Some(self.0.clone())
    }
}

/// The last two acceptance sentences of J4b, read through a contact
/// that really went over the twin: "a 403 on a private organisation
/// repository produces `needs_org_approval`" and "a 404 over https is
/// never reported as offline", each in the state names J5 defined.
///
/// Both need a token that has worked on this host before, which is the
/// fact that tells a 404 that means "no such repository" from a 404
/// that means "your organisation has not approved Joy" (D1.8b). The
/// push at the top is what establishes it.
#[test]
fn the_states_j5_defined_read_through_a_contact_over_the_twin() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");

    let checkout = tmp.path().join("checkout");
    checkout_ahead(&checkout, &forge_dir, "ssh://git@127.0.0.1/forge.git", base);
    forge::push(&checkout, &Auth::local(HostKind::Background))
        .expect("the token works on this host");

    // A 404 over https. It is NOT offline: the forge answered, and a
    // banner that says otherwise sends the person checking their
    // network for a fault in their own checkout.
    server.refuse.store(404, Ordering::SeqCst);
    let missing = forge::fetch_ref(
        &checkout,
        &Auth::local(HostKind::Background),
        "refs/joy/chats",
        "refs/joy/chats-tracking",
    )
    .expect_err("the forge answered 404");
    assert_ne!(
        contact::failure_of(&missing),
        contact::Failure::Offline,
        "a 404 over https is never reported as offline: {missing}"
    );
    assert_eq!(contact::failure_of(&missing), contact::Failure::Error);

    // A 403 on a private organisation repository, with the oracle of
    // D2.10 answering for a GitHub host - and the approval page it
    // named, which is what the acceptance sentence asks for.
    contact::set_host_family("127.0.0.1", contact::HostFamily::GitHub);
    contact::set_oracle(Arc::new(FixedOracle(
        contact::OracleAnswer::NeedsOrgApproval {
            url: Some(APPROVAL_PAGE.to_string()),
        },
    )));
    server.refuse.store(403, Ordering::SeqCst);
    let walled = forge::fetch_ref(
        &checkout,
        &Auth::local(HostKind::Background),
        "refs/joy/chats",
        "refs/joy/chats-tracking",
    )
    .expect_err("the forge answered 403");
    assert_eq!(
        contact::failure_of(&walled),
        contact::Failure::NeedsOrgApproval,
        "{walled}"
    );
    assert_eq!(
        contact::Failure::NeedsOrgApproval.next_step(),
        Some("open the approval page"),
        "the state names the step"
    );
    assert_eq!(
        contact::action_of(&walled).as_deref(),
        Some(APPROVAL_PAGE),
        "and the page that step opens travels with it: {walled}"
    );
    drop(machine);
}

/// The approval page a GitHub organisation's owner acts on. The oracle
/// reads it off the forge's answer (D2.7c); these cases hand it in.
const APPROVAL_PAGE: &str = "https://github.com/orgs/acme/policies/applications";

/// must_fix of the J4b review, and the acceptance sentence with it: on a
/// TWO leg plan the twin's verdict is what the person is told.
///
/// The plan here is the one D1.2 rule 3b makes on a machine that still
/// has an ssh credential: the memory says `ssh-failed`, so the twin goes
/// first and the configured ssh remote stays behind it. When the twin
/// answers 403 and the oracle says the organisation has not approved
/// Joy, that is the answer - not the ssh leg's own refusal, which is
/// what a run that follows every twin failure would report instead.
#[test]
fn a_twin_that_answered_403_is_not_followed_by_the_ssh_leg() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");
    with_a_readable_key(&machine.root);
    // The row of rule 3b, and a machine that HAS something to offer over
    // ssh: that is what makes the plan two legs long.
    resolver::remember(
        "joy-test.invalid",
        HostMemory::new(TransportState::SshFailed),
    );

    let checkout = tmp.path().join("checkout");
    checkout_ahead(
        &checkout,
        &forge_dir,
        "ssh://git@joy-test.invalid/forge.git",
        base,
    );
    // The push establishes what D1.8b needs to tell a 403 that means
    // "your organisation has not approved Joy" from one that means
    // anything else: a token that worked on this host before.
    forge::push(&checkout, &Auth::local(HostKind::Background)).expect("the twin carried the push");
    assert_eq!(server.pushes.load(Ordering::SeqCst), 1);

    contact::set_host_family("127.0.0.1", contact::HostFamily::GitHub);
    contact::set_oracle(Arc::new(FixedOracle(
        contact::OracleAnswer::NeedsOrgApproval {
            url: Some(APPROVAL_PAGE.to_string()),
        },
    )));
    server.refuse.store(403, Ordering::SeqCst);
    let walled = forge::fetch_ref(
        &checkout,
        &Auth::local(HostKind::Background),
        "refs/joy/chats",
        "refs/joy/chats-tracking",
    )
    .expect_err("the forge answered 403");
    assert_eq!(
        contact::failure_of(&walled),
        contact::Failure::NeedsOrgApproval,
        "the twin's verdict, and not the ssh leg's: {walled}"
    );
    assert_eq!(contact::action_of(&walled).as_deref(), Some(APPROVAL_PAGE));
    drop(machine);
}

/// D1.7: "The forge token is asked from the plugin per host, not per
/// contact ... A 1 Hz chat poll must not spawn a plugin or a .NET GCM
/// process per contact." And D4.1c: the login the project pinned in its
/// own app state travels on every connector call as `--login`.
///
/// The pin lives in the per project app state file joy-core computes,
/// never in `project.yaml`: that file is the project's shared,
/// committed file and joy syncs it to the forge, so a pin there
/// publishes one person's work account to the whole team.
#[test]
fn the_connector_is_asked_once_per_host_and_carries_the_project_s_pinned_login() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");

    let checkout = tmp.path().join("checkout");
    checkout_ahead(&checkout, &forge_dir, "ssh://git@127.0.0.1/forge.git", base);

    let pin = joy_core::auth::session::app_state_project_file(&checkout).expect("app state path");
    std::fs::create_dir_all(pin.parent().expect("a parent")).expect("state dir");
    std::fs::write(
        &pin,
        r#"{"member":"scotty","forgeLogin":{"127.0.0.1":"scotty-work"}}"#,
    )
    .expect("write the pin");
    assert_eq!(
        resolver::pinned_login(&checkout, "127.0.0.1").as_deref(),
        Some("scotty-work"),
        "the engine reads the pin out of the project's own app state"
    );

    forge::push(&checkout, &Auth::local(HostKind::Background)).expect("push");
    // A second operation on the same host IN THE SAME DIRECTION: the
    // connector answered once and its answer is what this one reads.
    forge::push(&checkout, &Auth::local(HostKind::Background)).expect("push again");
    // And one in the other direction, which is a different question:
    // `--for read` may be answered by a login that may not push, so the
    // write scoped answer is not replayed for it (D4.1c step 4).
    let advertised = forge::ls_remote_refs(
        &checkout,
        &Auth::local(HostKind::Background),
        &["refs/heads/main"],
    )
    .expect("ls-remote");
    assert!(advertised.contains_key("refs/heads/main"));

    let tokens = machine.calls("token");
    assert_eq!(
        tokens.len(),
        2,
        "asked per host and per direction, never per contact (D1.7): {tokens:?}"
    );
    for call in &tokens {
        assert!(
            call.contains("--login scotty-work"),
            "the pin travels on the call: {call}"
        );
        assert!(
            call.contains("--host-kind background"),
            "and so does the host kind of D1.1: {call}"
        );
    }
    assert_eq!(
        tokens
            .iter()
            .filter(|call| call.contains("--for write"))
            .count(),
        1,
        "a push asks for a login that may write, once for both pushes: {tokens:?}"
    );
    assert_eq!(
        tokens
            .iter()
            .filter(|call| call.contains("--for read"))
            .count(),
        1,
        "and a fetch asks for its own: {tokens:?}"
    );
    assert_eq!(
        machine.calls("web-url").len(),
        2,
        "the twin's address rides with the answer, once per direction"
    );
    drop(machine);
}

/// D1.5: "`probe_write_access_raw` runs on THE TRANSPORT THAT CARRIES
/// THE CREDENTIAL for this operation, not always on the configured
/// remote. If the operation would push over the twin, the probe uses
/// the twin."
#[test]
fn the_probe_runs_on_the_transport_that_carries_the_credential() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");

    let checkout = tmp.path().join("checkout");
    checkout_ahead(&checkout, &forge_dir, "ssh://git@127.0.0.1/forge.git", base);

    forge::probe_write_access(&checkout, &Auth::local(HostKind::Background))
        .expect("the twin carries the credential, so the twin is probed");
    assert!(
        server.requests.load(Ordering::SeqCst) > 0,
        "the probe really reached the twin"
    );
    drop(machine);
}

/// D1.5, the other half: "If neither transport has a credential, the
/// probe is not run at all and the state is `needs_sign_in`, never
/// `no_push_rights`." This closes the contradiction where the Windows
/// case could not produce the state the banner needs: WinCNG reads no
/// openssh-key-v1 file, so that machine has no ssh credential either.
#[test]
fn a_machine_with_no_credential_at_all_is_needs_sign_in_and_never_no_push_rights() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine_without_a_connector();

    let checkout = tmp.path().join("checkout");
    checkout_ahead(&checkout, &forge_dir, "ssh://git@127.0.0.1/forge.git", base);

    let refused = forge::probe_write_access(&checkout, &Auth::local(HostKind::Background))
        .expect_err("there is nothing to probe with");
    assert_eq!(
        contact::failure_of(&refused),
        contact::Failure::NeedsSignIn,
        "{refused}"
    );
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        0,
        "and nothing was dialled to find that out"
    );
    drop(machine);
}

/// D1.5: "`refs/joy/chats` has no libgit2 tracking ref on either remote
/// and needs none. joy keeps its own, `refs/joy/chats-remote` ... after
/// a successful push of the chat ref the engine sets it to the pushed
/// oid as well, so the union merge reconciles against what the forge
/// holds."
#[test]
fn a_chat_push_sets_the_chat_tracking_ref_to_what_the_forge_now_holds() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");

    let checkout = tmp.path().join("checkout");
    let tip = checkout_ahead(&checkout, &forge_dir, "ssh://git@127.0.0.1/forge.git", base);
    {
        let repo = git2::Repository::open(&checkout).expect("open");
        repo.reference("refs/joy/chats", tip, true, "a chat")
            .expect("chat ref");
        assert!(
            repo.find_reference("refs/joy/chats-remote").is_err(),
            "nothing has been pushed yet"
        );
    }

    forge::push_ref(
        &checkout,
        &Auth::local(HostKind::Background),
        "refs/joy/chats",
    )
    .expect("the twin carried the chat ref");

    let repo = git2::Repository::open(&checkout).expect("open");
    assert_eq!(
        repo.refname_to_id("refs/joy/chats-remote").ok(),
        Some(tip),
        "the reconcile runs against what the forge holds"
    );
    assert_eq!(server.pushes.load(Ordering::SeqCst), 1);
    drop(machine);
}

/// D1.2 rule 1: a CONFIGURED https remote takes the forge token and
/// then a credential from joy's own helper runner, all inside one
/// contact. There is no twin to build and no second contact to make.
///
/// It is also the other half of the tracking ref rule: over a NAMED
/// remote `git_remote_upload` rebuilds the active refspecs from the
/// remote's configured ones (remote.c:2995-2997), so libgit2 writes
/// `refs/remotes/origin/main` itself and the engine writes nothing.
#[test]
fn a_configured_https_remote_uses_the_connector_s_token_in_one_contact() {
    let _serial = lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge_dir = tmp.path().join("forge.git");
    let base = forge_repository(&forge_dir);
    let server = serve(forge_dir.clone(), Answer::Ok);
    let machine = machine(&server.url("forge.git"), "a-token");

    let checkout = tmp.path().join("checkout");
    let tip = checkout_ahead(&checkout, &forge_dir, &server.url("forge.git"), base);

    forge::push(&checkout, &Auth::local(HostKind::Background)).expect("the token carried it");

    assert_eq!(
        forge::ahead_behind(&checkout).expect("ahead behind"),
        (0, 0),
        "a named remote updates its own tracking ref"
    );
    assert_eq!(server.pushes.load(Ordering::SeqCst), 1, "one contact");
    let bare = git2::Repository::open_bare(&forge_dir).expect("bare");
    assert_eq!(bare.refname_to_id("refs/heads/main").expect("main"), tip);
    assert!(
        resolver::recall("127.0.0.1").is_none(),
        "an https remote has no ssh story to remember"
    );
    drop(machine);
}
