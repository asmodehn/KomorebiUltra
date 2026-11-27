//! Outbound `chat:*` event protocol.

use komorebi_cloud::{AcpProgress, AcpTurnRecord};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use tauri::{AppHandle, Emitter, Wry};

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatEventOut {
    Started {
        id: String,
        route: String,
    },
    Token {
        id: String,
        text: String,
    },
    Done {
        id: String,
        full_text: String,
    },
    Error {
        id: String,
        message: String,
    },
    Progress {
        id: String,
        #[serde(flatten)]
        progress: AcpProgress,
    },
}

impl ChatEventOut {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Started { id, .. }
            | Self::Token { id, .. }
            | Self::Done { id, .. }
            | Self::Error { id, .. }
            | Self::Progress { id, .. } => id,
        }
    }
}

/// Turn-opening values passed to the one emission boundary. A test supplies
/// explicit literals; production supplies the values recorded when the turn
/// opened. No field carries reply text into the opt-in log.
pub(super) struct EmissionProbeConfig<'a> {
    pub log_path: &'a Path,
    pub id: &'a str,
    pub prompt: &'a str,
    pub ts: &'a str,
    pub context: Value,
    pub begin_ms: u64,
    pub acp: Option<&'a AcpTurnRecord>,
}

/// The single boundary for app delivery and opt-in log observation. Production
/// `emit` delegates here; the injected variant drives the same code with
/// explicit literals, so neither half can be deleted without failing a test.
/// The recorder carries per-turn accumulation across calls; callers seed and
/// store it so progress never leaks into reply/log state.
pub(super) fn emit_boundary_probe(
    config: &EmissionProbeConfig<'_>,
    recorder: &mut super::turn_log::Recorder,
    events: &[(ChatEventOut, u64)],
) -> (Vec<(String, Value)>, Vec<Value>) {
    let mut delivered = Vec::new();
    let mut appended = Vec::new();
    for (evt, now) in events {
        if let Some(mut line) = recorder.on_event(evt, *now) {
            super::turn_log::enrich(&mut line);
            // An empty path is the log-disabled sentinel: delivery still runs.
            if !config.log_path.as_os_str().is_empty() {
                super::turn_log::append(config.log_path, &line);
            }
            appended.push(line);
        }
        let payload = serde_json::to_value(evt).unwrap_or(Value::Null);
        if let Some(app) = super::turn_log::app_handle() {
            if let Err(e) = app.emit("chat", &payload) {
                tracing::warn!(?e, "failed to emit chat event");
            }
        }
        delivered.push(("chat".to_string(), payload));
    }
    (delivered, appended)
}

