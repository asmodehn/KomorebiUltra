//! Agent Client Protocol (ACP) client: any ACP agent as a chat engine.
//!
//! The agent is a command speaking JSON-RPC over stdio (for example
//! `claude-agent-acp`), started once per turn and killed when the reply
//! stream is dropped. Each turn is stateless: Komorebi keeps the history
//! and replays it in the prompt, like the OpenRouter path, so only the
//! Spoken transcript is ever remembered.
//!
//! Komorebi advertises no file-system or terminal capability and rejects
//! every permission request, so the agent cannot change anything through
//! Komorebi. What the agent runs without asking is up to its own config,
//! except for `claude-agent-acp`, whose tools and settings are switched off
//! (see [`Turn::session_new`]).
//!
//! Every turn opens its session in a dedicated, empty workspace directory:
//! `cwd` selects the project configuration an agent discovers, and it is
//! the blast radius of anything the agent writes on its own.
//! `KOMOREBI_AGENT_WORKSPACE` overrides it for experiments.

use crate::{CloudError, StreamEvent, TokenStream};
use komorebi_router::{ChatMessage, Role};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const PROTOCOL_VERSION: u64 = 1;
const ID_INITIALIZE: u64 = 1;
const ID_SESSION_NEW: u64 = 2;
const ID_PROMPT: u64 = 3;

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

#[derive(Clone)]
pub struct AcpClient {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
}

impl AcpClient {
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

    pub async fn stream_chat(&self, messages: &[ChatMessage]) -> Result<TokenStream, CloudError> {
        std::fs::create_dir_all(&self.cwd).map_err(|e| {
            CloudError::Acp(format!(
                "cannot create the agent workspace {}: {e}",
                self.cwd.display()
            ))
        })?;
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Agents log freely on stderr; inherit it so it can never fill
            // a pipe and stall the turn.
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                CloudError::Acp(format!("failed to start {}: {e}", self.program.display()))
            })?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let (system, prompt) = flatten(messages);
        let mut turn = Turn::new(system, prompt).in_dir(self.cwd.clone());

        Ok(Box::pin(async_stream::stream! {
            // Owned by the stream: dropping it (on cancel) kills the agent.
            let mut child = child;
            let mut lines = BufReader::new(stdout).lines();
            let mut outgoing = vec![turn.start()];
            loop {
                for msg in outgoing.drain(..) {
                    let line = format!("{msg}\n");
                    if let Err(e) = stdin.write_all(line.as_bytes()).await {
                        yield Err(CloudError::Acp(format!("writing to agent: {e}")));
                        return;
                    }
                }
                let line = match lines.next_line().await {
                    Ok(Some(line)) => line,
                    Ok(None) => {
                        let status = child.wait().await.map(|s| s.to_string()).unwrap_or_default();
                        yield Err(CloudError::Acp(format!("agent exited before replying ({status})")));
                        return;
                    }
                    Err(e) => { yield Err(CloudError::Acp(format!("reading from agent: {e}"))); return; }
                };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                for step in turn.on_message(&msg) {
                    match step {
                        Step::Send(v) => outgoing.push(v),
                        Step::Token(t) => yield Ok(StreamEvent::Token(t)),
                        Step::Done => { yield Ok(StreamEvent::Done); return; }
                        Step::Fail(e) => { yield Err(CloudError::Acp(e)); return; }
                    }
                }
            }
        }))
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Send(Value),
    Token(String),
    Done,
    Fail(String),
}

/// One turn's protocol state: initialize, open a session, send the
/// prompt, stream its text until the prompt request returns.
struct Turn {
    system: String,
    prompt: String,
    /// Directory the agent's session is opened in.
    cwd: PathBuf,
    /// The agent is `claude-agent-acp` (it advertises `_meta.claudeCode`):
    /// the system text replaces its coding preset instead of opening the
    /// prompt.
    claude: bool,
}

impl Turn {
    fn new(system: String, prompt: String) -> Self {
        Self {
            system,
            prompt,
            cwd: workspace(),
            claude: false,
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

    fn prompt_text(&self) -> String {
        if self.claude || self.system.is_empty() {
            self.prompt.clone()
        } else {
            format!("{}\n\n{}", self.system, self.prompt)
        }
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
            // Request from the agent: always answer.
            (Some(method), Some(id)) => vec![Step::Send(answer(id, method, msg.get("params")))],
            // Notification: never answer.
            (Some("session/update"), None) => {
                let update = &msg["params"]["update"];
                let text = update["content"]["text"].as_str().unwrap_or_default();
                if update["sessionUpdate"] == "agent_message_chunk"
                    && update["content"]["type"] == "text"
                    && !text.is_empty()
                {
                    vec![Step::Token(text.to_string())]
                } else {
                    vec![]
                }
            }
            (Some(_), None) => vec![],
            // Response to one of our requests.
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
                        Some(session) => vec![Step::Send(request(
                            ID_PROMPT,
                            "session/prompt",
                            json!({
                                "sessionId": session,
                                "prompt": [{ "type": "text", "text": self.prompt_text() }]
                            }),
                        ))],
                        None => vec![Step::Fail("session/new returned no sessionId".into())],
                    },
                    Some(ID_PROMPT) => vec![Step::Done],
                    _ => vec![],
                }
            }
            (None, None) => vec![],
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

/// Leading system messages become the system text; the rest is replayed
/// as a labelled transcript (tool results arrive as system messages
/// mid-conversation, so they stay in place).
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

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn method_of(step: &Step) -> &str {
        match step {
            Step::Send(v) => v["method"].as_str().unwrap(),
            other => panic!("expected a request, got {other:?}"),
        }
    }

