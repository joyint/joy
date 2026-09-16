// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The classifier corpora and the budget tests of JOY-0295-36.
//!
//! The two corpora are the point of D1.8a: libgit2 compiles `http.c`
//! everywhere except Windows and `winhttp.c` only on Windows, the two
//! producers share no sentence, and every `GIT_ERROR_OS` message carries
//! a tail the operating system wrote in the user's own language. Each
//! corpus below is therefore read twice, once with an English tail and
//! once with a German one, and both readings must give the same state.

use super::*;
use git2::{ErrorClass as Class, ErrorCode as Code};

/// The throttle, the gate and the oracle are process-wide: tests that
/// touch them run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn error(code: Code, class: Class, message: &str) -> git2::Error {
    git2::Error::new(code, class, message)
}

/// Evidence for a contact to `host`, with everything else named.
fn evidence(
    err: git2::Error,
    host: &str,
    transport: Transport,
    direction: ContactDirection,
    credential: CredentialSource,
    token_worked_before: bool,
) -> ContactEvidence {
    ContactEvidence {
        error: err,
        transport,
        direction,
        credential,
        token_worked_before,
        host: host.to_string(),
    }
}

/// An https fetch with a token that has already worked on this host.
fn https_fetch(err: git2::Error, host: &str) -> ContactEvidence {
    evidence(
        err,
        host,
        Transport::Https,
        ContactDirection::Fetch,
        CredentialSource::TokenPresented,
        true,
    )
}

// ---- the vocabulary --------------------------------------------------

/// Every state has its own word, and a reader that knows only the four
/// old ones still gets a word it understands (D1.8b).
#[test]
fn every_state_has_a_word_and_an_old_word() {
    let all = [
        (Failure::NeedsSignIn, "needs_sign_in", "denied"),
        (Failure::NeedsOrgApproval, "needs_org_approval", "denied"),
        (Failure::NeedsSso, "needs_sso", "denied"),
        (Failure::NoPushRights, "no_push_rights", "denied"),
        (Failure::NeedsHostTrust, "needs_host_trust", "denied"),
        (Failure::ScopeMissing, "scope_missing", "denied"),
        (Failure::PluginMissing, "plugin_missing", "error"),
        (Failure::PluginOutdated, "plugin_outdated", "error"),
        (Failure::TlsUntrusted, "tls_untrusted", "error"),
        (Failure::ProxyAuth, "proxy_auth", "error"),
        (Failure::RateLimited, "rate_limited", "rate_limited"),
        (Failure::Offline, "offline", "offline"),
        (Failure::Denied, "denied", "denied"),
        (Failure::Error, "error", "error"),
    ];
    let mut words: Vec<&str> = all.iter().map(|(f, _, _)| f.reason()).collect();
    words.sort_unstable();
    words.dedup();
    assert_eq!(words.len(), all.len(), "no two states share a word");
    for (failure, word, old) in all {
        assert_eq!(failure.reason(), word);
        assert_eq!(failure.old_word(), old);
        assert!(
            !failure.sentence("github.com").is_empty(),
            "{word} has a sentence"
        );
    }
    // the sentence names the host, and never libgit2
    assert_eq!(
        Failure::NeedsSignIn.sentence("github.com"),
        "Sign in to GitHub to sync."
    );
    assert_eq!(
        Failure::Offline.sentence("codeberg.org"),
        "No connection to codeberg.org."
    );
}

/// The status number comes out of exactly the two formats libgit2
/// prints, and out of no other digits in the sentence (D1.8a).
#[test]
fn the_status_number_comes_only_from_the_two_libgit2_formats() {
    assert_eq!(
        http_status("unexpected http status code: 404"),
        Some(404),
        "the http.c format of every non Windows build"
    );
    assert_eq!(
        http_status("request failed with status code: 401"),
        Some(401),
        "the winhttp.c format of every Windows build"
    );
    // digits that are NOT a status: today's substring scan called all
    // three of these "denied" (contact.rs:88-96 before this package)
    assert_eq!(http_status("cannot push refs/heads/fix-403"), None);
    assert_eq!(http_status("object 401f0a2 is missing"), None);
    assert_eq!(http_status("could not find /srv/401/repo.git"), None);
}

