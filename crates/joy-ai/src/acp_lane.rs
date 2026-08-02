// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! THE lane-and-session machine (JI-017A-85, decided 2026-07-29): one
//! code for every ACP use case — chat turns on the desktop, chat turns on
//! the platform, and job rounds. It grew as two near-identical bodies
//! (the desktop's chat lane and the platform's SessionManager) plus a
//! second copy of the notification collector, and they drifted exactly
//! the way the adapter facts did.
//!
//! The shape, unchanged from what both hosts converged on:
//!
//! * A LANE is one long-lived agent process with one ACP connection,
//!   keyed by the host (the platform keys per project+adapter, the
//!   desktop per project root+entrypoint). The lane carries a config
//!   FINGERPRINT (key, model, adapter): a change respawns it so no turn
//!   runs with a stale secret.
//! * Inside a lane, ONE SESSION PER CHAT. A live session gets only the
//!   DELTA prompt; a fresh session replays the full transcript —
//!   `.joy/chats` stays the only truth, the agent session is a cache.
//! * A dead lane (container stop, process exit) is normal: the next turn
//!   respawns once and replays.
//!
//! What stays with the hosts is what really differs: HOW the process
//! starts (the registry entrypoint locally, or bridged through a
//! long-lived `docker exec`), and what surrounds it (container health,
//! budgets, delegation checks). Those arrive here as data — the command
//! line, a prepare hook for the spawn descriptor, a preamble for fresh
//! sessions — never as a second code path.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, Cost, Implementation, InitializeRequest, NewSessionRequest,
    PermissionOptionId, PermissionOptionKind, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionConfigKind, SessionConfigSelectOptions, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, StopReason, TextContent, ToolKind,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{AcpAgent, Agent, ConnectionTo};
use std::str::FromStr;
use tokio::sync::{mpsc, oneshot};

use crate::turn_engine::{TurnActivity, TurnOutcome, TurnSink, WireActivity};

// ---------------------------------------------------------------------------
// The collector: one ACP notification stream folded into one record.
// ---------------------------------------------------------------------------

/// What the `tool_call` notification told us about a call, kept so the
/// permission request that follows can be judged on facts instead of on
/// the nulls it carries itself (vibe sends title, kind and rawInput all
/// null there and names them only in the notification; measured against
/// vibe-acp 2.22, JAPP-0152-81).
#[derive(Default, Clone)]
pub struct KnownCall {
    pub title: Option<String>,
    pub raw_input: Option<serde_json::Value>,
    /// The ACP tool kind as it goes over the wire ("read", "execute", …).
    pub kind: Option<String>,
}

