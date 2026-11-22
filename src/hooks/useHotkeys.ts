import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

/**
 * Subscribes to the global hotkeys exposed by the Rust side:
 *   * `hotkey:toggle-input`  (default Alt+Space) — reveal/hide the controls.
 *   * `hotkey:vision-region` (default Alt+V)     — open region picker.
 *
 * Window presentation has no hotkey: there is one overlay surface. Callbacks
 * are invoked on the renderer thread; pass stable references (e.g.
 * `useCallback`) if you don't want the listeners to re-bind.
 */
export function useHotkeys(opts: {
  onToggleInput: () => void;
  onVisionRegion: () => void;
}): void {
  const { onToggleInput, onVisionRegion } = opts;

  useEffect(() => {
    const p = listen<string>("hotkey:toggle-input", () => onToggleInput());
    return () => {
      p.then((fn) => fn());
    };
  }, [onToggleInput]);

  useEffect(() => {
    const p = listen<string>("hotkey:vision-region", () => onVisionRegion());
    return () => {
      p.then((fn) => fn());
    };
  }, [onVisionRegion]);
}
