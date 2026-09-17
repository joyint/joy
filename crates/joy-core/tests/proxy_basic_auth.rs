// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The acceptance sentence of J4p, run: with a proxy variable set and a
//! proxy that requires Basic, a fetch succeeds, and no proxy password
//! appears in any log line or error text (design D1.11).
//!
//! The proxy is in this process: a minimal HTTP proxy on the loopback
//! interface that answers `407 Proxy Authentication Required` until it
//! is given `Proxy-Authorization: Basic ...`, counts every request it
//! answers, and then serves the git smart-HTTP conversation itself, as
//! a proxy that forwards to an origin would. It needs no network.
//!
//! The remote it is asked for is `http://forge.invalid/forge.git`.
//! `.invalid` is reserved and resolves nowhere (RFC 2606), so this test
//! cannot pass by accident: a joy that ignored the proxy would fail to
//! resolve the host, and only a contact that really went through the
//! proxy can return objects.
//!
//! Two notes on how literally to read it. The first test's remote
//! speaks plain http, as in tests/forge_request_budget.rs, so the
//! variable that decides there is `HTTP_PROXY`. `HTTPS_PROXY` is the
//! second test's: an https remote tunnels through the same proxy with
//! `CONNECT`, and the credential joy injected into the proxy URL
//! arrives on the wire there, which is the acceptance sentence as it is
//! written. That one stops at the TLS handshake, because a certificate
//! authority is the one thing this test cannot install. And the file
//! owns its process: the proxy variables and the tracing subscriber are
//! process state.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use joy_core::vcs::forge::{fetch_ref, Auth};
use joy_core::vcs::proxy::{self, Environment, Outcome};

/// The proxy variables, the gap table and the global tracing subscriber
/// are process state: the four tests below run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

const CHATS_REF: &str = "refs/joy/chats";
const PROXY_USER: &str = "picard";
/// The one string that must never leave this process.
const PROXY_PASSWORD: &str = "tea-earl-grey-hot";

/// One request the proxy answered.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    verb: String,
    target: String,
    /// Whether this request was addressed to a PROXY: an absolute
    /// request target, or a `CONNECT` tunnel. A contact that bypassed
    /// the proxy asks for a path and nothing else.
    proxied: bool,
    authenticated: bool,
    status: u16,
}

struct Proxy {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    requests: Arc<AtomicUsize>,
}

fn pkt(line: &str) -> Vec<u8> {
    format!("{:04x}{line}", line.len() + 4).into_bytes()
}

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

/// Base64 of `user:password`, which is all Basic is (RFC 7617). Written
/// here rather than taken from a crate, because the test has to know
/// the exact bytes it demands.
fn basic(user: &str, password: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let raw = format!("{user}:{password}");
    let bytes = raw.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// An HTTP proxy that requires Basic and then serves the git
/// conversation for the absolute-form request it was handed.
fn serve_proxy(forge: std::path::PathBuf, refs: Vec<(String, git2::Oid)>, tip: git2::Oid) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(AtomicUsize::new(0));
    let expected = format!("Basic {}", basic(PROXY_USER, PROXY_PASSWORD));
    let (seen_thread, count_thread) = (seen.clone(), requests.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let refs = refs.clone();
            let forge = forge.clone();
            let seen = seen_thread.clone();
            let count = count_thread.clone();
            let expected = expected.clone();
            std::thread::spawn(move || loop {
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                    return;
                }
                let mut parts = request_line.split_whitespace();
                let verb = parts.next().unwrap_or_default().to_string();
                let target = parts.next().unwrap_or_default().to_string();
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
                    let (name, value) = header.split_once(':').unwrap_or((header.as_str(), ""));
                    if name.eq_ignore_ascii_case("proxy-authorization") {
                        authenticated = value.trim() == expected;
                    }
                    if name.eq_ignore_ascii_case("content-length") {
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
                let proxied = verb == "CONNECT" || target.starts_with("http://");
                let status = if proxied && !authenticated {
                    respond(
                        &mut stream,
                        407,
                        "Proxy Authentication Required",
                        "Proxy-Authenticate: Basic realm=\"acme\"\r\n",
                        b"",
                    );
                    407
                } else if verb == "CONNECT" {
                    // The tunnel is opened and nothing comes through
                    // it: what is on the other side would need TLS.
                    respond(&mut stream, 200, "Connection Established", "", b"");
                    seen.lock().unwrap().push(Seen {
                        verb,
                        target,
                        proxied,
                        authenticated,
                        status: 200,
                    });
                    return;
                } else if target.contains("/info/refs") {
                    respond(
                        &mut stream,
                        200,
                        "OK",
                        "Content-Type: application/x-git-upload-pack-advertisement\r\nCache-Control: no-cache\r\n",
                        &advertisement(&refs),
                    );
                    200
                } else if target.ends_with("/git-upload-pack") {
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
                seen.lock().unwrap().push(Seen {
                    verb: verb.clone(),
                    target: target.clone(),
                    proxied,
                    authenticated,
                    status,
                });
            });
        }
    });
    Proxy {
        port,
        seen,
        requests,
    }
}

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