// ---- the OpenSSL corpus (every non Windows build) --------------------

/// The Linux and macOS corpus: libgit2 built against `http.c`, with the
/// operating system's own message appended to every `GIT_ERROR_OS` and
/// `GIT_ERROR_NET` sentence. The German reading must equal the English
/// one, because joy reads the code, the class and the status number and
/// never the prose.
#[test]
fn the_openssl_corpus_reads_the_same_in_english_and_in_german() {
    // (code, class, english message, german message, state)
    let corpus: Vec<(Code, Class, String, String, Failure)> = vec![
        (
            Code::GenericError,
            Class::Net,
            "failed to resolve address for codeberg.org: Name or service not known".into(),
            "failed to resolve address for codeberg.org: Name oder Dienst nicht bekannt".into(),
            Failure::Offline,
        ),
        (
            Code::GenericError,
            Class::Net,
            "connection failed: Connection reset by peer".into(),
            "connection failed: Die Verbindung wurde vom Kommunikationspartner zurückgesetzt"
                .into(),
            Failure::Offline,
        ),
        (
            Code::GenericError,
            Class::Os,
            "failed to connect to host: Connection timed out".into(),
            "failed to connect to host: Zeitüberschreitung der Verbindung".into(),
            Failure::Offline,
        ),
        (
            Code::Timeout,
            Class::Net,
            "the operation timed out".into(),
            "Zeitüberschreitung bei der Operation".into(),
            Failure::Offline,
        ),
        (
            // the wait bound joy sets itself (JOY-0278-85): an SSL
            // syscall failure is silence, not a certificate fault
            Code::GenericError,
            Class::Ssl,
            "SSL error: syscall failure: Resource temporarily unavailable".into(),
            "SSL error: syscall failure: Ressource vorübergehend nicht verfügbar".into(),
            Failure::Offline,
        ),
        (
            // streams/openssl.c:381-384, libgit2's own literal
            Code::Certificate,
            Class::Ssl,
            "the SSL certificate is invalid".into(),
            "the SSL certificate is invalid".into(),
            Failure::TlsUntrusted,
        ),
        (
            // stransport.c:117-120, the Apple build
            Code::Certificate,
            Class::Ssl,
            "untrusted connection error".into(),
            "untrusted connection error".into(),
            Failure::TlsUntrusted,
        ),
        (
            Code::Auth,
            Class::Http,
            "unexpected authentication failure".into(),
            "unexpected authentication failure".into(),
            Failure::NeedsSignIn,
        ),
        (
            // JP-00D8-94: the SERVER sentence, which must not be read
            // as the proxy one
            Code::Auth,
            Class::Http,
            "server requires authentication that we do not support".into(),
            "server requires authentication that we do not support".into(),
            Failure::NeedsSignIn,
        ),
        (
            // http.c:165-169 with server type "proxy" (http.c:98)
            Code::Auth,
            Class::Http,
            "proxy authentication required but no callback set".into(),
            "proxy authentication required but no callback set".into(),
            Failure::ProxyAuth,
        ),
        (
            // http.c:206-208
            Code::Auth,
            Class::Http,
            "proxy requires authentication that we do not support".into(),
            "proxy requires authentication that we do not support".into(),
            Failure::ProxyAuth,
        ),
        (
            // http.c:439-440, "not GIT_EAUTH, because the exact cause
            // is unclear"
            Code::GenericError,
            Class::Http,
            "too many redirects or authentication replays".into(),
            "too many redirects or authentication replays".into(),
            Failure::NeedsSignIn,
        ),
        (
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 401".into(),
            "unexpected http status code: 401".into(),
            Failure::NeedsSignIn,
        ),
        (
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 429".into(),
            "unexpected http status code: 429".into(),
            Failure::RateLimited,
        ),
        (
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 503".into(),
            "unexpected http status code: 503".into(),
            Failure::Offline,
        ),
        (
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 500".into(),
            "unexpected http status code: 500".into(),
            Failure::Error,
        ),
    ];

    for (code, class, english, german, expected) in corpus {
        for message in [&english, &german] {
            let ev = https_fetch(error(code, class, message), "codeberg.org");
            assert_eq!(classify(&ev), expected, "{class:?}/{code:?}: {message}");
        }
    }
}

