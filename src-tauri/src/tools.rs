//! LLM-facing tool dispatcher.
//!
//! The chat pipeline instructs the model to call desktop primitives by
//! emitting a small JSON object like `{"tool":"desktop_click", "args":{...}}`.
//! This module provides a single `run_tool` command the frontend can call
//! to execute one of those calls after user confirmation (when the user
//! enables desktop automation in settings). Centralising dispatch here
//! also gives us one place to apply permission checks.

use crate::{desktop_cmds, settings};
use komorebi_cloud::OpenRouterClient;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Wry};

#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ToolResult {
    pub ok: bool,
    pub value: serde_json::Value,
    pub error: Option<String>,
}

impl ToolResult {
    fn ok(value: serde_json::Value) -> Self {
        Self {
            ok: true,
            value,
            error: None,
        }
    }
    fn err(e: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: serde_json::Value::Null,
            error: Some(e.into()),
        }
    }
}

/// The one declaration of the body-command vocabulary. The dev harness reports
/// it; the chat gate and a future MCP adapter read the same names and effects.
/// Building this record only describes commands: it calls no desktop primitive
/// and grants no ability to run a mutating one. Mutating access stays unbuilt
/// until an explicit permissions system with grants and an audit record exists;
/// a settings switch is not that system.
struct BodyCommand {
    name: &'static str,
    mode: &'static str,
    dispatchable: bool,
    description: &'static str,
    args: &'static [(&'static str, &'static str)],
    // False for filesystem and terminal commands: ADR 0009 and ADR 0010
    // forbid advertising them through Komorebi. The policy lives on the one
    // command declaration, not in a second list kept by the MCP projection.
    mcp: bool,
}

const BODY_COMMANDS: [BodyCommand; 15] = [
    BodyCommand {
        name: "active_window",
        mcp: false,
        mode: "observe",
        dispatchable: true,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "context_snapshot",
        mcp: false,
        mode: "observe",
        dispatchable: true,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "list_screens",
        mcp: false,
        mode: "observe",
        dispatchable: true,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "top_processes",
        mcp: false,
        mode: "observe",
        dispatchable: true,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "read_file",
        mcp: false,
        mode: "observe",
        dispatchable: true,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "list_dir",
        mcp: false,
        mode: "observe",
        dispatchable: true,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "screen_vision",
        mcp: false,
        mode: "observe",
        dispatchable: true,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "cursor:global",
        mcp: false,
        mode: "observe",
        dispatchable: false,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "window:context",
        mcp: false,
        mode: "observe",
        dispatchable: false,
        description: "",
        args: &[],
    },
    BodyCommand {
        name: "click",
        mcp: true,
        mode: "mutate",
        dispatchable: true,
        description: "Click at a screen position",
        args: &[
            ("x", "integer"),
            ("y", "integer"),
            ("button", "string"),
            ("double", "boolean"),
        ],
    },
    BodyCommand {
        name: "type",
        mcp: true,
        mode: "mutate",
        dispatchable: true,
        description: "Type text into the focused window",
        args: &[("text", "string")],
    },
    BodyCommand {
        name: "key",
        mcp: true,
        mode: "mutate",
        dispatchable: true,
        description: "Send a key chord to the focused window",
        args: &[("chord", "string")],
    },
    BodyCommand {
        name: "move_cursor",
        mcp: true,
        mode: "mutate",
        dispatchable: true,
        description: "Move the pointer to a screen position",
        args: &[("x", "integer"), ("y", "integer")],
    },
    BodyCommand {
        name: "write_file",
        mcp: false,
        mode: "mutate",
        dispatchable: true,
        description: "Write a file inside the workspace root",
        args: &[("rel_path", "string"), ("contents", "string")],
    },
    BodyCommand {
        name: "scroll",
        mcp: true,
        mode: "mutate",
        dispatchable: true,
        description: "Scroll the focused window",
        args: &[("delta", "integer"), ("horizontal", "boolean")],
    },
];