/// An ACP tool kind as its wire string, the form the shared policy
/// classifies on.
pub fn wire_kind(kind: impl Into<Option<ToolKind>>) -> Option<String> {
    serde_json::to_value(kind.into()?)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

// The live view of a running turn rides the shared TurnSink
// (turn_engine): the lane owns the ordered delivery and the drain, the
// host owns only the transport of one WireActivity (JOY-0249-D2).

/// Everything one ACP session produced so far.
#[derive(Default)]
pub struct Collected {
    pub reply: String,
    pub thoughts: String,
    /// One row per tool call, in call order, gate answer included.
    pub tools: Vec<crate::activity::ToolStep>,
    /// toolCallId -> its row in `tools`. An agent sends several
    /// notifications for ONE call (vibe announces a bare "bash" and names
    /// the command in a second one, same id); keyed, a later notification
    /// UPDATES its row wherever it sits.
    pub tool_rows: HashMap<String, usize>,
    /// Answered permissions that matched no call (an agent that asks
    /// without announcing first). Normally empty.
    pub permissions: Vec<(String, String)>,
    /// toolCallId -> what its `tool_call` notification carried.
    pub tool_calls: HashMap<String, KnownCall>,
    /// The newest ACP UsageUpdate cost (CUMULATIVE per session,
    /// JP-0089-18). A chat lane turns it into a per-turn delta; a job
    /// round is one fresh session, so there it IS the round total.
    pub cost: Option<Cost>,
    /// Tokens used, same UsageUpdate, same cumulative semantics.
    pub used: u64,
    /// Non-text content blocks the agent sent (JOY-024B-AC interim):
    /// recorded as facts, never dropped.
    pub contents: Vec<crate::activity::ContentInfo>,
}

impl Collected {
    /// What the correlated `tool_call` notification said about this id,
    /// or nothing when the call was never announced.
    pub fn known(&self, call_id: &str) -> KnownCall {
        self.tool_calls.get(call_id).cloned().unwrap_or_default()
    }

    /// Record an answered permission: it belongs to the CALL it opens, and
    /// only an answer with no call to sit on keeps its own row (operator
    /// 2026-07-27).
    pub fn record_permission(&mut self, call_id: &str, title: String, answered: &str) {
        match self.tool_rows.get(call_id).copied() {
            Some(at) => self.tools[at].answered = Some(answered.to_string()),
            None => self.permissions.push((title, answered.to_string())),
        }
    }

    /// The finished record: reply, denied count and the persisted
    /// activity block, through THE one producer (`crate::activity`,
    /// JI-0179-4F step 3). Cost and tokens are the caller's business —
    /// a lane turns the cumulative usage into a per-turn delta, a round
    /// takes it whole, a local desktop turn has none.
    pub fn into_outcome(self) -> TurnOutcome {
        let tool_steps = self.tools.len() as u32;
        // Refused steps, so a turn that ends silently can say why
        // (JAPP-0152-81).
        let denied = self
            .tools
            .iter()
            .filter_map(|step| step.answered.as_deref())
            .chain(self.permissions.iter().map(|(_, answer)| answer.as_str()))
            .filter(|answer| *answer == "denied")
            .count() as u32;
        let details = crate::activity::Activity {
            thoughts: self.thoughts,
            tools: self.tools,
            permissions: self.permissions,
            contents: self.contents,
        }
        .to_details_json();
        TurnOutcome {
            reply: self.reply.trim().to_string(),
            details,
            tool_steps,
            denied,
            ..Default::default()
        }
    }
}

/// What a non-text content block IS, as record and live view show it
/// (JOY-024B-AC interim): kind for the icon, one human label with
/// name/MIME/size. The payload itself waits for content v2.
fn content_info(block: &ContentBlock) -> crate::activity::ContentInfo {
    fn human_size(bytes: usize) -> String {
        if bytes >= 1_000_000 {
            format!("{:.1} MB", bytes as f64 / 1_000_000.0)
        } else if bytes >= 1_000 {
            format!("{} kB", bytes / 1_000)
        } else {
            format!("{bytes} B")
        }
    }
    /// base64 payload -> approximate decoded size
    fn b64_size(data: &str) -> usize {
        data.len() * 3 / 4
    }
    let (kind, parts): (&str, Vec<String>) = match block {
        ContentBlock::Image(image) => (
            "image",
            vec![image.mime_type.clone(), human_size(b64_size(&image.data))],
        ),
        ContentBlock::Audio(audio) => (
            "audio",
            vec![audio.mime_type.clone(), human_size(b64_size(&audio.data))],
        ),
        ContentBlock::Resource(resource) => match &resource.resource {
            agent_client_protocol::schema::v1::EmbeddedResourceResource::TextResourceContents(
                text,
            ) => (
                "resource",
                [
                    Some(text.uri.clone()),
                    text.mime_type.clone(),
                    Some(human_size(text.text.len())),
                ]
                .into_iter()
                .flatten()
                .collect(),
            ),
            agent_client_protocol::schema::v1::EmbeddedResourceResource::BlobResourceContents(
                blob,
            ) => (
                "resource",
                [
                    Some(blob.uri.clone()),
                    blob.mime_type.clone(),
                    Some(human_size(b64_size(&blob.blob))),
                ]
                .into_iter()
                .flatten()
                .collect(),
            ),
            // non_exhaustive: an unknown payload still leaves a trace
            _ => ("resource", vec!["unknown resource payload".to_string()]),
        },
        ContentBlock::ResourceLink(link) => (
            "link",
            [
                Some(link.title.clone().unwrap_or_else(|| link.name.clone())),
                Some(link.uri.clone()),
                link.mime_type.clone(),
                link.size.map(|s| human_size(s.max(0) as usize)),
            ]
            .into_iter()
            .flatten()
            .collect(),
        ),
        // ContentBlock is non_exhaustive: an unknown kind still leaves
        // its trace instead of vanishing.
        _ => ("content", vec!["unknown content block".to_string()]),
    };
    crate::activity::ContentInfo {
        kind: kind.to_string(),
        label: parts.join(" · "),
    }
}

/// Fold one session notification into the collected state and return
/// the live activity it produced. The CALLER routes the events: a chat
/// turn queues them onto its ordered delivery wire, a job round drops
/// them (nobody watches a round live).
pub fn collect_notification(state: &mut Collected, update: SessionUpdate) -> Vec<TurnActivity> {
    let mut events = Vec::new();
    let mut stream = |event: TurnActivity| events.push(event);
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => match chunk.content {
            ContentBlock::Text(text) => {
                state.reply.push_str(&text.text);
                stream(TurnActivity::Chunk { text: text.text });
            }
            other => {
                // Never dropped (operator 2026-08-02): until content v2
                // carries the payload, the FACT goes on record and wire.
                let info = content_info(&other);
                state.contents.push(info.clone());
                stream(TurnActivity::Content {
                    kind: info.kind,
                    label: info.label,
                });
            }
        },
        SessionUpdate::AgentThoughtChunk(chunk) => match chunk.content {
            ContentBlock::Text(text) => {
                state.thoughts.push_str(&text.text);
                stream(TurnActivity::Thought { text: text.text });
            }
            other => {
                let info = content_info(&other);
                state.contents.push(info.clone());
                stream(TurnActivity::Content {
                    kind: info.kind,
                    label: info.label,
                });
            }
        },
        SessionUpdate::ToolCall(call) => {
            let row_id = call.tool_call_id.0.to_string();
            let status = format!("{:?}", call.status);
            match state.tool_rows.get(&row_id).copied() {
                Some(at) => {
                    state.tools[at].title = call.title.clone();
                    state.tools[at].status = status.clone();
                }
                None => {
                    let at = state.tools.len();
                    state.tool_rows.insert(row_id.clone(), at);
                    state.tools.push(crate::activity::ToolStep::new(
                        call.title.clone(),
                        status.clone(),
                    ));
                }
            }
            // Index the command by id so a following permission request
            // can recover what is about to run.
            state.tool_calls.insert(
                row_id.clone(),
                KnownCall {
                    title: Some(call.title.clone()),
                    raw_input: call.raw_input.clone(),
                    kind: wire_kind(call.kind),
                },
            );
            stream(TurnActivity::Tool {
                id: row_id,
                title: call.title,
                status: status.to_lowercase(),
            });
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let row_id = update.tool_call_id.0.to_string();
            if let Some(at) = state.tool_rows.get(&row_id).copied() {
                if let Some(status) = update.fields.status {
                    state.tools[at].status = format!("{status:?}");
                }
                if let Some(title) = update.fields.title.clone() {
                    state.tools[at].title = title;
                }
            }
            // Merge any newly revealed detail by id.
            let entry = state.tool_calls.entry(row_id.clone()).or_default();
            if update.fields.title.is_some() {
                entry.title = update.fields.title.clone();
            }
            if update.fields.raw_input.is_some() {
                entry.raw_input = update.fields.raw_input.clone();
            }
            if let Some(kind) = update.fields.kind.and_then(wire_kind) {
                entry.kind = Some(kind);
            }
            stream(TurnActivity::Tool {
                title: update
                    .fields
                    .title
                    .clone()
                    .unwrap_or_else(|| row_id.clone()),
                id: row_id,
                status: update
                    .fields
                    .status
                    .map(|s| format!("{s:?}").to_lowercase())
                    .unwrap_or_else(|| "update".into()),
            });
        }
        // The session's cumulative spend (JP-0089-18): keep the newest.
        // Cost is optional per adapter, tokens are not, so an update that
        // omits the cost must not erase the last one.
        SessionUpdate::UsageUpdate(usage) => {
            if usage.cost.is_some() {
                state.cost = usage.cost;
            }
            state.used = usage.used;
        }
        _ => {}
    }
    events
}

