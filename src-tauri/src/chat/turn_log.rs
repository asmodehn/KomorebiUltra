//! Opt-in raw turn log for reviewing replies (ADR 0007).
//!
//! Off unless the `KOMOREBI_TURN_LOG` environment variable names a file.
//! Each finished turn appends one JSON line with the question, the raw
//! reply, timings and the per-turn context a reviewer cannot derive from
//! the code. The app never reads or uploads it.

use super::events::ChatEventOut;
use super::speak::sanitize_for_tts;
use super::ChatService;
use crate::settings;
use komorebi_cloud::AcpTurnRecord;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, Wry};

const ENV_VAR: &str = "KOMOREBI_TURN_LOG";

fn path() -> Option<&'static PathBuf> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var_os(ENV_VAR)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
    .as_ref()
}

fn recorder() -> &'static Mutex<Recorder> {
    static RECORDER: OnceLock<Mutex<Recorder>> = OnceLock::new();
    RECORDER.get_or_init(|| Mutex::new(Recorder::default()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn append(path: &PathBuf, line: &Value) {
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        // One write per line, so overlapping turns cannot interleave.
        .and_then(|mut f| f.write_all(format!("{line}\n").as_bytes()));
    if let Err(e) = result {
        tracing::warn!(?e, ?path, "turn log write failed");
    }
}

/// Opens a turn record. Called before the generation is spawned, so the
/// record exists before its first event.
pub(super) fn begin(app: &AppHandle<Wry>, id: &str, prompt: &str, has_image: bool) {
    if path().is_none() {
        return;
    }
    let context = json!({
        "lang": settings::resolve_language(app),
        "stage": crate::relationship::load(app).stage.label(),
        "tools_enabled": settings::get_chat_tool_calls_enabled(app),
        "rag_enabled": settings::get_rag_enabled(app),
        "tts_enabled": settings::get_tts_enabled(app),
        "has_image": has_image,
    });
    let ts = chrono::Utc::now().to_rfc3339();
    if let Ok(mut r) = recorder().lock() {
        r.begin(id, prompt, &ts, context, now_ms());
    }
}

/// Attaches the exact outbound ACP text to an open turn. A request that fails
/// before this point has no record to write.
pub(super) fn record_acp(id: &str, record: &AcpTurnRecord) {
    if path().is_none() {
        return;
    }
    if let Ok(mut r) = recorder().lock() {
        r.record_acp(id, record);
    }
}

/// Feeds one outbound chat event; writes the line when the turn ends.
pub(super) fn observe(app: &AppHandle<Wry>, evt: &ChatEventOut) {
    let Some(path) = path() else { return };
    let finished = match recorder().lock() {
        Ok(mut r) => r.on_event(evt, now_ms()),
        Err(_) => None,
    };
    let Some(mut line) = finished else { return };
    line["model_label"] = json!(model_label(app, &line));
    line["cancelled"] = json!(app
        .try_state::<Arc<ChatService>>()
        .map(|s| s.cancel.load(Ordering::SeqCst))
        .unwrap_or(false));
    if let Some(reply) = line["reply"].as_str() {
        line["spoken"] = json!(sanitize_for_tts(reply));
    }
    append(path, &line);
}

/// Marks where the conversation history was cleared.
pub(super) fn reset() {
    if let Some(path) = path() {
        append(
            path,
            &json!({ "ts": chrono::Utc::now().to_rfc3339(), "event": "reset" }),
        );
    }
}

/// Settings the label depends on, read once so the mapping itself is testable.
struct Labels<'a> {
    has_image: bool,
    coach_model: &'a str,
    openrouter_model: &'a str,
    local_model_path: &'a str,
    acp_command: Option<&'a str>,
}

fn model_label(app: &AppHandle<Wry>, line: &Value) -> String {
    let coach_model = settings::get_game_coach_model(app);
    let openrouter_model = settings::get_openrouter_model(app);
    let local_model_path = settings::get_local_model_path(app).unwrap_or_default();
    let acp_command = settings::get_acp_agent_command(app);
    label_for(
        line["route"].as_str(),
        &Labels {
            has_image: line["context"]["has_image"].as_bool() == Some(true),
            coach_model: &coach_model,
            openrouter_model: &openrouter_model,
            local_model_path: &local_model_path,
            acp_command: acp_command.as_deref(),
        },
    )
}

/// Which engine answered, as the log records it. The route is the authority:
/// an agent turn says `acp` even if the command was cleared afterwards.
fn label_for(route: Option<&str>, l: &Labels<'_>) -> String {
    let file_name = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
    if l.has_image {
        return format!("openrouter:{}", l.coach_model);
    }
    match route {
        Some("acp") => match l.acp_command {
            Some(cmd) => format!(
                "acp:{}",
                file_name(cmd.split_whitespace().next().unwrap_or(""))
            ),
            None => "acp".to_string(),
        },
        Some("cloud") => format!("openrouter:{}", l.openrouter_model),
        Some("local") => format!("local:{}", file_name(l.local_model_path)),
        Some(other) => other.to_string(),
        None => "?".to_string(),
    }
}

#[derive(Default)]
struct Recorder {
    pending: HashMap<String, Pending>,
}

struct Pending {
    line: Value,
    started_ms: u64,
    first_token_ms: Option<u64>,
    streamed: String,
    acp: Option<AcpTurnRecord>,
}

impl Recorder {
    /// Held until Done: a failed request must not leave metadata on its line.
    fn record_acp(&mut self, id: &str, record: &AcpTurnRecord) {
        if let Some(p) = self.pending.get_mut(id) {
            p.acp = Some(record.clone());
        }
    }

    fn begin(&mut self, id: &str, prompt: &str, ts: &str, context: Value, now: u64) {
        let line = json!({
            "ts": ts,
            "id": id,
            "prompt": prompt,
            "route": null,
            "context": context,
        });
        self.pending.insert(
            id.to_string(),
            Pending {
                line,
                started_ms: now,
                first_token_ms: None,
                streamed: String::new(),
                acp: None,
            },
        );
    }

    fn on_event(&mut self, evt: &ChatEventOut, now: u64) -> Option<Value> {
        match evt {
            // A skill falling back to the LLM emits Started twice; the
            // last route is the one that answered.
            ChatEventOut::Started { id, route } => {
                if let Some(p) = self.pending.get_mut(id) {
                    p.line["route"] = json!(route);
                }
                None
            }
            ChatEventOut::Token { id, text } => {
                if let Some(p) = self.pending.get_mut(id) {
                    p.first_token_ms.get_or_insert(now);
                    p.streamed.push_str(text);
                }
                None
            }
            ChatEventOut::Done { id, full_text } => {
                let mut p = self.pending.remove(id)?;
                p.line["reply"] = json!(full_text);
                if let Some(record) = p.acp.take() {
                    p.line["session_prompt_text"] = json!(record.session_prompt_text);
                    p.line["system_roles"] = json!(record.system_roles);
                    p.line["system_message_count"] = json!(record.system_message_count);
                }
                Some(p.finish(now))
            }
            ChatEventOut::Error { id, message } => {
                let mut p = self.pending.remove(id)?;
                p.line["error"] = json!(message);
                Some(p.finish(now))
            }
        }
    }
}

impl Pending {
    fn finish(mut self, now: u64) -> Value {
        // `streamed` is what the bubble received, tool rounds included;
        // `reply` is only the final answer kept in history.
        self.line["streamed"] = json!(self.streamed);
        // Wall-clock millis can step backwards; never panic over it.
        self.line["first_token_ms"] = json!(self
            .first_token_ms
            .map(|t| t.saturating_sub(self.started_ms)));
        self.line["total_ms"] = json!(now.saturating_sub(self.started_ms));
        self.line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi_cloud::AcpTurnRecord;
    use komorebi_router::Role;

    const WINDOW_LINE: &str = "- window context: available source=harness-2 at=1900 app=org.example.Viewer x=200 y=300 w=640 h=480 state=background";
    const ACP_FIRST_TEXT_A: &str = "persona\n\ncontext\n\n- window context: available source=harness-2 at=1900 app=org.example.Viewer x=200 y=300 w=640 h=480 state=background\n\nhi";
    const ACP_FIRST_TEXT_B: &str = "context\n\n- window context: available source=harness-2 at=1900 app=org.example.Viewer x=200 y=300 w=640 h=480 state=background\n\nhi";

    fn acp_first_record() -> AcpTurnRecord {
        AcpTurnRecord {
            session_prompt_text: ACP_FIRST_TEXT_A.to_string(),
            system_roles: vec![Role::System, Role::System, Role::System],
            system_message_count: 3,
        }
    }

    fn started(id: &str, route: &str) -> ChatEventOut {
        ChatEventOut::Started {
            id: id.into(),
            route: route.into(),
        }
    }

    fn token(id: &str, text: &str) -> ChatEventOut {
        ChatEventOut::Token {
            id: id.into(),
            text: text.into(),
        }
    }

    #[test]
    fn labels_each_route_with_the_engine_that_answered() {
        let l = Labels {
            has_image: false,
            coach_model: "coach/model",
            openrouter_model: "openrouter/model",
            local_model_path: "/models/qwen.gguf",
            acp_command: Some("/usr/bin/opencode acp --flag"),
        };
        assert_eq!(label_for(Some("acp"), &l), "acp:opencode");
        assert_eq!(label_for(Some("cloud"), &l), "openrouter:openrouter/model");
        assert_eq!(label_for(Some("local"), &l), "local:qwen.gguf");
        assert_eq!(label_for(Some("skill"), &l), "skill");
        assert_eq!(label_for(None, &l), "?");

        let image = Labels {
            has_image: true,
            ..l
        };
        assert_eq!(label_for(Some("acp"), &image), "openrouter:coach/model");
    }

    #[test]
    fn an_agent_turn_keeps_its_route_when_the_command_is_gone() {
        let l = Labels {
            has_image: false,
            coach_model: "coach/model",
            openrouter_model: "openrouter/model",
            local_model_path: "",
            acp_command: None,
        };
        assert_eq!(label_for(Some("acp"), &l), "acp");
    }

    /// Given opted-in non-ACP turns on the skill, local and cloud routes
    /// When each turn-log line is finished
    /// Then ts, id, prompt, route, context, reply, streamed, first_token_ms and total_ms are unchanged (retained)
    /// And the route-label mapping for local, cloud and skill is unchanged (retained)
    /// And none of session_prompt_text, system_roles or system_message_count is present on any line
    #[test]
    fn records_a_turn_from_begin_to_done() {
        let mut r = Recorder::default();
        r.begin(
            "t1",
            "why is the sky blue?",
            "ts",
            json!({"lang": "en"}),
            1000,
        );
        assert!(r.on_event(&started("t1", "skill"), 1001).is_none());
        assert!(r.on_event(&started("t1", "cloud"), 1002).is_none());
        assert!(r
            .on_event(&token("t1", "<mood:happy>Light "), 1300)
            .is_none());
        assert!(r.on_event(&token("t1", "scatters."), 1400).is_none());
        let line = r
            .on_event(
                &ChatEventOut::Done {
                    id: "t1".into(),
                    full_text: "<mood:happy>Light scatters.".into(),
                },
                2000,
            )
            .expect("done finishes the turn");
        assert_eq!(line["prompt"], "why is the sky blue?");
        assert_eq!(line["route"], "cloud");
        assert_eq!(line["context"]["lang"], "en");
        assert_eq!(line["reply"], "<mood:happy>Light scatters.");
        assert_eq!(line["streamed"], "<mood:happy>Light scatters.");
        assert_eq!(line["first_token_ms"], 300);
        assert_eq!(line["total_ms"], 1000);
        assert!(line.get("error").is_none());
        assert!(line.get("session_prompt_text").is_none());
        assert!(line.get("system_roles").is_none());
        assert!(line.get("system_message_count").is_none());
        assert!(r.pending.is_empty());

        r.begin("t2", "local prompt", "ts", json!({}), 1000);
        assert!(r.on_event(&started("t2", "local"), 1001).is_none());
        assert!(r.on_event(&token("t2", "local reply"), 1300).is_none());
        let local = r
            .on_event(
                &ChatEventOut::Done {
                    id: "t2".into(),
                    full_text: "local reply".into(),
                },
                2000,
            )
            .expect("done finishes the local turn");
        assert_eq!(local["prompt"], "local prompt");
        assert_eq!(local["route"], "local");
        assert_eq!(local["reply"], "local reply");
        assert_eq!(local["streamed"], "local reply");
        assert_eq!(local["first_token_ms"], 300);
        assert_eq!(local["total_ms"], 1000);
        assert!(local.get("error").is_none());
        assert!(local.get("session_prompt_text").is_none());
        assert!(local.get("system_roles").is_none());
        assert!(local.get("system_message_count").is_none());

        r.begin("t3", "skill prompt", "ts", json!({}), 1000);
        assert!(r.on_event(&started("t3", "skill"), 1001).is_none());
        assert!(r.on_event(&token("t3", "skill reply"), 1300).is_none());
        let skill = r
            .on_event(
                &ChatEventOut::Done {
                    id: "t3".into(),
                    full_text: "skill reply".into(),
                },
                2000,
            )
            .expect("done finishes the skill turn");
        assert_eq!(skill["prompt"], "skill prompt");
        assert_eq!(skill["route"], "skill");
        assert_eq!(skill["reply"], "skill reply");
        assert_eq!(skill["streamed"], "skill reply");
        assert_eq!(skill["first_token_ms"], 300);
        assert_eq!(skill["total_ms"], 1000);
        assert!(skill.get("error").is_none());
        assert!(skill.get("session_prompt_text").is_none());
        assert!(skill.get("system_roles").is_none());
        assert!(skill.get("system_message_count").is_none());
        assert!(r.pending.is_empty());
    }

    /// Given an ACP first turn and a later turn on a reused session, each carrying a literal
    ///   expected text, role list and count for its leading system messages, with the bounded
    ///   window line present in the text
    /// When each finished turn-log line is written
    /// Then the line records that exact text as top-level session_prompt_text and the literal roles and count at top level
    /// And the window line is verbatim in both texts, with no `id=`, `output=` or `workspace=` token
    /// And the persona text is present in the first line only, while both role lists still contain the persona message
    /// And all pre-existing turn-log fields and the acp route label are unchanged (retained)
    #[test]
    fn records_the_exact_acp_turn_metadata() {
        let mut r = Recorder::default();
        let first = AcpTurnRecord {
            session_prompt_text: ACP_FIRST_TEXT_A.to_string(),
            system_roles: vec![Role::System, Role::System, Role::System],
            system_message_count: 3,
        };
        r.begin("t1", "hi", "ts", json!({"lang": "en"}), 1000);
        r.record_acp("t1", &first);
        assert!(r.on_event(&started("t1", "acp"), 1001).is_none());
        assert!(r.on_event(&token("t1", "reply "), 1300).is_none());
        assert!(r.on_event(&token("t1", "text"), 1400).is_none());
        let line = r
            .on_event(
                &ChatEventOut::Done {
                    id: "t1".into(),
                    full_text: "reply text".into(),
                },
                2000,
            )
            .expect("done finishes the turn");
        assert_eq!(line["session_prompt_text"], ACP_FIRST_TEXT_A);
        assert!(line["session_prompt_text"]
            .as_str()
            .unwrap()
            .contains(WINDOW_LINE));
        for token in ["id=", "output=", "workspace="] {
            assert!(
                !line["session_prompt_text"]
                    .as_str()
                    .unwrap()
                    .contains(token),
                "ACP turn log must not carry {token}"
            );
        }
        assert_eq!(line["system_roles"], json!(["system", "system", "system"]));
        assert_eq!(line["system_message_count"], 3);
        assert_eq!(line["prompt"], "hi");
        assert_eq!(line["route"], "acp");
        assert_eq!(line["context"]["lang"], "en");
        assert_eq!(line["reply"], "reply text");
        assert_eq!(line["streamed"], "reply text");
        assert_eq!(line["first_token_ms"], 300);
        assert_eq!(line["total_ms"], 1000);
        assert!(line.get("error").is_none());
        assert!(r.pending.is_empty());

        let later = AcpTurnRecord {
            session_prompt_text: ACP_FIRST_TEXT_B.to_string(),
            system_roles: vec![Role::System, Role::System, Role::System],
            system_message_count: 3,
        };
        r.begin("t2", "hi", "ts", json!({}), 1000);
        r.record_acp("t2", &later);
        assert!(r.on_event(&started("t2", "acp"), 1001).is_none());
        assert!(r.on_event(&token("t2", "reply "), 1300).is_none());
        assert!(r.on_event(&token("t2", "text"), 1400).is_none());
        let line = r
            .on_event(
                &ChatEventOut::Done {
                    id: "t2".into(),
                    full_text: "reply text".into(),
                },
                2000,
            )
            .expect("done finishes the later turn");
        assert_eq!(line["session_prompt_text"], ACP_FIRST_TEXT_B);
        assert!(line["session_prompt_text"]
            .as_str()
            .unwrap()
            .contains(WINDOW_LINE));
        for token in ["id=", "output=", "workspace="] {
            assert!(
                !line["session_prompt_text"]
                    .as_str()
                    .unwrap()
                    .contains(token),
                "ACP turn log must not carry {token}"
            );
        }
        assert_eq!(line["system_roles"], json!(["system", "system", "system"]));
        assert_eq!(line["system_message_count"], 3);
        assert_eq!(line["prompt"], "hi");
        assert_eq!(line["route"], "acp");
        assert_eq!(line["reply"], "reply text");
        assert_eq!(line["streamed"], "reply text");
        assert_eq!(line["first_token_ms"], 300);
        assert_eq!(line["total_ms"], 1000);
        assert!(line.get("error").is_none());
        assert!(r.pending.is_empty());
    }

    /// Given an ACP turn whose request fails after its record was attached, an ACP turn
    ///   that fails before Started, and an unknown turn id with an attached record
    /// When error events and an unknown-turn token are observed
    /// Then no session_prompt_text, system_roles or system_message_count field is written to any line
    /// And error/first_token_ms/reply and unknown-turn handling are unchanged (retained)
    #[test]
    fn records_errors_and_ignores_unknown_turns() {
        let mut r = Recorder::default();
        r.record_acp("ghost", &acp_first_record());
        assert!(r.on_event(&token("ghost", "x"), 5).is_none());
        r.begin("t2", "hi", "ts", json!({}), 0);
        assert!(r.on_event(&started("t2", "acp"), 1).is_none());
        r.record_acp("t2", &acp_first_record());
        let line = r
            .on_event(
                &ChatEventOut::Error {
                    id: "t2".into(),
                    message: "acp agent: boom".into(),
                },
                50,
            )
            .expect("error finishes the turn");
        assert_eq!(line["error"], "acp agent: boom");
        assert_eq!(line["first_token_ms"], Value::Null);
        assert!(line.get("reply").is_none());
        assert!(line.get("session_prompt_text").is_none());
        assert!(line.get("system_roles").is_none());
        assert!(line.get("system_message_count").is_none());
        assert!(r.pending.is_empty());

        r.begin("t3", "hi", "ts", json!({}), 0);
        r.record_acp("t3", &acp_first_record());
        let early = r
            .on_event(
                &ChatEventOut::Error {
                    id: "t3".into(),
                    message: "acp agent: boom".into(),
                },
                50,
            )
            .expect("error before Started finishes the turn");
        assert_eq!(early["error"], "acp agent: boom");
        assert_eq!(early["first_token_ms"], Value::Null);
        assert!(early.get("reply").is_none());
        assert!(early.get("session_prompt_text").is_none());
        assert!(early.get("system_roles").is_none());
        assert!(early.get("system_message_count").is_none());
        assert!(r.pending.is_empty());
    }
}
