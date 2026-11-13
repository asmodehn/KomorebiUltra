import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

/**
 * Subscribes to the global hotkeys exposed by the Rust side:
 *   * `hotkey:toggle-input`  (default Alt+Space) — toggle the chat input.
 *   * `hotkey:vision-region` (default Alt+V)     — open region picker.
 *   * `hotkey:overlay`       (default Alt+O)     — cycle overlay modes.
 *   * `hotkey:toggle-window` (default F11)      — toggle the monitor-covering
 *     overlay ↔ windowed (the only two window states we offer).
 *
 * Callbacks are invoked on the renderer thread; pass stable references
 * (e.g. `useCallback`) if you don't want the listeners to re-bind.
 */
export function useHotkeys(opts: {
  onToggleInput: () => void;
  onVisionRegion: () => void;
  onOverlay: () => void;
  onToggleWindow?: () => void;
}): void {
  const { onToggleInput, onVisionRegion, onOverlay, onToggleWindow } = opts;

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

  useEffect(() => {
    const p = listen<string>("hotkey:overlay", () => onOverlay());
    return () => {
      p.then((fn) => fn());
    };
  }, [onOverlay]);

  useEffect(() => {
    const p = listen<string>("hotkey:toggle-window", () => onToggleWindow?.());
    return () => {
      p.then((fn) => fn());
    };
  }, [onToggleWindow]);
}