// ---- every tracing line this process wrote ----------------------------

/// The subscriber is installed GLOBALLY, once, and not with the thread
/// local `with_default`: "no proxy password in any log line" is a
/// promise about the whole process, and joy's credential helper runner
/// writes on threads of its own (it reads the helper's two pipes on two
/// spawned threads), which a thread local subscriber would never see.
/// This binary owns its process, so it may take the global slot.
static RECORDER: std::sync::OnceLock<Recorder> = std::sync::OnceLock::new();

/// The global recorder, emptied for the test that is about to run. The
/// tests hold [`SERIAL`] while they use it.
fn recorder() -> Recorder {
    let recorder = RECORDER.get_or_init(|| {
        let recorder = Recorder::default();
        tracing::subscriber::set_global_default(recorder.clone())
            .expect("this test binary owns its process");
        recorder
    });
    recorder.lines.lock().unwrap().clear();
    recorder.clone()
}

#[derive(Clone, Default)]
struct Recorder {
    lines: Arc<Mutex<Vec<String>>>,
}

struct Visitor<'a>(&'a mut String);

impl tracing::field::Visit for Visitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={:?} ", field.name(), value));
    }
}

impl tracing::Subscriber for Recorder {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut text = format!("span {} ", span.metadata().name());
        span.record(&mut Visitor(&mut text));
        self.lines.lock().unwrap().push(text);
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut text = String::new();
        values.record(&mut Visitor(&mut text));
        self.lines.lock().unwrap().push(text);
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = format!("event {} ", event.metadata().target());
        event.record(&mut Visitor(&mut text));
        self.lines.lock().unwrap().push(text);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// A credential helper that answers for the proxy and for nothing else.
fn helper_script(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(
        path,
        format!(
            "#!/bin/sh\nif [ \"$1\" = get ]; then echo username={PROXY_USER}; \
             echo password={PROXY_PASSWORD}; fi\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The acceptance sentence, run: a proxy that requires Basic, a fetch
/// that succeeds, and no password in any log line or error text.
///
/// One listener plays both parts, because that is what libgit2 does
/// with a plain http remote: `use_connect_proxy` is
/// `client->proxy.url.host && !strcmp(client->server.url.scheme,
/// "https")` (httpclient.c:704-706), so for an `http://` remote it
/// opens no proxy connection at all and `server_connect` dials the
/// origin (httpclient.c:1032-1045), while `generate_request` still
/// writes the ABSOLUTE form and the `Proxy-Authorization` header
/// (httpclient.c:727-729, :696). The proxy protocol is therefore
/// exactly what goes over this socket, and the assertions below read
/// it: an absolute request target, one 407, and the replay that
/// carries Basic. An `https://` remote takes the CONNECT path instead,
/// which the next test drives.
#[test]
fn a_fetch_through_a_proxy_that_requires_basic_succeeds_and_leaks_nothing() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge = tmp.path().join("forge.git");
    let (refs, tip) = forge_repository(&forge);
    let proxy = serve_proxy(forge.clone(), refs, tip);
    let address = format!("http://127.0.0.1:{}", proxy.port);

    // the throttle is not what this test measures
    joy_core::vcs::contact::set_gaps("127.0.0.1=0,default=0");
    // The desktop and the CLI read the same variables; this process is
    // the shell profile.
    std::env::set_var("HTTP_PROXY", &address);
    std::env::set_var("HTTPS_PROXY", &address);
    std::env::remove_var("http_proxy");
    std::env::remove_var("https_proxy");
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");

    let checkout = tmp.path().join("checkout");
    let repo = git2::Repository::init(&checkout).expect("init");
    repo.remote("origin", &format!("{address}/forge.git"))
        .expect("remote");
    // The proxy's own login, where a person would put it: a credential
    // helper keyed on the proxy (D1.11: `protocol=http`,
    // `host=<proxyhost>[:port]`).
    let helper = tmp.path().join("proxy-helper");
    helper_script(&helper);
    let mut config = repo.config().expect("config");
    config
        .set_str(
            &format!("credential.{address}.helper"),
            helper.to_str().expect("utf-8 path"),
        )
        .expect("set helper");
    drop(config);
    drop(repo);

    let recorder = recorder();
    let fetched = fetch_ref(
        &checkout,
        &Auth::LocalAs(joy_core::vcs::HostKind::Background),
        CHATS_REF,
        "refs/joy/chats-tracking",
    )
    .expect("the fetch went through the proxy");
    assert!(fetched, "the ref was advertised through the proxy");

    // The objects really arrived.
    let repo = git2::Repository::open(&checkout).expect("open");
    assert_eq!(
        repo.find_reference("refs/joy/chats-tracking")
            .expect("tracking ref")
            .target(),
        Some(tip)
    );
    assert!(repo.find_commit(tip).is_ok(), "the commit was downloaded");

    // The proxy was really asked, and it really demanded a login: the
    // first request carried no credential and was refused.
    let seen = proxy.seen.lock().unwrap().clone();
    let counted = proxy.requests.load(Ordering::SeqCst);
    assert!(
        counted >= 3,
        "one 407, one replay and one upload-pack: {seen:#?}"
    );
    assert_eq!(
        seen.iter().filter(|r| r.status == 407).count(),
        1,
        "exactly one challenge: {seen:#?}"
    );
    assert!(
        seen.iter().any(|r| r.authenticated && r.status == 200),
        "the replay carried Basic and was served: {seen:#?}"
    );
    assert!(
        seen.iter()
            .all(|r| r.target.starts_with("http://127.0.0.1")),
        "a proxy is asked in absolute form, which is the proof that joy's \
         ProxyOptions reached libgit2: {seen:#?}"
    );

    // And not one line joy wrote carries the password.
    let lines = recorder.lines.lock().unwrap().clone();
    assert!(!lines.is_empty(), "the run logged something at all");
    for line in &lines {
        assert!(
            !line.contains(PROXY_PASSWORD),
            "a log line carries the proxy password: {line}"
        );
    }
    // the proxy IS named, because a 407 has to name it (D1.8c)
    assert!(
        lines.iter().any(|line| line.contains("127.0.0.1")),
        "the proxy is named in the log: {lines:#?}"
    );

    // `HTTPS_PROXY` is the variable the acceptance sentence names: it
    // is read, and it is the one that decides for an https remote.
    let env = Environment::of_this_process();
    assert_eq!(env.https_proxy.as_deref(), Some(address.as_str()));
    let decided = proxy::options_for("https://github.com/o/r.git", None)
        .expect("an http proxy is not refused");
    assert_eq!(decided.outcome(), Outcome::Specified);
    assert_eq!(
        decided.name(),
        Some(format!("127.0.0.1:{}", proxy.port).as_str())
    );

    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("HTTPS_PROXY");
    joy_core::vcs::contact::set_gaps("");
}

/// The https half: an `https://` remote goes through `CONNECT`
/// (httpclient.c:978-1020), and that is where the credential joy
/// injected into the proxy URL has to arrive, because git2 hardwires
/// `credentials: None` in `ProxyOptions::raw` and there is no callback
/// to answer a 407 with.
///
/// The tunnel is opened and nothing comes through it: a TLS handshake
/// would need a certificate authority this test cannot install, so the
/// fetch fails after the proxy said 200. What is asserted is the whole
/// proxy conversation up to that point, and that the failure text
/// carries no password.
///
/// It needs no network and no TLS backend, so it runs in the default
/// suite with the rest: the proxy is on the loopback interface and
/// `forge.invalid` is never resolved by anything, because libgit2 dials
/// the PROXY and asks it for `CONNECT forge.invalid:443`. This is the
/// test that proves the acceptance sentence as it is written, with
/// `HTTPS_PROXY` set, so it must not be one that nobody runs.
#[test]
fn an_https_remote_reaches_the_proxy_through_connect_with_its_credential() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge = tmp.path().join("forge.git");
    let (refs, tip) = forge_repository(&forge);
    let proxy = serve_proxy(forge, refs, tip);
    let address = format!("http://127.0.0.1:{}", proxy.port);

    joy_core::vcs::contact::set_gaps("forge.invalid=0,127.0.0.1=0,default=0");
    std::env::set_var("HTTPS_PROXY", &address);
    std::env::remove_var("https_proxy");
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");

    let checkout = tmp.path().join("checkout");
    let repo = git2::Repository::init(&checkout).expect("init");
    repo.remote("origin", "https://forge.invalid/forge.git")
        .expect("remote");
    let helper = tmp.path().join("proxy-helper");
    helper_script(&helper);
    let mut config = repo.config().expect("config");
    config
        .set_str(
            &format!("credential.{address}.helper"),
            helper.to_str().expect("utf-8 path"),
        )
        .expect("set helper");
    drop(config);
    drop(repo);

    let recorder = recorder();
    let outcome = fetch_ref(
        &checkout,
        &Auth::LocalAs(joy_core::vcs::HostKind::Background),
        CHATS_REF,
        "refs/joy/chats-tracking",
    );
    let text = match outcome {
        Ok(_) => String::new(),
        Err(e) => format!("{e:#}"),
    };

    let seen = proxy.seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|r| r.verb == "CONNECT"),
        "an https remote tunnels through the proxy: {seen:#?}"
    );
    assert!(
        seen.iter()
            .any(|r| r.verb == "CONNECT" && r.target.starts_with("forge.invalid:")),
        "the tunnel names the forge and its port: {seen:#?}"
    );
    assert_eq!(
        seen.iter().filter(|r| r.status == 407).count(),
        1,
        "one challenge, and only one: {seen:#?}"
    );
    assert!(
        seen.iter().any(|r| r.verb == "CONNECT" && r.authenticated),
        "the credential joy injected into the proxy URL arrived: {seen:#?}"
    );

