import { invoke } from "@tauri-apps/api/core";

export interface WindowOverlayInfo {
  supported: boolean;
  mode: WindowOverlayMode;
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

/** Let the layer surface take keyboard focus once the user picked it. */
export async function allowOverlayKeyboard(): Promise<void> {
  await invoke("overlay_keyboard_on_demand");
}
