import { invoke } from "@tauri-apps/api/core";

export type ProviderId = "eleven_labs" | "groq";

export interface Cleanup {
  enabled: boolean;
  fillers: boolean;
  spoken_punctuation: boolean;
  whitespace: boolean;
  dictionary: boolean;
}

export interface Config {
  version: number;
  trigger_key: string;
  active_provider: ProviderId;
  providers: {
    elevenlabs_key: string | null;
    groq_key: string | null;
  };
  cleanup: Cleanup;
  dictionary: string[];
  retention_days: number | null;
}

export interface ConfigSnapshot {
  path: string;
  mode: string;
  config: Config;
}

export interface AppInfo {
  version: string;
  license: string;
  repository: string;
}

/** Rejects with the Rust-side message, cause chain included. */
export function readConfig(): Promise<ConfigSnapshot> {
  return invoke<ConfigSnapshot>("config_snapshot");
}

export function readAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

export interface Timings {
  stopMs: number;
  encodeMs: number;
  uploadMs: number;
  providerMs: number;
  totalMs: number;
}

export interface Attempt {
  transcript: string;
  provider: string;
  model: string;
  durationMs: number;
  timings: Timings;
}

/** Opens the microphone. Rejects before recording if there is no key. */
export function startDictation(): Promise<void> {
  return invoke<void>("dictation_start");
}

/** Stops the microphone, transcribes, and returns the text with its timings. */
export function stopDictation(): Promise<Attempt> {
  return invoke<Attempt>("dictation_stop");
}

/** True when the backend is still recording, e.g. after a window reload. */
export function isRecording(): Promise<boolean> {
  return invoke<boolean>("dictation_recording");
}

const KEY_NAMES: Record<string, string> = {
  ControlRight: "Right Ctrl",
  ControlLeft: "Left Ctrl",
  AltRight: "Right Alt",
  ShiftRight: "Right Shift",
  SuperRight: "Right Super",
};

export function keyLabel(key: string): string {
  return KEY_NAMES[key] ?? key;
}

export function providerLabel(provider: ProviderId): string {
  return provider === "groq" ? "Groq" : "ElevenLabs";
}
