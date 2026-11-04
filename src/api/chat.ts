import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type {
  ChatEvent,
  Mode,
  OpenRouterModel,
  PublicSettings,
} from "./types";

export async function getSettings(): Promise<PublicSettings> {
  return invoke<PublicSettings>("get_settings");
}

export async function setOpenRouterKey(key: string): Promise<void> {
  await invoke("set_openrouter_key", { key });
}

export async function setMode(mode: Mode): Promise<void> {
  await invoke("set_mode", { mode });
}

export async function sendMessage(prompt: string): Promise<string> {
  return invoke<string>("send_message", { prompt });
}

export async function cancelGeneration(): Promise<void> {
  await invoke("cancel_generation");
}

export async function resetChat(): Promise<void> {
  await invoke("reset_chat");
}

export async function setSmartRouting(enabled: boolean): Promise<void> {
  await invoke("set_smart_routing", { enabled });
}

export async function setClassifierModel(model: string): Promise<void> {
  await invoke("set_classifier_model", { model });
}

export async function setAcpAgentCommand(command: string): Promise<void> {
  await invoke("set_acp_agent_command", { command });
}

export async function listOpenRouterModels(): Promise<OpenRouterModel[]> {
  return invoke<OpenRouterModel[]>("list_openrouter_models");
}

export async function setOpenRouterModel(model: string): Promise<void> {
  await invoke("set_openrouter_model", { model });
}

export function onChat(cb: (e: ChatEvent) => void): Promise<UnlistenFn> {
  return listen<ChatEvent>("chat", (evt) => cb(evt.payload));
}
