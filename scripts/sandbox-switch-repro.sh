#!/usr/bin/env bash
# Sandbox reproduction driver: start the debug binary inside private sway and
# toggle overlay <-> windowed repeatedly. Prints pid/state after each switch.
# Usage: scripts/sandbox-switch-repro.sh [cycles]
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SB="$ROOT/.sandbox"
CYCLES="${1:-10}"
AGENT="$SB/agent"

export XDG_RUNTIME_DIR="$SB/run"
export WAYLAND_DISPLAY=wayland-1
export SWAYSOCK="$(ls "$SB"/run/sway-ipc.*.sock | head -1)"
export LD_LIBRARY_PATH="$SB/root/usr/lib/x86_64-linux-gnu"
export PATH="$SB/root/usr/bin:$PATH"
export KOMOREBI_AGENT_DIR="$AGENT"
export KOMOREBI_INPUT_REGION=1
export KOMOREBI_LAYER_SHELL=1
export KOMOREBI_START_MODE=overlay
export RUST_LOG=info,komorebi=debug
export DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/komorebi-no-bus
unset DISPLAY

mkdir -p "$AGENT"
rm -f "$AGENT/state.json"
: > "$SB/repro.log"

setsid "$ROOT/src-tauri/target/debug/komorebi" >>"$SB/repro.log" 2>&1 &
APP=$!
echo "$APP" > "$SB/repro.pid"

for _ in $(seq 1 200); do
    [[ -s "$AGENT/state.json" ]] && break
    kill -0 "$APP" 2>/dev/null || { echo "app died during startup"; tail -20 "$SB/repro.log"; exit 1; }
    sleep 0.1
done
echo "app up pid=$APP mode=$(python3 -c "import json;print(json.load(open('$AGENT/state.json'))['mode'])")"

for i in $(seq 1 "$CYCLES"); do
    printf 'eval [...document.querySelectorAll("button")].find(e=>(e.title||"").startsWith("Overlay mode")).click()\n' > "$AGENT/cmd"
    sleep 1.5
    kill -0 "$APP" 2>/dev/null || { echo "cycle $i: app DIED after switch"; tail -30 "$SB/repro.log"; exit 2; }
    echo "cycle $i mode=$(python3 -c "import json;d=json.load(open('$AGENT/state.json'));print(d['mode'], d['window']['size'])")"
done

echo "survived $CYCLES switches; pid=$APP still alive"
grep -c "corrupted double-linked list" "$SB/repro.log" || true
