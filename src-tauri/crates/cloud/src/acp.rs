//! Agent Client Protocol (ACP) client: any ACP agent as a chat engine.
//!
//! One agent process and one ACP session are kept for the lifetime of the
//! app (per agent command). A turn is a `session/prompt` on that session, so
//! the agent keeps its own context instead of being respawned and fed a
//! replayed transcript every message. Changing `acp_agent_command` replaces
//! the process. Dropping a reply stream cancels only that turn, not the
//! session.
//!
//! Komorebi advertises no file-system or terminal capability and rejects
//! every permission request, so the agent cannot change anything through
//! Komorebi. What the agent runs without asking is up to its own config,
//! except for `claude-agent-acp`, whose tools and settings are switched off
//! (see [`Turn::session_new`]).
//!
//! The session is opened in a dedicated, empty workspace directory: `cwd`
//! selects the project configuration an agent discovers, and it is the blast
//! radius of anything the agent writes on its own. `KOMOREBI_AGENT_WORKSPACE`
//! overrides it for experiments. OpenCode reads an `opencode.json` placed in
//! that directory, which is how a model can be pinned without a code change.

use crate::{AcpProgress, CloudError, StreamEvent, TokenStream};
use komorebi_router::{ChatMessage, Role};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, Mutex};

const PROTOCOL_VERSION: u64 = 1;
const ID_INITIALIZE: u64 = 1;
const ID_SESSION_NEW: u64 = 2;

type Stdin = ChildStdin;
type StdoutLines = Lines<BufReader<ChildStdout>>;
type Sink = mpsc::UnboundedSender<Result<StreamEvent, CloudError>>;

/// Dev-readable record of the exact text sent in one `session/prompt`.
#[derive(Debug, Clone)]
pub struct AcpTurnRecord {
    pub session_prompt_text: String,
    pub system_roles: Vec<Role>,
    pub system_message_count: usize,
}

impl AcpTurnRecord {
    /// Build the dev record from the same messages and persona flag the wire
    /// renderer uses. Roles describe the leading system run, which includes
    /// the persona even on a later turn whose prompt omits its text.
    pub fn from_messages(messages: &[ChatMessage], include_persona: bool) -> Self {
        let lead = messages
            .iter()
            .take_while(|m| m.role == Role::System)
            .count();
        Self {
            session_prompt_text: turn_text(messages, include_persona),
            system_roles: messages.iter().take(lead).map(|m| m.role).collect(),
            system_message_count: lead,
        }
    }
}

/// The one `session/prompt` builder. `send_prompt` sends exactly the string the
/// dev record holds, so a renderer change cannot desynchronise the two.
pub fn session_prompt_request(session_id: &str, id: u64, text: &str) -> Value {
    request(
        id,
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": text }]
        }),
    )
}

/// Per-turn facts the observation seam needs: which prompt is active and
/// which tool calls this turn has already reported. The caller owns the view
/// so the same observation can be driven without a live agent process.
#[derive(Debug, Clone, Default)]
pub struct TurnView {
    pub active_prompt_id: Option<u64>,
    pub reported_tool_ids: Vec<String>,
}

/// Observe one parsed JSON-RPC line from the agent. Progress updates stay
/// distinct from reply tokens; a tool update is only reported after its call
/// has been observed through this same seam. Raw tool input/output is never
/// read, so it cannot reach a chat event.
pub fn turn_events(msg: &Value, view: &mut TurnView) -> Vec<Result<StreamEvent, CloudError>> {
    let Some(active) = view.active_prompt_id else {
        return Vec::new();
    };
    if msg.get("method").and_then(Value::as_str) == Some("session/update") {
        let update = &msg["params"]["update"];
        let kind = update["sessionUpdate"].as_str().unwrap_or_default();
        let text = |field: &str| -> Option<String> {
            let content = &update[field];
            if content["type"].as_str() != Some("text") {
                return None;
            }
            let text = content["text"].as_str().unwrap_or_default();
            (!text.is_empty()).then(|| text.to_string())
        };
        let progress = match kind {
            "agent_message_chunk" => text("content").map(StreamEvent::Token),
            "agent_thought_chunk" => {
                text("content").map(|t| StreamEvent::Progress(AcpProgress::Thought { text: t }))
            }
            "tool_call" => {
                let id = update["toolCallId"].as_str().unwrap_or_default();
                let title = update["title"].as_str().unwrap_or_default();
                if id.is_empty() || title.is_empty() {
                    None
                } else {
                    if !view.reported_tool_ids.iter().any(|t| t == id) {
                        view.reported_tool_ids.push(id.to_string());
                    }
                    let status = update["status"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    Some(StreamEvent::Progress(AcpProgress::ToolCall {
                        tool_id: id.to_string(),
                        title: title.to_string(),
                        status,
                    }))
                }
            }
            "tool_call_update" => {
                let id = update["toolCallId"].as_str().unwrap_or_default();
                let status = update["status"].as_str().unwrap_or_default();
                if id.is_empty()
                    || status.is_empty()
                    || !view.reported_tool_ids.iter().any(|t| t == id)
                {
                    None
                } else {
                    Some(StreamEvent::Progress(AcpProgress::ToolUpdate {
                        tool_id: id.to_string(),
                        status: status.to_string(),
                    }))
                }
            }
            _ => None,
        };
        return progress.into_iter().map(Ok).collect();
    }
    let Some(id) = msg.get("id").and_then(Value::as_u64) else {
        return Vec::new();
    };
    if id != active {
        return Vec::new();
    }
    if let Some(err) = msg.get("error") {
        let message = err["message"].as_str().unwrap_or("unknown error");
        return vec![Err(CloudError::Acp(message.to_string()))];
    }
    if msg.get("result").is_some() {
        return vec![Ok(StreamEvent::Done)];
    }
    Vec::new()
}

/// The JSON-RPC requests written to an agent's stdin for one cancel command.
/// Only the abandoned turn may be cancelled; a stale id, a repeat or a queued
/// turn writes nothing. The request id is supplied by the caller because it
/// comes from the client's id generator, not from the turn id.
pub fn cancel_written_to_stdin(
    session_id: &str,
    active_id: Option<u64>,
    _pending_id: Option<u64>,
    already_cancelled: bool,
    cancel_id: u64,
    request_id: u64,
) -> Vec<Value> {
    if already_cancelled || active_id != Some(cancel_id) {
        return Vec::new();
    }
    vec![request(
        request_id,
        "session/cancel",
        json!({ "sessionId": session_id }),
    )]
}

/// Directory every session is opened in. `cwd` decides which project
/// configuration an agent discovers (opencode reads the directory's models,
/// agents, skills, MCP servers) and where anything it writes lands, so it is
/// a dedicated directory rather than the shared temp dir.
/// `KOMOREBI_AGENT_WORKSPACE` overrides it for experiments.
fn workspace() -> PathBuf {
    if let Some(dir) = std::env::var_os("KOMOREBI_AGENT_WORKSPACE").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("komorebi")
        .join("agent-workspace")
}

/// One live agent process, shared by every turn. A different command (or a
/// dead process) replaces it on the next turn.
struct SharedSession {
    key: String,
    cmd: mpsc::UnboundedSender<SessionCmd>,
    claude: bool,
    prompted: AtomicBool,
}

static SESSION: OnceLock<Mutex<Option<SharedSession>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(3);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

enum SessionCmd {
    Prompt { id: u64, text: String, sink: Sink },
    Cancel { id: u64 },
    Shutdown,
}

/// Sends `session/cancel` when a reply stream is dropped early. The id keeps
/// a late drop from cancelling the next turn.
struct CancelOnDrop(mpsc::UnboundedSender<SessionCmd>, u64);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(SessionCmd::Cancel { id: self.1 });
    }
}

