#!/usr/bin/env bash
# Wait for the agent frame harness (or a specific overlay mode) with a hard
# deadline. Never wait on log lines: logs miss events and race with restarts.
#
#   scripts/agent-wait.sh <harness-dir> [--mode overlay] [--timeout 60]
#                                      [--fresh 3] [--log <app-log>]
#                                      [--after <marker-file>]
#
# Exits 0 when the app is up (state.json fresh) and, if --mode is given, in
# that mode. `--after` additionally rejects a state.json written before the
# marker file, so an orphaned instance cannot fake readiness.
# Exits 2 on timeout, printing what it saw so the caller can tell
# "app never started" from "mode never changed".
set -euo pipefail

dir="${1:?usage: agent-wait.sh <harness-dir> [--mode M] [--timeout S]}"
shift || true

mode=""
timeout=60
fresh=3
log=""
after=""
want_ui=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode) mode="${2:?}"; shift 2 ;;
        --timeout) timeout="${2:?}"; shift 2 ;;
        --fresh) fresh="${2:?}"; shift 2 ;;
        --log) log="${2:?}"; shift 2 ;;
        # Only accept a state.json written after this marker file: readiness
        # must come from the process we just started, not from an orphan.
        --after) after="${2:?}"; shift 2 ;;
        # Also require the frontend UI to be mounted (an input/button exists),
        # not just the Rust harness writing state during early boot.
        --ui) want_ui=1; shift ;;
        *) echo "unknown option: $1" >&2; exit 64 ;;
    esac
done

state="$dir/state.json"
start=$SECONDS
last="no state.json yet"

while :; do
    if [[ -f "$state" ]]; then
        read -r age cur_mode displayed genesis mounted < <(python3 - "$state" "$after" <<'PY'
import json, os, sys, time
p = sys.argv[1]
marker = sys.argv[2] if len(sys.argv) > 2 else ""
try:
    d = json.load(open(p))
except Exception:
    print("999 ? ? 0 0")
    raise SystemExit(0)
win = d.get("window") or {}
size = win.get("size") or {}
# 1 if the file was written before our marker (i.e. it is a leftover).
stale = 0
if marker and os.path.exists(marker):
    stale = 1 if os.path.getmtime(p) < os.path.getmtime(marker) else 0
ui = d.get("ui") or {}
tags = {n.get("tag") for n in (ui.get("nodes") or []) if isinstance(n, dict)}
mounted = 1 if tags & {"button", "input", "textarea", "select"} else 0
print(
    int(time.time() - os.path.getmtime(p)),
    d.get("mode", "?"),
    "%sx%s" % (size.get("w", "?"), size.get("h", "?")),
    stale,
    mounted,
)
PY
)
        last="mode=$cur_mode size=$displayed state_age=${age}s ui=$([[ "$mounted" == "1" ]] && echo mounted || echo starting)"
        if [[ "$genesis" == "1" ]]; then
            last="$last (leftover state.json from before this run)"
        elif [[ "$age" -le "$fresh" ]] \
            && [[ "$want_ui" == "0" || "$mounted" == "1" ]] \
            && [[ -z "$mode" || "$cur_mode" == "$mode" ]]; then
            echo "ready: $last"
            exit 0
        fi
    fi

    if (( SECONDS - start >= timeout )); then
        echo "timeout after ${timeout}s: $last" >&2
        if [[ -n "$log" && -f "$log" ]]; then
            echo "--- last lines of $log ---" >&2
            tail -15 "$log" | sed 's/\x1b\[[0-9;]*m//g' >&2
        else
            echo "--- files in $dir ---" >&2
            ls -la "$dir" >&2 || true
        fi
        exit 2
    fi
    sleep 0.25
done
