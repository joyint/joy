// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The two sign in doors, and the refresh behind them (D2.7).
//!
//! There is no single door, because the three forge families do not
//! offer one:
//!
//! - **GitHub** has the device authorization grant. `POST
//!   https://github.com/login/device/code`, then poll `POST
//!   https://github.com/login/oauth/access_token` with
//!   `grant_type=urn:ietf:params:oauth:grant-type:device_code` and no
//!   client secret. `interval`, `slow_down` (plus 5 s), `expired_token`,
//!   `access_denied` and `device_flow_disabled` are all honoured.
//! - **GitLab** has the device grant from 17.3. `POST
//!   {base}/oauth/authorize_device`, poll `POST {base}/oauth/token`.
//!   gitlab.com's OIDC discovery does not advertise the device
//!   endpoint, so the path is written down rather than discovered.
//! - **Gitea, Forgejo and Codeberg** have no device grant in any
//!   released version, so the door is the authorization code flow with
//!   PKCE S256 on a loopback listener. The redirect URI is registered
//!   as exactly `http://127.0.0.1`, with no port and no path, and bound
//!   to an ephemeral port at runtime.
//!
//! Two rules bind every path here. The connector **never opens a
//! browser**: it says where the person must go and the host decides
//! (the desktop opens it, the CLI prints it, a delegated session is
//! refused before the verb starts). And a token is **never printed**:
//! it goes into the entry, and the answer names the login, not the
//! secret.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::http::{Answer, Http, HttpError};

/// Which door a forge offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// The device authorization grant (GitHub, GitLab).
    Device,
    /// Authorization code with PKCE S256 on a loopback listener (the
    /// Gitea family).
    Pkce,
}

/// The OAuth application one host signs in through. Every field can
/// come from `forges.yaml` (D2.5), because an Enterprise Server or a
/// self hosted GitLab registers its own application and joy must carry
/// no instance in its code.
#[derive(Debug, Clone)]
pub struct OAuth {
    pub client_id: String,
    pub flow: Flow,
    /// Device grant: where the device code is asked for.
    pub device_endpoint: String,
    /// PKCE: where the person authorises.
    pub auth_endpoint: String,
    pub token_endpoint: String,
    /// The set this login asks for, from `--for` and D2.7a.
    pub scopes: String,
}

/// The public OAuth clients joy registers per public forge.
///
/// **These are placeholders.** They are marked so on purpose and in one
/// place: OAuth client ids are configuration (D2.5), and the real ones
/// exist only once the operator has registered the public clients with
/// the three forges. Until then a person signs in with
/// `joy forge login --token-stdin`, or an operator puts a `client_id`
/// for the host into `forges.yaml`, and `login` says exactly that
/// instead of sending a request nobody can answer.
pub mod clients {
    /// The marker every unregistered client id carries.
    pub const PLACEHOLDER: &str = "REPLACE-ME";

    /// The public client "Joyint Desktop" the operator registered on
    /// github.com on 2026-09-18 (owner: the joyint organisation): device
    /// flow enabled, redirect `http://127.0.0.1`, token expiry off, so no
    /// refresh token exists (D2.7, decision 3). A client id is public by
    /// design; the client secret GitHub generates is never used.
    pub const GITHUB_COM: &str = "Ov23liNJt50pUmo28YPy";

    /// The public application the operator registered on gitlab.com on
    /// 2026-09-18: Confidential off, redirect `http://127.0.0.1`, scopes
    /// registered as the union `api write_repository` so a device request
    /// may narrow (D2.7, decision 28). A GitLab application id is public
    /// by design.
    pub const GITLAB_COM: &str = "299407d616bd7e050092e60b512a5e8e6c2d35bde91ae612f8e68e9b0dcef1dc";

    /// The public application "Joyint Desktop" registered on codeberg.org
    /// on 2026-09-18 through the Gitea API (owner joydev-horst): redirect
    /// exactly `http://127.0.0.1`, confidential off (D2.7). No secret is
    /// used; a client id is public by design.
    pub const CODEBERG_ORG: &str = "9cf771c7-6d91-459b-8947-05d45ec3637b";

    /// Whether this id is still one of the three above.
    pub fn is_placeholder(client_id: &str) -> bool {
        client_id.starts_with(PLACEHOLDER)
    }
}

/// The sentence a person reads when the client id is still a
/// placeholder. It names both ways out, because both work today.
pub fn unregistered_sentence(host: &str) -> String {
    format!(
        "joy has no registered OAuth client for {host} yet. \
         Store a token instead (joy forge login --token-stdin), \
         or put a client_id for {host} into forges.yaml."
    )
}

