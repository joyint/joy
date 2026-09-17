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
