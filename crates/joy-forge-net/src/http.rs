// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The connector's own HTTP client (D2.8).
//!
//! Until now every forge call was a `curl` or `gh` subprocess, which is
//! why a machine without curl could not answer `identity` and why
//! publishing a release needed gh. The client is in process now, over
//! rustls, and it honours the proxy sources of D1.11 and the trust
//! store of D1.12 exactly as the engine does.
//!
//! Two rules hold for every call and are why this is one door:
//!
//! - a token travels in a header, never in an argument, so it cannot
//!   appear in a process list (D5, "the token stays out of argv");
//! - no header value is ever printed, so an error text cannot carry a
//!   token or a proxy password.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use crate::gitconfig::GitConfig;
use crate::proxy::{self, ProxyChoice};
use crate::trust::{self, Trust};

/// The default per request bound. The verb deadlines of D2.3 bound the
/// whole call; this bounds one request inside it, so a host that
/// accepts a connection and then says nothing cannot eat the verb's
/// whole budget.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// One answer: the status, the headers a classifier reads, the body.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub body: String,
    headers: Vec<(String, String)>,
}

impl Answer {
    /// Build one directly (the fake API and the tests use this).
    pub fn new(status: u16, body: impl Into<String>, headers: Vec<(String, String)>) -> Self {
        Answer {
            status,
            body: body.into(),
            headers,
        }
    }

    /// One header value, case insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Whether the forge answered 2xx.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as JSON, or `None` when it is not JSON.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.body).ok()
    }
}

/// Why a request produced no answer at all. It never carries a header
/// value, so a token cannot travel inside it.
#[derive(Debug, Clone)]
pub enum HttpError {
    /// The request did not complete: DNS, connection, TLS, timeout.
    Transport { url: String, reason: String },
    /// joy refused to make the request: a SOCKS proxy, a CA file it
    /// could not read.
    Refused { message: String },
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Transport { url, reason } => {
                write!(f, "{url} could not be reached: {reason}")
            }
            HttpError::Refused { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for HttpError {}

/// The client. One per process; it builds an agent per proxy decision
/// and keeps it, so a verb that asks several questions of one host
/// reuses one connection pool.
pub struct Http {
    trust: Trust,
    config: GitConfig,
    user_agent: String,
    agents: Mutex<HashMap<String, ureq::Agent>>,
}

impl Http {
    /// A client with the trust of D1.12 and the git configuration the
    /// proxy decision of D1.11 reads.
    pub fn new(trust: Trust, config: GitConfig, user_agent: impl Into<String>) -> Self {
        Http {
            trust,
            config,
            user_agent: user_agent.into(),
            agents: Mutex::new(HashMap::new()),
        }
    }

    /// A GET.
    pub fn get(&self, url: &str) -> Request<'_> {
        self.request("GET", url)
    }

    /// A POST.
    pub fn post(&self, url: &str) -> Request<'_> {
        self.request("POST", url)
    }

    /// A PATCH.
    pub fn patch(&self, url: &str) -> Request<'_> {
        self.request("PATCH", url)
    }

    /// A request with any method.
    pub fn request(&self, method: &'static str, url: &str) -> Request<'_> {
        Request {
            http: self,
            method,
            url: url.to_string(),
            headers: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// The agent for this URL's proxy decision.
    fn agent(&self, url: &str) -> Result<ureq::Agent, HttpError> {
        let choice = proxy::choose(url, &self.config, &proxy::ProcessEnv).map_err(|refusal| {
            HttpError::Refused {
                message: refusal.message,
            }
        })?;
        let key = match &choice {
            ProxyChoice::Direct => String::new(),
            ProxyChoice::Proxy(url) => url.clone(),
        };
        let mut agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = agents.get(&key) {
            return Ok(agent.clone());
        }
        let agent = self.build_agent(&choice)?;
        agents.insert(key, agent.clone());
        Ok(agent)
    }

    fn build_agent(&self, choice: &ProxyChoice) -> Result<ureq::Agent, HttpError> {
        let tls = self.tls_config()?;
        // The proxy is joy's decision, never ureq's: `Config::default`
        // reads ALL_PROXY, HTTPS_PROXY and HTTP_PROXY with its own
        // order and its own NO_PROXY grammar, and D1.11 names a
        // different one. Setting it explicitly (`None` included) is
        // what turns that off.
        let proxy = match choice {
            ProxyChoice::Direct => None,
            ProxyChoice::Proxy(url) => {
                Some(ureq::Proxy::new(url).map_err(|e| HttpError::Refused {
                    message: format!("joy cannot use the proxy {}: {e}", proxy::redact(url)),
                })?)
            }
        };
        let config = ureq::config::Config::builder()
            .proxy(proxy)
            // joy classifies statuses itself (D2.7c), so a 4xx is an
            // answer here and not an error.
            .http_status_as_error(false)
            .user_agent(self.user_agent.clone())
            .tls_config(tls)
            .build();
        Ok(config.new_agent())
    }

    fn tls_config(&self) -> Result<ureq::tls::TlsConfig, HttpError> {
        let roots = match &self.trust {
            Trust::Platform => ureq::tls::RootCerts::PlatformVerifier,
            Trust::Files { bundle, dir } => {
                let ders = trust::certificates(bundle.as_deref(), dir.as_deref())
                    .map_err(|message| HttpError::Refused { message })?;
                let certs: Vec<ureq::tls::Certificate<'static>> = ders
                    .iter()
                    .map(|der| ureq::tls::Certificate::from_der(der).to_owned())
                    .collect();
                ureq::tls::RootCerts::new_with_certs(&certs)
            }
        };
        Ok(ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::Rustls)
            .root_certs(roots)
            .build())
    }
}

/// One request being built. The token goes in through [`Request::bearer`]
/// or [`Request::token_header`] and never anywhere else.
pub struct Request<'a> {
    http: &'a Http,
    method: &'static str,
    url: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
}