/// The ssh half of the same build (`ssh_libssh2.c`): four states, and
/// the remote's own stderr never reaches the banner.
#[test]
fn the_ssh_corpus_separates_a_host_key_from_a_login_from_a_refusal() {
    let key = evidence(
        error(
            Code::Certificate,
            Class::Ssh,
            "invalid or unknown remote ssh hostkey",
        ),
        "github.com",
        Transport::Ssh,
        ContactDirection::Fetch,
        CredentialSource::AgentPresented,
        false,
    );
    assert_eq!(classify(&key), Failure::NeedsHostTrust);

    let login = evidence(
        error(
            Code::Auth,
            Class::Ssh,
            "username does not match previous request",
        ),
        "github.com",
        Transport::Ssh,
        ContactDirection::Fetch,
        CredentialSource::AgentPresented,
        false,
    );
    assert_eq!(classify(&login), Failure::NeedsSignIn);

    // ssh_libssh2.c:138 hands the remote's own stderr through
    let refusal = |direction| {
        evidence(
            error(
                Code::Eof,
                Class::Ssh,
                "ERROR: Permission to joyint/joy.git denied to someone.",
            ),
            "github.com",
            Transport::Ssh,
            direction,
            CredentialSource::AgentPresented,
            true,
        )
    };
    assert_eq!(
        classify(&refusal(ContactDirection::Push)),
        Failure::NoPushRights
    );
    assert_eq!(classify(&refusal(ContactDirection::Fetch)), Failure::Denied);
    // the forge's sentence is in the detail line, not in the banner
    let v = verdict(&refusal(ContactDirection::Push));
    assert!(!v.sentence.contains("Permission to joyint/joy.git"));
    assert!(v.detail.contains("Permission to joyint/joy.git"));
}

// ---- the WinHTTP corpus (every Windows build) ------------------------

/// The Windows corpus: `winhttp.c` shares no sentence with `http.c`, a
/// 401 that no credential satisfies is not `GIT_EAUTH` at all
/// (winhttp.c:998-1046, :1274), and every `GIT_ERROR_OS` message carries
/// `FormatMessageW` output in the user default language.
#[test]
fn the_winhttp_corpus_reads_the_same_in_english_and_in_german() {
    let corpus: Vec<(Code, Class, String, String, Failure)> = vec![
        (
            // the Windows 401: a positive pass through, then -1 with a
            // status sentence
            Code::GenericError,
            Class::Http,
            "request failed with status code: 401".into(),
            "request failed with status code: 401".into(),
            Failure::NeedsSignIn,
        ),
        (
            Code::GenericError,
            Class::Http,
            "request failed with status code: 429".into(),
            "request failed with status code: 429".into(),
            Failure::RateLimited,
        ),
        (
            Code::GenericError,
            Class::Http,
            "request failed with status code: 502".into(),
            "request failed with status code: 502".into(),
            Failure::Offline,
        ),
        (
            // winhttp.c:950, GIT_ERROR_OS with the German tail
            Code::GenericError,
            Class::Os,
            "failed to send request: The operation timed out".into(),
            "failed to send request: Zeitüberschreitung des Vorgangs".into(),
            Failure::Offline,
        ),
        (
            // winhttp.c:870
            Code::GenericError,
            Class::Os,
            "failed to connect to host: A connection attempt failed".into(),
            "failed to connect to host: Es konnte keine Verbindung hergestellt werden".into(),
            Failure::Offline,
        ),
    ];
    for (code, class, english, german, expected) in corpus {
        for message in [&english, &german] {
            let ev = https_fetch(error(code, class, message), "github.com");
            assert_eq!(classify(&ev), expected, "{class:?}/{code:?}: {message}");
        }
    }

    // the seven certificate sentences of winhttp.c:718-740, class Http
    for sentence in WINHTTP_CERTIFICATE_SENTENCES {
        let ev = https_fetch(
            error(Code::GenericError, Class::Http, sentence),
            "github.com",
        );
        assert_eq!(classify(&ev), Failure::TlsUntrusted, "{sentence}");
        assert_eq!(
            verdict(&ev).sentence,
            "The certificate for github.com is not trusted by this machine's certificate store."
        );
    }
}

