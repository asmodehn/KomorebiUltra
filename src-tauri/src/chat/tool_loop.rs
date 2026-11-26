//! Tool-call protocol parsing, execution, and the streaming retry loop.

use super::engines::{stream_cloud, stream_local};
use super::events::{emit, ChatEventOut};
use super::ChatService;
use crate::settings;
use komorebi_router::{ChatMessage, Route};
use tauri::{AppHandle, Wry};

#[derive(Debug)]
pub(super) struct ParsedToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

/// Scan `text` for the first `<tool_call>{...}</tool_call>` block.
/// Tolerant of stray whitespace and surrounding `<mood:X>` tags.
pub(super) fn extract_tool_call(text: &str) -> Option<ParsedToolCall> {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    let start = text.find(OPEN)?;
    let after = start + OPEN.len();
    let end_rel = text[after..].find(CLOSE)?;
    let json = text[after..after + end_rel].trim();
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let name = v.get("tool")?.as_str()?.to_string();
    let args = v.get("args").cloned().unwrap_or(serde_json::Value::Null);
    Some(ParsedToolCall { name, args })
}

/// Tools that don't mutate system state and can run without the
/// `desktop_automation_enabled` flag. Mutating tools require it.
fn is_readonly_tool(name: &str) -> bool {
    crate::tools::is_readonly_dispatchable_tool(name)
}

async fn execute_chat_tool(
    app: &AppHandle<Wry>,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let automation = settings::get_desktop_automation_enabled(app);
    if !is_readonly_tool(name) && !automation {
        return serde_json::json!({
            "ok": false,
            "error": "mutating tools require desktop_automation_enabled in settings",
        });
    }
    // Re-use the dispatcher, but bypass the global automation gate for
    // read-only tools by inlining a minimal version here. This keeps
    // run_tool's user-facing semantics (frontend explicit confirmations)
    // intact while letting the chat-pipeline do safe queries silently.
    let call = crate::tools::ToolCall {
        tool: name.to_string(),
        args,
    };
    let result =
        crate::tools::dispatch_inner(app.clone(), call, /*allow_mutating=*/ automation).await;
    serde_json::json!({
        "ok": result.ok,
        "value": result.value,
        "error": result.error,
    })
}

/// Run streaming with tool-call support. Loops up to `MAX_TOOL_ITERATIONS`
/// so the model can chain a tool call → see the result → answer. The
/// tool's output never reaches the spoken/displayed reply directly —
/// only the text after the final iteration is returned.
pub(super) const MAX_TOOL_ITERATIONS: usize = 4;

