#!/usr/bin/env bash
# Headless Wayland sandbox for GUI experiments.
#
# Runs a nested sway compositor with its own Wayland socket, so the app can be
# exercised (real pointer events, real compositing, tiling) without touching
# the user's desktop. Nothing is installed system-wide: sway and its tools are
# unpacked into `.sandbox/` from .deb packages.
#
#   scripts/agent-sandbox.sh up          # fetch tools (first run) + start sway
#   scripts/agent-sandbox.sh app [secs]  # run `pnpm tauri dev` inside it and wait
#                                        # (bounded) until the harness is live; the
#                                        # app is stopped at `secs` (default 1800)
#   scripts/agent-sandbox.sh shot out.png   # capture the virtual screen
#   scripts/agent-sandbox.sh env         # `eval $(... env)` to work inside it
#   scripts/agent-sandbox.sh status      # sway, app pid, deadline, harness state age
#   scripts/agent-sandbox.sh app-stop    # stop just the app
#   scripts/agent-sandbox.sh down        # stop sway and anything started by `app`
#
# See docs/agents/sandbox.md for what this can and cannot test.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SANDBOX="$ROOT/.sandbox"
RUNTIME="$SANDBOX/run"
DEBS="$SANDBOX/debs"
PREFIX="$SANDBOX/root"
AGENT_DIR="${KOMOREBI_AGENT_DIR:-$SANDBOX/agent}"
APPTITLE="komorebi-sandbox"

# sway plus everything needed for headless capture and input injection.
PACKAGES=(sway libwlroots12t64 libliftoff0 libxcb-ewmh2 libxcb-res0 wtype wlrctl grim)

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

