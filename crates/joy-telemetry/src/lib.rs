// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The telemetry core of joy's Rust components (JOY-0266-CA, the Rust
//! half of the app's JAPP-01AD-85; rules in the observability ADR,
//! JP-00ED-EE): a `tracing` layer turns every event into an OTel-shaped
//! log record and every span into an OTel-shaped span, both carrying the
//! same resource (service, version, build, session id, project) and
//! both redacted before a sink sees them. The records are the SAME
//! shape the TypeScript core writes, so one incident reads the same in
//! every component.
//!
//! Where records go is a sink's business: the rolling file sink under
//! the personal state directory is the default, so the CLI and the
//! desktop backend keep their last records on the device whatever the
//! telemetry switch says. Exporting them is the frontend's job on the
//! desktop; a CLI has no platform session to export with.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes as SpanAttributes, Id};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// The resource every record of this process shares.
#[derive(Clone, Debug, Serialize)]
pub struct Resource {
    #[serde(rename = "service.name")]
    pub service_name: String,
    #[serde(rename = "service.version")]
    pub service_version: String,
    #[serde(rename = "joyint.build")]
    pub build: String,
    /// One id per process run: the key every analysis starts from.
    #[serde(rename = "joyint.session")]
    pub session: String,
    #[serde(rename = "joyint.project", skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

impl Resource {
    pub fn new(service: &str, version: &str) -> Self {
        Self {
            service_name: service.to_string(),
            service_version: version.to_string(),
            build: option_env!("JOY_BUILD_HASH").unwrap_or("").to_string(),
            session: random_hex(8),
            project: None,
        }
    }
}

/// A log record: why something happened (ADR rule 3).
#[derive(Clone, Debug, Serialize)]
pub struct LogRecord {
    pub kind: &'static str,
    #[serde(rename = "timeMs")]
    pub time_ms: i64,
    pub level: String,
    #[serde(rename = "severityNumber")]
    pub severity_number: u8,
    pub message: String,
    pub attributes: BTreeMap<String, Value>,
    #[serde(rename = "traceId", skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(rename = "spanId", skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
}

/// A span: what ran, and how long (ADR rule 2).
#[derive(Clone, Debug, Serialize)]
pub struct Span {
    pub kind: &'static str,
    #[serde(rename = "traceId")]
    pub trace_id: String,
    #[serde(rename = "spanId")]
    pub span_id: String,
    #[serde(rename = "parentSpanId", skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub name: String,
    #[serde(rename = "startMs")]
    pub start_ms: i64,
    #[serde(rename = "endMs")]
    pub end_ms: i64,
    pub attributes: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where finished records go.
pub trait Sink: Send + Sync {
    fn log(&self, record: &LogRecord);
    fn span(&self, span: &Span);
}

// ---- redaction (ADR rule 5) -------------------------------------------

const SECRET_KEYS: &[&str] = &[
    "passphrase",
    "password",
    "token",
    "secret",
    "authorization",
    "cookie",
    "session_env",
    "api_key",
    "api-key",
    "apikey",
];
pub const REDACTED: &str = "[redacted]";

fn secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEYS.iter().any(|s| lower.contains(s))
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '=' | '+' | '/' | '.' | '~')
}

/// Secrets by shape inside free text: joy delegation tokens, HTTP basic
/// credentials, bearer tokens.
pub fn redact_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let hit = ["joy_t_", "Basic ", "Bearer "]
            .iter()
            .filter_map(|marker| rest.find(marker).map(|at| (at, *marker)))
            .min_by_key(|(at, _)| *at);
        let Some((at, marker)) = hit else {
            out.push_str(rest);
            break;
        };
        let body_start = at + marker.len();
        let body_len = rest[body_start..]
            .chars()
            .take_while(|c| token_char(*c))
            .count();
        let min = if marker == "joy_t_" { 20 } else { 8 };
        if body_len >= min {
            out.push_str(&rest[..at]);
            out.push_str(REDACTED);
            let consumed = body_start
                + rest[body_start..]
                    .chars()
                    .take(body_len)
                    .map(char::len_utf8)
                    .sum::<usize>();
            rest = &rest[consumed..];
        } else {
            out.push_str(&rest[..body_start]);
            rest = &rest[body_start..];
        }
    }
    out
}

