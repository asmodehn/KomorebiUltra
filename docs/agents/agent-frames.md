# Agent frames (dev harness)

Lets an agent *see* and *drive* the real Komorebi window without a screenshot
tool, input injection, or any special permissions. Debug builds only, and
only when `KOMOREBI_AGENT_DIR` is set.

```bash
KOMOREBI_AGENT_DIR=/tmp/opencode/komorebi-agent pnpm tauri dev
```

## What it writes

| File | Content |
| --- | --- |
| `frame-<n>.png` | The **window area only**: the webview rendered through `webkit_web_view_get_snapshot` (`src-tauri/src/window/frame.rs`), downscaled to ≤480 px wide and flattened over a checkerboard so transparent (click-through) areas are visible. |
| `state.json` | The same frame as text: `mode` (always `overlay`), `backend` (the selected surface: `wayland-layer-shell` or `plain-window`), `window` (visible/focused/size/scale), `input_rects` (the OS click-through region), and `ui` (the frontend's DOM snapshot pushed by `src/agentFrame.ts`: tag, class, `label` (title / aria-label / text), rect, pointer-events, `disabled`, `focus` (active element + value) and the last `keys` the page saw). It also carries `capabilities.cursor:global` (`available`, `source`, `reason`), `last_observations.cursor:global` (the last `{ kind, source, at, payload: { x, y } }` record, or `null`) and `observations` (a bounded array of those records, newest last). |

`ui` is the token-cheap half: a DOM walk answers "what is on screen, where can
I click" exactly, while the PNG covers the composited Live2D canvas, which the
DOM cannot describe. Interactive roots (`.interactive`), every real control
(`button`, `input`, …) and the avatar stage are listed, so top-bar buttons are
addressable by label.

`ui.avatar` is `null` until the Live2D model is mounted, then
`{ ready: true, readyAtMs }` — `readyAtMs` is milliseconds since that page
started. It distinguishes "the character is mounted" from "an empty PIXI
canvas exists" and makes a slow mount measurable.

## What it accepts

Write newline-separated commands to `<dir>/cmd`; the harness consumes the file
within ~250 ms.

```text
frame                 capture one frame now (after a ~300 ms settle)
watch 2000            capture every 2 s (0 = off)
hide / show           unmap / remap the window (headless experiments)
focus                 restore focusable + activate the window
gtk                   log GTK/webview focus state (is_active, accept_focus, …)
eval <js>             run JS in the webview
cursor <x> <y> <source> <at_ms>
                      inject one read-only global pointer observation
```

`cursor` is a fake source: it records a `cursor:global` observation in
`state.json` when the position changes and at most every 34 ms. It never moves
the real pointer, changes the input region or focusses a window. A build with no
real source reports `available: false` and a reason.

## Real cursor source (X11, opt-in)

Set `KOMOREBI_CURSOR_SOURCE=x11` to select the read-only X11 `XQueryPointer`
source (debug Linux builds only). It is **never** auto-selected: hosts with a
`DISPLAY` such as `:1` must not be polled without an explicit choice. Use
`KOMOREBI_CURSOR_DISPLAY` to override the display; without it the process
`DISPLAY` is used. The source name in records, capability and logs is exactly
`x11-xquerypointer`.

Sampling runs at ~30 Hz in its own thread. `at` is absolute
`CLOCK_MONOTONIC` milliseconds at sample time, not the 250 ms harness publish
time. Records are change-only, gated at 34 ms, and kept in `state.json` as a
bounded `observations` array. On success the startup log says
`cursor source selected source=x11-xquerypointer`; on failure
`cursor source unavailable source=x11-xquerypointer reason=<reason>` (for a
missing display: `XOpenDisplay failed for DISPLAY=:98`) and no position is
invented. Every emit also logs one
`kind=cursor:global source=x11-xquerypointer at=<ms> x=<x> y=<y>` line.

Observation does not change who receives input. The X11 source only reads the
root pointer; the GTK input shape stays the character rect, clicks outside it
reach the desktop, and no pointer/keyboard event is injected. Verify it only on
a private X server (headless Xvfb `:99`), never on the operator's `DISPLAY`.
The five by-hand checks and their fixture are in `docs/agents/cursor-x11-checks.md`.

There is no mode-switch command: the surface is selected once at startup from
the OS and the build's features. Building without `--features layer-shell`
selects the plain fallback for debugging; nothing can switch it at runtime.

`eval` is how the real UI is driven. Labels come from `state.json` (`ui.nodes`),
so a click can be written as a lookup instead of a guess:

```bash
echo 'eval [...document.querySelectorAll("button")].find(b => (b.title||"").includes("Reset conversation"))?.click()' \
  > /tmp/opencode/komorebi-agent/cmd
```

Then read `state.json` to confirm the UI changed and `frame-*.png` to check
the pixels. A `console.log` inside the eval'd JS shows up in the `tauri dev`
output, which is useful to get values back.

## Reliability

`frame` waits ~300 ms before the snapshot so the compositor has repainted.
Without that settle, a capture issued right after an `eval` click can still
show the *previous* frame — which looks exactly like "the click did nothing".
`watch` does not wait, since it samples continuously.

`state.json` is rewritten whenever it changes, so after clicking a control
poll `state.json` (authoritative) before trusting a frame; frames are the
pixel truth, the JSON is the state truth.

## Headless experiments

Set `KOMOREBI_HEADLESS=<seconds>` to keep the app running with the main window
unmapped, so nothing appears on the desktop while frames keep coming. The value
is a **mandatory deadline**: the app hides the window and exits on its own when
the deadline is reached, so a headless run can never outlive the experiment
that started it (or keep the single-instance lock for the next session). A
missing, non-numeric or zero value is refused with exit code 2.

```bash
KOMOREBI_HEADLESS=300 KOMOREBI_AGENT_DIR=/tmp/opencode/komorebi-agent pnpm tauri dev
```

The window still exists (WebKit renders it for `frame` snapshots), but the
compositor never shows it. Drawbacks: window-manager behaviour (always-on-top,
tiling, input regions at the OS level) is not exercised while hidden — use the
headless compositor sandbox (`docs/agents/sandbox.md`) when you need those, or
a nested compositor. Headless is also the right mode for pure UI work.

## Never wait on a log line

Wait for **state**, with a hard deadline, via `scripts/agent-wait.sh`:

```bash
scripts/agent-wait.sh /tmp/opencode/komorebi-agent --timeout 60 --ui --mode overlay
```

It exits 0 only when `state.json` is fresh (≤ `--fresh` seconds), the requested
mode matches, and (with `--ui`) the frontend has mounted an input/button.
`--after <marker>` rejects a `state.json` written before the marker file (a
leftover from a previous run). On timeout it prints the last state it saw and
the tail of the app log, and exits 2 — so "never started" and "never revealed
the UI" are distinguishable instead of both looking like a hang.

## Why the frames are cheap

- Downscaled + limited palette: a 480-px-wide frame costs a few hundred
  tokens instead of a full-screen capture.
- `ui` replaces pixels with text for everything except the avatar.
- No crop math, no monitor offsets: the frame *is* the window.

## Limitations

- `window/frame.rs` currently implements Linux (WebKitGTK). WebView2
  `CapturePreview` (Windows) and `WKWebView.takeSnapshot` (macOS) are stubs.
- The harness is compiled out of release builds (`#[cfg(debug_assertions)]`).