// ---- the rules that need more than the error -------------------------

/// A 404 on a fetch is never "offline": either the organisation has not
/// approved Joy, or the repository is not there for this login (D1.8b).
#[test]
fn a_404_is_never_offline() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    clear_oracle();
    let not_found = error(
        Code::GenericError,
        Class::Http,
        "unexpected http status code: 404",
    );
    let github = https_fetch(not_found, "github.com");
    assert_eq!(
        classify(&github),
        Failure::NeedsOrgApproval,
        "a token that works elsewhere plus a 404 on GitHub is the approval wall"
    );

    let elsewhere = evidence(
        error(
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 404",
        ),
        "codeberg.org",
        Transport::Https,
        ContactDirection::Fetch,
        CredentialSource::TokenPresented,
        false,
    );
    let v = verdict(&elsewhere);
    assert_eq!(v.failure, Failure::Error);
    assert!(
        v.sentence.contains("does not have this repository"),
        "{}",
        v.sentence
    );
    assert_ne!(v.failure, Failure::Offline);
}

/// A push that is refused where a read worked is "you can read this and
/// not write it", not "sign in" (D1.8b).
#[test]
fn a_refused_push_after_a_working_read_is_no_push_rights() {
    for status in ["403", "404"] {
        let ev = evidence(
            error(
                Code::GenericError,
                Class::Http,
                &format!("unexpected http status code: {status}"),
            ),
            "codeberg.org",
            Transport::Https,
            ContactDirection::Push,
            CredentialSource::TokenPresented,
            true,
        );
        assert_eq!(classify(&ev), Failure::NoPushRights, "status {status}");
    }
}

/// GitLab's 403 on git is the failed authentication ban: it sends no
/// header and cannot be cleared by signing in, so the wait comes from
/// GitLab's own documentation and the connector is never asked (D2.10).
#[test]
fn a_gitlab_403_waits_the_documented_ban_and_asks_nobody() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    clear_oracle();
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    set_oracle(std::sync::Arc::new(CountingOracle {
        asked: asked.clone(),
        answer: Some(OracleAnswer::Denied),
    }));

    let ban = |host: &str| {
        let ev = https_fetch(
            error(
                Code::GenericError,
                Class::Http,
                "unexpected http status code: 403",
            ),
            host,
        );
        verdict(&ev)
    };
    let com = ban("gitlab.com");
    assert_eq!(com.failure, Failure::RateLimited);
    assert_eq!(com.wait, Some(Duration::from_secs(15 * 60)));
    let own = ban("gitlab.acme.example");
    assert_eq!(own.failure, Failure::RateLimited);
    assert_eq!(own.wait, Some(Duration::from_secs(60 * 60)));
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the oracle is not consulted on GitLab hosts"
    );

    // nor on the Gitea family (Codeberg's limiter is the same per IP
    // bucket the git request just met)
    let _ = ban("codeberg.org");
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0);
    clear_oracle();
}

