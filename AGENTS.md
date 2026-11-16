# Komorebi — agent instructions

## Next sessions

Queued work for fresh sessions: `docs/agents/next-sessions.md`. When you finish a queued block, delete it there.

## Agent skills

### Issue tracker

GitHub Issues on the fork `dexterfire/KomorebiUltra` via `gh` (always `-R dexterfire/KomorebiUltra`, never upstream). See `docs/agents/issue-tracker.md`.

### Triage labels

Default five roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`) as GitHub labels. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `GLOSSARY.md` + `docs/adr/`. See `docs/agents/domain.md`.

### GUI experiments

Do not test the window by opening it on the operator's desktop, and never
inject input into their session. Two dev-only tools cover it:

- **Agent frames** — `KOMOREBI_AGENT_DIR=<dir>` (debug builds) makes the app
  write window-area PNGs and a `state.json` (overlay mode, OS click-through
  rects, DOM nodes with labels, focus, last keys), and accept commands
  (`frame`, `overlay|normal|always_on_top`, `eval <js>`, `hide`/`show`). Run
  with `KOMOREBI_HEADLESS=<seconds>` to keep the window unmapped; the deadline
  is mandatory and the app exits by itself. See `docs/agents/agent-frames.md`.
- **Headless sandbox** — `scripts/agent-sandbox.sh up && scripts/agent-sandbox.sh
  app [seconds]` runs the app inside a private sway on its own Wayland socket (no
  root, no packages installed, nothing on any real output) with real pointer
  injection (`pointer move/click`), full-screen `shot`, and tiling behaviour. The
  app stops by itself at the deadline (default 1800 s), and `status` reports the
  harness mode and state age. Keyboard injection does not reach WebKit there (no
  seat keyboard → window never active); drive keyboard through `eval` instead.
  If `status` warns that the seat has no pointer/keyboard capabilities
  (`caps 0`), input injection is silently broken — `down` and `up` again.
  For the layer-shell overlay, set `KOMOREBI_SANDBOX_FEATURES=layer-shell
  KOMOREBI_LAYER_SHELL=1`. See `docs/agents/sandbox.md`.

Take screenshots from the harness frame for *page* truth and from `grim` in the
sandbox for *window-management* truth; when they disagree, the bug is in
windowing, not in the UI. Compositor fullscreen loses transparency, so
`overlay` uses Wayland layer-shell instead (see `docs/overlay.md`); never
"fix" coverage by fullscreening the window.