// ---------------------------------------------------------------------------
// Permission answers: the shared policy, spoken in ACP.
// ---------------------------------------------------------------------------

/// The machine's answer to one permission request.
pub struct PermissionAnswer {
    /// The option to select; None answers Cancelled.
    pub selected: Option<PermissionOptionId>,
    pub title: String,
    /// "allowed" | "allowed (joy)" | "denied" | "escalated to operator" —
    /// the vocabulary the activity block renders.
    pub answered: &'static str,
    /// Set when a HUMAN must decide (job rounds: a request without an
    /// allow option is a gate escalation).
    pub question: Option<String>,
}

fn pick_option(
    request: &RequestPermissionRequest,
    wanted: [PermissionOptionKind; 2],
) -> Option<PermissionOptionId> {
    request
        .options
        .iter()
        .find(|o| wanted.contains(&o.kind))
        .map(|o| o.option_id.clone())
}

/// A CHAT turn's permission: THE shared policy (JI-0179-4F step 2,
/// `joy_chat::model::permission`) under the turn's mode. joy is ALWAYS
/// allowed, even in plan mode (operator 2026-07-21): it is the agents'
/// governed item interface and enforces its own capability/mode rules.
/// The request may not carry the command, so facts are recovered from the
/// correlated notification (`known`). A Deny rejects — no human is
/// attached mid-turn, on either host.
pub fn answer_chat_permission(
    mode: joy_chat::model::AgentMode,
    request: &RequestPermissionRequest,
    known: &KnownCall,
) -> PermissionAnswer {
    use joy_chat::model::permission::{self, Decision, ToolAction};
    // The KIND decides read vs. edit vs. mutating, and an absent kind
    // means "mutating" — so falling back to the notification is not a
    // nicety: without it every vibe tool call, a plain file read
    // included, is refused at the proposing level (JAPP-0152-81).
    let wire = wire_kind(request.tool_call.fields.kind).or_else(|| known.kind.clone());
    let (cmd_title, cmd_raw) = {
        let req_title = request.tool_call.fields.title.clone();
        let req_raw = request.tool_call.fields.raw_input.clone();
        if req_title.is_some() || req_raw.is_some() {
            (req_title, req_raw)
        } else {
            (known.title.clone(), known.raw_input.clone())
        }
    };
    let title = cmd_title.clone().unwrap_or_else(|| "tool call".into());
    let is_joy = permission::command_invokes_joy(cmd_title.as_deref(), cmd_raw.as_ref());
    let allow =
        permission::permission_decision(mode, ToolAction::from_wire(wire.as_deref()), is_joy)
            == Decision::Allow;
    let answered = if !allow {
        "denied"
    } else if is_joy {
        "allowed (joy)"
    } else {
        "allowed"
    };
    let selected = if allow {
        pick_option(
            request,
            [
                PermissionOptionKind::AllowOnce,
                PermissionOptionKind::AllowAlways,
            ],
        )
    } else {
        pick_option(
            request,
            [
                PermissionOptionKind::RejectOnce,
                PermissionOptionKind::RejectAlways,
            ],
        )
    }
    // never Cancelled just because the agent worded its options oddly
    .or_else(|| request.options.first().map(|o| o.option_id.clone()));
    PermissionAnswer {
        selected,
        title,
        answered,
        question: None,
    }
}

/// A JOB round's permission: the human approved the job, so whatever
/// offers an allow option is allowed (the joy CLI's guard bounds item
/// writes inside the sandbox). A request WITHOUT an allow option is a
/// gate escalation — a human must decide; the round ends cleanly and the
/// question rides back to the operator.
pub fn answer_job_permission(request: &RequestPermissionRequest) -> PermissionAnswer {
    let title = request
        .tool_call
        .fields
        .title
        .clone()
        .unwrap_or_else(|| "tool call".into());
    match pick_option(
        request,
        [
            PermissionOptionKind::AllowOnce,
            PermissionOptionKind::AllowAlways,
        ],
    ) {
        Some(option) => PermissionAnswer {
            selected: Some(option),
            title,
            answered: "allowed",
            question: None,
        },
        None => PermissionAnswer {
            selected: None,
            title: title.clone(),
            answered: "escalated to operator",
            question: Some(title),
        },
    }
}

// ---------------------------------------------------------------------------
// The lane machine.
// ---------------------------------------------------------------------------

/// How a lane's agent process comes to life. Everything here is DATA the
/// host supplies; the machine never guesses.
pub struct LaneConfig {
    /// The full command line (the registry entrypoint, locally or behind
    /// `docker exec`), parsed by `AcpAgent::from_str`.
    pub command: String,
    /// Working directory for new sessions (the repo checkout).
    pub cwd: PathBuf,
    /// ACP client_info. REQUIRED in practice: vibe-acp forwards it to the
    /// Mistral API as request metadata, and Mistral rejects empty values.
    pub client_name: String,
    pub client_version: String,
    /// Prepended once to a fresh session's first prompt (the desktop
    /// leads with its in-app posture; the platform's agents read theirs
    /// from the repo).
    pub fresh_preamble: Option<String>,
    /// The pinned model, set on every session this lane creates via
    /// `session/set_config_option` (config id `model`). The env default
    /// alone is not enough: an agent with persisted per-chat state keeps
    /// its previously active model across spawns, so only the explicit
    /// session-level set makes a changed pin actually run (JP-00B8-4B).
    /// None = the agent's own default.
    pub model: Option<String>,
    /// Host hook over the parsed agent before it connects — the desktop
    /// injects the delegated JOY_SESSION, the git identity and the tool's
    /// state directory into the spawn descriptor.
    pub prepare: Option<Arc<dyn Fn(AcpAgent) -> AcpAgent + Send + Sync>>,
}