impl Request<'_> {
    /// Add a header.
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    /// `Authorization: Bearer <token>`, the shape GitHub and GitLab take.
    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }

    /// `Authorization: token <token>`, the shape the Gitea family takes.
    pub fn token_header(self, token: &str) -> Self {
        self.header("Authorization", format!("token {token}"))
    }

    /// A different bound for this one request (an asset upload).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Send with no body.
    pub fn call(self) -> Result<Answer, HttpError> {
        self.send(None)
    }

    /// Send a JSON body.
    pub fn send_json(self, body: &serde_json::Value) -> Result<Answer, HttpError> {
        let raw = serde_json::to_vec(body).unwrap_or_default();
        self.header("Content-Type", "application/json")
            .send(Some(raw))
    }

    /// Send raw bytes with a content type (an asset upload).
    pub fn send_bytes(self, content_type: &str, body: Vec<u8>) -> Result<Answer, HttpError> {
        self.header("Content-Type", content_type).send(Some(body))
    }

    fn send(self, body: Option<Vec<u8>>) -> Result<Answer, HttpError> {
        let agent = self.http.agent(&self.url)?;
        let mut builder = ureq::http::Request::builder()
            .method(self.method)
            .uri(&self.url);
        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let build_error = |e: ureq::http::Error| HttpError::Transport {
            url: redact_url(&self.url),
            reason: e.to_string(),
        };
        let result = match body {
            Some(body) => {
                let request = builder.body(body).map_err(build_error)?;
                let request = agent
                    .configure_request(request)
                    .timeout_global(Some(self.timeout))
                    .build();
                agent.run(request)
            }
            None => {
                let request = builder.body(()).map_err(build_error)?;
                let request = agent
                    .configure_request(request)
                    .timeout_global(Some(self.timeout))
                    .build();
                agent.run(request)
            }
        };
        let mut response = result.map_err(|e| HttpError::Transport {
            url: redact_url(&self.url),
            reason: e.to_string(),
        })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| HttpError::Transport {
                url: redact_url(&self.url),
                reason: e.to_string(),
            })?;
        Ok(Answer {
            status,
            body,
            headers,
        })
    }
}

/// A URL as it may appear in a message: without a query string, which
/// is where a forge puts a name, and without userinfo.
fn redact_url(url: &str) -> String {
    let url = url.split('?').next().unwrap_or(url);
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), url),
    };
    match rest.split_once('@') {
        Some((_userinfo, host)) => format!("{scheme}{host}"),
        None => format!("{scheme}{rest}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_reads_its_headers_case_insensitively() {
        let answer = Answer::new(
            403,
            "{}",
            vec![("x-oauth-scopes".into(), "read:user".into())],
        );
        assert_eq!(answer.header("X-OAuth-Scopes"), Some("read:user"));
        assert_eq!(answer.header("x-github-sso"), None);
        assert!(!answer.ok());
    }

    #[test]
    fn a_message_carries_neither_a_query_string_nor_userinfo() {
        assert_eq!(
            redact_url("https://u:p@uploads.github.com/x/assets?name=joy.tar.gz"),
            "https://uploads.github.com/x/assets"
        );
    }
}