fn redact_value(key: &str, value: Value) -> Value {
    if secret_key(key) {
        return Value::String(REDACTED.into());
    }
    match value {
        Value::String(s) => Value::String(redact_text(&s)),
        other => other,
    }
}

// ---- the tracing layer -----------------------------------------------

fn random_hex(bytes: usize) -> String {
    let raw = uuid::Uuid::new_v4();
    raw.as_bytes()[..bytes]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn severity(level: &Level) -> (&'static str, u8) {
    match *level {
        Level::TRACE => ("debug", 1),
        Level::DEBUG => ("debug", 5),
        Level::INFO => ("info", 9),
        Level::WARN => ("warn", 13),
        Level::ERROR => ("error", 17),
    }
}

/// Fields of a span or event, as JSON values with the message apart.
#[derive(Default)]
struct Fields {
    message: String,
    attributes: BTreeMap<String, Value>,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}");
        if field.name() == "message" {
            self.message = text;
        } else {
            self.attributes.insert(
                field.name().to_string(),
                redact_value(field.name(), Value::String(text)),
            );
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.attributes.insert(
                field.name().to_string(),
                redact_value(field.name(), Value::String(value.to_string())),
            );
        }
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.attributes.insert(
            field.name().to_string(),
            redact_value(field.name(), value.into()),
        );
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.attributes.insert(
            field.name().to_string(),
            redact_value(field.name(), value.into()),
        );
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.attributes.insert(
            field.name().to_string(),
            redact_value(field.name(), value.into()),
        );
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.attributes.insert(
            field.name().to_string(),
            Value::String(redact_text(&value.to_string())),
        );
    }
}

/// What the layer remembers per open span.
struct OpenSpan {
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
    name: String,
    start_ms: i64,
    attributes: BTreeMap<String, Value>,
    error: Option<String>,
}

/// The layer: events become log records, closed spans become spans, both
/// go to every sink. Filtering by level happens here, from `level()`.
pub struct TelemetryLayer {
    resource: Arc<Mutex<Resource>>,
    sinks: Vec<Arc<dyn Sink>>,
    max_level: Level,
}

impl TelemetryLayer {
    pub fn new(resource: Resource, sinks: Vec<Arc<dyn Sink>>, max_level: Level) -> Self {
        Self {
            resource: Arc::new(Mutex::new(resource)),
            sinks,
            max_level,
        }
    }

    /// The shared resource handle: the host sets the project once it
    /// knows it.
    pub fn resource(&self) -> Arc<Mutex<Resource>> {
        self.resource.clone()
    }
}

impl<S> Layer<S> for TelemetryLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        *metadata.level() <= self.max_level
    }

    fn on_new_span(&self, attrs: &SpanAttributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let parent = span.parent().and_then(|p| {
            p.extensions()
                .get::<OpenSpan>()
                .map(|o| (o.trace_id.clone(), o.span_id.clone()))
        });
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        span.extensions_mut().insert(OpenSpan {
            trace_id: parent
                .as_ref()
                .map(|(t, _)| t.clone())
                .unwrap_or_else(|| random_hex(16)),
            span_id: random_hex(8),
            parent_span_id: parent.map(|(_, s)| s),
            name: span.name().to_string(),
            start_ms: now_ms(),
            attributes: fields.attributes,
            error: None,
        });
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let (level, severity_number) = severity(event.metadata().level());
        let current = ctx.event_span(event).and_then(|span| {
            span.extensions()
                .get::<OpenSpan>()
                .map(|o| (o.trace_id.clone(), o.span_id.clone()))
        });
        // an error event inside a span marks the span as failed
        if *event.metadata().level() == Level::ERROR {
            if let Some(span) = ctx.event_span(event) {
                if let Some(open) = span.extensions_mut().get_mut::<OpenSpan>() {
                    open.error.get_or_insert_with(|| fields.message.clone());
                }
            }
        }
        let record = LogRecord {
            kind: "log",
            time_ms: now_ms(),
            level: level.to_string(),
            severity_number,
            message: redact_text(&fields.message),
            attributes: fields.attributes,
            trace_id: current.as_ref().map(|(t, _)| t.clone()),
            span_id: current.map(|(_, s)| s),
        };
        for sink in &self.sinks {
            sink.log(&record);
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let Some(open) = span.extensions_mut().remove::<OpenSpan>() else {
            return;
        };
        let finished = Span {
            kind: "span",
            trace_id: open.trace_id,
            span_id: open.span_id,
            parent_span_id: open.parent_span_id,
            name: open.name,
            start_ms: open.start_ms,
            end_ms: now_ms(),
            attributes: open.attributes,
            error: open.error,
        };
        for sink in &self.sinks {
            sink.span(&finished);
        }
    }
}