    assert!(
        !text.contains(PROXY_PASSWORD),
        "the failure text carries the proxy password: {text}"
    );
    for line in recorder.lines.lock().unwrap().iter() {
        assert!(
            !line.contains(PROXY_PASSWORD),
            "a log line carries the proxy password: {line}"
        );
    }

    std::env::remove_var("HTTPS_PROXY");
    joy_core::vcs::contact::set_gaps("");
}

/// The other half of the acceptance sentence: with
/// `NO_PROXY="a.com, b.com"` the contact to the second host bypasses
/// the proxy. The entry after the comma is the one libgit2 loses,
/// because `git_net_url_matches_pattern_list` compares the bytes
/// between two commas as they stand (net.c:1126-1141).
///
/// The proof is on the wire: a contact that bypassed the proxy asks
/// for a PATH and sends no `Proxy-Authorization`, where the test above
/// saw an absolute target and a 407. The same listener serves both,
/// and it demands the proxy login only from a request that was
/// addressed to a proxy.
#[test]
fn no_proxy_keeps_a_named_host_away_from_the_proxy() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let forge = tmp.path().join("forge.git");
    let (refs, tip) = forge_repository(&forge);
    let proxy = serve_proxy(forge.clone(), refs, tip);
    let address = format!("http://127.0.0.1:{}", proxy.port);

    joy_core::vcs::contact::set_gaps("127.0.0.1=0,default=0");
    std::env::set_var("HTTP_PROXY", &address);
    std::env::remove_var("http_proxy");
    std::env::set_var("NO_PROXY", "a.com, 127.0.0.1");

    let checkout = tmp.path().join("checkout");
    let repo = git2::Repository::init(&checkout).expect("init");
    repo.remote("origin", &format!("{address}/forge.git"))
        .expect("remote");
    drop(repo);

    let fetched = fetch_ref(
        &checkout,
        &Auth::LocalAs(joy_core::vcs::HostKind::Background),
        CHATS_REF,
        "refs/joy/chats-tracking",
    )
    .expect("the bypassed contact reached the forge itself");
    assert!(fetched, "and it brought the ref back");

    let seen = proxy.seen.lock().unwrap().clone();
    assert!(!seen.is_empty(), "the forge was contacted: {seen:#?}");
    assert!(
        seen.iter().all(|r| !r.proxied),
        "not one request was addressed to a proxy: {seen:#?}"
    );
    assert_eq!(
        seen.iter().filter(|r| r.status == 407).count(),
        0,
        "and no proxy login was ever demanded: {seen:#?}"
    );

    std::env::remove_var("NO_PROXY");
    std::env::remove_var("HTTP_PROXY");
    joy_core::vcs::contact::set_gaps("");
}

/// A SOCKS proxy is refused by name before anything is dialled (D1.11).
#[test]
fn a_socks_proxy_is_refused_before_the_contact() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let checkout = tmp.path().join("checkout");
    let repo = git2::Repository::init(&checkout).expect("init");
    repo.remote("origin", "http://forge.invalid/forge.git")
        .expect("remote");
    drop(repo);

    joy_core::vcs::contact::set_gaps("forge.invalid=0,default=0");
    std::env::set_var("HTTP_PROXY", "socks5://127.0.0.1:1080");
    std::env::remove_var("NO_PROXY");

    let failure = fetch_ref(
        &checkout,
        &Auth::LocalAs(joy_core::vcs::HostKind::Background),
        CHATS_REF,
        "refs/joy/chats-tracking",
    )
    .expect_err("joy cannot speak SOCKS");
    assert_eq!(
        format!("{failure}"),
        "joy cannot use the SOCKS proxy socks5://127.0.0.1:1080; it supports HTTP and HTTPS \
         proxies only."
    );

    std::env::remove_var("HTTP_PROXY");
    joy_core::vcs::contact::set_gaps("");
}
