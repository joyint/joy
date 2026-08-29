// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE forge contact (JOY-0268-2A, incident JP-00EF-CC): every network
//! verb of the git engine - clone, fetch, ls-remote, push, probe - goes
//! through [`run`], and only through it. One place therefore
//!
//!   - opens a span per contact (ADR JP-00ED-EE: every forge contact is
//!     a span with forge, verb, outcome and duration),
//!   - classifies a failure into what the person needs to know
//!     ([`Failure`]: rate-limited, denied, offline, other), reading the
//!     only evidence libgit2 hands out, the message text,
//!   - keeps a backoff gate PER FORGE HOST: after a 429 no verb touches
//!     that host until the gate opens again, whoever asks and however
//!     often (the chat poll keeps its one-second cadence and simply
//!     gets the answer 'limited, next try at T' without a request).
//!
//! libgit2 does not expose response headers, so Retry-After is never
//! known; the gate doubles from thirty seconds up to ten minutes and
//! resets on the first successful contact.
//!
//! Before any of that, the THROTTLE (Horst, 2026-08-29): contacts to one
//! host are spaced by a minimum gap, a plain wait in line, never a
//! refusal. One value per host ([`set_gaps`], `codeberg.org=800,
//! default=100` built in): Codeberg braked at about one request per
//! second from one address (JP-00EF-CC); GitLab allows 10,000 git
//! requests per minute per user, GitHub publishes no git limit at all.
//! Every caller - chat poll, item sync, pushes, jobs - lines up here, so
//! the sum stays under the forge's patience whatever runs.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What a failed contact means for the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The forge throttles us (HTTP 429); it will serve us again.
    RateLimited,
    /// The forge refuses our identity (401/403, bad token); no retry helps.
    Denied,
    /// Nobody answered: DNS, connection, timeout, or the forge is down.
    Offline,
    /// Something else (a rejected ref, a missing branch, a local fault).
    Other,
}

impl Failure {
    /// The wire word the status carries (platform proto, desktop DTO).
    pub fn reason(self) -> &'static str {
        match self {
            Failure::RateLimited => "rate_limited",
            Failure::Denied => "denied",
            Failure::Offline => "offline",
            Failure::Other => "",
        }
    }
}

/// The error every contact returns: the engine's message, its meaning,
/// and - when the forge limits us - the moment the gate opens again.
#[derive(Debug)]
pub struct ContactError {
    pub failure: Failure,
    pub message: String,
    pub next_try: Option<SystemTime>,
}

impl std::fmt::Display for ContactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ContactError {}

/// Read the meaning out of an engine message. The order matters: a
/// 429 names itself; a refusal is anything about credentials; the rest
/// of the transport's wording means nobody answered.
pub fn classify(message: &str) -> Failure {
    let m = message.to_ascii_lowercase();
    if m.contains("429") || m.contains("too many requests") || m.contains("rate limit") {
        return Failure::RateLimited;
    }
    if m.contains("auth")
        || m.contains("credential")
        || m.contains("username/password")
        || m.contains("permission")
        || m.contains("denied")
        || m.contains("403")
        || m.contains("401")
        || m.contains("invalid username or token")
        || m.contains("too many redirects")
    {
        return Failure::Denied;
    }
    if m.contains("offline?")
        || m.contains("could not resolve")
        || m.contains("failed to resolve")
        || m.contains("unable to access")
        || m.contains("connection")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("network")
    {
        return Failure::Offline;
    }
    Failure::Other
}

/// The meaning of any error that came out of the engine: a
/// [`ContactError`] says it directly, a plain message is classified.
pub fn failure_of(error: &anyhow::Error) -> Failure {
    match error.downcast_ref::<ContactError>() {
        Some(c) => c.failure,
        None => classify(&error.to_string()),
    }
}

/// The next-try moment an error carries, if the forge limits us.
pub fn next_try_of(error: &anyhow::Error) -> Option<SystemTime> {
    error
        .downcast_ref::<ContactError>()
        .and_then(|c| c.next_try)
}

// ---- the per-host throttle ------------------------------------------

const DEFAULT_GAPS: &str = "codeberg.org=800,default=100";

struct Throttle {
    gaps: HashMap<String, Duration>,
    /// when the next contact to a host may leave
    next_free: HashMap<String, Instant>,
}

static THROTTLE: Mutex<Option<Throttle>> = Mutex::new(None);

fn with_throttle<T>(f: impl FnOnce(&mut Throttle) -> T) -> T {
    let mut guard = THROTTLE.lock().unwrap_or_else(|e| e.into_inner());
    let t = guard.get_or_insert_with(|| Throttle {
        gaps: parse_gaps(DEFAULT_GAPS),
        next_free: HashMap::new(),
    });
    f(t)
}