running() {
    local pid
    pid="$(cat "$SANDBOX/sway.pid" 2>/dev/null || true)"
    [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null
}

sway_socket() {
    ls "$RUNTIME"/sway-ipc.*.sock 2>/dev/null | head -1
}

wayland_display() {
    local s
    s="$(ls "$RUNTIME"/wayland-[0-9]* 2>/dev/null | grep -v '\.lock$' | head -1 || true)"
    basename "${s:-wayland-1}"
}

fetch_tools() {
    if [[ -x "$PREFIX/usr/bin/sway" ]]; then
        return
    fi
    command -v apt-get >/dev/null || die "apt-get is required to fetch sway"
    command -v dpkg-deb >/dev/null || die "dpkg-deb is required to unpack packages"
    say "fetching sway + tools into .sandbox (no system install)"
    mkdir -p "$DEBS" "$PREFIX"
    ( cd "$DEBS" && apt-get download "${PACKAGES[@]}" >/dev/null 2>&1 ) \
        || die "apt-get download failed (are these packages available?)"
    for deb in "$DEBS"/*.deb; do
        dpkg-deb -x "$deb" "$PREFIX"
    done
}

write_config() {
    mkdir -p "$RUNTIME"
    cat > "$SANDBOX/sway.conf" <<EOF
output HEADLESS-1 resolution 1280x800
# A non-black background: transparency bugs are invisible on a black output.
output HEADLESS-1 bg ${KOMOREBI_SANDBOX_BG:-#1e5f8a} solid_color
default_border none
default_floating_border none
xwayland disable
# Keep the avatar window floating, like a compositor with a tiling exception.
for_window [app_id="komorebi"] floating enable
for_window [app_id="komorebi"] resize set 480 760
# A backdrop window used to prove that clicks pass through the overlay.
for_window [title="clickprobe"] floating enable
for_window [title="clickprobe"] move position 0 0
for_window [title="clickprobe"] resize set 800 600
EOF
}

up() {
    fetch_tools
    write_config
    if running; then
        say "sandbox already up (pid $(cat "$SANDBOX/sway.pid"))"
        status
        return
    fi
    say "starting headless sway (1280x800, socket in .sandbox/run)"
    env \
        WLR_BACKENDS=headless \
        WLR_HEADLESS_OUTPUTS=1 \
        WLR_LIBINPUT_NO_DEVICES=1 \
        WLR_RENDERER=pixman \
        XDG_RUNTIME_DIR="$RUNTIME" \
        LD_LIBRARY_PATH="$PREFIX/usr/lib/x86_64-linux-gnu" \
        "$PREFIX/usr/bin/sway" -c "$SANDBOX/sway.conf" -d \
        > "$SANDBOX/sway.log" 2>&1 &
    echo $! > "$SANDBOX/sway.pid"
    for _ in $(seq 1 50); do
        [[ -n "$(sway_socket)" ]] && break
        sleep 0.2
    done
    running || die "sway did not start (see .sandbox/sway.log)"
    status
}

down() {
    app_stop
    if running; then
        say "stopping sway"
        kill "$(cat "$SANDBOX/sway.pid")" 2>/dev/null || true
        rm -f "$SANDBOX/sway.pid"
    else
        say "sandbox not running"
    fi
}

app_pid() {
    cat "$SANDBOX/app.pid" 2>/dev/null || true
}

app_running() {
    local pid
    pid="$(app_pid)"
    [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null
}

# Komorebi processes started by a previous sandbox run (same harness dir, so
# necessarily ours). They are killed before a new run: an orphan would keep
# writing state.json and fake readiness, and would hold the single-instance
# lock.
stray_app_pids() {
    local pid
    for pid in $(pgrep -f 'target/debug/komoreb[i]' 2>/dev/null); do
        if tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null \
            | grep -qx "KOMOREBI_AGENT_DIR=$AGENT_DIR"; then
            echo "$pid"
        fi
    done
}

app_stop() {
    if [[ -n "$(app_pid)" ]]; then
        local pid
        pid="$(app_pid)"
        # The app is started with setsid, so kill the whole process group:
        # pnpm/tauri-run/cargo/webview children must not survive.
        kill -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
        rm -f "$SANDBOX/app.pid" "$SANDBOX/app.deadline"
        say "stopped the app (pid $pid)"
    fi
    # Belt and braces: any process still bound to this harness dir is ours.
    local strays
    strays="$(stray_app_pids | tr '\n' ' ')"
    if [[ -n "${strays// /}" ]]; then
        # shellcheck disable=SC2086
        kill $strays 2>/dev/null || true
        say "stopped a leftover app process (pid ${strays% })"
    fi
}

env_prefix() {
    local sock
    sock="$(sway_socket)"
    printf 'export XDG_RUNTIME_DIR=%q\n' "$RUNTIME"
    printf 'export WAYLAND_DISPLAY=%q\n' "$(wayland_display)"
    printf 'export SWAYSOCK=%q\n' "$sock"
    printf 'export LD_LIBRARY_PATH=%q\n' "$PREFIX/usr/lib/x86_64-linux-gnu"
    printf 'export PATH=%q:$PATH\n' "$PREFIX/usr/bin"
}

status() {
    if running; then
        say "sandbox: up (pid $(cat "$SANDBOX/sway.pid"), 1280x800)"
        env_prefix
    else
        say "sandbox: down"
        return
    fi
    if app_running; then
        local extra=""
        if [[ -f "$SANDBOX/app.deadline" ]]; then
            local left=$(( $(cat "$SANDBOX/app.deadline") - $(date +%s) ))
            (( left < 0 )) && left=0
            extra=", stops in ${left}s"
        fi
        say "app: running (pid $(app_pid)$extra)"
    else
        say "app: not running"
    fi
    if [[ -f "$AGENT_DIR/state.json" ]]; then
        python3 - "$AGENT_DIR/state.json" <<'PY'
import json, os, sys, time
p = sys.argv[1]
try:
    d = json.load(open(p))
except Exception:
    print("harness: state.json unreadable (still starting or stale)")
    raise SystemExit(0)
age = time.time() - os.path.getmtime(p)
win = d.get("window") or {}
size = win.get("size") or {}
print(
    "harness: mode=%s size=%sx%s rects=%d state_age=%.1fs"
    % (
        d.get("mode", "?"),
        size.get("w", "?"),
        size.get("h", "?"),
        len(d.get("input_rects") or []),
        age,
    )
)
if age > 10:
    print("harness: state is stale (>10s) — the webview may be gone")
PY
    else
        say "harness: no state.json yet (app not up, or no KOMOREBI_AGENT_DIR)"
    fi

    # The seat occasionally loses its capabilities (`caps` becomes empty or 0).
    # Pointer and keyboard injection then silently do nothing, so a "no click
    # reached it" result means nothing until the sandbox is restarted.
    if ! seat_has_caps; then
        say "warning: seat reports no pointer/keyboard capabilities; input" \
            "injection is broken — '$(basename "$0") down && $(basename "$0") up'"
    fi
}

# True when sway reports a seat with at least pointer (1) or keyboard (2) caps.
seat_has_caps() {
    run_in swaymsg -t get_seats 2>/dev/null | python3 -c '
import json, sys
try:
    seats = json.load(sys.stdin)
except Exception:
    raise SystemExit(1)
def bits(caps):
    # sway >= 1.9 reports a bitmask, older ones a list of names.
    if isinstance(caps, int):
        return caps
    return sum(
        {"pointer": 1, "keyboard": 2}.get(str(c), 0) for c in (caps or [])
    )
raise SystemExit(0 if any(bits(s.get("capabilities")) & 3 for s in seats) else 1)
'
}

run_in() {
    running || die "sandbox is down; run '$0 up' first"
    env \
        XDG_RUNTIME_DIR="$RUNTIME" \
        WAYLAND_DISPLAY="$(wayland_display)" \
        SWAYSOCK="$(sway_socket)" \
        GDK_BACKEND=wayland \
        LD_LIBRARY_PATH="$PREFIX/usr/lib/x86_64-linux-gnu" \
        PATH="$PREFIX/usr/bin:$PATH" \
        "$@"
}

app() {
    running || die "sandbox is down; run '$0 up' first"
    local secs="${1:-1800}"
    [[ "$secs" =~ ^[0-9]+$ ]] && (( secs > 0 )) \
        || die "app takes an optional positive number of seconds, got '$secs'"
    app_running && die "app already running (pid $(app_pid)); '$0 app-stop' first"

    local strays
    strays="$(stray_app_pids | tr '\n' ' ')"
    if [[ -n "${strays// /}" ]]; then
        say "killing a leftover sandbox app from a previous run (pid ${strays% })"
        # shellcheck disable=SC2086
        kill $strays 2>/dev/null || true
        sleep 1
        strays="$(stray_app_pids | tr '\n' ' ')"
        [[ -z "${strays// /}" ]] \
            || die "leftover app still running (pid ${strays% }); kill it and retry"
    fi

    mkdir -p "$AGENT_DIR"
    # A stale state.json from a previous run must not be mistaken for readiness.
    rm -f "$AGENT_DIR"/state.json
    touch "$SANDBOX/app.started"

    say "starting the app inside the sandbox (harness dir: $AGENT_DIR, deadline ${secs}s)"
    : > "$SANDBOX/app.pid"
    # The pidfile is written by the session leader itself, so it is the process
    # group we must kill later (`$!` would be the short-lived wrapper).
    local features=()
    if [[ -n "${KOMOREBI_SANDBOX_FEATURES:-}" ]]; then
        features=(--features "$KOMOREBI_SANDBOX_FEATURES")
        say "cargo features: $KOMOREBI_SANDBOX_FEATURES"
    fi
    run_in setsid bash -c 'echo $$ > "$1"; shift; exec "$@"' _ \
        "$SANDBOX/app.pid" \
        env KOMOREBI_AGENT_DIR="$AGENT_DIR" \
        pnpm tauri dev "${features[@]}" \
        > "$SANDBOX/app.log" 2>&1 &
    for _ in $(seq 1 200); do
        [[ -s "$SANDBOX/app.pid" ]] && break
        sleep 0.05
    done
    [[ -s "$SANDBOX/app.pid" ]] || die "could not learn the app pid"
    date +%s | awk -v s="$secs" '{ print $1 + s }' > "$SANDBOX/app.deadline"

    # Safety net: stop the app at the deadline even if the caller walks away.
    setsid bash -c '
        sleep "$1"
        pid="$(cat "$2/app.pid" 2>/dev/null || true)"
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            echo "deadline reached, stopping the app" >&2
            kill -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
            rm -f "$2/app.pid" "$2/app.deadline"
        fi
    ' _ "$secs" "$SANDBOX" >/dev/null 2>&1 &

    say "pid $(app_pid); waiting up to 240s for the harness (log: .sandbox/app.log)"
    if ! "$ROOT/scripts/agent-wait.sh" "$AGENT_DIR" --timeout 240 --fresh 3 --ui \
        --after "$SANDBOX/app.started" --log "$SANDBOX/app.log"; then
        # The app may still be compiling; keep it, but say so plainly.
        say "app is not ready yet; it will stop by itself at the deadline"
        return 2
    fi
}

shot() {
    local out="${1:-$SANDBOX/screenshot.png}"
    mkdir -p "$(dirname "$out")"
    run_in grim "$out"
    say "wrote $out"
}

# Pointer helper: absolute-ish moves relative to the last position.
pointer() {
    run_in wlrctl pointer "$@"
}

case "${1:-help}" in
    up) up ;;
    down) down ;;
    status) status ;;
    env) env_prefix ;;
    run) shift; run_in "$@" ;;
    app) shift; app "$@" ;;
    app-stop) app_stop ;;
    shot) shift; shot "$@" ;;
    pointer) shift; pointer "$@" ;;
    *)
        sed -n '2,22p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
        ;;
esac
