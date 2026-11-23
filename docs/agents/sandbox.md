# Headless sandbox (dev)

Runs the real app inside a private **sway** compositor on its own Wayland
socket: real pointer events, real compositing and tiling, no window on the
user's desktop, nothing installed system-wide.

```bash
scripts/agent-sandbox.sh up       # fetch tooling (first run) + start sway
scripts/agent-sandbox.sh app 300  # run `pnpm tauri dev` inside it for ≤300s
scripts/agent-sandbox.sh shot /tmp/shot.png
scripts/agent-sandbox.sh pointer move 0 300 && scripts/agent-sandbox.sh pointer click left
scripts/agent-sandbox.sh status   # sway, app pid, deadline, harness mode/age
scripts/agent-sandbox.sh app-stop # stop just the app (down stops everything)
scripts/agent-sandbox.sh down     # stop the app and sway
```

`app` does not return until the harness is actually usable (bounded at 240 s),
and it never runs forever: the duration (`app <seconds>`, default 1800) is a
hard deadline enforced by a detached watchdog, so a forgotten sandbox cannot
hold the single-instance lock or burn CPU overnight. Before starting, `app`
kills any komorebi process still bound to this harness dir (an orphan from an
earlier run would otherwise fake readiness by writing `state.json`), and
readiness is checked by `scripts/agent-wait.sh` — state-based, never log-based.

`app` runs `pnpm tauri dev`, so it needs port 1420. If a dev server already
runs on the host (or another instance owns the DBus single-instance name),
start the built binary directly inside the sandbox with
`KOMOREBI_AGENT_DIR=<dir>` and a mandatory `KOMOREBI_HEADLESS=<seconds>`
(build `src-tauri/target/debug/komorebi` with `--features layer-shell` first).
There is no mode-switch script: the surface is selected once at startup.

`up` downloads `sway`, `libwlroots12t64`, `libliftoff0`, `libxcb-ewmh2`,
`libxcb-res0`, `wtype`, `wlrctl` and `grim` with `apt-get download` and unpacks
them into `.sandbox/` (`gitignored`). No `sudo`, no packages added to the
system. sway runs with `WLR_BACKENDS=headless` (1280×800), a private
`XDG_RUNTIME_DIR`, and a config that keeps the Komorebi window floating so it
behaves like a compositor with a tiling exception (`docs/overlay.md`).

## What it is good for

| Need | Tool |
| --- | --- |
| Does the compositor tile / resize / stack the window correctly? | `grim` screenshot of the virtual screen |
| Does the app's own UI look right (page content)? | harness frame in `.sandbox/agent/frame-*.png` |
| Do real pointer clicks reach the intended control? | `wlrctl pointer`, then `state.json` → `focus` |
| Do click-through rects match the UI? | `state.json` → `input_rects` + frames |

The two views answer different questions: `grim` shows what the *compositor*
presents (window size/placement), while the harness frame is the *page* as
WebKit renders it. If they disagree, the bug is in window management, not in
the UI — that is exactly how the off-screen placement of a merely *sized*
overlay window was caught (Wayland clients cannot position themselves, so the
overlay covers the screen with a layer surface instead).

## Pointer injection

`wlrctl` moves the pointer **relative** to its current position; there is no
absolute seek. Start from a known state or take a screenshot, then:

```bash
scripts/agent-sandbox.sh pointer move 0 300   # dx dy
scripts/agent-sandbox.sh pointer click left   # left | right | middle
```

**Check the seat before believing a negative result.** Between runs the sway
seat sometimes loses its capabilities: `swaymsg -t get_seats` then reports
`caps 0`, `wlrctl pointer click` and `swaymsg seat seat0 cursor press button1`
both succeed but no button event arrives anywhere (a plain GTK window logs
nothing either), while `cursor set` still moves the pointer. `status` prints a
warning when that happens; the fix is `down && up`.

Because of that, several things are unverified here and must be checked on the
host: click-through of the layer-surface overlay (the input shape is applied,
but clicks cannot be injected), and anything that needs a real button press.

## Keyboard: known limitation

The sandbox has no seat keyboard, so sway never marks a window as *active*.
Plain GTK widgets still receive injected keys (`wtype "text"` types into a
`Gtk.Entry`), but WebKitGTK drops key events while its toplevel is inactive,
so `wtype` does **not** reach the webview.

Consequences:

- Keyboard-driven UI (typing a message, focused shortcuts) cannot be verified
  here. Drive it through the DOM instead: `eval document.querySelector(...)`
  or dispatch `KeyboardEvent`s, which tests the app's own handlers.
- OS-level global shortcuts (`Alt+O`, registered through the desktop portal)
  are not exercised either; verify those on the host.
- Window activation/focus itself can be inspected with the harness `focus`
  and `gtk` commands (they log `is_active`, `has_toplevel_focus`,
  `accept_focus`, webview `has_focus`).

## Isolation guarantees

- Own Wayland socket and `XDG_RUNTIME_DIR` (`.sandbox/run`); the app cannot see
  the host compositor.
- `DISPLAY` is removed from every process launched through `run`, `app`, `shot`
  and `pointer`, and from the environment printed by `env`. The caller's own
  `DISPLAY` is unchanged.
- Headless backend: nothing is drawn on any real output.
- Injected input goes to the sandbox's seat only.
- `down` stops sway and the app started by `app`; no system state changes
  remain (aside from the `.sandbox/` cache).

By-hand check, with the sandbox up and the host exporting `DISPLAY=:1`: launch
a child that prints its own environment through each of the five paths above
and require `DISPLAY=<UNSET>` plus the private `WAYLAND_DISPLAY`,
`XDG_RUNTIME_DIR` and `SWAYSOCK` values; `GDK_BACKEND` must stay `wayland` where
the path sets it. The host and caller shells must still print `DISPLAY=:1`
after every check. For `shot` and `pointer`, use a small `LD_PRELOAD`
constructor observer filtered to `grim` / `wlrctl`; for `pointer` pass no
arguments so no input event is injected. For `app`, clear inherited
`KOMOREBI_AGENT_DIR`, put a stub `pnpm` first on PATH so no real app or port
1420 listener starts, read the `APPENV` lines from `.sandbox/app.log`, read
`/proc/<pid>/environ` of any `agent-wait.sh` using this repo's
`--after .sandbox/app.started` marker, kill that waiter, then remove
`.sandbox/app.pid`, `.sandbox/app.deadline` and `.sandbox/app.started`.
Require `gcc` and the sandbox tools, or fail the check. Finish with a guarded
`down` / `up`, re-read the sway pid, private socket names and seat caps, and
repeat the `run` check. The control child in the host shell must still print
`DISPLAY=:1`. Record any seat-capability warning rather than treating it as a
failure of this check.
