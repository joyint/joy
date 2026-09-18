// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! An in process fake forge API, for the connector's own tests.
//!
//! The connectors now speak HTTP themselves (D2.8), so their tests need
//! a forge to speak to. This is one: a tiny HTTP/1.1 server on the
//! loopback interface that answers whatever the test's handler says and
//! records every request it saw, so a test can assert on the URL a
//! connector built and on the header a token travelled in.
//!
//! No real network is ever touched: the address is `127.0.0.1` on a
//! port the operating system picks.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// One request the fake saw.
#[derive(Debug, Clone)]
pub struct Call {
    pub method: String,
    /// Path and query string, as the connector built it.
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Call {
    /// One header, case insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The `Authorization` header, where one arrived.
    pub fn authorization(&self) -> Option<&str> {
        self.header("authorization")
    }

    /// The body as JSON.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.body).ok()
    }
}

/// What the fake answers.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.into(),
        }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: body.into(),
        }
    }

    pub fn not_found() -> Self {
        Reply::json(404, r#"{"message":"Not Found"}"#)
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// A running fake. Dropping it stops the thread.
pub struct FakeForge {
    addr: SocketAddr,
    calls: Arc<Mutex<Vec<Call>>>,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeForge {
    /// Start one. The handler sees every request and answers it.
    pub fn start<H>(handler: H) -> FakeForge
    where
        H: Fn(&Call) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake forge");
        let addr = listener.local_addr().expect("the fake forge's address");
        listener
            .set_nonblocking(true)
            .expect("the fake forge must not block its own shutdown");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));
        let thread = {
            let calls = calls.clone();
            let running = running.clone();
            let handler = Arc::new(handler);
            std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            serve(stream, &calls, handler.as_ref());
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        FakeForge {
            addr,
            calls,
            running,
            thread: Some(thread),
        }
    }

    /// The base URL a connector is pointed at, e.g. through
    /// `forges.yaml`'s `api_base`.
    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The host and port, as a remote URL carries it.
    pub fn authority(&self) -> String {
        self.addr.to_string()
    }

    /// Every request the fake saw, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Whether any request went to this path.
    pub fn saw(&self, method: &str, path: &str) -> bool {
        self.calls()
            .iter()
            .any(|call| call.method == method && call.path == path)
    }
}

impl Drop for FakeForge {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve<H>(stream: TcpStream, calls: &Arc<Mutex<Vec<Call>>>, handler: &H)
where
    H: Fn(&Call) -> Reply,
{
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }
    let call = Call {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    let reply = handler(&call);
    calls.lock().unwrap_or_else(|e| e.into_inner()).push(call);
    let mut out = stream;
    let mut response = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reason(reply.status),
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    response.push_str(&reply.body);
    let _ = out.write_all(response.as_bytes());
    let _ = out.flush();
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

// -- a forge to drive the sign in verbs against (package J3) ------------------

/// A forge whose every endpoint is the in process fake, for the tests
/// of `auth::verbs`.
///
/// It exists so that `login`, `token`, `token-store` and `logout` can
/// be proved without a real forge and without the three connector
/// crates: the shapes of D2.4, the event stream, the refresh lock and
/// the login order of D4.1c are the connector's own business, and this
/// is the forge that stands still while they are checked.
///
/// Its id is `github`, so the scope tables of D2.7a apply unchanged.
pub struct TestForge {
    /// The fake's base URL, e.g. `http://127.0.0.1:34567`.
    pub base: String,
    pub flow: crate::auth::oauth::Flow,
    pub client_id: String,
    /// What a forge CLI would report as signed in on this host.
    pub foreign: Vec<String>,
}

impl TestForge {
    /// A device grant forge on this fake.
    pub fn device(base: impl Into<String>) -> TestForge {
        TestForge {
            base: base.into(),
            flow: crate::auth::oauth::Flow::Device,
            client_id: "test-client".to_string(),
            foreign: Vec::new(),
        }
    }

    /// A PKCE loopback forge on this fake (the Gitea family's door).
    pub fn pkce(base: impl Into<String>) -> TestForge {
        TestForge {
            base: base.into(),
            flow: crate::auth::oauth::Flow::Pkce,
            client_id: "test-client".to_string(),
            foreign: Vec::new(),
        }
    }

    /// Say which logins a forge CLI holds on this host.
    pub fn with_foreign(mut self, logins: &[&str]) -> TestForge {
        self.foreign = logins.iter().map(|login| login.to_string()).collect();
        self
    }
}

impl crate::forge::Forge for TestForge {
    fn id(&self) -> &'static str {
        "github"
    }

    fn display(&self) -> &'static str {
        "TestForge"
    }

    fn claims(&self, _host: &str, _ctx: &crate::forge::Ctx) -> bool {
        true
    }

    fn identity(&self, _t: &crate::forge::Target, _c: &crate::forge::Ctx) -> serde_json::Value {
        crate::forge::unknown()
    }

    fn resolve(&self, _email: &str) -> serde_json::Value {
        crate::forge::unknown()
    }

    fn store(&self, _t: &crate::forge::Target, _c: &crate::forge::Ctx) -> serde_json::Value {
        crate::forge::unknown_state()
    }

    fn files(&self, _t: &crate::forge::Target, _c: &crate::forge::Ctx) -> serde_json::Value {
        crate::forge::unknown_state()
    }

    fn repositories(
        &self,
        _t: &crate::forge::Target,
        _l: &crate::forge::Listing,
        _c: &crate::forge::Ctx,
    ) -> serde_json::Value {
        crate::forge::unknown_state()
    }

    fn create_repository(
        &self,
        _t: &crate::forge::Target,
        _n: &crate::forge::NewRepository,
        _c: &crate::forge::Ctx,
    ) -> serde_json::Value {
        crate::forge::unknown_state()
    }

    fn release(
        &self,
        _t: &crate::forge::Target,
        _r: &crate::forge::ReleaseRequest,
        _c: &crate::forge::Ctx,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({ "unsupported": true }))
    }

    fn scopes(&self, purpose: crate::auth::Purpose) -> &'static str {
        match purpose {
            crate::auth::Purpose::Read => "repo:read user:email",
            _ => "repo user:email",
        }
    }

    fn oauth(
        &self,
        _host: &str,
        purpose: crate::auth::Purpose,
        _ctx: &crate::forge::Ctx,
    ) -> Option<crate::auth::oauth::OAuth> {
        Some(crate::auth::oauth::OAuth {
            client_id: self.client_id.clone(),
            flow: self.flow,
            device_endpoint: format!("{}/login/device/code", self.base),
            auth_endpoint: format!("{}/login/oauth/authorize", self.base),
            token_endpoint: format!("{}/login/oauth/access_token", self.base),
            scopes: self.scopes(purpose).to_string(),
        })
    }

    fn account(
        &self,
        host: &str,
        token: &str,
        ctx: &crate::forge::Ctx,
    ) -> crate::forge::AccountAnswer {
        use crate::forge::AccountAnswer;
        let http = match ctx.http(host) {
            Ok(http) => http,
            Err(error) => return AccountAnswer::Unreachable(error.to_string()),
        };
        let answer = match http
            .get(&format!("{}/user", self.base))
            .bearer(token)
            .call()
        {
            Ok(answer) => answer,
            Err(error) => return AccountAnswer::Unreachable(error.to_string()),
        };
        if !answer.ok() {
            return AccountAnswer::Refused;
        }
        let Some(body) = answer.json() else {
            return AccountAnswer::Refused;
        };
        let Some(login) = body.get("login").and_then(|v| v.as_str()) else {
            return AccountAnswer::Refused;
        };
        AccountAnswer::Known(crate::forge::Account {
            login: login.to_string(),
            user_id: body
                .get("id")
                .and_then(|v| v.as_i64())
                .map(|id| id.to_string()),
            emails: body
                .get("emails")
                .and_then(|v| v.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            // The granted set, space separated, as D2.7c asks for it.
            scopes: answer
                .header("x-oauth-scopes")
                .map(|raw| crate::scope::parse_granted(raw).join(" ")),
        })
    }

    fn reaches(
        &self,
        host: &str,
        repo_path: &str,
        token: &str,
        ctx: &crate::forge::Ctx,
    ) -> Option<crate::forge::Reach> {
        let http = ctx.http(host).ok()?;
        let answer = http
            .get(&format!("{}/repos/{repo_path}", self.base))
            .bearer(token)
            .call()
            .ok()?;
        if !answer.ok() {
            return Some(crate::forge::Reach::default());
        }
        let body = answer.json().unwrap_or_default();
        Some(crate::forge::Reach {
            read: true,
            push: body
                .pointer("/permissions/push")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    fn web_url(&self, target: &crate::forge::Target, ctx: &crate::forge::Ctx) -> serde_json::Value {
        crate::auth::verbs::https_twin(target, ctx)
    }

    fn revoke(
        &self,
        host: &str,
        record: &crate::auth::store::Record,
        ctx: &crate::forge::Ctx,
    ) -> bool {
        let Some(client_id) = record.client_id.as_deref() else {
            return false;
        };
        let Ok(http) = ctx.http(host) else {
            return false;
        };
        http.delete(&format!("{}/applications/{client_id}/token", self.base))
            .basic(client_id, "")
            .send_json(&serde_json::json!({ "access_token": record.token }))
            .map(|answer| answer.status == 204)
            .unwrap_or(false)
    }

    fn https_username(&self) -> &'static str {
        "x-access-token"
    }

    fn foreign_cli(&self) -> &'static str {
        "gh"
    }

    fn foreign_logout_command(&self, host: &str) -> String {
        format!("gh auth logout --hostname {host}")
    }

    fn foreign_logins(&self, _host: &str) -> Vec<String> {
        self.foreign.clone()
    }
}