/// One turn requested from a lane.
pub struct TurnRequest {
    pub chat_id: String,
    pub prompt_full: String,
    pub prompt_delta: Option<String>,
    pub mode: joy_chat::model::AgentMode,
    /// Per-message spend cap in cents (JI-014A); 0 disables it. The lane
    /// watches the turn's cumulative ACP cost and cancels the session once
    /// this turn's delta crosses the cap.
    pub max_price_cents: u64,
    /// Live activity out (JI-0172-EE) while the turn runs: the shared
    /// delivery contract (ordered, drained before the result returns).
    pub activity: Option<Arc<dyn TurnSink>>,
    /// The waiting-marker id (JAPP-0129-A7): delivered as the turn's
    /// first wire event when a sink listens.
    pub marker_id: Option<String>,
}

struct QueuedTurn {
    request: TurnRequest,
    respond: oneshot::Sender<anyhow::Result<TurnOutcome>>,
}

struct Lane {
    /// Spawn-env identity (key, model, adapter): a change respawns the
    /// lane so turns never run with a stale secret.
    fingerprint: u64,
    tx: mpsc::UnboundedSender<QueuedTurn>,
}

/// The per-process lane registry, keyed however the host scopes lanes
/// (the platform: project+adapter; the desktop: root+entrypoint). Lanes
/// die with their child process; entries clean up lazily on the next
/// turn.
pub struct LaneSet<K: std::hash::Hash + Eq + Clone> {
    lanes: Mutex<HashMap<K, Lane>>,
}

impl<K: std::hash::Hash + Eq + Clone> Default for LaneSet<K> {
    fn default() -> Self {
        Self {
            lanes: Mutex::new(HashMap::new()),
        }
    }
}

impl<K: std::hash::Hash + Eq + Clone> LaneSet<K> {
    /// Run one chat turn through the keyed lane, spawning or respawning
    /// it as needed. One respawn attempt: a dead lane (stopped container,
    /// exited process) is normal, the second failure is a real error.
    pub async fn turn(
        &self,
        key: K,
        fingerprint: u64,
        config: &LaneConfig,
        request: TurnRequest,
        timeout: std::time::Duration,
    ) -> anyhow::Result<TurnOutcome> {
        for attempt in 0..2 {
            let tx = self.lane(key.clone(), fingerprint, config, attempt > 0);
            let (respond, rx) = oneshot::channel();
            let queued = QueuedTurn {
                request: TurnRequest {
                    chat_id: request.chat_id.clone(),
                    prompt_full: request.prompt_full.clone(),
                    prompt_delta: request.prompt_delta.clone(),
                    mode: request.mode,
                    max_price_cents: request.max_price_cents,
                    activity: request.activity.clone(),
                    marker_id: request.marker_id.clone(),
                },
                respond,
            };
            if tx.send(queued).is_err() {
                // lane closed between lookup and send: retry respawns
                continue;
            }
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(result)) => return result,
                Ok(Err(_recv_dropped)) => {
                    // the lane died mid-turn: drop it and retry once
                    self.drop_lane(&key);
                    continue;
                }
                Err(_) => {
                    self.drop_lane(&key);
                    anyhow::bail!("acp chat turn timed out after {}s", timeout.as_secs());
                }
            }
        }
        anyhow::bail!("acp chat lane could not be established")
    }

    /// Forget a lane (project close, undelegate, config change): the
    /// child ends when its channel closes.
    pub fn drop_lane(&self, key: &K) {
        self.lanes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    /// Forget every lane whose key fails the predicate (the desktop drops
    /// a whole project's lanes when the person leaves it).
    pub fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        self.lanes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|key, _| keep(key));
    }

    /// The live lane sender, (re)spawning when absent, stale-keyed,
    /// closed, or when the caller forces it after a failed attempt.
    fn lane(
        &self,
        key: K,
        fingerprint: u64,
        config: &LaneConfig,
        force_respawn: bool,
    ) -> mpsc::UnboundedSender<QueuedTurn> {
        let mut lanes = self.lanes.lock().unwrap_or_else(|e| e.into_inner());
        if !force_respawn {
            if let Some(lane) = lanes.get(&key) {
                if lane.fingerprint == fingerprint && !lane.tx.is_closed() {
                    return lane.tx.clone();
                }
            }
        }
        let (tx, rx) = mpsc::unbounded_channel();
        spawn_lane_thread(
            config.command.clone(),
            config.cwd.clone(),
            config.client_name.clone(),
            config.client_version.clone(),
            config.fresh_preamble.clone(),
            config.model.clone(),
            config.prepare.clone(),
            rx,
        );
        lanes.insert(
            key,
            Lane {
                fingerprint,
                tx: tx.clone(),
            },
        );
        tx
    }
}

/// The lane lives on its own thread with its own single-threaded runtime,
/// so the machine works identically under the platform's tokio runtime
/// and the desktop's Tauri process — no executor assumption leaks out.
#[allow(clippy::too_many_arguments)]
fn spawn_lane_thread(
    command: String,
    cwd: PathBuf,
    client_name: String,
    client_version: String,
    fresh_preamble: Option<String>,
    model: Option<String>,
    prepare: Option<Arc<dyn Fn(AcpAgent) -> AcpAgent + Send + Sync>>,
    rx: mpsc::UnboundedReceiver<QueuedTurn>,
) {
    std::thread::Builder::new()
        .name("acp-lane".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return,
            };
            runtime.block_on(run_lane(
                command,
                cwd,
                client_name,
                client_version,
                fresh_preamble,
                model,
                prepare,
                rx,
            ));
        })
        .ok();
}