#[derive(Clone)]
pub struct AcpClient {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
}

impl AcpClient {
    /// Drop the shared session so the next prompt starts a fresh process and
    /// ACP session. Chat reset calls this: the agent must not remember turns
    /// whose history Komorebi just discarded.
    pub async fn reset_session() {
        let slot = SESSION.get_or_init(|| Mutex::new(None));
        if let Some(old) = slot.lock().await.take() {
            let _ = old.cmd.send(SessionCmd::Shutdown);
        }
    }

    /// Parse a whitespace-separated command line (no quoting). A bare
    /// program name is looked up on `PATH`, then in `~/.local/bin`, which a
    /// desktop session's `PATH` often lacks.
    pub fn from_command(command: &str) -> Result<Self, CloudError> {
        let mut words = command.split_whitespace().map(str::to_string);
        let name = words
            .next()
            .ok_or_else(|| CloudError::Acp("agent command is empty".into()))?;
        let program = if name.contains('/') {
            PathBuf::from(&name)
        } else {
            let on_path = std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
                .unwrap_or_default();
            let home_bin = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/bin"));
            on_path
                .into_iter()
                .chain(home_bin)
                .map(|dir| dir.join(&name))
                .find(|p| p.is_file())
                .ok_or_else(|| {
                    CloudError::Acp(format!("`{name}` not found on PATH or in ~/.local/bin"))
                })?
        };
        Ok(Self {
            program,
            args: words.collect(),
            cwd: workspace(),
        })
    }

    /// Send one prompt to the shared session and stream its reply. Only the
    /// latest user message is sent on a resumed session: the agent already
    /// holds the earlier turns. Static persona text is sent with the first
    /// prompt; fresh context (time, machine, RAG) is sent with every prompt.
    pub async fn stream_chat(
        &self,
        messages: &[ChatMessage],
    ) -> Result<(TokenStream, AcpTurnRecord), CloudError> {
        std::fs::create_dir_all(&self.cwd).map_err(|e| {
            CloudError::Acp(format!(
                "cannot create the agent workspace {}: {e}",
                self.cwd.display()
            ))
        })?;
        let key = format!("{:?}|{:?}|{:?}", self.program, self.args, self.cwd);
        let slot = SESSION.get_or_init(|| Mutex::new(None));
        let mut guard = slot.lock().await;
        let reusable = guard
            .as_ref()
            .filter(|s| s.key == key && !s.cmd.is_closed())
            .map(|s| (s.cmd.clone(), s.claude, s.prompted.load(Ordering::SeqCst)));

        let (cmd, claude, first) = if let Some(reuse) = reusable {
            reuse
        } else {
            if let Some(old) = guard.take() {
                let _ = old.cmd.send(SessionCmd::Shutdown);
            }
            let system = flatten(messages).0;
            let session = start_session(self, key.clone(), system).await?;
            let handle = (
                session.cmd.clone(),
                session.claude,
                session.prompted.load(Ordering::SeqCst),
            );
            *guard = Some(session);
            handle
        };
        let prompted = guard.as_ref().expect("session installed");
        prompted.prompted.store(true, Ordering::SeqCst);
        drop(guard);

        let id = next_id();
        let record = AcpTurnRecord::from_messages(messages, !claude && !first);
        let text = record.session_prompt_text.clone();
        let (sink_tx, mut sink_rx) = mpsc::unbounded_channel();
        cmd.send(SessionCmd::Prompt {
            id,
            text,
            sink: sink_tx,
        })
        .map_err(|_| CloudError::Acp("agent session closed".into()))?;

        Ok((
            Box::pin(async_stream::stream! {
                let _cancel = CancelOnDrop(cmd, id);
                while let Some(evt) = sink_rx.recv().await {
                    yield evt;
                }
            }),
            record,
        ))
    }
}