/// `host=ms,host=ms,default=ms` into a table; unknown shapes are skipped.
pub fn parse_gaps(spec: &str) -> HashMap<String, Duration> {
    spec.split(',')
        .filter_map(|entry| {
            let (host, ms) = entry.split_once('=')?;
            let ms: u64 = ms.trim().parse().ok()?;
            Some((host.trim().to_ascii_lowercase(), Duration::from_millis(ms)))
        })
        .collect()
}

/// Install the per-host gaps (the platform hands its
/// JOYINT_FORGE_MIN_GAP_MS here; hosts left out keep `default`).
pub fn set_gaps(spec: &str) {
    let parsed = parse_gaps(spec);
    with_throttle(|t| {
        let mut gaps = parse_gaps(DEFAULT_GAPS);
        gaps.extend(parsed);
        t.gaps = gaps;
    });
}

fn gap_for(host: &str) -> Duration {
    with_throttle(|t| {
        t.gaps
            .get(host)
            .or_else(|| t.gaps.get("default"))
            .copied()
            .unwrap_or(Duration::from_millis(100))
    })
}

/// Wait for this host's turn: the gap since the previous contact. Takes
/// the slot on the way out, so concurrent callers line up one behind the
/// other instead of leaving together.
fn take_turn(host: &str) -> Duration {
    let gap = gap_for(host);
    let waited = with_throttle(|t| {
        let now = Instant::now();
        let free = t.next_free.get(host).copied().unwrap_or(now);
        let start = free.max(now);
        t.next_free.insert(host.to_string(), start + gap);
        start.saturating_duration_since(now)
    });
    if !waited.is_zero() {
        std::thread::sleep(waited);
    }
    waited
}

#[cfg(test)]
pub(crate) fn reset_throttle() {
    with_throttle(|t| t.next_free.clear());
}

// ---- the per-host gate ----------------------------------------------

const FIRST_BACKOFF: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(600);

struct Limit {
    until: Instant,
    /// consecutive 429s, the doubling exponent
    strikes: u32,
}

static LIMITS: Mutex<Option<HashMap<String, Limit>>> = Mutex::new(None);

fn with_limits<T>(f: impl FnOnce(&mut HashMap<String, Limit>) -> T) -> T {
    let mut guard = LIMITS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// The host part of a forge URL (`https://codeberg.org/o/r.git` ->
/// `codeberg.org`, `git@github.com:o/r` -> `github.com`); the whole
/// string when it has no recognisable host.
pub fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let rest = rest.rsplit('@').next().unwrap_or(rest);
    rest.split(['/', ':'])
        .next()
        .unwrap_or(rest)
        .to_ascii_lowercase()
}

/// A user-facing name for the forge behind a URL.
pub fn forge_name(url: &str) -> String {
    let host = host_of(url);
    match host.as_str() {
        "github.com" => "GitHub".into(),
        "gitlab.com" => "GitLab".into(),
        "codeberg.org" => "Codeberg".into(),
        "" => "the forge".into(),
        other => other.to_string(),
    }
}

/// When the gate for this host opens, if it is closed right now.
pub fn limited_until(host: &str) -> Option<SystemTime> {
    let now = Instant::now();
    with_limits(|limits| {
        limits
            .get(host)
            .filter(|l| l.until > now)
            .map(|l| SystemTime::now() + (l.until - now))
    })
}

/// The gate's answer for a checkout's forge, for status displays:
/// `Some(next try)` while the forge limits us.
pub fn limited_for(repo_dir: &std::path::Path) -> Option<SystemTime> {
    let url = super::forge::remote_url(repo_dir)?;
    limited_until(&host_of(&url))
}

fn strike(host: &str) -> SystemTime {
    let now = Instant::now();
    with_limits(|limits| {
        let entry = limits.entry(host.to_string()).or_insert(Limit {
            until: now,
            strikes: 0,
        });
        let wait = FIRST_BACKOFF
            .saturating_mul(1u32 << entry.strikes.min(5))
            .min(MAX_BACKOFF);
        entry.strikes = entry.strikes.saturating_add(1);
        entry.until = now + wait;
        SystemTime::now() + wait
    })
}

fn clear(host: &str) {
    with_limits(|limits| {
        limits.remove(host);
    });
}