    #[test]
    fn walks_handshake_and_streams_text() {
        let mut t = Turn::new(String::new(), "hi".into());
        assert_eq!(t.start()["method"], "initialize");
        let s = t.on_message(&parse(
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#,
        ));
        assert_eq!(method_of(&s[0]), "session/new");
        let s = t.on_message(&parse(
            r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#,
        ));
        let Step::Send(prompt) = &s[0] else { panic!() };
        assert_eq!(prompt["params"]["sessionId"], "s1");
        assert_eq!(prompt["params"]["prompt"][0]["text"], "hi");
        let chunk = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hel"}}}}"#;
        assert_eq!(t.on_message(&parse(chunk)), vec![Step::Token("Hel".into())]);
        let done = r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#;
        assert_eq!(t.on_message(&parse(done)), vec![Step::Done]);
    }

    #[test]
    fn ignores_notifications_and_other_updates() {
        let mut t = Turn::new(String::new(), "hi".into());
        for line in [
            r#"{"jsonrpc":"2.0","method":"_auth/status_update","params":{}}"#,
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"usage_update","used":1}}}"#,
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"hm"}}}}"#,
        ] {
            assert!(t.on_message(&parse(line)).is_empty(), "{line}");
        }
    }

    #[test]
    fn rejects_permission_by_kind() {
        let mut t = Turn::new(String::new(), "hi".into());
        let req = r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"options":[{"optionId":"allow-once","kind":"allow_once"},{"optionId":"reject","kind":"reject_once"}]}}"#;
        let Step::Send(reply) = &t.on_message(&parse(req))[0] else {
            panic!()
        };
        assert_eq!(reply["id"], 7);
        assert_eq!(
            reply["result"]["outcome"],
            json!({"outcome":"selected","optionId":"reject"})
        );
        let no_reject = r#"{"jsonrpc":"2.0","id":8,"method":"session/request_permission","params":{"options":[]}}"#;
        let Step::Send(reply) = &t.on_message(&parse(no_reject))[0] else {
            panic!()
        };
        assert_eq!(reply["result"]["outcome"], json!({"outcome":"cancelled"}));
    }

    #[test]
    fn refuses_client_methods_and_reports_errors() {
        let mut t = Turn::new(String::new(), "hi".into());
        let req = r#"{"jsonrpc":"2.0","id":9,"method":"fs/read_text_file","params":{}}"#;
        let Step::Send(reply) = &t.on_message(&parse(req))[0] else {
            panic!()
        };
        assert_eq!(reply["error"]["code"], -32601);
        let err = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"auth required"}}"#;
        assert_eq!(
            t.on_message(&parse(err)),
            vec![Step::Fail("auth required".into())]
        );
    }

    #[test]
    fn single_turn_is_sent_verbatim() {
        let (system, prompt) = flatten(&[
            ChatMessage::system("persona"),
            ChatMessage::system("context"),
            ChatMessage::user("hi"),
        ]);
        assert_eq!(system, "persona\n\ncontext");
        assert_eq!(prompt, "hi");
    }

    #[test]
    fn history_is_replayed_in_order() {
        let (system, prompt) = flatten(&[
            ChatMessage::system("persona"),
            ChatMessage::user("hi"),
            ChatMessage::assistant("<tool_call>x</tool_call>"),
            ChatMessage::system("tool result"),
        ]);
        assert_eq!(system, "persona");
        assert!(prompt.ends_with(
            "User: hi\n\nAssistant: <tool_call>x</tool_call>\n\nSystem: tool result\n\n"
        ));
    }

    fn handshake(t: &mut Turn, init: &str) -> (Value, Value) {
        assert_eq!(t.start()["method"], "initialize");
        let Step::Send(new) = t.on_message(&parse(init)).remove(0) else {
            panic!()
        };
        let s = t.on_message(&parse(
            r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#,
        ));
        let Step::Send(prompt) = &s[0] else { panic!() };
        (new, prompt.clone())
    }

    #[test]
    fn generic_agent_gets_system_text_in_the_prompt() {
        let mut t = Turn::new("persona".into(), "hi".into());
        let (new, prompt) = handshake(
            &mut t,
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#,
        );
        assert!(new["params"].get("_meta").is_none());
        assert_eq!(prompt["params"]["prompt"][0]["text"], "persona\n\nhi");
    }

    #[test]
    fn claude_agent_gets_system_prompt_and_options_in_meta() {
        let mut t = Turn::new("persona".into(), "hi".into());
        let init = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"_meta":{"claudeCode":{"promptQueueing":true}}}}}"#;
        let (new, prompt) = handshake(&mut t, init);
        let meta = &new["params"]["_meta"];
        assert_eq!(meta["systemPrompt"], "persona");
        assert_eq!(
            meta["claudeCode"]["options"],
            json!({"settingSources": [], "tools": [], "persistSession": false})
        );
        assert_eq!(prompt["params"]["prompt"][0]["text"], "hi");
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
        let t = Turn::new(String::new(), "hi".into()).in_dir(PathBuf::from("/work/agent"));
        assert_eq!(t.session_new()["cwd"], "/work/agent");
    }
}