/// Spawn the agent, do the ACP handshake once, then hand the process to the
/// actor task that owns it until the command changes or the app exits.
async fn start_session(
    client: &AcpClient,
    key: String,
    system: String,
) -> Result<SharedSession, CloudError> {
    let mut child = Command::new(&client.program)
        .args(&client.args)
        .current_dir(&client.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Agents log freely on stderr; inherit it so it can never fill
        // a pipe and stall the turn.
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            CloudError::Acp(format!("failed to start {}: {e}", client.program.display()))
        })?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut lines = BufReader::new(stdout).lines();
    let mut turn = Turn::new(system, String::new()).in_dir(client.cwd.clone());

    write_msg(&mut stdin, &turn.start()).await?;
    loop {
        let line = next_line(&mut lines).await?;
        let Some(msg) = parse(line) else { continue };
        if let Some(reply) = agent_request(&msg) {
            write_msg(&mut stdin, &reply).await?;
            continue;
        }
        for step in turn.on_message(&msg) {
            match step {
                Step::Send(v) => write_msg(&mut stdin, &v).await?,
                Step::Fail(e) => return Err(CloudError::Acp(e)),
            }
        }
        if turn.session_id.is_some() {
            break;
        }
    }
    let claude = turn.claude;
    let session_id = turn.session_id.expect("handshake set session id");
    let (cmd, rx) = mpsc::unbounded_channel();
    tokio::spawn(run_session(child, stdin, lines, session_id, rx));
    Ok(SharedSession {
        key,
        cmd,
        claude,
        prompted: AtomicBool::new(false),
    })
}

/// Owns one agent process. Turns are serialised here: one prompt at a time,
/// with tokens routed to that turn's sink until its response arrives.
async fn run_session(
    mut child: Child,
    mut stdin: Stdin,
    mut lines: StdoutLines,
    session_id: String,
    mut rx: mpsc::UnboundedReceiver<SessionCmd>,
) {
    let mut active: Option<(u64, Sink)> = None;
    let mut pending: Option<(u64, String, Sink)> = None;
    let mut cancelled = false;
    let mut view = TurnView::default();
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    SessionCmd::Prompt { id, text, sink } => {
                        if active.is_some() {
                            // OpenCode runs one prompt at a time, so cancel
                            // the active turn and queue this one until its
                            // response arrives instead of sending a prompt
                            // it would reject.
                            cancel_active(&mut stdin, &session_id, &active, &pending, &mut cancelled).await;
                            if let Some((old_id, _, old)) = pending.take() {
                                if old_id != id {
                                    let _ = old.send(Err(CloudError::Acp(
                                        "superseded by a newer turn".into(),
                                    )));
                                }
                            }
                            pending = Some((id, text, sink));
                            continue;
                        }
                        match send_prompt(&mut stdin, &session_id, id, text, sink).await {
                            Ok(turn) => {
                                active = Some(turn);
                                view.active_prompt_id = Some(id);
                                view.reported_tool_ids.clear();
                            }
                            Err((sink, e)) => { let _ = sink.send(Err(e)); }
                        }
                        cancelled = false;
                    }
                    SessionCmd::Cancel { id } => {
                        // A queued turn has not reached the agent yet: drop
                        // it; there is nothing for the agent to cancel.
                        if pending.as_ref().is_some_and(|(p, _, _)| *p == id) {
                            let _ = pending.take();
                            continue;
                        }
                        if active.as_ref().is_some_and(|(a, _)| *a == id) {
                            cancel_active(&mut stdin, &session_id, &active, &pending, &mut cancelled).await;
                        }
                    }
                    SessionCmd::Shutdown => {
                        fail_active(&mut active, "agent session closed");
                        fail_pending(&mut pending, "agent session closed");
                        break;
                    }
                }
            }
            line = lines.next_line() => {
                let line = match line {
                    Ok(Some(line)) => line,
                    Ok(None) => {
                        fail_active(&mut active, "agent exited before replying");
                        fail_pending(&mut pending, "agent exited before replying");
                        break;
                    }
                    Err(e) => {
                        let message = format!("reading from agent: {e}");
                        fail_active(&mut active, &message);
                        fail_pending(&mut pending, &message);
                        break;
                    }
                };
                let Some(msg) = parse(line) else { continue };
                if let Some(reply) = agent_request(&msg) {
                    let _ = write_msg(&mut stdin, &reply).await;
                    continue;
                }
                view.active_prompt_id = active.as_ref().map(|(id, _)| *id);
                let mut finished = false;
                for evt in turn_events(&msg, &mut view) {
                    match evt {
                        Ok(StreamEvent::Done) => {
                            if let Some((_, sink)) = active.as_ref() {
                                let _ = sink.send(Ok(StreamEvent::Done));
                            }
                            finished = true;
                        }
                        Err(e) => {
                            if let Some((_, sink)) = active.take() {
                                let _ = sink.send(Err(e));
                            }
                            finished = true;
                        }
                        other => {
                            if let Some((_, sink)) = active.as_ref() {
                                let _ = sink.send(other);
                            }
                        }
                    }
                }
                if finished {
                    let _ = active.take();
                    view.reported_tool_ids.clear();
                    start_pending(
                        &mut stdin,
                        &session_id,
                        &mut active,
                        &mut pending,
                        &mut cancelled,
                    )
                    .await;
                }
            }
        }
    }
    let _ = child.kill().await;
}

