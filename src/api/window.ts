import { invoke } from "@tauri-apps/api/core";

/** Let the layer surface take keyboard focus once the user picked it. */
export async function allowOverlayKeyboard(): Promise<void> {
  await invoke("overlay_keyboard_on_demand");
}