struct CountingOracle {
    asked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    answer: Option<OracleAnswer>,
}

impl RateLimitOracle for CountingOracle {
    fn ask(&self, _host: &str, _status: u16) -> Option<OracleAnswer> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.answer.clone()
    }
}

/// The oracle hook of D2.10: asked on a GitHub host after a 403, asked
/// once per host per strike window, and its answer is the state.
#[test]
fn the_oracle_is_asked_once_per_host_and_decides_the_github_403() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    clear_oracle();
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    set_oracle(std::sync::Arc::new(CountingOracle {
        asked: asked.clone(),
        answer: Some(OracleAnswer::NeedsOrgApproval),
    }));
    let forbidden = || {
        https_fetch(
            error(
                Code::GenericError,
                Class::Http,
                "unexpected http status code: 403",
            ),
            "github.com",
        )
    };
    assert_eq!(classify(&forbidden()), Failure::NeedsOrgApproval);
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    // inside the strike window the connector is not asked again; without
    // an answer the state stays the honest "error"
    assert_eq!(classify(&forbidden()), Failure::Error);
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    clear_oracle();
}

/// The connector's own answers are states too (D1.8b, first four rows).
#[test]
fn the_connectors_answers_are_states() {
    assert_eq!(
        classify_plugin(&PluginEvidence::Missing),
        Failure::PluginMissing
    );
    assert_eq!(
        classify_plugin(&PluginEvidence::Outdated),
        Failure::PluginOutdated
    );
    assert_eq!(
        classify_plugin(&PluginEvidence::ScopeMissing),
        Failure::ScopeMissing
    );
    assert_eq!(
        classify_plugin(&PluginEvidence::NeedsSso {
            url: Some("https://github.com/orgs/acme/sso".into())
        }),
        Failure::NeedsSso
    );
}

/// The detail line's grammar (D1.8b): `source: outcome` parts joined by
/// "; ", and it never reaches a surface.
#[test]
fn the_detail_line_lists_every_source_that_was_tried() {
    let mut detail = DetailLine::new();
    assert!(detail.is_empty());
    detail.tried("agent", "no identities");
    detail.tried(
        "key ~/.ssh/id_ed25519",
        "passphrase needed (skipped, background)",
    );
    detail.tried(
        "helper 'manager'",
        "fatal: Cannot prompt because user interactivity has been disabled.",
    );
    detail.note("no forge login for github.com");
    assert_eq!(
        detail.to_string(),
        "agent: no identities; key ~/.ssh/id_ed25519: passphrase needed (skipped, background); helper 'manager': fatal: Cannot prompt because user interactivity has been disabled.; no forge login for github.com"
    );
}

/// A failure carries the plain sentence, and libgit2's words stay in the
/// detail line where only a log and a details view can see them.
#[test]
fn the_error_shows_the_sentence_and_hides_the_engine() {
    let ev = https_fetch(
        error(
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 401",
        ),
        "github.com",
    );
    let e = failed(&ev);
    assert_eq!(failure_of(&e), Failure::NeedsSignIn);
    assert_eq!(e.to_string(), "Sign in to GitHub to sync. (sign in)");
    assert!(!e.to_string().contains("401"));
    assert!(detail_of(&e).unwrap().contains("401"));
}

/// An error that never passed the classifier is not guessed at from its
/// prose: the forge answered, so it is `error` and never `offline`.
#[test]
fn an_unclassified_error_is_not_read_for_words() {
    let e = anyhow::anyhow!("chat x is in the retired pre-sealing layout");
    assert_eq!(failure_of(&e), Failure::Error);
    assert_eq!(failure_of(&e).reason(), "error");
    let e = anyhow::anyhow!("could not resolve host: timed out, 403, rate limit");
    assert_eq!(
        failure_of(&e),
        Failure::Error,
        "no state is decided by digits or words in a random sentence"
    );
}

