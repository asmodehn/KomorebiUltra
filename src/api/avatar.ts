import { invoke } from "@tauri-apps/api/core";

export async function setLive2dModel(url: string): Promise<void> {
  await invoke("set_live2d_model", { url });
}

export async function setAvatarZoom(value: number): Promise<void> {
  await invoke("set_avatar_zoom", { value });
}

/** True while this launch still has to place the character in the corner: a
 * window-type switch reloads the page inside the same process, so the page
 * cannot tell a reload from a launch by itself. */
export async function avatarCornerPending(): Promise<boolean> {
  return invoke("avatar_corner_pending");
}

/** Records that this launch's corner placement is stored. */
export async function markAvatarCornerDone(): Promise<void> {
  await invoke("avatar_corner_done");
}

export async function setAvatarOffset(
  offsetX: number,
  offsetY: number,
): Promise<void> {
  await invoke("set_avatar_offset", { offsetX, offsetY });
}
