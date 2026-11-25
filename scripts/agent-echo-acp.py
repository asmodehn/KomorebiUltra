#!/usr/bin/env python3
"""Dev-only ACP agent that echoes each prompt to stdout and a log file.

Used by the headless window-context check. It implements only the handshake and
prompt messages Komorebi's ACP client sends; it has no tools and starts no
model. `KOMOREBI_ECHO_OUT` names a file where each turn's prompt is appended,
separated by a `---` line, so a harness check can read the exact context the
agent received.
"""

import json
import os
import sys


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


for raw in sys.stdin:
    try:
        msg = json.loads(raw)
    except json.JSONDecodeError:
        continue
    if not isinstance(msg, dict):
        continue

    method = msg.get("method")
    msg_id = msg.get("id")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"protocolVersion": 1},
        })
    elif method == "session/new":
        send({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"sessionId": "echo-session"},
        })
    elif method == "session/prompt":
        params = msg.get("params") or {}
        parts = params.get("prompt") or []
        text = "".join(
            part.get("text", "")
            for part in parts
            if isinstance(part, dict) and part.get("type") == "text"
        )
        out = os.environ.get("KOMOREBI_ECHO_OUT")
        if out:
            with open(out, "a", encoding="utf-8") as log:
                log.write(text + "\n---\n")
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": params.get("sessionId", "echo-session"),
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": text},
                },
            },
        })
        send({"jsonrpc": "2.0", "id": msg_id, "result": {}})