// ---- the budget, the poll period and the strikes ---------------------

#[test]
fn hosts_and_names_come_out_of_every_url_shape() {
    assert_eq!(host_of("https://codeberg.org/joyint/x.git"), "codeberg.org");
    assert_eq!(host_of("git@github.com:joyint/x.git"), "github.com");
    assert_eq!(host_of("https://user:tok@gitlab.com/a/b"), "gitlab.com");
    assert_eq!(forge_name("https://codeberg.org/a/b"), "Codeberg");
    assert_eq!(forge_name("https://git.example.org/a/b"), "git.example.org");
    assert_eq!(transport_of("https://codeberg.org/a/b"), Transport::Https);
    assert_eq!(transport_of("git@github.com:a/b.git"), Transport::Ssh);
    assert_eq!(transport_of("ssh://git@github.com/a/b"), Transport::Ssh);
    assert_eq!(transport_of("/tmp/forge.git"), Transport::Local);
    assert_eq!(transport_of("file:///tmp/forge.git"), Transport::Local);
}

/// The request weights of D1.9: the first request of a connection to a
/// private repository is answered 401 and replayed, so a credentialed
/// verb costs one request more; an ssh contact makes no HTTP request and
/// pays the anonymous column; a path on this machine costs nothing.
#[test]
fn a_verb_costs_the_requests_the_design_counted() {
    assert_eq!(requests("ls-remote", Transport::Https, false), 1);
    assert_eq!(requests("ls-remote", Transport::Https, true), 2);
    assert_eq!(requests("probe", Transport::Https, true), 2);
    assert_eq!(
        requests("fetch", Transport::Https, false),
        2,
        "one connection since download_ref holds it"
    );
    assert_eq!(requests("fetch", Transport::Https, true), 3);
    assert_eq!(requests("clone", Transport::Https, true), 3);
    assert_eq!(requests("push", Transport::Https, true), 3);
    assert_eq!(requests("fetch", Transport::Ssh, true), 2);
    assert_eq!(requests("fetch", Transport::Local, true), 0);
}

/// The budget table is canonical and every millisecond is derived from
/// it: 1000 / budget, and nothing written down twice (D1.9).
#[test]
fn every_millisecond_is_derived_from_the_budget_table() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    set_gaps("");
    assert_eq!(gap_for("codeberg.org"), Duration::from_millis(1111));
    assert_eq!(gap_for("github.com"), Duration::from_millis(1000));
    assert_eq!(gap_for("gitlab.com"), Duration::from_millis(200));
    assert_eq!(
        gap_for("git.acme.example"),
        Duration::from_millis(1000),
        "an unknown self hosted host gets the 1.0 budget"
    );
    assert_eq!(
        gap_for(""),
        Duration::ZERO,
        "a path on this machine rides no forge's bucket"
    );
    // the platform may still hand its own numbers in
    set_gaps("git.acme.example=250");
    assert_eq!(gap_for("git.acme.example"), Duration::from_millis(250));
    assert_eq!(
        gap_for("codeberg.org"),
        Duration::from_millis(1111),
        "hosts left out keep the table's value"
    );
    set_gaps("");
}