pub(crate) fn body_command_catalogue() -> serde_json::Value {
    let mut commands = serde_json::Map::new();
    for cmd in &BODY_COMMANDS {
        let entry = if cmd.mode == "observe" {
            serde_json::json!({
                "mode": "observe",
                "available": true,
                "reason": serde_json::Value::Null,
            })
        } else {
            serde_json::json!({
                "mode": "mutate",
                "available": false,
                "reason": "desktop_automation_enabled is not a permissions system; mutating ports stay unbuilt until explicit grants and audit exist",
            })
        };
        commands.insert(cmd.name.to_string(), entry);
    }
    serde_json::Value::Object(commands)
}

/// Test-only seam: read one command's description from the one declaration.
/// Compiled only for tests; changes no published record and no production API.
#[cfg(test)]
pub(crate) fn test_body_command_description(name: &str) -> &'static str {
    BODY_COMMANDS
        .iter()
        .find(|cmd| cmd.name == name)
        .expect("named body command exists")
        .description
}

/// The MCP-facing schema declaration. It describes the mutating path only:
/// observation travels Komorebi -> agent over ACP (ADR 0010 rule 8). Filesystem
/// and terminal commands are excluded on the declaration itself (ADR 0009/0010);
/// declaring a schema grants no ability to execute it.
pub(crate) fn mcp_tool_schemas() -> serde_json::Value {
    let catalogue = body_command_catalogue();
    let tools: Vec<serde_json::Value> = BODY_COMMANDS
        .iter()
        .filter(|cmd| cmd.mode == "mutate" && cmd.mcp)
        .map(|cmd| {
            let mut properties = serde_json::Map::new();
            for (arg, ty) in cmd.args {
                properties.insert((*arg).to_string(), serde_json::json!({ "type": ty }));
            }
            serde_json::json!({
                "name": cmd.name,
                "description": cmd.description,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                },
                "available": catalogue[cmd.name]["available"],
                "reason": catalogue[cmd.name]["reason"],
            })
        })
        .collect();
    serde_json::json!({ "tools": tools })
}

/// Whether a named command is a read-only tool the chat dispatcher can run.
/// Observation-only catalogue kinds stay here as declarations, not tool calls.
pub(crate) fn is_readonly_dispatchable_tool(name: &str) -> bool {
    BODY_COMMANDS
        .iter()
        .any(|cmd| cmd.name == name && cmd.dispatchable && cmd.mode == "observe")
}

/// Execute one tool call. Frontend is responsible for gating behind user
/// confirmation when `desktop_automation_enabled` is false.
#[tauri::command]
pub async fn run_tool(app: AppHandle<Wry>, call: ToolCall) -> ToolResult {
    let enabled = settings::get_desktop_automation_enabled(&app);
    if !enabled {
        return ToolResult::err("desktop automation disabled in settings");
    }
    dispatch_inner(app, call, /*allow_mutating=*/ true).await
}

