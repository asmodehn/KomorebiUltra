import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { modelClientRect } from "../components/Live2DCanvas/mountScene";
import type { WindowOverlayMode } from "../api";

interface InputRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

// Padding around the model, and the grid rects are rounded out to, so
// her idle animation does not resend the region every tick.
const MODEL_PAD = 16;
const GRID = 8;
const INTERVAL_MS = 100;

// Elements that take pointer input: the same opt-in the stylesheet uses
// (`#root` is pointer-events: none), plus inline overrides.
const INTERACTIVE = '.interactive, [style*="pointer-events: auto"]';

function snap(r: DOMRect, pad = 0): InputRect {
  const x0 = Math.floor((r.left - pad) / GRID) * GRID;
  const y0 = Math.floor((r.top - pad) / GRID) * GRID;
  const x1 = Math.ceil((r.right + pad) / GRID) * GRID;
  const y1 = Math.ceil((r.bottom + pad) / GRID) * GRID;
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

function collect(): InputRect[] {
  const rects: InputRect[] = [];
  const model = modelClientRect();
  const stage = document.querySelector("[data-avatar-stage]");
  for (const el of document.querySelectorAll(INTERACTIVE)) {
    // The stage box itself must not capture input — only the model bounds
    // do, so the empty stage area stays click-through to the desktop.
    if (el.hasAttribute("data-avatar-stage")) continue;
    // The stage spans the window; the model's bounds stand in for it.
    if (model && stage?.contains(el)) continue;
    const r = el.getBoundingClientRect();
    if (r.width > 0 && r.height > 0) rects.push(snap(r));
  }
  if (model) rects.push(snap(model, MODEL_PAD));
  return rects;
}

/**
 * The surface is overlay-like in both modes: report where the page takes
 * pointer input (the model bounds + interactive elements) so the empty stage
 * is click-through; the backend answers `false` when the region is
 * unsupported or disabled, and reporting stops. Keyed on the mode so a switch
 * re-derives the rects.
 */
export function useInputRegion(mode: WindowOverlayMode): void {
  useEffect(() => {
    let last = "";
    let stopped = false;
    const report = () => {
      if (stopped) return;
      const rects = collect();
      // Never send an empty region: the window would take no input.
      if (rects.length === 0) return;
      const key = JSON.stringify(rects);
      if (key === last) return;
      last = key;
      invoke<boolean>("set_input_region", { rects })
        .then((on) => {
          if (!on) stop();
        })
        .catch(() => stop());
    };
    const timer = window.setInterval(report, INTERVAL_MS);
    window.addEventListener("resize", report);
    const stop = () => {
      stopped = true;
      window.clearInterval(timer);
      window.removeEventListener("resize", report);
    };
    report();
    return stop;
  }, [mode]);
}