/// Send a queued turn as soon as the previous one's response has arrived.
async fn start_pending(
    stdin: &mut Stdin,
    session_id: &str,
    active: &mut Option<(u64, Sink)>,
    pending: &mut Option<(u64, String, Sink)>,
    cancelled: &mut bool,
) {
    if let Some((id, text, sink)) = pending.take() {
        match send_prompt(stdin, session_id, id, text, sink).await {
            Ok(turn) => {
                *active = Some(turn);
                *cancelled = false;
            }
            Err((sink, e)) => {
                let _ = sink.send(Err(e));
            }
        }
    }
}

/// Send `session/cancel` once for the active turn. The flag is per turn:
/// `start_pending` clears it when the next prompt goes out. The request lines
/// come from the same observation seam the cancellation test drives.
async fn cancel_active(
    stdin: &mut Stdin,
    session_id: &str,
    active: &Option<(u64, Sink)>,
    pending: &Option<(u64, String, Sink)>,
    cancelled: &mut bool,
) {
    let active_id = active.as_ref().map(|(id, _)| *id);
    let Some(cancel_id) = active_id else {
        return;
    };
    let pending_id = pending.as_ref().map(|(id, _, _)| *id);
    let requests = cancel_written_to_stdin(
        session_id,
        active_id,
        pending_id,
        *cancelled,
        cancel_id,
        next_id(),
    );
    if requests.is_empty() {
        return;
    }
    *cancelled = true;
    for req in requests {
        let _ = write_msg(stdin, &req).await;
    }
}

/// Write `session/prompt`. On failure the sink comes back so the caller can
/// report the error to that turn instead of losing it.
async fn send_prompt(
    stdin: &mut Stdin,
    session_id: &str,
    id: u64,
    text: String,
    sink: Sink,
) -> Result<(u64, Sink), (Sink, CloudError)> {
    let req = session_prompt_request(session_id, id, &text);
    match write_msg(stdin, &req).await {
        Ok(()) => Ok((id, sink)),
        Err(e) => Err((sink, e)),
    }
}

fn fail_active(active: &mut Option<(u64, Sink)>, message: &str) {
    if let Some((_, sink)) = active.take() {
        let _ = sink.send(Err(CloudError::Acp(message.to_string())));
    }
}

fn fail_pending(pending: &mut Option<(u64, String, Sink)>, message: &str) {
    if let Some((_, _, sink)) = pending.take() {
        let _ = sink.send(Err(CloudError::Acp(message.to_string())));
    }
}

async fn write_msg(stdin: &mut Stdin, msg: &Value) -> Result<(), CloudError> {
    let line = format!("{msg}\n");
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| CloudError::Acp(format!("writing to agent: {e}")))
}

async fn next_line(lines: &mut StdoutLines) -> Result<String, CloudError> {
    lines
        .next_line()
        .await
        .map_err(|e| CloudError::Acp(format!("reading from agent: {e}")))?
        .ok_or_else(|| CloudError::Acp("agent exited before replying".into()))
}

fn parse(line: String) -> Option<Value> {
    serde_json::from_str(&line).ok()
}

/// Agent-to-client requests get an answer; notifications do not.
fn agent_request(msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id")?;
    Some(answer(id, method, msg.get("params")))
}

#[derive(Debug, PartialEq)]
enum Step {
    Send(Value),
    Fail(String),
}

/// One handshake's protocol state: initialize, open a session, remember its
/// id. Prompting is done by [`run_session`], which owns the live process.
struct Turn {
    system: String,
    cwd: PathBuf,
    claude: bool,
    session_id: Option<String>,
}

impl Turn {
    fn new(system: String, _prompt: String) -> Self {
        Self {
            system,
            cwd: workspace(),
            claude: false,
            session_id: None,
        }
    }

    /// Open the session in `cwd` instead of the default workspace.
    fn in_dir(mut self, cwd: PathBuf) -> Self {
        self.cwd = cwd;
        self
    }

    /// `session/new`, with `claude-agent-acp`'s extension options when the
    /// agent supports them: Komorebi's system prompt instead of Claude
    /// Code's, no user settings (hooks, CLAUDE.md, skills), no built-in
    /// tools (Komorebi's own tool loop stays in charge), no transcript left
    /// on disk.
    fn session_new(&self) -> Value {
        let mut params = json!({ "cwd": self.cwd.to_string_lossy(), "mcpServers": [] });
        if self.claude {
            params["_meta"] = json!({
                "systemPrompt": self.system,
                "claudeCode": { "options": {
                    "settingSources": [],
                    "tools": [],
                    "persistSession": false
                }}
            });
        }
        params
    }

