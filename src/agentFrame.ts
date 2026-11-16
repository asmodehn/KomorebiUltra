// Dev-only UI snapshot for the agent frame harness.
//
// Pushes a compact, token-friendly description of the interactive UI to the
// Rust side (`src-tauri/src/agent_frame.rs`), which writes it to `state.json`
// next to the window frame PNG. A DOM walk is far cheaper than pixels for
// "what is on screen / where can I click", and it is exact.
//
// Only started by `main.tsx` under `import.meta.env.DEV`.

import { invoke } from "@tauri-apps/api/core";

interface UiNode {
  tag: string;
  /** Class list, trimmed; `interactive` elements get a stable `path`. */
  cls: string;
  /** Best human label: title, aria-label, then text. */
  label: string;
  /** Trimmed text content, capped — enough to identify a control. */
  text: string;
  /** DOM order index among its siblings (for `eval` clicks). */
  idx: number;
  rect: [number, number, number, number];
  /** CSS pointer-events, so click-through surfaces are visible as such. */
  pe: string;
  /** Whether the control is disabled (buttons that do nothing yet). */
  disabled?: boolean;
}

const MAX_NODES = 48;
const MAX_TEXT = 48;
const INTERVAL_MS = 500;

/** Last few key events seen by the page — proves OS keyboard input arrives. */
const keys: Array<{ k: string; mods: string; t: number; target: string }> = [];

function recordKeys(): void {
  window.addEventListener(
    "keydown",
    (e) => {
      if (keys.length >= 8) keys.shift();
      const mods = [e.altKey && "alt", e.ctrlKey && "ctrl", e.shiftKey && "shift"]
        .filter(Boolean)
        .join("+");
      keys.push({
        k: e.key,
        mods,
        t: Math.round(performance.now()),
        target: (e.target as HTMLElement | null)?.tagName.toLowerCase() ?? "?",
      });
    },
    true,
  );
}

function node(el: Element): UiNode {
  const r = el.getBoundingClientRect();
  const parent = el.parentElement;
  const idx = parent ? Array.prototype.indexOf.call(parent.children, el) : 0;
  const text = (el.textContent ?? "").replace(/\s+/g, " ").trim();
  const title = el.getAttribute("title") ?? "";
  const aria = el.getAttribute("aria-label") ?? "";
  const label = (title || aria || text).replace(/\s+/g, " ").trim();
  const out: UiNode = {
    tag: el.tagName.toLowerCase(),
    cls: typeof el.className === "string" ? el.className.trim() : "",
    label: label.length > MAX_TEXT ? `${label.slice(0, MAX_TEXT)}…` : label,
    text: text.length > MAX_TEXT ? `${text.slice(0, MAX_TEXT)}…` : text,
    idx,
    rect: [
      Math.round(r.left),
      Math.round(r.top),
      Math.round(r.width),
      Math.round(r.height),
    ],
    pe: getComputedStyle(el).pointerEvents,
  };
  if ((el as HTMLButtonElement).disabled) out.disabled = true;
  return out;
}

function snapshot(): unknown {
  const nodes: UiNode[] = [];
  // Opt-in interactive roots (styles.css / input-region reporter) plus every
  // real control, so the top-bar buttons are addressable by label instead of
  // by guessing their DOM position.
  const els = document.querySelectorAll(
    '.interactive, button, [role="button"], input, textarea, select, ' +
      '[data-avatar-stage], [style*="pointer-events: auto"]',
  );
  const seen = new Set<Element>();
  for (const el of els) {
    if (seen.has(el)) continue;
    seen.add(el);
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) continue;
    nodes.push(node(el));
    if (nodes.length >= MAX_NODES) break;
  }
  return {
    vw: window.innerWidth,
    vh: window.innerHeight,
    dpr: window.devicePixelRatio,
    overlay: localStorage.getItem("komorebi.overlay_mode"),
    /** Live2D mount: `readyAtMs` is ms since this page started. */
    avatar: avatarInfo(),
    focus: focusInfo(),
    keys: keys.slice(-8),
    debug: (window as { __agentDebug?: unknown }).__agentDebug,
    nodes,
  };
}

/** Whether the character is mounted, and when it became ready (page-clock). */
function avatarInfo(): unknown {
  const el = document.querySelector<HTMLElement>(
    "[data-avatar-stage] [data-model-ready]",
  );
  if (!el) return null;
  return {
    ready: true,
    readyAtMs: Number(el.dataset.modelReadyAt ?? "") || null,
  };
}

/** What holds keyboard focus — proves an injected click/type reached the UI. */
function focusInfo(): unknown {
  const el = document.activeElement as HTMLElement | null;
  if (!el || el === document.body || el === document.documentElement) return null;
  const input = el as HTMLInputElement;
  return {
    tag: el.tagName.toLowerCase(),
    label: (el.getAttribute("aria-label") ?? el.getAttribute("placeholder") ?? "").slice(0, 48),
    value: typeof input.value === "string" ? input.value.slice(0, 80) : undefined,
  };
}

/** Starts the periodic snapshot push; safe to call once. */
export function startAgentFrames(): void {
  recordKeys();
  const push = () => {
    let json: string;
    try {
      json = JSON.stringify(snapshot());
    } catch {
      return;
    }
    void invoke("agent_report", { json }).catch(() => {});
  };
  push();
  window.setInterval(push, INTERVAL_MS);
}