/// The lane body: one ACP connection, sessions per chat, turns in
/// arrival order. Ends when the turn channel closes or the transport
/// dies; pending turns learn it through their dropped responders.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn run_lane(
    command: String,
    cwd: PathBuf,
    client_name: String,
    client_version: String,
    fresh_preamble: Option<String>,
    model: Option<String>,
    prepare: Option<Arc<dyn Fn(AcpAgent) -> AcpAgent + Send + Sync>>,
    mut rx: mpsc::UnboundedReceiver<QueuedTurn>,
) {
    let agent = match AcpAgent::from_str(&command) {
        Ok(agent) => agent,
        Err(_) => return,
    };
    let agent = match prepare {
        Some(hook) => hook(agent),
        None => agent,
    };
    // per-SESSION turn buffers and modes: notifications and permission
    // requests carry the session id, so concurrent chats never mix
    let buffers: Arc<Mutex<HashMap<String, Collected>>> = Arc::default();
    let modes: Arc<Mutex<HashMap<String, joy_chat::model::AgentMode>>> = Arc::default();
    // per-SESSION live-activity wires (JI-0172-EE): the running turn
    // listens on the receiving end and owns delivery order + drain
    // (JOY-0249-D2); the notification handler only queues.
    let live: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TurnActivity>>>> = Arc::default();
    let notify_live = live.clone();
    let notes = buffers.clone();
    let perms = buffers.clone();
    let perm_modes = modes.clone();

    let _ = agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                let sid = notification.session_id.0.to_string();
                // the live wire of the RUNNING turn, if one listens
                let wire = notify_live
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&sid)
                    .cloned();
                let events = {
                    let mut map = notes.lock().unwrap_or_else(|e| e.into_inner());
                    let state = map.entry(sid).or_default();
                    collect_notification(state, notification.update)
                };
                if let Some(wire) = wire {
                    for event in events {
                        // queue only: the turn loop delivers in order and
                        // drains before it responds (JOY-0249-D2)
                        let _ = wire.send(event);
                    }
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                // THE shared permission policy: the mode was set per
                // SESSION before the prompt; the host's sandbox (container
                // mounts, reach) is the hard boundary, this is the belt.
                let sid = request.session_id.0.to_string();
                let mode = perm_modes
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&sid)
                    .copied()
                    .unwrap_or(joy_chat::model::AgentMode::Plan);
                let known: KnownCall = perms
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&sid)
                    .map(|c| c.known(request.tool_call.tool_call_id.0.as_ref()))
                    .unwrap_or_default();
                let answer = answer_chat_permission(mode, &request, &known);
                {
                    let mut map = perms.lock().unwrap_or_else(|e| e.into_inner());
                    map.entry(sid).or_default().record_permission(
                        request.tool_call.tool_call_id.0.as_ref(),
                        answer.title.clone(),
                        answer.answered,
                    );
                }
                match answer.selected {
                    Some(option_id) => responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                            option_id,
                        )),
                    )),
                    None => responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Cancelled,
                    )),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, |connection: ConnectionTo<Agent>| async move {
            connection
                .send_request(
                    InitializeRequest::new(ProtocolVersion::V1)
                        .client_info(Implementation::new(client_name, client_version)),
                )
                .block_task()
                .await?;
            // chat id -> live agent session
            let mut sessions: HashMap<String, agent_client_protocol::schema::v1::SessionId> =
                HashMap::new();
            // session id -> last-seen CUMULATIVE cost/tokens, so a turn's
            // numbers are the delta (a fresh session starts at 0). JP-0089-18.
            let mut session_cost: HashMap<String, f64> = HashMap::new();
            let mut session_tokens: HashMap<String, u64> = HashMap::new();
            while let Some(QueuedTurn {
                request: turn,
                respond,
            }) = rx.recv().await
            {
                // The waiting marker opens the skeleton BEFORE the slow
                // parts (session spawn, model pin), as the first event on
                // the same ordered wire the chunks ride (JOY-0249-D2).
                if let (Some(sink), Some(marker)) = (&turn.activity, &turn.marker_id) {
                    sink.deliver(WireActivity::pending(marker)).await;
                }
                let (session_id, prompt) = match sessions.get(&turn.chat_id) {
                    Some(sid) => (
                        sid.clone(),
                        // a live session: only the delta; a member who
                        // never spoke replays the full transcript
                        turn.prompt_delta
                            .clone()
                            .unwrap_or_else(|| turn.prompt_full.clone()),
                    ),
                    None => {
                        let created = connection
                            .send_request(NewSessionRequest::new(cwd.clone()))
                            .block_task()
                            .await?;
                        // The pinned model is SESSION config, set before the
                        // first prompt: the spawn env only suggests a default,
                        // and an agent with persisted chat state ignores it
                        // (JP-00B8-4B). A pin the agent refuses is an error
                        // the person must see, never a silently different
                        // model.
                        if let Some(model) = &model {
                            connection
                                .send_request(SetSessionConfigOptionRequest::new(
                                    created.session_id.clone(),
                                    "model",
                                    model.as_str(),
                                ))
                                .block_task()
                                .await?;
                        }
                        sessions.insert(turn.chat_id.clone(), created.session_id.clone());
                        // fresh session: the chat history IS the resume,
                        // led once by the host's preamble if it has one
                        let first = match &fresh_preamble {
                            Some(preamble) => format!("{preamble}\n\n{}", turn.prompt_full),
                            None => turn.prompt_full.clone(),
                        };
                        (created.session_id, first)
                    }
                };
                let sid_key = session_id.0.to_string();
                modes
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(sid_key.clone(), turn.mode);
                // The live wire opens with the turn: the notification
                // handler queues into this turn's channel; THIS loop is
                // the only deliverer, in order, and drains before it
                // responds (JOY-0249-D2).
                let (act_tx, mut act_rx) = mpsc::unbounded_channel::<TurnActivity>();
                {
                    let mut wires = live.lock().unwrap_or_else(|e| e.into_inner());
                    if turn.activity.is_some() {
                        wires.insert(sid_key.clone(), act_tx.clone());
                    } else {
                        wires.remove(&sid_key);
                    }
                }
                // the map holds the one live sender; removing it later
                // closes the wire and ends the drain
                drop(act_tx);
                let mut live_open = turn.activity.is_some();
                buffers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(sid_key.clone(), Collected::default());
                // Per-message spend cap (JI-014A): drive the prompt while
                // watching this turn's spend (cumulative UsageUpdate minus
                // the session's prior total). At `> max_price_cents` send
                // `session/cancel`; the agent stops the turn, so the shared
                // lane stays healthy for other chats.
                let cap_cents = turn.max_price_cents;
                let prev_cum = session_cost.get(&sid_key).copied().unwrap_or(0.0);
                let prompt_fut = connection
                    .send_request(PromptRequest::new(
                        session_id.clone(),
                        vec![ContentBlock::Text(TextContent::new(prompt))],
                    ))
                    .block_task();
                tokio::pin!(prompt_fut);
                let mut capped = false;
                let prompted = loop {
                    tokio::select! {
                        r = &mut prompt_fut => break r,
                        queued = act_rx.recv(), if live_open => {
                            match queued {
                                Some(event) => {
                                    if let Some(sink) = &turn.activity {
                                        sink.deliver(WireActivity::of(&event)).await;
                                    }
                                }
                                None => live_open = false,
                            }
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(250)),
                            if cap_cents > 0 && !capped =>
                        {
                            let cur = buffers
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .get(&sid_key)
                                .and_then(|c| c.cost.as_ref().map(|x| x.amount))
                                .unwrap_or(prev_cum);
                            let turn_cents = ((cur - prev_cum).max(0.0) * 100.0).round() as u64;
                            if turn_cents > cap_cents {
                                capped = true;
                                // best effort: a failed send just means we
                                // keep awaiting; the timeout bounds the turn
                                let _ = connection.send_notification(
                                    CancelNotification::new(session_id.clone()),
                                );
                            }
                        }
                    }
                };
                // A capped turn is a DELIBERATE stop, not a lane failure:
                // the agent may answer the cancel with Ok or a JSON-RPC
                // error. Both drain what streamed and keep the shared lane;
                // only a non-capped error poisons the connection.
                // The wire closes with the turn: removing the sender ends
                // the queue, then DRAIN what already streamed — every
                // event of this turn is delivered before its result
                // returns, so nothing can overtake the reply (JOY-0249-D2).
                live.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&sid_key);
                if let Some(sink) = &turn.activity {
                    while let Some(event) = act_rx.recv().await {
                        sink.deliver(WireActivity::of(&event)).await;
                    }
                }
                let stop_ok = prompted.is_ok() || capped;
                // Did the cap actually TRUNCATE this turn? The 250ms poll
                // may cancel just as the agent finishes; a whole reply must
                // not get the "stopped" notice. Only a Cancelled stop (or a
                // cancel answered with an error) was really cut short.
                let truncated = match &prompted {
                    Ok(resp) => resp.stop_reason == StopReason::Cancelled,
                    Err(_) => capped,
                };
                let result = if stop_ok {
                    let state = buffers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&sid_key)
                        .unwrap_or_default();
                    // per-turn cost = this turn's cumulative minus the
                    // last-seen for this session (JP-0089-18). The amount is
                    // treated as ~cents: a spend ceiling needs no FX
                    // precision, and over-counting stops marginally early,
                    // the safe direction.
                    let cost_cents = state.cost.as_ref().map(|c| {
                        let prev = session_cost.get(&sid_key).copied().unwrap_or(0.0);
                        session_cost.insert(sid_key.clone(), c.amount);
                        ((c.amount - prev).max(0.0) * 100.0).round() as u64
                    });
                    let tokens = {
                        let prev = session_tokens.get(&sid_key).copied().unwrap_or(0);
                        session_tokens.insert(sid_key.clone(), state.used);
                        state.used.saturating_sub(prev)
                    };
                    let mut output = state.into_outcome();
                    output.cost_cents = cost_cents;
                    output.tokens = (tokens > 0).then_some(tokens);
                    output.capped = truncated;
                    Ok(output)
                } else {
                    Err(anyhow::anyhow!(
                        "acp prompt: {}",
                        prompted.err().map(|e| e.to_string()).unwrap_or_default()
                    ))
                };
                let failed = result.is_err();
                let _ = respond.send(result);
                if failed {
                    // a failed prompt poisons the connection state: end
                    // the lane; the next turn respawns and replays
                    break;
                }
            }
            Ok(())
        })
        .await;
}