/// Tool-dispatch core. Pulled out so the chat pipeline can re-use the same
/// switch table without going through the public command's automation
/// gate. `allow_mutating=false` rejects state-changing tools (click,
/// type, key, scroll, write_file).
pub async fn dispatch_inner(
    app: AppHandle<Wry>,
    call: ToolCall,
    allow_mutating: bool,
) -> ToolResult {
    macro_rules! mutating {
        () => {
            if !allow_mutating {
                return ToolResult::err("mutating tools require desktop_automation_enabled");
            }
        };
    }
    match call.tool.as_str() {
        "active_window" => ToolResult::ok(desktop_cmds::desktop_active_window()),
        "context_snapshot" => {
            match serde_json::to_value(desktop_cmds::desktop_context_snapshot()) {
                Ok(v) => ToolResult::ok(v),
                Err(e) => ToolResult::err(e.to_string()),
            }
        }
        "list_screens" => match desktop_cmds::desktop_list_screens() {
            Ok(v) => ToolResult::ok(v),
            Err(e) => ToolResult::err(e),
        },
        "top_processes" => {
            let limit = call
                .args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            match desktop_cmds::desktop_top_processes(limit) {
                Ok(v) => ToolResult::ok(v),
                Err(e) => ToolResult::err(e),
            }
        }
        "click" => {
            mutating!();
            match serde_json::from_value::<desktop_cmds::ClickArgs>(call.args) {
                Ok(a) => match desktop_cmds::desktop_click(a) {
                    Ok(_) => ToolResult::ok(serde_json::Value::Bool(true)),
                    Err(e) => ToolResult::err(e),
                },
                Err(e) => ToolResult::err(e.to_string()),
            }
        }
        "type" => {
            mutating!();
            let text = call
                .args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            match desktop_cmds::desktop_type(text) {
                Ok(_) => ToolResult::ok(serde_json::Value::Bool(true)),
                Err(e) => ToolResult::err(e),
            }
        }
        "key" => {
            mutating!();
            let chord = call
                .args
                .get("chord")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            match desktop_cmds::desktop_key(chord) {
                Ok(_) => ToolResult::ok(serde_json::Value::Bool(true)),
                Err(e) => ToolResult::err(e),
            }
        }
        "move_cursor" => {
            mutating!();
            let x = call.args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = call.args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            match desktop_cmds::desktop_move_cursor(x, y) {
                Ok(_) => ToolResult::ok(serde_json::Value::Bool(true)),
                Err(e) => ToolResult::err(e),
            }
        }
        "write_file" => {
            mutating!();
            match serde_json::from_value::<desktop_cmds::WriteFileArgs>(call.args) {
                Ok(a) => match desktop_cmds::desktop_write_file(app, a) {
                    Ok(p) => ToolResult::ok(serde_json::Value::String(p)),
                    Err(e) => ToolResult::err(e),
                },
                Err(e) => ToolResult::err(e.to_string()),
            }
        }
        "read_file" => {
            let rel = call
                .args
                .get("rel_path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            match desktop_cmds::desktop_read_file(app, rel) {
                Ok(s) => ToolResult::ok(serde_json::Value::String(s)),
                Err(e) => ToolResult::err(e),
            }
        }
        "list_dir" => {
            let rel = call
                .args
                .get("rel_path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            match desktop_cmds::desktop_list_dir(app, rel) {
                Ok(v) => ToolResult::ok(serde_json::to_value(v).unwrap_or(serde_json::Value::Null)),
                Err(e) => ToolResult::err(e),
            }
        }
        "scroll" => {
            mutating!();
            let delta = call.args.get("delta").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let horizontal = call.args.get("horizontal").and_then(|v| v.as_bool());
            match desktop_cmds::desktop_scroll(delta, horizontal) {
                Ok(_) => ToolResult::ok(serde_json::Value::Bool(true)),
                Err(e) => ToolResult::err(e),
            }
        }
        // Vision tool: lets the LLM "look" at the user's screen and return
        // a textual description. Args: { question: string, monitor?: usize }.
        // Requires an OpenRouter key (uses the configured Game Coach
        // vision model by default).
        "screen_vision" => {
            let question = call
                .args
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("Describe what is currently on the screen.")
                .to_string();
            let monitor = call
                .args
                .get("monitor")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);
            let key = match settings::get_openrouter_key(&app) {
                Some(k) => k,
                None => return ToolResult::err("OpenRouter API key required for vision"),
            };
            let model = settings::get_game_coach_model(&app);
            let bytes = match tokio::task::spawn_blocking(move || {
                komorebi_desktop::capture::capture_screen(monitor)
            })
            .await
            {
                Ok(Ok(b)) => b,
                Ok(Err(e)) => return ToolResult::err(e.to_string()),
                Err(e) => return ToolResult::err(e.to_string()),
            };
            let small = crate::chat::downscale_for_vision(&bytes, 1280).unwrap_or(bytes);
            let client = match OpenRouterClient::new(key) {
                Ok(c) => c,
                Err(e) => return ToolResult::err(e.to_string()),
            };
            match client
                .complete_vision(
                    &model,
                    "You are looking at a screenshot of the user's screen. \
                     Answer briefly and factually in the user's language.",
                    &question,
                    &small,
                    300,
                )
                .await
            {
                Ok(text) => ToolResult::ok(serde_json::Value::String(text)),
                Err(e) => ToolResult::err(e.to_string()),
            }
        }
        other => ToolResult::err(format!("unknown tool: {other}")),
    }
}