/// What the forge answered when it granted the token.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Grant {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
    /// The granted set as the forge wrote it. Gitea's answer has no
    /// scope field at all, so there the caller stores what it asked for
    /// (D2.7c).
    pub scope: Option<String>,
}

/// Rule 1 of this module's parent: both tokens print as their
/// fingerprint. A `Poll` carries a `Grant`, and `Poll` is what a failed
/// assertion or a stray `{:?}` prints.
impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("access_token", &super::Redacted(&self.access_token))
            .field(
                "refresh_token",
                &self.refresh_token.as_deref().map(super::Redacted),
            )
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .finish()
    }
}

impl Grant {
    /// The RFC 3339 moment this token dies, when the forge named a
    /// lifetime.
    pub fn expires_at(&self) -> Option<String> {
        let seconds = self.expires_in?;
        Some((chrono::Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339())
    }
}

/// What one poll of the token endpoint said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// `authorization_pending`: the person has not finished yet.
    Pending,
    /// `slow_down`: the next interval is five seconds longer (D2.7).
    SlowDown,
    Granted(Grant),
    /// A named end: `access_denied`, `expired_token`, `invalid_scope`,
    /// `device_flow_disabled`, or a transport failure as `network`.
    Failed {
        code: String,
        message: String,
    },
}

/// What a device grant started.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    pub interval: i64,
}

/// The device code is the credential the poll spends, so it prints as
/// its fingerprint like every other secret here. The USER code is the
/// one the person reads aloud, and it prints as itself.
impl std::fmt::Debug for DeviceStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceStart")
            .field("device_code", &super::Redacted(&self.device_code))
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("verification_uri_complete", &self.verification_uri_complete)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish()
    }
}

/// Where the newline delimited events of D2.4 go. One object per line,
/// each flushed explicitly, because the caller shows the verification
/// code while the connector is still polling.
pub trait Events {
    fn emit(&mut self, event: Value);
}

/// The real sink: stdout, one line per event, flushed.
pub struct Stdout;

impl Events for Stdout {
    fn emit(&mut self, event: Value) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{event}");
        let _ = out.flush();
    }
}

impl Events for Vec<Value> {
    fn emit(&mut self, event: Value) {
        self.push(event);
    }
}

/// How long the connector waits between two polls. Real time in a
/// shipped connector; no time at all in a test, which is the only way
/// to prove a five second `slow_down` without spending five seconds.
pub trait Clock {
    fn sleep(&self, how_long: Duration);
}

/// The clock a connector runs on.
pub struct RealClock;

impl Clock for RealClock {
    fn sleep(&self, how_long: Duration) {
        std::thread::sleep(how_long);
    }
}

/// A clock that records what it was asked to wait and waits nothing.
/// Behind the `fake-api` feature, so a shipped connector never carries
/// it (the same rule as the in process fake forge).
#[cfg(feature = "fake-api")]
#[derive(Default)]
pub struct NoWait {
    waits: std::sync::Mutex<Vec<Duration>>,
}

#[cfg(feature = "fake-api")]
impl NoWait {
    /// Every wait this clock was asked for, in order.
    pub fn waits(&self) -> Vec<Duration> {
        self.waits.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(feature = "fake-api")]
impl Clock for NoWait {
    fn sleep(&self, how_long: Duration) {
        self.waits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(how_long);
    }
}

/// Ask the forge to start a device grant (D2.7).
pub fn start_device(http: &Http, oauth: &OAuth) -> Result<DeviceStart, Poll> {
    let body = form(&[("client_id", &oauth.client_id), ("scope", &oauth.scopes)]);
    let answer = http
        .post(&oauth.device_endpoint)
        .header("Accept", "application/json")
        .send_bytes("application/x-www-form-urlencoded", body.into_bytes())
        .map_err(transport)?;
    let Some(json) = answer.json() else {
        return Err(unexpected(&answer));
    };
    if let Some(failure) = named_error(&json) {
        return Err(failure);
    }
    let text = |key: &str| {
        json.get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|value| !value.is_empty())
    };
    let (Some(device_code), Some(user_code), Some(verification_uri)) = (
        text("device_code"),
        text("user_code"),
        text("verification_uri").or_else(|| text("verification_url")),
    ) else {
        return Err(unexpected(&answer));
    };
    Ok(DeviceStart {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete: text("verification_uri_complete"),
        expires_in: number(&json, "expires_in").unwrap_or(900),
        // RFC 8628 says five seconds when the forge names none.
        interval: number(&json, "interval").unwrap_or(5),
    })
}

