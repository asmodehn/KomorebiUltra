# Komorebi — agent instructions

## Next sessions

Queued work for fresh sessions: `docs/agents/next-sessions.md`. When you finish a queued block, delete it there.

## Test-first for every change

"you should also test every change you make tdd style" (owner, 2026-11-22)

Facts:
- The full local CI matrix in `.github/workflows/ci.yml` found two red gates in files from earlier commits on the integrated line: `crates/cloud/src/acp.rs` (fmt) and the close handler in `src-tauri/src/lib.rs` (clippy). Narrower checks on the current branch were green.
- The owner corrected that the earlier commits are agent work too.

Rules:
- Run the exact local CI matrix from `.github/workflows/ci.yml`, from its working directories, before declaring any change done — because a narrower suite can be green while CI is red.
- Treat every commit on the integrated line as our work — because a red from an earlier agent commit is still ours, not "pre-existing" to dismiss.
- Use the context-split TDD sequence for behaviour changes; for lint/format-only changes and every done declaration, step 9 review still runs — because the skill excludes the sequence for those changes, not the review.
- Keep spec, test author and implementer separate: only the spec crosses, never the plan — because tests written by the implementer otherwise encode the intended code.
- Keep the test set in the tree, or name by-hand checks when tests are waived — because an unrecorded check guards nothing.

Out of reach:
- A green suite cannot prove absence of defects; it is a floor, not proof.

Costs (derived):
- Full CI and a fresh-context review on every change cost time and a subagent dispatch.

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
  write window-area PNGs and a `state.json` (overlay presentation, selected
  backend, OS click-through rects, DOM nodes with labels, focus, last keys),
  and accept commands
  (`frame`, `watch`, `eval <js>`, `hide`/`show`, `focus`, `gtk`). Run
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

### Window keyboard focus

Never let the layer-surface overlay ask for keyboard focus while it maps. It
must map with `KeyboardMode::None`, then switch to `OnDemand` only when the
page receives its first pointer event on the character or a UI control. COSMIC
hands the keyboard to a mapping layer surface immediately otherwise, so this
would steal keys from whichever window the user was typing in. The window
also starts unfocused. Do not change this to fix a typing or focus complaint
without measuring startup focus first. See `docs/overlay.md`.
