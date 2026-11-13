import { invoke } from "@tauri-apps/api/core";

export interface WindowOverlayInfo {
  supported: boolean;
  mode: WindowOverlayMode;
  /** The mode was pinned by KOMOREBI_START_MODE (explicit launch). */
  forced: boolean;
}

export type WindowOverlayMode = "normal" | "overlay";

export async function getWindowOverlayInfo(): Promise<WindowOverlayInfo> {
  return invoke<WindowOverlayInfo>("window_overlay_info");
}

export async function setWindowOverlay(
  mode: WindowOverlayMode,
): Promise<WindowOverlayMode> {
  return invoke<WindowOverlayMode>("set_window_overlay", { mode });
}