// ---- sinks -------------------------------------------------------------

/// JSON lines into a rolling file: past `roll_bytes` the current file
/// becomes `.1` and a fresh one starts, so the history spans roughly two
/// files and never grows without bound.
pub struct FileSink {
    path: PathBuf,
    roll_bytes: u64,
    lock: Mutex<()>,
}

impl FileSink {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            roll_bytes: 5 * 1024 * 1024,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One finished JSON line into the file - the sink's own records, or a
    /// line another process handed over (the desktop frontend's records
    /// arrive this way, JAPP-01AE-E5).
    pub fn append_line(&self, line: &str) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(meta) = std::fs::metadata(&self.path) {
            if meta.len() >= self.roll_bytes {
                let rolled = self.path.with_extension("jsonl.1");
                let _ = std::fs::rename(&self.path, rolled);
            }
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            return; // a full or missing disk must not take the process down
        };
        let _ = file
            .write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"));
    }

    /// The last `max` lines of the current file, newest last: what a
    /// feedback report attaches while telemetry is off.
    pub fn tail(&self, max: usize) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(max);
        lines[start..].iter().map(|l| l.to_string()).collect()
    }
}

impl Sink for FileSink {
    fn log(&self, record: &LogRecord) {
        if let Ok(line) = serde_json::to_string(record) {
            self.append_line(&line);
        }
    }
    fn span(&self, span: &Span) {
        if let Ok(line) = serde_json::to_string(span) {
            self.append_line(&line);
        }
    }
}

/// Records kept in memory, for tests and for hosts that want to look.
#[derive(Default)]
pub struct MemorySink {
    pub logs: Mutex<Vec<LogRecord>>,
    pub spans: Mutex<Vec<Span>>,
}

impl Sink for MemorySink {
    fn log(&self, record: &LogRecord) {
        self.logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(record.clone());
    }
    fn span(&self, span: &Span) {
        self.spans
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(span.clone());
    }
}

// ---- level and paths ---------------------------------------------------

/// The level records are kept at: `JOY_LOG` in the environment, else
/// `telemetry.level` in the personal config (`~/.config/joy/config.yaml`,
/// JAPP-01B2-35: a file only, no UI - pros only), else `info`.
pub fn level() -> Level {
    let from_env = std::env::var("JOY_LOG")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let from_config = || {
        let text = std::fs::read_to_string(joy_core::store::global_config_path()).ok()?;
        let doc: Value = serde_yaml_ng::from_str(&text).ok()?;
        doc.get("telemetry")?
            .get("level")?
            .as_str()
            .map(str::to_string)
    };
    parse_level(from_env.or_else(from_config).as_deref()).unwrap_or(Level::INFO)
}

fn parse_level(text: Option<&str>) -> Option<Level> {
    match text?.trim().to_ascii_lowercase().as_str() {
        "error" => Some(Level::ERROR),
        "warn" | "warning" => Some(Level::WARN),
        "info" => Some(Level::INFO),
        "debug" => Some(Level::DEBUG),
        "trace" => Some(Level::TRACE),
        _ => None,
    }
}

/// The telemetry file of this machine's person: `<state>/joy/telemetry.jsonl`.
pub fn default_file() -> PathBuf {
    joy_core::store::state_base_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("joy")
        .join("telemetry.jsonl")
}

/// What `init` hands back: the resource handle (set the project when it
/// is known) and the file the records go to.
pub struct Telemetry {
    pub resource: Arc<Mutex<Resource>>,
    pub file: PathBuf,
}