    fn start(&self) -> Value {
        request(
            ID_INITIALIZE,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false
                }
            }),
        )
    }

    fn on_message(&mut self, msg: &Value) -> Vec<Step> {
        let id = msg.get("id");
        match (msg.get("method").and_then(Value::as_str), id) {
            (None, Some(id)) => {
                if let Some(err) = msg.get("error") {
                    let text = err["message"].as_str().unwrap_or("unknown error");
                    return vec![Step::Fail(text.to_string())];
                }
                match id.as_u64() {
                    Some(ID_INITIALIZE) => {
                        self.claude =
                            msg["result"]["agentCapabilities"]["_meta"]["claudeCode"].is_object();
                        vec![Step::Send(request(
                            ID_SESSION_NEW,
                            "session/new",
                            self.session_new(),
                        ))]
                    }
                    Some(ID_SESSION_NEW) => match msg["result"]["sessionId"].as_str() {
                        Some(session) => {
                            self.session_id = Some(session.to_string());
                            vec![]
                        }
                        None => vec![Step::Fail("session/new returned no sessionId".into())],
                    },
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// Reject every permission request (by the option's kind, since option
/// ids are agent-specific); refuse every other method, as Komorebi
/// advertises no client capability.
fn answer(id: &Value, method: &str, params: Option<&Value>) -> Value {
    if method == "session/request_permission" {
        let reject = params
            .and_then(|p| p["options"].as_array())
            .and_then(|opts| opts.iter().find(|o| o["kind"] == "reject_once"))
            .and_then(|o| o["optionId"].as_str());
        let outcome = match reject {
            Some(option) => json!({ "outcome": "selected", "optionId": option }),
            None => json!({ "outcome": "cancelled" }),
        };
        return json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": outcome } });
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32601, "message": format!("{method} is not supported by Komorebi") }
    })
}

/// Leading system messages become the persona; the rest are fresh context.
/// The final user message is this turn's prompt. Earlier turns stay in the
/// agent's session, so they are not replayed.
fn turn_text(messages: &[ChatMessage], include_persona: bool) -> String {
    let lead = messages
        .iter()
        .take_while(|m| m.role == Role::System)
        .count();
    let mut parts = Vec::new();
    if include_persona {
        if let Some(first) = messages.first() {
            if first.role == Role::System {
                parts.push(first.content.clone());
            }
        }
    }
    for m in messages.iter().skip(1).take(lead.saturating_sub(1)) {
        parts.push(m.content.clone());
    }
    if let Some(last) = messages.last() {
        if last.role == Role::User {
            parts.push(last.content.clone());
        }
    }
    if parts.is_empty() {
        return flatten(messages).1;
    }
    parts.join("\n\n")
}

/// Leading system messages become the system text; the rest is replayed
/// as a labelled transcript (tool results arrive as system messages
/// mid-conversation, so they stay in place). Used for the session's system
/// text and as the fallback when a turn has no usable user message.
fn flatten(messages: &[ChatMessage]) -> (String, String) {
    let lead = messages
        .iter()
        .take_while(|m| m.role == Role::System)
        .count();
    let system = messages[..lead]
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let prompt = match &messages[lead..] {
        [only] if only.role == Role::User => only.content.clone(),
        rest => {
            let mut s = String::from(
                "Conversation so far. Write only your next reply as the Assistant.\n\n",
            );
            for m in rest {
                let label = match m.role {
                    Role::System => "System",
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                };
                s.push_str(&format!("{label}: {}\n\n", m.content));
            }
            s
        }
    };
    (system, prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    // CONTRACT: send_prompt builds every session/prompt request through
    // session_prompt_request. The test below pins the builder; review checks
    // the call site because send_prompt owns a ChildStdin.
    const WINDOW_LINE: &str = "- window context: available source=harness-2 at=1900 app=org.example.Viewer x=200 y=300 w=640 h=480 state=background";
    const ACP_FIRST_TEXT_A: &str = "persona\n\ncontext\n\n- window context: available source=harness-2 at=1900 app=org.example.Viewer x=200 y=300 w=640 h=480 state=background\n\nhi";
    const ACP_FIRST_TEXT_SENTINEL: &str = "persona\n\ncontext\n\n- window context: available source=harness-2 at=1900 app=org.example.Viewer x=200 y=300 w=640 h=480 state=background\n\nhi \n";

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn method_of(step: &Step) -> &str {
        match step {
            Step::Send(v) => v["method"].as_str().unwrap(),
            other => panic!("expected a request, got {other:?}"),
        }
    }

    fn update(method: &str, update: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": { "update": update }
        })
    }

    fn turn_view(active_id: Option<u64>, reported_tool_ids: &[&str]) -> TurnView {
        TurnView {
            active_prompt_id: active_id,
            reported_tool_ids: reported_tool_ids.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn ok_events(msg: &Value, view: &mut TurnView) -> Vec<StreamEvent> {
        turn_events(msg, view)
            .into_iter()
            .map(|r| r.expect("expected an Ok stream event"))
            .collect()
    }

    fn progress_of(evt: &StreamEvent) -> &AcpProgress {
        match evt {
            StreamEvent::Progress(p) => p,
            other => panic!("expected progress, got {other:?}"),
        }
    }

    /// Given an active ACP prompt and an `agent_message_chunk` with non-empty text
    /// When the notification is observed
    /// Then exactly one Token event is returned and no Progress event is returned (retained)
    /// Given the same active turn and an `agent_thought_chunk` with a non-empty text part
    /// When observed
    /// Then one Progress(Thought) event carries that exact text and no Token event is produced
    /// And empty text, a non-text part and no active turn emit nothing
    /// Given a `tool_call` with id, title and status on the same turn
    /// When observed through the same seam
    /// Then one Progress(ToolCall) event carries id, title and status, raw input is absent,
    /// And the call id becomes reported state produced by that observation
    /// Given the following `tool_call_update` for that produced id with a changed status
    /// When observed through the same seam
    /// Then one Progress(ToolUpdate) event carries the same id and the changed status, raw output is absent
    /// And an update with no reported call, an unreported id, or no active turn emits nothing
    /// Given unknown, empty-text and no-active-turn notifications
    /// When observed
    /// Then no progress event is emitted
    /// Given a matching prompt result and a matching error
    /// When observed
    /// Then Done and CloudError::Acp are unchanged; non-matching ids emit nothing (retained)
    #[test]
    fn agent_message_chunk_still_emits_one_token_and_no_progress() {
        let msg = update(
            "session/update",
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "reply " }
            }),
        );
        let mut view = turn_view(Some(7), &[]);
        assert_eq!(
            ok_events(&msg, &mut view),
            vec![StreamEvent::Token("reply ".into())]
        );
        assert!(view.reported_tool_ids.is_empty());

        let thought = update(
            "session/update",
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "text", "text": "thinking out loud" }
            }),
        );
        assert_eq!(
            ok_events(&thought, &mut turn_view(Some(7), &[])),
            vec![StreamEvent::Progress(AcpProgress::Thought {
                text: "thinking out loud".into(),
            })]
        );
        assert!(ok_events(&thought, &mut turn_view(None, &[])).is_empty());

        let empty_thought = update(
            "session/update",
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "text", "text": "" }
            }),
        );
        assert!(ok_events(&empty_thought, &mut turn_view(Some(7), &[])).is_empty());

        let non_text_thought = update(
            "session/update",
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "image", "text": "x" }
            }),
        );
        assert!(ok_events(&non_text_thought, &mut turn_view(Some(7), &[])).is_empty());

        let input_sentinel = "RAW-INPUT-SENTINEL-DO-NOT-EMIT";
        let tool_call = update(
            "session/update",
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "call-1",
                "title": "Read file",
                "status": "pending",
                "rawInput": { "secret": input_sentinel }
            }),
        );
        let mut view = turn_view(Some(7), &[]);
        let call_events = ok_events(&tool_call, &mut view);
        assert_eq!(
            call_events,
            vec![StreamEvent::Progress(AcpProgress::ToolCall {
                tool_id: "call-1".into(),
                title: "Read file".into(),
                status: Some("pending".into()),
            })]
        );
        assert_eq!(view.reported_tool_ids, vec!["call-1".to_string()]);

        let call_wire = serde_json::to_value(progress_of(&call_events[0])).unwrap();
        assert_eq!(
            call_wire,
            json!({
                "stage": "tool_call",
                "tool_id": "call-1",
                "title": "Read file",
                "status": "pending"
            })
        );
        assert!(call_wire.get("input").is_none());
        assert!(!call_wire.to_string().contains(input_sentinel));
        assert!(ok_events(&tool_call, &mut turn_view(None, &[])).is_empty());

        let output_sentinel = "RAW-OUTPUT-SENTINEL-DO-NOT-EMIT";
        let tool_update = update(
            "session/update",
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call-1",
                "status": "completed",
                "rawOutput": { "blob": output_sentinel }
            }),
        );
        let update_events = ok_events(&tool_update, &mut view);
        assert_eq!(
            update_events,
            vec![StreamEvent::Progress(AcpProgress::ToolUpdate {
                tool_id: "call-1".into(),
                status: "completed".into(),
            })]
        );
        let update_wire = serde_json::to_value(progress_of(&update_events[0])).unwrap();
        assert_eq!(
            update_wire,
            json!({
                "stage": "tool_update",
                "tool_id": "call-1",
                "status": "completed"
            })
        );
        assert!(update_wire.get("output").is_none());
        assert!(!update_wire.to_string().contains(output_sentinel));

        assert!(ok_events(&tool_update, &mut turn_view(Some(7), &[])).is_empty());
        assert!(ok_events(&tool_update, &mut turn_view(Some(7), &["call-2"])).is_empty());
        assert!(ok_events(&tool_update, &mut turn_view(None, &["call-1"])).is_empty());

        let negatives = [
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": ""}}),
            json!({"sessionUpdate": "tool_call", "title": "no id"}),
            json!({"sessionUpdate": "something_else", "content": {"type": "text", "text": "x"}}),
        ];
        for case in negatives {
            assert!(ok_events(
                &update("session/update", case),
                &mut turn_view(Some(7), &[])
            )
            .is_empty());
        }
        let not_update = json!({"jsonrpc": "2.0", "method": "session/prompt", "params": {}});
        assert!(ok_events(&not_update, &mut turn_view(Some(7), &[])).is_empty());

        let done = json!({"jsonrpc": "2.0", "id": 7, "result": {"stopReason": "end_turn"}});
        assert_eq!(
            ok_events(&done, &mut turn_view(Some(7), &[])),
            vec![StreamEvent::Done]
        );
        assert!(ok_events(&done, &mut turn_view(Some(8), &[])).is_empty());

        let err = json!({"jsonrpc": "2.0", "id": 7, "error": {"message": "boom"}});
        let mut err_it = turn_events(&err, &mut turn_view(Some(7), &[])).into_iter();
        match err_it.next().expect("error event") {
            Err(CloudError::Acp(message)) => assert_eq!(message, "boom"),
            other => panic!("expected CloudError::Acp, got {other:?}"),
        }
        assert!(err_it.next().is_none());
        assert!(ok_events(
            &json!({"jsonrpc": "2.0", "id": 8, "error": {"message": "boom"}}),
            &mut turn_view(Some(7), &[])
        )
        .is_empty());
    }

    fn cancel_writes(
        session_id: &str,
        active_id: Option<u64>,
        pending_id: Option<u64>,
        already_cancelled: bool,
        cancel_id: u64,
        request_id: u64,
    ) -> Vec<Value> {
        cancel_written_to_stdin(
            session_id,
            active_id,
            pending_id,
            already_cancelled,
            cancel_id,
            request_id,
        )
    }

    /// Given an active turn 7 with no prior cancel and request id 41 supplied independently
    /// When a drop for turn 7 is written to the in-memory agent stdin
    /// Then exactly one session/cancel request is written with jsonrpc 2.0, request id 41 and sessionId s1
    /// Given the same stale cancel id 7 while turn 8 is active
    /// When written with request id 42
    /// Then no session/cancel request is written
    /// Given turn 8 is active and not yet cancelled
    /// When turn 8 is dropped with request id 43
    /// Then exactly one session/cancel request is written with request id 43 and sessionId s1
    /// Given a repeated cancel after one was already written, or a queued turn with no active turn
    /// When written
    /// Then no session/cancel request is written
    #[test]
    fn cancellation_writes_only_the_abandoned_turn_id_to_agent_stdin() {
        assert_eq!(
            cancel_writes("s1", Some(7), None, false, 7, 41),
            vec![json!({
                "jsonrpc": "2.0",
                "id": 41,
                "method": "session/cancel",
                "params": { "sessionId": "s1" }
            })]
        );

        assert!(cancel_writes("s1", Some(8), None, false, 7, 42).is_empty());

        let next_turn = cancel_writes("s1", Some(8), None, false, 8, 43);
        assert_eq!(
            next_turn,
            vec![json!({
                "jsonrpc": "2.0",
                "id": 43,
                "method": "session/cancel",
                "params": { "sessionId": "s1" }
            })]
        );
        assert_eq!(next_turn.len(), 1);

        assert!(cancel_writes("s1", Some(7), None, true, 7, 44).is_empty());
        assert!(cancel_writes("s1", None, Some(9), false, 9, 45).is_empty());

        let other_session = cancel_writes("s2", Some(7), None, false, 7, 46);
        assert_eq!(other_session.len(), 1);
        assert_eq!(other_session[0]["params"]["sessionId"], "s2");
    }

    #[test]
    fn walks_handshake_and_remembers_the_session() {
        let mut t = Turn::new(String::new(), String::new());
        assert_eq!(t.start()["method"], "initialize");
        let s = t.on_message(&parse(
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#,
        ));
        assert_eq!(method_of(&s[0]), "session/new");
        assert!(t
            .on_message(&parse(
                r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#,
            ))
            .is_empty());
        assert_eq!(t.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn refuses_client_methods_and_reports_errors() {
        let mut t = Turn::new(String::new(), String::new());
        let err = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"auth required"}}"#;
        assert_eq!(
            t.on_message(&parse(err)),
            vec![Step::Fail("auth required".into())]
        );
    }

    #[test]
    fn rejects_permission_by_kind() {
        let req = r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"options":[{"optionId":"allow-once","kind":"allow_once"},{"optionId":"reject","kind":"reject_once"}]}}"#;
        let reply = agent_request(&parse(req)).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(
            reply["result"]["outcome"],
            json!({"outcome":"selected","optionId":"reject"})
        );
        let no_reject = r#"{"jsonrpc":"2.0","id":8,"method":"session/request_permission","params":{"options":[]}}"#;
        let reply = agent_request(&parse(no_reject)).unwrap();
        assert_eq!(reply["result"]["outcome"], json!({"outcome":"cancelled"}));
    }

    #[test]
    fn refuses_unknown_agent_requests() {
        let req = r#"{"jsonrpc":"2.0","id":9,"method":"fs/read_text_file","params":{}}"#;
        let reply = agent_request(&parse(req)).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
        assert!(agent_request(&parse(r#"{"jsonrpc":"2.0","method":"session/update"}"#)).is_none());
    }

    /// Given the ACP turn record built from the existing first-turn fixture of three
    ///   leading system messages (persona, context, window line) and one user message,
    ///   plus a sentinel row whose user text ends in a space and a newline
    /// When the record is built with and without the persona
    /// Then its session_prompt_text is the exact renderer literal for that flag, including the sentinel's trailing bytes
    /// And its system_roles are three ordered lowercase "system" labels with count three in every row
    /// And the persona is in the text only when the renderer's flag requests it (retained renderer decision)
    /// And both existing renderer outputs remain unchanged (retained)
    ///
    /// RETAINED-BEHAVIOUR PIN ONLY. Given a turn-text input that already contains a
    ///   low-risk window-context line from the harness source
    /// When the turn text is built with and without the persona
    /// Then that line is preserved verbatim in both, in the existing message order
    ///
    /// This passes on unmodified production and does not prove that the context producer emits the line.
    #[test]
    fn first_turn_carries_persona_and_context() {
        let messages = [
            ChatMessage::system("persona"),
            ChatMessage::system("context"),
            ChatMessage::system(WINDOW_LINE),
            ChatMessage::user("hi"),
        ];
        assert_eq!(turn_text(&messages, true), ACP_FIRST_TEXT_A);
        assert_eq!(
            turn_text(&messages, false),
            format!("context\n\n{WINDOW_LINE}\n\nhi")
        );

        let first = AcpTurnRecord::from_messages(&messages, true);
        assert_eq!(first.session_prompt_text, ACP_FIRST_TEXT_A);
        assert_eq!(
            first.system_roles,
            vec![Role::System, Role::System, Role::System]
        );
        assert_eq!(first.system_message_count, 3);

        let later = AcpTurnRecord::from_messages(&messages, false);
        assert_eq!(
            later.session_prompt_text,
            format!("context\n\n{WINDOW_LINE}\n\nhi")
        );
        assert_eq!(
            later.system_roles,
            vec![Role::System, Role::System, Role::System]
        );
        assert_eq!(later.system_message_count, 3);

        let sentinel_messages = [
            ChatMessage::system("persona"),
            ChatMessage::system("context"),
            ChatMessage::system(WINDOW_LINE),
            ChatMessage::user("hi \n"),
        ];
        let sentinel = AcpTurnRecord::from_messages(&sentinel_messages, true);
        assert_eq!(sentinel.session_prompt_text, ACP_FIRST_TEXT_SENTINEL);
        assert_eq!(
            sentinel.system_roles,
            vec![Role::System, Role::System, Role::System]
        );
        assert_eq!(sentinel.system_message_count, 3);
    }

    /// Given a later turn with fresh context and an earlier exchange, plus a turn whose
    ///   only system message is the leading persona and whose tool result is mid-conversation
    /// When the per-turn ACP record is built on a reused session without the persona
    /// Then its session_prompt_text is the exact later-turn renderer literal and omits the persona and the replayed exchange
    /// And system_roles and system_message_count still include the persona as a leading system message
    /// And the mid-conversation system message is excluded from both roles and count
    /// And the fresh-line and no-replay renderer output remain unchanged (retained)
    ///
    /// RETAINED-BEHAVIOUR PIN ONLY. Given a later turn whose fresh context already
    ///   contains a low-risk window-context line and whose history contains an earlier exchange
    /// When the turn text is built
    /// Then the fresh line is preserved and the earlier exchange is still not replayed
    ///
    /// This passes on unmodified production and does not prove that the context producer emits the line.
    #[test]
    fn later_turns_drop_the_replayed_history() {
        let messages = [
            ChatMessage::system("persona"),
            ChatMessage::system("fresh context"),
            ChatMessage::system(WINDOW_LINE),
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello"),
            ChatMessage::user("again"),
        ];
        let expected = format!("fresh context\n\n{WINDOW_LINE}\n\nagain");
        assert_eq!(turn_text(&messages, false), expected);

        let record = AcpTurnRecord::from_messages(&messages, false);
        assert_eq!(record.session_prompt_text, expected);
        assert_eq!(
            record.system_roles,
            vec![Role::System, Role::System, Role::System]
        );
        assert_eq!(record.system_message_count, 3);

        let mid = [
            ChatMessage::system("persona"),
            ChatMessage::user("hi"),
            ChatMessage::system("tool result"),
            ChatMessage::user("again"),
        ];
        let with_persona = AcpTurnRecord::from_messages(&mid, true);
        assert_eq!(with_persona.session_prompt_text, "persona\n\nagain");
        assert_eq!(with_persona.system_roles, vec![Role::System]);
        assert_eq!(with_persona.system_message_count, 1);

        let without_persona = AcpTurnRecord::from_messages(&mid, false);
        assert_eq!(without_persona.session_prompt_text, "again");
        assert_eq!(without_persona.system_roles, vec![Role::System]);
        assert_eq!(without_persona.system_message_count, 1);
    }

    /// Given a first-turn record built from persona, context and window-line messages and a
    ///   sentinel user text with a trailing space and newline
    /// When the session/prompt request params are built from that record's text
    /// Then params.prompt[0].text is byte-identical to the record text, with no trimming or re-encoding
    /// And the record text contains the window line verbatim
    /// And the existing session/prompt method, sessionId and prompt-part shape are unchanged (retained)
    /// And the module contract above records that send_prompt uses this seam,
    ///   which review checks because send_prompt owns a ChildStdin
    #[test]
    fn session_prompt_request_keeps_the_record_text_verbatim() {
        let messages = [
            ChatMessage::system("persona"),
            ChatMessage::system("context"),
            ChatMessage::system(WINDOW_LINE),
            ChatMessage::user("hi \n"),
        ];
        let record = AcpTurnRecord::from_messages(&messages, true);
        let req = session_prompt_request("s1", 3, &record.session_prompt_text);
        assert_eq!(req["method"], "session/prompt");
        assert_eq!(req["params"]["sessionId"], "s1");
        assert_eq!(req["params"]["prompt"][0]["type"], "text");
        assert_eq!(
            req["params"]["prompt"][0]["text"].as_str(),
            Some(record.session_prompt_text.as_str())
        );
        assert!(record.session_prompt_text.contains(WINDOW_LINE));
    }

    #[test]
    fn command_line_is_split() {
        let c = AcpClient::from_command("/opt/agent --flag value").unwrap();
        assert_eq!(c.program, PathBuf::from("/opt/agent"));
        assert_eq!(c.args, vec!["--flag", "value"]);
        assert!(AcpClient::from_command("   ").is_err());
    }

    #[test]
    fn session_is_opened_in_the_given_workspace() {
        let t = Turn::new(String::new(), String::new()).in_dir(PathBuf::from("/work/agent"));
        assert_eq!(t.session_new()["cwd"], "/work/agent");
    }
}