/// The poll period, one rule (D1.9): requests(verb) / budget, rounded up
/// to the next whole second, times the number of open projects on the
/// host. The requests per minute that come out stay inside the budget.
#[test]
fn the_poll_period_is_the_verbs_requests_over_the_hosts_budget() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    set_gaps("");
    let poll = |host| poll_period(host, "ls-remote", Transport::Https, true);

    // 2 requests / 0.9 = 2.222 s, rounded up
    assert_eq!(poll("codeberg.org"), Duration::from_secs(3));
    // 2 / 1.0 = 2 s exactly
    assert_eq!(poll("github.com"), Duration::from_secs(2));
    // 2 / 5.0 = 0.4 s, and a poll never runs faster than once a second
    assert_eq!(poll("gitlab.com"), Duration::from_secs(1));
    assert_eq!(poll("git.acme.example"), Duration::from_secs(2));

    // the acceptance criterion of J5, in requests per minute per host
    let per_minute = |host, budget: f64| {
        let period = poll(host).as_secs();
        let ticks = 60 / period;
        let requests = ticks * requests("ls-remote", Transport::Https, true) as u64;
        assert!(
            requests as f64 <= budget * 60.0,
            "{host}: {requests} requests per minute against a budget of {}",
            budget * 60.0
        );
        requests
    };
    assert_eq!(per_minute("codeberg.org", 0.9), 40);
    assert_eq!(per_minute("github.com", 1.0), 60);

    // N open projects on one host divide the budget
    assert_eq!(
        poll_period_for("codeberg.org", "ls-remote", Transport::Https, true, 2),
        Duration::from_secs(5)
    );
    set_gaps("");
}

/// No anonymous polling (D1.9): a remote nobody is signed in for is
/// checked once every fifteen minutes per host, and the surface says so.
#[test]
fn an_https_remote_without_a_credential_is_polled_every_fifteen_minutes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    set_gaps("");
    reset_anonymous_polls();
    assert_eq!(
        poll_period("github.com", "ls-remote", Transport::Https, false),
        ANONYMOUS_POLL_INTERVAL
    );
    assert_eq!(ANONYMOUS_POLL_INTERVAL, Duration::from_secs(900));
    // the gate lets the first one out and holds the next
    assert!(anonymous_poll_gate("github.com").is_none());
    let next = anonymous_poll_gate("github.com").expect("the second tick waits");
    assert!(next > SystemTime::now());
    // another host is not held by this one
    assert!(anonymous_poll_gate("codeberg.org").is_none());
    let why = anonymous_poll_reason("github.com");
    assert!(
        why.contains("github.com") && why.contains("15 minutes"),
        "{why}"
    );
    reset_anonymous_polls();
    set_gaps("");
}

/// Contacts to one host are spaced by what the verb costs, and another
/// host is not paced by them.
#[test]
fn contacts_to_one_host_are_spaced_by_what_the_verb_costs() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    reset_limits();
    reset_throttle();
    set_gaps("paced.test=60,default=0");
    // one credentialed ls-remote is two requests: 120 ms
    let t0 = Instant::now();
    for _ in 0..3 {
        run("https://paced.test/a/b", "ls-remote", true, || Ok(())).unwrap();
    }
    assert!(
        t0.elapsed() >= Duration::from_millis(240),
        "the first leaves at once, the next two wait 120 ms each"
    );
    let t1 = Instant::now();
    run("https://elsewhere.test/x", "fetch", true, || Ok(())).unwrap();
    assert!(t1.elapsed() < Duration::from_millis(50));
    set_gaps("");
}

fn rate_limited(url: &str) -> anyhow::Error {
    failed(&https_fetch(
        error(
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 429",
        ),
        &host_of(url),
    ))
}