pub(super) fn emit(app: &AppHandle<Wry>, evt: ChatEventOut) {
    super::turn_log::bind_app(app);
    let state = super::turn_log::emission_state(evt.id());
    let acp = state.acp.clone();
    let config = EmissionProbeConfig {
        log_path: super::turn_log::active_log_path(),
        id: &state.id,
        prompt: &state.prompt,
        ts: &state.ts,
        context: state.context.clone(),
        begin_ms: state.begin_ms,
        acp: acp.as_ref(),
    };
    let mut recorder = super::turn_log::Recorder::default();
    recorder.begin(
        config.id,
        config.prompt,
        config.ts,
        config.context.clone(),
        config.begin_ms,
    );
    if let Some(record) = config.acp {
        recorder.record_acp(config.id, record);
    }
    super::turn_log::seed_recorder(&mut recorder, config.id);
    let _ = emit_boundary_probe(&config, &mut recorder, &[(evt, super::turn_log::now_ms())]);
    super::turn_log::store_recorder(&recorder, config.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi_cloud::AcpProgress;
    use serde_json::{json, Value};
    use std::path::Path;

    fn delivered_from_one(path: &Path, event: ChatEventOut, now: u64) -> Vec<(String, Value)> {
        let _ = std::fs::remove_file(path);
        let config = super::EmissionProbeConfig {
            log_path: path,
            id: "t1",
            prompt: "hi",
            ts: "ts",
            context: json!({"lang": "en"}),
            begin_ms: 1000,
            acp: None,
        };
        let mut recorder = super::super::turn_log::Recorder::default();
        recorder.begin(
            config.id,
            config.prompt,
            config.ts,
            config.context.clone(),
            config.begin_ms,
        );
        let (delivered, _appended) =
            super::emit_boundary_probe(&config, &mut recorder, &[(event, now)]);
        delivered
    }

    /// Given the four existing chat event kinds
    /// When each event is delivered through the emission boundary
    /// Then its delivered chat payload is unchanged (retained)
    /// Given each progress stage for turn t1
    /// When each event is delivered through the same boundary
    /// Then kind is `progress`, id and stage are exact, and no input/output/raw field is present
    /// Given a sequence containing Started, three progress events, Token, Token and Done
    /// When that one sequence is delivered and the opt-in log observes it
    /// Then the delivered payloads are exact and one line is appended to the explicit log path
    /// And that line's reply and streamed fields contain only the answer tokens
    #[test]
    fn existing_event_wire_shape_is_unchanged() {
        let dir =
            std::env::temp_dir().join(format!("komorebi-emission-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let one = dir.join("one.jsonl");

        assert_eq!(
            delivered_from_one(
                &one,
                ChatEventOut::Started {
                    id: "t1".into(),
                    route: "acp".into(),
                },
                1001,
            ),
            vec![(
                "chat".to_string(),
                json!({"kind": "started", "id": "t1", "route": "acp"})
            )]
        );
        assert_eq!(
            delivered_from_one(
                &one,
                ChatEventOut::Token {
                    id: "t1".into(),
                    text: "reply".into(),
                },
                1300,
            ),
            vec![(
                "chat".to_string(),
                json!({"kind": "token", "id": "t1", "text": "reply"})
            )]
        );
        assert_eq!(
            delivered_from_one(
                &one,
                ChatEventOut::Done {
                    id: "t1".into(),
                    full_text: "reply".into(),
                },
                2000,
            ),
            vec![(
                "chat".to_string(),
                json!({"kind": "done", "id": "t1", "full_text": "reply"})
            )]
        );
        assert_eq!(
            delivered_from_one(
                &one,
                ChatEventOut::Error {
                    id: "t1".into(),
                    message: "boom".into(),
                },
                2000,
            ),
            vec![(
                "chat".to_string(),
                json!({"kind": "error", "id": "t1", "message": "boom"})
            )]
        );

        let progress_cases = [
            (
                AcpProgress::Thought {
                    text: "thinking".into(),
                },
                json!({"kind": "progress", "id": "t1", "stage": "thought", "text": "thinking"}),
            ),
            (
                AcpProgress::ToolCall {
                    tool_id: "call-1".into(),
                    title: "Read file".into(),
                    status: Some("pending".into()),
                },
                json!({
                    "kind": "progress",
                    "id": "t1",
                    "stage": "tool_call",
                    "tool_id": "call-1",
                    "title": "Read file",
                    "status": "pending"
                }),
            ),
            (
                AcpProgress::ToolUpdate {
                    tool_id: "call-1".into(),
                    status: "completed".into(),
                },
                json!({
                    "kind": "progress",
                    "id": "t1",
                    "stage": "tool_update",
                    "tool_id": "call-1",
                    "status": "completed"
                }),
            ),
        ];
        for (progress, expected) in progress_cases {
            let delivered = delivered_from_one(
                &one,
                ChatEventOut::Progress {
                    id: "t1".into(),
                    progress,
                },
                1002,
            );
            assert_eq!(delivered, vec![("chat".to_string(), expected.clone())]);
            assert_ne!(delivered[0].1["kind"], "token");
            assert!(delivered[0].1.get("input").is_none());
            assert!(delivered[0].1.get("output").is_none());
            assert!(delivered[0].1.get("raw").is_none());
        }

        let log = dir.join("progress.jsonl");
        let _ = std::fs::remove_file(&log);
        let sequence = vec![
            (
                ChatEventOut::Started {
                    id: "t1".into(),
                    route: "acp".into(),
                },
                1001,
            ),
            (
                ChatEventOut::Progress {
                    id: "t1".into(),
                    progress: AcpProgress::Thought {
                        text: "LOG-SENTINEL-THOUGHT".into(),
                    },
                },
                1002,
            ),
            (
                ChatEventOut::Progress {
                    id: "t1".into(),
                    progress: AcpProgress::ToolCall {
                        tool_id: "call-1".into(),
                        title: "Read file".into(),
                        status: Some("pending".into()),
                    },
                },
                1003,
            ),
            (
                ChatEventOut::Progress {
                    id: "t1".into(),
                    progress: AcpProgress::ToolUpdate {
                        tool_id: "call-1".into(),
                        status: "completed".into(),
                    },
                },
                1004,
            ),
            (
                ChatEventOut::Token {
                    id: "t1".into(),
                    text: "reply ".into(),
                },
                1300,
            ),
            (
                ChatEventOut::Token {
                    id: "t1".into(),
                    text: "text".into(),
                },
                1400,
            ),
            (
                ChatEventOut::Done {
                    id: "t1".into(),
                    full_text: "reply text".into(),
                },
                2000,
            ),
        ];

        let config = super::EmissionProbeConfig {
            log_path: &log,
            id: "t1",
            prompt: "hi",
            ts: "ts",
            context: json!({"lang": "en"}),
            begin_ms: 1000,
            acp: None,
        };
        let mut recorder = super::super::turn_log::Recorder::default();
        recorder.begin(
            config.id,
            config.prompt,
            config.ts,
            config.context.clone(),
            config.begin_ms,
        );
        let (delivered, appended) = super::emit_boundary_probe(&config, &mut recorder, &sequence);

        let expected_delivered = vec![
            (
                "chat".to_string(),
                json!({"kind": "started", "id": "t1", "route": "acp"}),
            ),
            (
                "chat".to_string(),
                json!({"kind": "progress", "id": "t1", "stage": "thought", "text": "LOG-SENTINEL-THOUGHT"}),
            ),
            (
                "chat".to_string(),
                json!({"kind": "progress", "id": "t1", "stage": "tool_call", "tool_id": "call-1", "title": "Read file", "status": "pending"}),
            ),
            (
                "chat".to_string(),
                json!({"kind": "progress", "id": "t1", "stage": "tool_update", "tool_id": "call-1", "status": "completed"}),
            ),
            (
                "chat".to_string(),
                json!({"kind": "token", "id": "t1", "text": "reply "}),
            ),
            (
                "chat".to_string(),
                json!({"kind": "token", "id": "t1", "text": "text"}),
            ),
            (
                "chat".to_string(),
                json!({"kind": "done", "id": "t1", "full_text": "reply text"}),
            ),
        ];
        assert_eq!(delivered, expected_delivered);

        let file = std::fs::read_to_string(&log).unwrap();
        assert_eq!(appended.len(), 1);
        let line: Value = serde_json::from_str(file.trim()).unwrap();
        assert_eq!(line, appended[0]);
        assert_eq!(line["ts"], "ts");
        assert_eq!(line["id"], "t1");
        assert_eq!(line["prompt"], "hi");
        assert_eq!(line["route"], "acp");
        assert_eq!(line["context"], json!({"lang": "en"}));
        assert_eq!(line["reply"], "reply text");
        assert_eq!(line["streamed"], "reply text");
        assert_eq!(line["first_token_ms"], 300);
        assert_eq!(line["total_ms"], 1000);
        assert!(line.get("error").is_none());
        assert!(line.get("progress").is_none());
        assert!(!file.contains("LOG-SENTINEL-THOUGHT"));
        assert!(!file.contains("Read file"));
        assert!(!file.contains("call-1"));
        assert!(!file.contains("completed"));
    }
}
