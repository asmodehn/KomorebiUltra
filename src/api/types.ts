// Shared API types — mirrors the Rust commands/ + state structs.
//
// Live values + invoke wrappers live in sibling modules grouped by domain.
// Keeping types in one place avoids import cycles between those modules.

export type ChatEvent =
  | { kind: "started"; id: string; route: "local" | "cloud" | "skill" }
  | { kind: "token"; id: string; text: string }
  | { kind: "done"; id: string; full_text: string }
  | { kind: "error"; id: string; message: string };

export type Mode = "auto" | "local" | "cloud";

export interface PublicSettings {
  has_openrouter_key: boolean;
  openrouter_model: string;
  mode: string;
  local_model_path: string | null;
  local_classifier_model_path?: string | null;
  tts_enabled: boolean;
  piper_binary_path: string | null;
  piper_voice_path: string | null;
  live2d_model_url: string | null;
  whisper_model_path: string | null;
  stt_available: boolean;
  wake_word: string | null;
  listen_enabled: boolean;
  smart_routing: boolean;
  classifier_model: string;
  rag_enabled: boolean;
  acp_agent_command: string | null;
  audio_input_device: string | null;
  audio_output_device: string | null;
  llm_gpu_layers: number | null;
  auto_listen: boolean;
  tts_provider: string;
  tts_length_scale: number | null;
  tts_noise_scale: number | null;
  tts_noise_w: number | null;
  tts_volume: number;
  sovits_endpoint: string | null;
  sovits_ref_audio: string | null;
  sovits_prompt_text: string | null;
  sovits_prompt_lang: string;
  sovits_text_lang: string;
  sovits_speed: number;
  openrouter_tts_enabled?: boolean;
  openrouter_tts_model?: string;
  openrouter_tts_voice?: string;
  openrouter_stt_enabled?: boolean;
  openrouter_stt_model?: string;
  game_coach_enabled?: boolean;
  game_coach_model?: string;
  game_coach_use_vision?: boolean;
  auto_screen_watch_enabled?: boolean;
  chat_tool_calls_enabled?: boolean;
  faster_whisper_enabled?: boolean;
  faster_whisper_url?: string;
  faster_whisper_model?: string;
  faster_whisper_language?: string | null;
  has_deepgram_key?: boolean;
  deepgram_enabled?: boolean;
  deepgram_model?: string;
  deepgram_language?: string | null;
  avatar_zoom?: number;
  avatar_offset_x?: number;
  avatar_offset_y?: number;
  imagegen_provider: string;
  imagegen_openrouter_model: string;
  imagegen_replicate_model: string;
  imagegen_local_binary: string | null;
  imagegen_local_model: string | null;
  imagegen_device: string;
  imagegen_width: number;
  imagegen_height: number;
  imagegen_steps: number;
  imagegen_negative_prompt: string | null;
  has_replicate_token: boolean;
  weather_provider?: string;
  weather_default_city?: string | null;
  weather_use_ip?: boolean;
  weather_units?: string;
  has_weather_api_key?: boolean;
  user_name?: string | null;
  relationship_visibility?: string;
  relationship_nsfw_allowed?: boolean;
  relationship_decay_enabled?: boolean;
  language?: string;
  // Phase 1: feedback telemetry
  telemetry_enabled?: boolean;
  telemetry_endpoint?: string;
  anon_token?: string | null;
  // Phase 2 stub: local LoRA training
  training_enabled?: boolean;
  training_max_cpu_pct?: number;
  training_battery_floor_pct?: number;
  training_min_examples?: number;
  training_schedule?: string;
}

export interface FolderStats {
  path: string;
  doc_count: number;
  chunk_count: number;
  indexed_at: number | null;
}

export interface IndexReport {
  files_scanned: number;
  files_indexed: number;
  files_skipped: number;
  chunks_written: number;
}

export type AssetKind =
  | "llm_gguf"
  | "piper_voice"
  | "piper_config"
  | "whisper_ggml";

export interface Asset {
  id: string;
  kind: AssetKind;
  title: string;
  description: string;
  file_name: string;
  approx_size_mb: number;
  installed: boolean;
  path: string | null;
}

export type DownloadEvent =
  | {
      kind: "started";
      file_name: string;
      total: number | null;
      resumed_from: number;
    }
  | {
      kind: "progress";
      file_name: string;
      downloaded: number;
      total: number | null;
    }
  | { kind: "verifying"; file_name: string }
  | { kind: "finished"; file_name: string; path: string }
  | { kind: "failed"; file_name: string; message: string };

export interface AudioDevices {
  inputs: string[];
  outputs: string[];
  default_input: string | null;
  default_output: string | null;
}

export interface SystemInfo {
  os: string;
  cpu: string;
  cpu_cores: number;
  ram_gb: number;
  gpus: string[];
  has_nvidia: boolean;
  hostname: string;
}

export interface OpenRouterModel {
  id: string;
  name?: string;
  context_length?: number;
  pricing?: { prompt?: string; completion?: string };
  architecture?: {
    modality?: string;
    input_modalities?: string[];
    output_modalities?: string[];
  };
}

export interface VisionRegion {
  monitor?: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ScreenInfo {
  id: number;
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  is_primary: boolean;
  scale_factor: number;
}

export type ImageEvent =
  | {
      kind: "started";
      id: string;
      provider: string;
      width: number;
      height: number;
    }
  | {
      kind: "done";
      id: string;
      png_base64: string;
      save_path: string | null;
      mime: string;
    }
  | { kind: "error"; id: string; message: string };

export interface WeatherLocation {
  name: string;
  country: string | null;
  lat: number;
  lon: number;
}

export interface WeatherReport {
  location: WeatherLocation;
  provider: string;
  temperature: number;
  feels_like: number | null;
  humidity: number | null;
  wind_speed: number | null;
  description: string;
  icon: string;
  units: string;
}

export type RelationshipStage =
  | "stranger"
  | "acquaintance"
  | "friend"
  | "close"
  | "trusted"
  | "romantic"
  | "lover";

export interface RelationshipEvent {
  ts: number;
  kind: string;
  delta: number;
  note: string;
}

export interface RelationshipState {
  score: number;
  stage: RelationshipStage;
  last_interaction_at: number;
  last_decay_at: number;
  total_interactions: number;
  daily_streak: number;
  last_compliment_at: number;
  events: RelationshipEvent[];
}

export interface RelationshipStageChange {
  previous: RelationshipStage;
  current: RelationshipStage;
  score: number;
}

export interface FeedbackStats {
  pending: number;
  uploaded: number;
  telemetry_enabled: boolean;
  anon_token: string | null;
}

export interface SoVitsConfigInput {
  endpoint: string;
  refAudio: string;
  promptText: string;
  promptLang: string;
  textLang: string;
  speed: number;
}
