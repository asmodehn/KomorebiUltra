# By-hand checks: X11 `CursorSource` (ADR 0010 slice 2)

These checks prove what a unit test cannot: that the adapter reads the **live**
root pointer on a private X server, that observations bind to independent XTest
marks, and that observing does not consume desktop clicks. They run only on a
debug Linux build, on an isolated Xvfb display. Xvfb needs no parent display,
so nothing is mapped on the operator's desktop. Never aim them at the
operator's `DISPLAY`.

The transcribed, executable set lives in `/tmp/opencode/komorebi-x11-xqueryprobe`
(`run.sh old|new`, `xprobe.c`, `c1.sh` … `c5.sh`, `c3.py`). `/tmp` is ephemeral;
recreate it from this contract if it is gone.

## Fixture

1. Build the debug binary with no features: `cd src-tauri && cargo build`.
2. Compile the probe:
   `gcc -O2 -Wall -Wextra -o xprobe xprobe.c $(pkg-config --cflags --libs x11 xtst) -lXext`.
   It refuses any `DISPLAY` other than `:99`. Its commands are `query`, `clock`,
   `move X Y`, `click X Y LOG`, `win X Y W H LOG SECONDS`, `script PLAN LOG` and
   `shape WINID`. `script` writes `MARK <n> <x> <y> <CLOCK_MONOTONIC_ms>` lines.
3. Fetch the `xvfb` .deb into the fixture dir and unpack it with `dpkg-deb -x`
   (no system install; `xserver-common` is already present). Start
   `env -u DISPLAY -u WAYLAND_DISPLAY Xvfb :99 -screen 0 1280x800x24 -nolisten
   tcp -noreset -ac` under a timeout. Assert `:99` is free first; remove a
   stale socket and lock only when no process owns them. Xephyr is not a
   substitute: it needs a parent X display and would map a window on the
   operator's desktop.
4. Launch the app with `env -u WAYLAND_DISPLAY -u GDK_BACKEND DISPLAY=:99
   GDK_BACKEND=x11 RUST_LOG=info,komorebi=debug KOMOREBI_CURSOR_SOURCE=x11
   KOMOREBI_AGENT_DIR=<dir> timeout … komorebi`. Never set `DISPLAY=:1`.
5. Wait with `scripts/agent-wait.sh <dir> --timeout 45 --mode overlay`, then
   wait until `input_rects` is non-empty (the frontend reports it after boot).
6. Select the main window by `xwininfo -root -tree | awk '$2=="\"Komorebi\":"'`.
   Other windows (tray icon, hidden helper) also contain the program name.
7. Save `baseline.json` (state) and `baseline-shape.txt` (`xprobe shape WID`);
   the shape must be `shape_bbox 1080,464 200x264` — the character rect, not
   the screen. Map a small desktop click recorder window at `200,200 300x200`.
8. Kill every process group on EXIT/INT/TERM.

## Checks

| Check | Oracle | Pass condition | Falsifies |
| --- | --- | --- | --- |
| C1 | `xprobe query` root coordinate | After `MOVE 300 300`, `capabilities.cursor:global` is available with source `x11-xquerypointer`; `last_observations` and `observations[-1]` are the same record at `(300,300)` with integer `at` | cached/window-local/fabricated coordinate, wrong source, no record |
| C2 | XTest `BUTTON` in the desktop window; `XShapeGetRectangles(ShapeInput)` | Click at `(300,300)` reaches the desktop window; shape and `input_rects` equal the baseline; `window.focused == false` | full-screen input shape, click swallowed, focus stolen |
| C3 | 13 `MARK` lines from an XTest script | Records bind to mark coordinates and absolute mark times; gaps ≥34 ms; no repeats; ramp has 2–4 emissions; the repeat mark emits nothing; the final `(300,300)` binds to its mark | startup-once sampling, no clock truth, no rate gate, fabricated sequence |
| C4 | Missing socket for `:98`; relaunched window id | With `KOMOREBI_CURSOR_DISPLAY=:98`, capability is unavailable with reason `XOpenDisplay failed for DISPLAY=:98`, no record exists, the startup log names source and reason, and the **relaunched** window's shape matches the baseline | invented position, wrong reason, stale window id, changed surface |
| C5 | Live `DISPLAY=:99` with the selector absent | Capability stays unavailable with reason `no passive global pointer source selected`, no records, no selection log | auto-selection of a host display |

## Soak smoke test (60 s)

`soak.sh` in the fixture dir drives 300 XTest moves at 200 ms intervals (60 s)
over the same headless Xvfb, then checks the long-run path C1–C5 do not cover:

- at 20 s, 40 s and 60 s: `observations` has exactly 32 records, every record
  carries `source=x11-xquerypointer` and numeric `at`, times are non-decreasing,
  and `observations[-1]` is `last_observations.cursor:global`;
- the last record's position and `at` bind to the final XTest mark (within one
  34 ms sampling period) and match the live `xprobe query` root coordinate;
- all 300 marks were written, the app process is still alive, no
  `panic`/`fatal error`/`SIGSEGV` appears in its log, and `input_rects` and
  `window.focused` equal their pre-soak values.

Pass condition: one 60 s run printing `SOAK PASS`. A failure exits 2 with a
`FAIL <reason>` line; the log is `soak.log` and the app log is `soak-app.log`.

C3's window for its first records starts at the first mark of its own script:
C1 and C2 leave older records in the same bounded `observations` array. Its
early-count bound is 2–3, because a correct 34 ms poller may legitimately emit
either the first fast-pair position, the second, or both (the second only at
least 34 ms after the first).

## Failure convention

A check exits 2 on any failed assertion. `run.sh new` prints `CHECK cN exit=…`
for each check. The slice is green only when C1–C5 all exit 0 and the OLD pin
prints `OLD capability assertion: PASS`, `OLD preservation assertions: PASS`,
and every NEW check on the unmodified tree exits 2.