/// The 429 brake of D1.9: the strike is the doubling exponent, it
/// SURVIVES the next success, and every contact still goes out.
#[test]
fn a_429_doubles_the_gap_and_the_strike_survives_the_next_success() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    clear_oracle();
    reset_limits();
    reset_throttle();
    set_gaps("limited.test=50,default=0");
    let url = "https://limited.test/a/b";

    let first = run(url, "ls-remote", true, || -> anyhow::Result<()> {
        Err(rate_limited(url))
    });
    let e = first.unwrap_err();
    assert_eq!(failure_of(&e), Failure::RateLimited);
    let next = next_try_of(&e).expect("a strike names when it ends");
    assert!(next > SystemTime::now());
    assert!(limited_until("limited.test").is_some());

    // the next contact still goes out, and it waits the DOUBLED gap:
    // 2 requests * 50 ms * 2^1 = 200 ms
    reset_throttle();
    let mut called = false;
    run(url, "ls-remote", true, || {
        called = true;
        Ok(())
    })
    .unwrap();
    assert!(called);
    let t0 = Instant::now();
    run(url, "ls-remote", true, || Ok(())).unwrap();
    assert!(
        t0.elapsed() >= Duration::from_millis(180),
        "the doubled gap of 200 ms, waited {:?}",
        t0.elapsed()
    );

    // THE change of this package: the success above did not clear the
    // strike (joy used to remove the entry and run straight back into
    // the limit)
    assert!(
        limited_until("limited.test").is_some(),
        "the strike survives a success"
    );

    // a second 429 makes the exponent 2: 2 * 50 * 4 = 400 ms
    let _ = run(url, "ls-remote", true, || -> anyhow::Result<()> {
        Err(rate_limited(url))
    });
    reset_throttle();
    run(url, "ls-remote", true, || Ok(())).unwrap();
    let t1 = Instant::now();
    run(url, "ls-remote", true, || Ok(())).unwrap();
    assert!(
        t1.elapsed() >= Duration::from_millis(360),
        "the twice doubled gap of 400 ms, waited {:?}",
        t1.elapsed()
    );

    // another host is not affected
    assert!(run("https://other.test/x", "fetch", true, || Ok(1)).is_ok());
    assert!(limited_until("other.test").is_none());

    reset_limits();
    assert!(limited_until("limited.test").is_none());
    set_gaps("");
}

/// The GitLab ban's documented wait is what the gate uses, not the ten
/// minute default.
#[test]
fn a_documented_ban_sets_its_own_next_try() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    clear_oracle();
    reset_limits();
    reset_throttle();
    set_gaps("default=0");
    let url = "https://gitlab.com/a/b";
    let e = run(url, "fetch", true, || -> anyhow::Result<()> {
        Err(failed(&https_fetch(
            error(
                Code::GenericError,
                Class::Http,
                "unexpected http status code: 403",
            ),
            "gitlab.com",
        )))
    })
    .unwrap_err();
    assert_eq!(failure_of(&e), Failure::RateLimited);
    let next = next_try_of(&e).expect("the ban names its end");
    let waits = next
        .duration_since(SystemTime::now())
        .unwrap_or(Duration::ZERO);
    assert!(
        waits > Duration::from_secs(14 * 60) && waits <= Duration::from_secs(15 * 60),
        "gitlab.com bans for 15 minutes, got {waits:?}"
    );
    reset_limits();
    set_gaps("");
}

/// A credentialed contact that worked is remembered per host: it is what
/// tells a 403 on a push from a 403 on a fetch (D1.8b).
#[test]
fn a_credential_that_worked_is_remembered_for_the_host() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    reset_limits();
    reset_throttle();
    reset_token_memory();
    set_gaps("default=0");
    assert!(!token_worked_before("worked.test"));
    run("https://worked.test/a/b", "ls-remote", true, || Ok(())).unwrap();
    assert!(token_worked_before("worked.test"));
    // an anonymous contact proves nothing about a credential
    reset_token_memory();
    run("https://worked.test/a/b", "ls-remote", false, || Ok(())).unwrap();
    assert!(!token_worked_before("worked.test"));
    reset_token_memory();
    set_gaps("");
}

/// D1.7's one re-ask: only a token that has already worked here and is
/// now refused is worth a refresh; anything else is a sign in.
#[test]
fn only_a_spent_token_is_worth_one_refresh() {
    let spent = https_fetch(
        error(
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 401",
        ),
        "github.com",
    );
    assert!(wants_token_refresh(&spent));
    let never_worked = evidence(
        error(
            Code::GenericError,
            Class::Http,
            "unexpected http status code: 401",
        ),
        "github.com",
        Transport::Https,
        ContactDirection::Fetch,
        CredentialSource::NonePresented,
        false,
    );
    assert!(!wants_token_refresh(&never_worked));
}