// ---------------------------------------------------------------------------
// One-shot sessions: job rounds and the model roster.
// ---------------------------------------------------------------------------

/// What one job round produced.
pub struct RoundOutcome {
    pub outcome: TurnOutcome,
    /// A permission the policy may NOT grant (no allow option offered —
    /// the agent escalated a gate): routed to the OPERATOR instead of
    /// silently denied.
    pub question: Option<String>,
}

/// Run ONE round as a fresh ACP session (a job's `--rm` container, or any
/// other single-prompt use). The cumulative UsageUpdate IS the round
/// total, no delta bookkeeping needed.
pub async fn single_round(
    config: &LaneConfig,
    prompt: &str,
    timeout: std::time::Duration,
) -> anyhow::Result<RoundOutcome> {
    match tokio::time::timeout(timeout, single_round_inner(config, prompt)).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!("acp round timed out after {}s", timeout.as_secs()),
    }
}

async fn single_round_inner(config: &LaneConfig, prompt: &str) -> anyhow::Result<RoundOutcome> {
    let agent =
        AcpAgent::from_str(&config.command).map_err(|e| anyhow::anyhow!("acp agent spawn: {e}"))?;
    let agent = match &config.prepare {
        Some(hook) => hook(agent),
        None => agent,
    };
    let collected = Arc::new(Mutex::new(Collected::default()));
    let question = Arc::new(Mutex::new(None::<String>));
    let notes = collected.clone();
    let perms = collected.clone();
    let escalated = question.clone();
    let cwd = config.cwd.clone();
    let prompt = prompt.to_string();
    let pinned_model = config.model.clone();
    let client = Implementation::new(config.client_name.clone(), config.client_version.clone());

    agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                let mut state = notes.lock().unwrap_or_else(|e| e.into_inner());
                // nobody watches a round live: the events are dropped
                let _ = collect_notification(&mut state, notification.update);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                let answer = answer_job_permission(&request);
                {
                    let mut state = perms.lock().unwrap_or_else(|e| e.into_inner());
                    state.record_permission(
                        request.tool_call.tool_call_id.0.as_ref(),
                        answer.title.clone(),
                        answer.answered,
                    );
                }
                if let Some(q) = answer.question {
                    *escalated.lock().unwrap_or_else(|e| e.into_inner()) = Some(q);
                }
                match answer.selected {
                    Some(option) => responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option)),
                    )),
                    None => responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Cancelled,
                    )),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, |connection: ConnectionTo<Agent>| async move {
            connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1).client_info(client))
                .block_task()
                .await?;
            let session = connection
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await?;
            // same rule as the chat lane: the pin is session config
            // (JP-00B8-4B), and a refused pin is a loud error
            if let Some(model) = &pinned_model {
                connection
                    .send_request(SetSessionConfigOptionRequest::new(
                        session.session_id.clone(),
                        "model",
                        model.as_str(),
                    ))
                    .block_task()
                    .await?;
            }
            connection
                .send_request(PromptRequest::new(
                    session.session_id.clone(),
                    vec![ContentBlock::Text(TextContent::new(prompt))],
                ))
                .block_task()
                .await?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("acp round: {e}"))?;

    let state = std::mem::take(&mut *collected.lock().unwrap_or_else(|e| e.into_inner()));
    let question = question.lock().unwrap_or_else(|e| e.into_inner()).take();
    // one fresh session: the cumulative usage IS the round total
    let cost_cents = state
        .cost
        .as_ref()
        .map(|c| (c.amount.max(0.0) * 100.0).round() as u64);
    let tokens = state.used;
    let mut outcome = state.into_outcome();
    outcome.cost_cents = cost_cents;
    outcome.tokens = (tokens > 0).then_some(tokens);
    Ok(RoundOutcome { outcome, question })
}