/// One poll of the token endpoint for a device grant.
pub fn poll_device(http: &Http, oauth: &OAuth, device_code: &str) -> Poll {
    let body = form(&[
        ("client_id", &oauth.client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ]);
    token_request(http, &oauth.token_endpoint, body)
}

/// Exchange an authorization code for a token, with the PKCE verifier
/// in place of the client secret joy does not have.
pub fn exchange_code(
    http: &Http,
    oauth: &OAuth,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Poll {
    let body = form(&[
        ("client_id", &oauth.client_id),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ]);
    token_request(http, &oauth.token_endpoint, body)
}

/// Spend a refresh token. Rotation safe by construction: the caller
/// writes the WHOLE answer back, so a forge that rotates the refresh
/// token (Forgejo does, on every use) does not leave a stale one behind.
pub fn refresh(http: &Http, token_endpoint: &str, client_id: &str, refresh_token: &str) -> Poll {
    let body = form(&[
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ]);
    token_request(http, token_endpoint, body)
}

fn token_request(http: &Http, endpoint: &str, body: String) -> Poll {
    let answer = match http
        .post(endpoint)
        .header("Accept", "application/json")
        .send_bytes("application/x-www-form-urlencoded", body.into_bytes())
    {
        Ok(answer) => answer,
        Err(error) => return transport(error),
    };
    let Some(json) = answer.json() else {
        return unexpected(&answer);
    };
    if let Some(failure) = named_error(&json) {
        return failure;
    }
    let Some(access_token) = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .filter(|token| !token.is_empty())
    else {
        return unexpected(&answer);
    };
    Poll::Granted(Grant {
        access_token: access_token.to_string(),
        refresh_token: json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        expires_in: number(&json, "expires_in"),
        scope: json
            .get("scope")
            .and_then(|v| v.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string),
    })
}

/// The `error` field of an OAuth answer, mapped to the codes D2.4's
/// `error` event names.
fn named_error(json: &Value) -> Option<Poll> {
    let code = json.get("error").and_then(|v| v.as_str())?;
    let message = json
        .get("error_description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(match code {
        "authorization_pending" => Poll::Pending,
        "slow_down" => Poll::SlowDown,
        other => Poll::Failed {
            code: other.to_string(),
            message,
        },
    })
}

/// An answer that is not OAuth at all. The status is named and the body
/// is not, because a body can carry a token.
fn unexpected(answer: &Answer) -> Poll {
    Poll::Failed {
        code: "unsupported".to_string(),
        message: format!(
            "the forge answered {} to an OAuth request joy could not read",
            answer.status
        ),
    }
}

fn transport(error: HttpError) -> Poll {
    Poll::Failed {
        code: "network".to_string(),
        message: error.to_string(),
    }
}

fn number(json: &Value, key: &str) -> Option<i64> {
    let value = json.get(key)?;
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

/// `application/x-www-form-urlencoded`, which is what RFC 6749 asks
/// for. Every value is escaped, so a scope string with a colon in it
/// (`user:email`) survives.
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                crate::url::encode_segment(key),
                crate::url::encode_segment(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

// -- PKCE on a loopback listener (D2.7) ---------------------------------------

/// One PKCE attempt: the verifier stays here, the challenge goes to the
/// forge, and the listener waits for the one redirect that carries the
/// code.
pub struct Pkce {
    verifier: String,
    challenge: String,
    state: String,
    listener: TcpListener,
    addr: SocketAddr,
}

impl Pkce {
    /// Bind the loopback listener on an ephemeral port and draw a
    /// verifier. `127.0.0.1` and never `localhost`: the registered
    /// redirect URI is the literal address, and a name could resolve to
    /// something else on a machine with a creative hosts file.
    pub fn start() -> std::io::Result<Pkce> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let verifier = random_token();
        let challenge = s256(&verifier);
        Ok(Pkce {
            verifier,
            challenge,
            state: random_token(),
            listener,
            addr,
        })
    }

    /// The redirect URI this attempt hands the forge. The registration
    /// carries `http://127.0.0.1` with no port; RFC 8252 lets the port
    /// be chosen at runtime, and this is that port.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    /// The URL the person opens. The connector prints it and never
    /// opens it (D2.4).
    pub fn authorize_url(&self, oauth: &OAuth) -> String {
        format!(
            "{}?{}",
            oauth.auth_endpoint,
            form(&[
                ("client_id", &oauth.client_id),
                ("redirect_uri", &self.redirect_uri()),
                ("response_type", "code"),
                ("scope", &oauth.scopes),
                ("state", &self.state),
                ("code_challenge", &self.challenge),
                ("code_challenge_method", "S256"),
            ])
        )
    }

    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    pub fn state(&self) -> &str {
        &self.state
    }

    /// Wait for the one redirect. `tick` is called about once a second
    /// with the seconds left, which is where the `waiting` events of
    /// D2.4 come from.
    pub fn wait(&self, total: Duration, mut tick: impl FnMut(i64)) -> Result<String, Poll> {
        let started = Instant::now();
        let mut last_tick = Instant::now();
        loop {
            let elapsed = started.elapsed();
            if elapsed >= total {
                return Err(Poll::Failed {
                    code: "expired_token".to_string(),
                    message: "nobody finished the sign in in time".to_string(),
                });
            }
            match self.listener.accept() {
                Ok((stream, _)) => return self.read_redirect(stream),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                    if last_tick.elapsed() >= Duration::from_secs(1) {
                        last_tick = Instant::now();
                        tick((total - elapsed).as_secs() as i64);
                    }
                }
                Err(e) => {
                    return Err(Poll::Failed {
                        code: "network".to_string(),
                        message: format!("the loopback listener stopped: {e}"),
                    })
                }
            }
        }
    }

    fn read_redirect(&self, stream: TcpStream) -> Result<String, Poll> {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let mut reader = BufReader::new(match stream.try_clone() {
            Ok(stream) => stream,
            Err(e) => {
                return Err(Poll::Failed {
                    code: "network".to_string(),
                    message: e.to_string(),
                })
            }
        });
        if reader.read_line(&mut line).is_err() {
            return Err(Poll::Failed {
                code: "network".to_string(),
                message: "the browser's redirect could not be read".to_string(),
            });
        }
        let query = line
            .split_whitespace()
            .nth(1)
            .and_then(|path| path.split_once('?').map(|(_, query)| query.to_string()))
            .unwrap_or_default();
        let result = self.code_of(&query);
        // The person sees a page either way: a browser left on a failed
        // connection is the one part of this flow they cannot fix.
        answer_browser(stream, result.is_ok());
        result
    }

    /// The `code` of a redirect query, once its `state` matches. A
    /// mismatched state is a cross site request and is refused without
    /// spending the code.
    pub fn code_of(&self, query: &str) -> Result<String, Poll> {
        let mut code = None;
        let mut state = None;
        let mut error = None;
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let value = decode_component(value);
            match key {
                "code" => code = Some(value),
                "state" => state = Some(value),
                "error" => error = Some(value),
                _ => {}
            }
        }
        if let Some(error) = error {
            return Err(Poll::Failed {
                code: error,
                message: "the forge refused the sign in".to_string(),
            });
        }
        if state.as_deref() != Some(self.state.as_str()) {
            return Err(Poll::Failed {
                code: "unsupported".to_string(),
                message: "the redirect did not carry this sign in's own state".to_string(),
            });
        }
        code.filter(|code| !code.is_empty()).ok_or(Poll::Failed {
            code: "unsupported".to_string(),
            message: "the redirect carried no authorization code".to_string(),
        })
    }
}