#[cfg(test)]
pub(crate) fn reset_limits() {
    with_limits(|limits| limits.clear());
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run one forge contact against `url`: the gate first, then the verb,
/// then the verdict - a span around all of it. `verb` names the contact
/// (`clone`, `fetch`, `ls-remote`, `push`, `probe`).
pub fn run<T>(
    url: &str,
    verb: &'static str,
    work: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let host = host_of(url);
    let span = tracing::info_span!("forge.contact", verb, forge = %host);
    let _s = span.enter();
    if let Some(next) = limited_until(&host) {
        tracing::warn!(
            next_try_unix = unix(next),
            "forge contact skipped: rate limited"
        );
        return Err(anyhow::Error::new(ContactError {
            failure: Failure::RateLimited,
            message: format!(
                "{host} is limiting our requests (429); next try at {}",
                unix(next)
            ),
            next_try: Some(next),
        }));
    }
    let waited = take_turn(&host);
    if !waited.is_zero() {
        tracing::debug!(
            waited_ms = waited.as_millis() as u64,
            "forge contact throttled"
        );
    }
    let started = Instant::now();
    match work() {
        Ok(value) => {
            clear(&host);
            tracing::debug!(
                took_ms = started.elapsed().as_millis() as u64,
                "forge contact ok"
            );
            Ok(value)
        }
        Err(e) => {
            let failure = failure_of(&e);
            let next_try = match failure {
                Failure::RateLimited => Some(strike(&host)),
                _ => None,
            };
            tracing::error!(
                outcome = failure.reason(),
                took_ms = started.elapsed().as_millis() as u64,
                error = %e,
                "forge contact failed"
            );
            Err(anyhow::Error::new(ContactError {
                failure,
                message: e.to_string(),
                next_try,
            }))
        }
    }
}

/// [`run`] for a checkout: the forge is the checkout's remote.
pub fn run_for<T>(
    repo_dir: &std::path::Path,
    verb: &'static str,
    work: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let url = super::forge::remote_url(repo_dir).unwrap_or_default();
    run(&url, verb, work)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate is process-wide: tests that touch it run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn messages_are_read_for_their_meaning() {
        assert_eq!(
            classify("push failed: unexpected http status code: 429"),
            Failure::RateLimited
        );
        assert_eq!(classify("git: invalid username or token"), Failure::Denied);
        assert_eq!(
            classify("fetch failed (offline?): failed to resolve address"),
            Failure::Offline
        );
        assert_eq!(
            classify("branch main not found on the forge"),
            Failure::Other
        );
    }

    #[test]
    fn hosts_and_names_come_out_of_every_url_shape() {
        assert_eq!(host_of("https://codeberg.org/joyint/x.git"), "codeberg.org");
        assert_eq!(host_of("git@github.com:joyint/x.git"), "github.com");
        assert_eq!(host_of("https://user:tok@gitlab.com/a/b"), "gitlab.com");
        assert_eq!(forge_name("https://codeberg.org/a/b"), "Codeberg");
        assert_eq!(forge_name("https://git.example.org/a/b"), "git.example.org");
    }

    #[test]
    fn a_429_closes_the_gate_and_the_next_contact_is_skipped_until_it_opens() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_limits();
        let url = "https://limited.test/a/b";
        let first = run(url, "push", || -> anyhow::Result<()> {
            anyhow::bail!("push failed: unexpected http status code: 429")
        });
        let e = first.unwrap_err();
        assert_eq!(failure_of(&e), Failure::RateLimited);
        let next = next_try_of(&e).expect("a limit names its next try");
        assert!(next > SystemTime::now());
        // the gate is closed: the verb is not even called
        let mut called = false;
        let second = run(url, "ls-remote", || -> anyhow::Result<()> {
            called = true;
            Ok(())
        });
        assert!(!called);
        assert_eq!(failure_of(&second.unwrap_err()), Failure::RateLimited);
        assert!(limited_until("limited.test").is_some());
        // another host is not affected
        assert!(run("https://other.test/x", "fetch", || Ok(1)).is_ok());
        reset_limits();
        assert!(limited_until("limited.test").is_none());
    }

    #[test]
    fn contacts_to_one_host_are_spaced_by_its_gap() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_limits();
        reset_throttle();
        set_gaps("paced.test=120,default=0");
        let gaps = parse_gaps("codeberg.org=800, default=100");
        assert_eq!(gaps["codeberg.org"], Duration::from_millis(800));
        assert_eq!(gaps["default"], Duration::from_millis(100));
        assert_eq!(gap_for("paced.test"), Duration::from_millis(120));
        assert_eq!(gap_for("elsewhere.test"), Duration::ZERO);
        let t0 = Instant::now();
        for _ in 0..3 {
            run("https://paced.test/a/b", "ls-remote", || Ok(())).unwrap();
        }
        // the first leaves at once, the next two wait their gap: >= 240 ms
        assert!(t0.elapsed() >= Duration::from_millis(240));
        // another host is not paced by this one
        let t1 = Instant::now();
        run("https://elsewhere.test/x", "fetch", || Ok(())).unwrap();
        assert!(t1.elapsed() < Duration::from_millis(50));
        set_gaps(DEFAULT_GAPS);
    }

    #[test]
    fn a_success_clears_the_strikes() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_limits();
        let url = "https://recover.test/a/b";
        let _ = run(url, "push", || -> anyhow::Result<()> {
            anyhow::bail!("429")
        });
        reset_limits(); // the clock cannot be moved; clearing stands in for the gate opening
        assert!(run(url, "push", || Ok(())).is_ok());
        assert!(limited_until("recover.test").is_none());
    }
}