/// Install the layer as the process's tracing subscriber with the file
/// sink. A subscriber already installed (a host with its own) wins, and
/// the returned handle then only names the file that would have been
/// written.
pub fn init(service: &str, version: &str) -> Telemetry {
    use tracing_subscriber::layer::SubscriberExt;
    let file = default_file();
    let sink: Arc<dyn Sink> = Arc::new(FileSink::new(file.clone()));
    let layer = TelemetryLayer::new(Resource::new(service, version), vec![sink], level());
    let resource = layer.resource();
    let subscriber = tracing_subscriber::registry().with(layer);
    let _ = tracing::subscriber::set_global_default(subscriber);
    Telemetry { resource, file }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    fn with_layer<F: FnOnce()>(sink: Arc<MemorySink>, f: F) {
        let layer = TelemetryLayer::new(
            Resource::new("joy-test", "0.0.0"),
            vec![sink as Arc<dyn Sink>],
            Level::DEBUG,
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, f);
    }

    #[test]
    fn events_become_records_and_spans_nest_with_their_error() {
        let sink = Arc::new(MemorySink::default());
        with_layer(sink.clone(), || {
            let outer = tracing::info_span!("click", view = "board");
            let _o = outer.enter();
            {
                let inner = tracing::info_span!("rpc", method = "ListItems");
                let _i = inner.enter();
                tracing::error!(item = "JAPP-1", "boom");
            }
            tracing::info!("decided");
        });
        let logs = sink.logs.lock().unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].level, "error");
        assert_eq!(logs[0].attributes["item"], "JAPP-1");
        let spans = sink.spans.lock().unwrap();
        assert_eq!(
            spans.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["rpc", "click"]
        );
        let rpc = &spans[0];
        let click = &spans[1];
        assert_eq!(rpc.parent_span_id.as_deref(), Some(click.span_id.as_str()));
        assert_eq!(rpc.trace_id, click.trace_id);
        assert_eq!(rpc.error.as_deref(), Some("boom"));
        assert_eq!(logs[0].span_id.as_deref(), Some(rpc.span_id.as_str()));
        assert_eq!(logs[1].span_id.as_deref(), Some(click.span_id.as_str()));
    }

    #[test]
    fn secrets_are_redacted_by_key_and_by_shape() {
        let sink = Arc::new(MemorySink::default());
        with_layer(sink.clone(), || {
            tracing::warn!(
                passphrase = "seven nations army",
                note = "sent Bearer abcdefghijklmnop today",
                item = "X",
                "redeemed joy_t_abcdefghijklmnopqrstuvwxyz0123456789 now"
            );
        });
        let logs = sink.logs.lock().unwrap();
        assert_eq!(logs[0].message, "redeemed [redacted] now");
        assert_eq!(logs[0].attributes["passphrase"], "[redacted]");
        assert_eq!(logs[0].attributes["note"], "sent [redacted] today");
        assert_eq!(logs[0].attributes["item"], "X");
        assert_eq!(redact_text("Basic abc"), "Basic abc"); // too short to be a credential
        assert_eq!(redact_text("no secrets here"), "no secrets here");
    }

    #[test]
    fn the_level_parses_and_defaults_to_info() {
        assert_eq!(parse_level(Some("DEBUG")), Some(Level::DEBUG));
        assert_eq!(parse_level(Some("warning")), Some(Level::WARN));
        assert_eq!(parse_level(Some("loud")), None);
        assert_eq!(parse_level(None), None);
    }

    #[test]
    fn the_file_sink_appends_lines_and_rolls_over() {
        let dir = std::env::temp_dir().join(format!("joy-telemetry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut sink = FileSink::new(dir.join("telemetry.jsonl"));
        sink.roll_bytes = 200;
        let record = LogRecord {
            kind: "log",
            time_ms: 1,
            level: "info".into(),
            severity_number: 9,
            message: "x".repeat(120),
            attributes: BTreeMap::new(),
            trace_id: None,
            span_id: None,
        };
        sink.log(&record);
        sink.log(&record); // past 200 bytes: the next append rolls
        sink.log(&record);
        assert!(dir.join("telemetry.jsonl.1").exists());
        assert_eq!(sink.tail(10).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