fn answer_browser(mut stream: TcpStream, ok: bool) {
    let body = if ok {
        "<!doctype html><meta charset=\"utf-8\"><title>Signed in</title>\
         <p>joy is signed in. You can close this tab.</p>"
    } else {
        "<!doctype html><meta charset=\"utf-8\"><title>Not signed in</title>\
         <p>joy did not get a sign in from this page. You can close this tab.</p>"
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// A 256 bit random value as base64url without padding: the PKCE
/// verifier and the CSRF state both.
pub fn random_token() -> String {
    use base64ct::{Base64UrlUnpadded, Encoding};
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    Base64UrlUnpadded::encode_string(&bytes)
}

/// The S256 challenge of a verifier: base64url, unpadded, of its
/// SHA-256. `plain` is never offered.
pub fn s256(verifier: &str) -> String {
    use base64ct::{Base64UrlUnpadded, Encoding};
    use sha2::{Digest, Sha256};
    Base64UrlUnpadded::encode_string(&Sha256::digest(verifier.as_bytes()))
}

/// Percent decoding for one query component, plus the `+` of a form
/// encoded space.
fn decode_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `verification` event of D2.4.
pub fn verification_event(
    host: &str,
    url: &str,
    url_complete: Option<&str>,
    code: Option<&str>,
    expires_in: i64,
    interval: i64,
) -> Value {
    json!({
        "event": "verification",
        "host": host,
        "url": url,
        "url_complete": url_complete,
        "code": code,
        "expires_in": expires_in,
        "interval": interval,
    })
}

/// The `error` event of D2.4.
pub fn error_event(code: &str, message: &str) -> Value {
    json!({ "event": "error", "code": code, "message": message })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636's own test vector, so the challenge is right and not
    /// merely consistent with itself.
    #[test]
    fn the_s256_challenge_is_the_one_rfc_7636_writes_down() {
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_verifier_is_random_and_url_safe() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        // 32 bytes as base64url without padding
        assert_eq!(a.len(), 43);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn a_form_body_escapes_every_value_it_carries() {
        assert_eq!(
            form(&[("scope", "repo user:email"), ("client_id", "a b")]),
            "scope=repo%20user%3Aemail&client_id=a%20b"
        );
    }

    #[test]
    fn the_placeholder_client_ids_are_marked_as_placeholders() {
        for id in [
            clients::GITHUB_COM,
            clients::GITLAB_COM,
            clients::CODEBERG_ORG,
        ] {
            assert!(!clients::is_placeholder(id), "{id}");
        }
        assert!(clients::is_placeholder("REPLACE-ME-anything"));
        assert!(!clients::is_placeholder("Ov23liRealLookingId"));
        let sentence = unregistered_sentence("github.com");
        assert!(sentence.contains("--token-stdin"));
        assert!(sentence.contains("forges.yaml"));
    }

    /// D2.7's list, each mapped to the event code a host renders.
    #[test]
    fn every_named_oauth_error_becomes_its_own_poll_answer() {
        assert_eq!(
            named_error(&json!({"error": "authorization_pending"})),
            Some(Poll::Pending)
        );
        assert_eq!(
            named_error(&json!({"error": "slow_down"})),
            Some(Poll::SlowDown)
        );
        for code in [
            "access_denied",
            "expired_token",
            "device_flow_disabled",
            "invalid_scope",
        ] {
            assert_eq!(
                named_error(&json!({"error": code, "error_description": "why"})),
                Some(Poll::Failed {
                    code: code.to_string(),
                    message: "why".to_string()
                })
            );
        }
        assert_eq!(named_error(&json!({"access_token": "x"})), None);
    }

    #[test]
    fn a_redirect_is_read_only_when_it_carries_this_attempts_state() {
        let pkce = Pkce::start().unwrap();
        let good = format!("code=abc123&state={}", pkce.state());
        assert_eq!(pkce.code_of(&good).unwrap(), "abc123");
        let wrong = pkce.code_of("code=abc123&state=somebody-elses");
        assert!(matches!(wrong, Err(Poll::Failed { ref code, .. }) if code == "unsupported"));
        let denied = pkce.code_of(&format!("error=access_denied&state={}", pkce.state()));
        assert!(matches!(denied, Err(Poll::Failed { ref code, .. }) if code == "access_denied"));
    }

    /// The registered redirect is `http://127.0.0.1` with no port; the
    /// runtime one adds the ephemeral port and nothing else.
    #[test]
    fn the_authorize_url_carries_the_challenge_the_state_and_the_loopback_port() {
        let pkce = Pkce::start().unwrap();
        let oauth = OAuth {
            client_id: "cid".into(),
            flow: Flow::Pkce,
            device_endpoint: String::new(),
            auth_endpoint: "https://codeberg.org/login/oauth/authorize".into(),
            token_endpoint: "https://codeberg.org/login/oauth/access_token".into(),
            scopes: "read:user write:repository".into(),
        };
        let url = pkce.authorize_url(&oauth);
        assert!(url.starts_with("https://codeberg.org/login/oauth/authorize?"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={}", s256(pkce.verifier()))));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=read%3Auser%20write%3Arepository"));
        assert!(url.contains(&crate::url::encode_segment(&pkce.redirect_uri())));
        assert!(pkce.redirect_uri().starts_with("http://127.0.0.1:"));
        // the verifier itself never travels to the authorize endpoint
        assert!(!url.contains(pkce.verifier()));
    }

    #[test]
    fn a_query_component_decodes_its_escapes() {
        assert_eq!(decode_component("a%2Fb+c"), "a/b c");
        assert_eq!(decode_component("plain"), "plain");
    }
}