pub(super) async fn run_with_tools(
    app: &AppHandle<Wry>,
    service: &ChatService,
    id: &str,
    route: Route,
    initial: Vec<ChatMessage>,
) -> Result<String, String> {
    let mut messages = initial;
    let mut last_visible = String::new();
    for iter in 0..MAX_TOOL_ITERATIONS {
        let raw = match route {
            Route::Cloud => stream_cloud(app, service, id, &messages).await?,
            Route::Local => stream_local(app, service, id, &messages).await?,
            Route::Skill => unreachable!("skills don't enter the tool loop"),
        };
        if let Some(call) = extract_tool_call(&raw) {
            tracing::info!(tool = %call.name, "chat: executing tool call");
            // Notify the UI that we're running a tool — purely cosmetic,
            // lets the bubble show a status line. Frontend treats this
            // as a token.
            emit(
                app,
                ChatEventOut::Token {
                    id: id.into(),
                    text: format!("\n<tool_status>{}</tool_status>\n", call.name),
                },
            );
            let result = execute_chat_tool(app, &call.name, call.args.clone()).await;
            // Append assistant's tool-call message + a system message with
            // the result, then loop. The model will continue from there.
            messages.push(ChatMessage::assistant(raw.clone()));
            let result_text = serde_json::to_string(&result).unwrap_or_default();
            messages.push(ChatMessage::system(format!(
                "Tool `{}` returned:\n{}\n\nNow write the final answer for \
                 the user using this result. Do NOT call another tool unless \
                 strictly necessary.",
                call.name, result_text
            )));
            last_visible = raw;
            if iter + 1 == MAX_TOOL_ITERATIONS {
                tracing::warn!("chat: tool loop hit max iterations");
            }
            continue;
        }
        return Ok(raw);
    }
    Ok(last_visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given the chat gate with desktop automation disabled and the dispatchable names of the dev
    ///   catalogue
    /// When the gate decides each dispatchable name, the two observation-only catalogue kinds and
    ///   two absent names
    /// Then each dispatchable `observe` name is allowed and each dispatchable `mutate` name is
    ///   refused
    /// And `cursor:global` and `window:context` stay refused though the catalogue marks them
    ///   `observe`
    /// And absent names stay refused
    /// (These rows assert allow/deny only, not refusal text, dispatch calls or any record field.)
    /// And the seven dispatchable observe names stay allowed and the six mutate names stay refused
    ///   as an explicit retained set, while the two observation kinds and absent names stay refused
    #[test]
    fn gate_allow_set_follows_dispatchable_catalogue_effects() {
        let cat = crate::tools::body_command_catalogue();
        const DISPATCHABLE: [&str; 13] = [
            "active_window",
            "context_snapshot",
            "list_screens",
            "top_processes",
            "click",
            "type",
            "key",
            "move_cursor",
            "write_file",
            "read_file",
            "list_dir",
            "scroll",
            "screen_vision",
        ];
        for name in DISPATCHABLE {
            let entry = cat
                .get(name)
                .unwrap_or_else(|| panic!("{name} missing from catalogue"));
            assert_eq!(
                is_readonly_tool(name),
                entry["mode"].as_str() == Some("observe"),
                "{name}"
            );
        }
        for name in ["cursor:global", "window:context"] {
            assert_eq!(cat[name]["mode"], serde_json::json!("observe"));
            assert!(
                !is_readonly_tool(name),
                "observation kind reached chat dispatcher: {name}"
            );
        }
        for name in ["desktop_click", "not_a_tool"] {
            assert!(!is_readonly_tool(name), "absent name allowed: {name}");
        }
        const OBSERVE_ALLOWED: [&str; 7] = [
            "active_window",
            "context_snapshot",
            "list_screens",
            "top_processes",
            "read_file",
            "list_dir",
            "screen_vision",
        ];
        const MUTATE_DENIED: [&str; 6] = [
            "click",
            "type",
            "key",
            "move_cursor",
            "write_file",
            "scroll",
        ];
        for name in OBSERVE_ALLOWED {
            assert_eq!(cat[name]["mode"], serde_json::json!("observe"), "{name}");
            assert!(
                is_readonly_tool(name),
                "{name}: retained dispatchable observe name refused"
            );
        }
        for name in MUTATE_DENIED {
            assert_eq!(cat[name]["mode"], serde_json::json!("mutate"), "{name}");
            assert!(
                !is_readonly_tool(name),
                "{name}: retained mutating name allowed"
            );
        }
    }

    /// Given one mood-wrapped tool call, one mutating call, one absent name and one truncated block
    /// When `extract_tool_call` scans each text
    /// Then name, args and missing-args Null are unchanged, and malformed text still yields None
    #[test]
    fn tool_call_parser_is_unchanged() {
        let wrapped = extract_tool_call(
            "<mood:thinking><tool_call>{\"tool\":\"cursor:global\",\"args\":{}}</tool_call>",
        )
        .expect("mood-wrapped call");
        assert_eq!(wrapped.name, "cursor:global");
        assert_eq!(wrapped.args, serde_json::json!({}));

        let mutating = extract_tool_call(
            "<tool_call>{\"tool\":\"click\",\"args\":{\"x\":1,\"y\":2}}</tool_call>",
        )
        .expect("mutating call");
        assert_eq!(mutating.name, "click");
        assert_eq!(mutating.args, serde_json::json!({"x": 1, "y": 2}));

        let absent = extract_tool_call("<tool_call>{\"tool\":\"not_a_tool\"}</tool_call>")
            .expect("absent name");
        assert_eq!(absent.name, "not_a_tool");
        assert_eq!(absent.args, serde_json::Value::Null);

        assert!(extract_tool_call("<tool_call>{\"tool\":\"click\"}").is_none());
    }
}