/// The models an agent advertises, as (value, label) pairs (JI-0162).
#[derive(Clone, Debug, Default)]
pub struct AgentModels {
    pub current: String,
    pub options: Vec<(String, String)>,
    /// The agent's self-reported version from the ACP initialize
    /// response (settings info line); empty = the tool sent none.
    pub agent_version: String,
}

/// List an agent's selectable models by opening a short ACP session and
/// reading the `model` select from the session's config_options. No
/// prompt, no cost. Empty options mean the agent advertises none.
pub async fn list_models(
    config: &LaneConfig,
    timeout: std::time::Duration,
) -> anyhow::Result<AgentModels> {
    match tokio::time::timeout(timeout, list_models_inner(config)).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!("acp model list timed out after {}s", timeout.as_secs()),
    }
}

async fn list_models_inner(config: &LaneConfig) -> anyhow::Result<AgentModels> {
    let agent =
        AcpAgent::from_str(&config.command).map_err(|e| anyhow::anyhow!("acp agent spawn: {e}"))?;
    let agent = match &config.prepare {
        Some(hook) => hook(agent),
        None => agent,
    };
    let out: Arc<Mutex<AgentModels>> = Arc::default();
    let cap = out.clone();
    let cwd = config.cwd.clone();
    let client = Implementation::new(config.client_name.clone(), config.client_version.clone());
    agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |_n: SessionNotification, _cx| Ok(()),
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |_r: RequestPermissionRequest, responder, _c| {
                responder.respond(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Cancelled,
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, |connection: ConnectionTo<Agent>| async move {
            let init = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1).client_info(client))
                .block_task()
                .await?;
            if let Some(info) = init.agent_info {
                cap.lock().unwrap_or_else(|e| e.into_inner()).agent_version = info.version;
            }
            let session = connection
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await?;
            if let Some(opts) = session.config_options.as_ref() {
                for opt in opts.iter().filter(|o| o.id.0.as_ref() == "model") {
                    if let SessionConfigKind::Select(sel) = &opt.kind {
                        let mut m = cap.lock().unwrap_or_else(|e| e.into_inner());
                        m.current = sel.current_value.0.to_string();
                        match &sel.options {
                            SessionConfigSelectOptions::Ungrouped(v) => {
                                for o in v {
                                    m.options.push((o.value.0.to_string(), o.name.clone()));
                                }
                            }
                            SessionConfigSelectOptions::Grouped(groups) => {
                                for g in groups {
                                    for o in &g.options {
                                        m.options.push((o.value.0.to_string(), o.name.clone()));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("acp list models: {e}"))?;
    let result = out.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One notification, built from the wire shape an agent really sends.
    /// Going through JSON keeps the test honest: the schema structs are
    /// non-exhaustive, and a hand-built value could not carry the nulls
    /// that caused these bugs.
    fn update(value: serde_json::Value) -> SessionUpdate {
        serde_json::from_value(value).expect("a valid ACP session update")
    }

    #[test]
    fn a_non_text_block_leaves_a_content_row_instead_of_vanishing() {
        // Operator 2026-08-02 (JOY-024B-AC): dropping is unacceptable.
        // Until content v2 carries the payload, the FACT goes on record
        // (details JSON) and on the live wire as a turn-content event.
        let mut state = Collected::default();
        let events = collect_notification(
            &mut state,
            update(serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {
                    "type": "image",
                    "mimeType": "image/png",
                    // 8 base64 chars ≈ 6 decoded bytes
                    "data": "aGFsbG8h",
                },
            })),
        );
        assert_eq!(state.contents.len(), 1);
        assert_eq!(state.contents[0].kind, "image");
        assert_eq!(state.contents[0].label, "image/png · 6 B");
        match &events[..] {
            [TurnActivity::Content { kind, label }] => {
                assert_eq!(kind, "image");
                assert_eq!(label, "image/png · 6 B");
            }
            other => panic!("expected one content event, got {other:?}"),
        }
        // the reply stays untouched; the record carries the block
        assert!(state.reply.is_empty());
        let details = state.into_outcome().details.expect("a details block");
        assert!(details.contains("\"contents\""), "{details}");
        assert!(details.contains("image/png · 6 B"), "{details}");
    }

    #[test]
    fn a_resource_link_names_itself_in_the_content_row() {
        let mut state = Collected::default();
        collect_notification(
            &mut state,
            update(serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {
                    "type": "resource_link",
                    "name": "diagram.svg",
                    "uri": "file:///tmp/diagram.svg",
                    "mimeType": "image/svg+xml",
                    "size": 4200,
                },
            })),
        );
        assert_eq!(state.contents.len(), 1);
        assert_eq!(state.contents[0].kind, "link");
        assert_eq!(
            state.contents[0].label,
            "diagram.svg · file:///tmp/diagram.svg · image/svg+xml · 4 kB"
        );
    }

    #[test]
    fn one_call_stays_one_row_however_often_it_is_announced() {
        // vibe announces a bare "bash" and names the command in a SECOND
        // notification under the same id (JP-00AD-A9).
        let mut state = Collected::default();
        for title in ["bash", "bash: joy ls"] {
            collect_notification(
                &mut state,
                update(serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "t1",
                    "title": title,
                    "kind": "execute",
                    "status": "pending",
                })),
            );
        }
        assert_eq!(state.tools.len(), 1);
        assert_eq!(state.tools[0].title, "bash: joy ls");
    }

    #[test]
    fn a_permission_answer_rides_on_its_call_and_orphans_keep_their_row() {
        let mut state = Collected::default();
        collect_notification(
            &mut state,
            update(serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "t1",
                "title": "bash: joy ls",
                "kind": "execute",
                "status": "pending",
            })),
        );
        state.record_permission("t1", "bash: joy ls".into(), "allowed (joy)");
        assert_eq!(state.tools[0].answered.as_deref(), Some("allowed (joy)"));
        assert!(state.permissions.is_empty());
        // an answer with no call to sit on keeps its own row
        state.record_permission("never-announced", "mystery".into(), "denied");
        assert_eq!(state.permissions, vec![("mystery".into(), "denied".into())]);
    }

    #[test]
    fn the_usage_update_keeps_the_newest_cost_and_never_erases_it() {
        let mut state = Collected::default();
        collect_notification(
            &mut state,
            update(serde_json::json!({
                "sessionUpdate": "usage_update",
                "used": 100,
                "size": 128000,
                "cost": { "amount": 0.02, "currency": "USD" },
            })),
        );
        // a later update WITHOUT cost keeps the last-known amount
        collect_notification(
            &mut state,
            update(serde_json::json!({
                "sessionUpdate": "usage_update",
                "used": 250,
                "size": 128000,
            })),
        );
        assert_eq!(state.used, 250);
        assert_eq!(state.cost.as_ref().map(|c| c.amount), Some(0.02));
    }

    fn permission_request(value: serde_json::Value) -> RequestPermissionRequest {
        serde_json::from_value(value).expect("a valid permission request")
    }

    #[test]
    fn a_kindless_permission_is_judged_on_the_announced_call() {
        // vibe's request carries only the id (JAPP-0152-81); the verdict
        // must come from the correlated notification's facts.
        let request = permission_request(serde_json::json!({
            "sessionId": "s1",
            "toolCall": { "toolCallId": "t1" },
            "options": [
                { "optionId": "y", "name": "allow", "kind": "allow_once" },
                { "optionId": "n", "name": "reject", "kind": "reject_once" },
            ],
        }));
        let known = KnownCall {
            title: Some("read file".into()),
            raw_input: None,
            kind: Some("read".into()),
        };
        // a plain READ is allowed even at the proposing level
        let answer = answer_chat_permission(joy_chat::model::AgentMode::Plan, &request, &known);
        assert_eq!(answer.answered, "allowed");
        assert_eq!(answer.selected.as_ref().map(|o| o.0.as_ref()), Some("y"));
        // without the recovered kind the same request is refused
        let blind = answer_chat_permission(
            joy_chat::model::AgentMode::Plan,
            &request,
            &KnownCall::default(),
        );
        assert_eq!(blind.answered, "denied");
        assert_eq!(blind.selected.as_ref().map(|o| o.0.as_ref()), Some("n"));
    }

    #[test]
    fn a_job_gate_without_an_allow_option_escalates_to_the_operator() {
        let request = permission_request(serde_json::json!({
            "sessionId": "s1",
            "toolCall": { "toolCallId": "t1", "title": "push to production" },
            "options": [
                { "optionId": "n", "name": "reject", "kind": "reject_once" },
            ],
        }));
        let answer = answer_job_permission(&request);
        assert_eq!(answer.answered, "escalated to operator");
        assert!(answer.selected.is_none());
        assert_eq!(answer.question.as_deref(), Some("push to production"));
        // with an allow option the approved job just runs
        let allowed = answer_job_permission(&permission_request(serde_json::json!({
            "sessionId": "s1",
            "toolCall": { "toolCallId": "t2", "title": "bash: cargo test" },
            "options": [
                { "optionId": "y", "name": "allow", "kind": "allow_once" },
            ],
        })));
        assert_eq!(allowed.answered, "allowed");
        assert!(allowed.question.is_none());
    }
}
