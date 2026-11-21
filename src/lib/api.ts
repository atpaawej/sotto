import { invoke } from "@tauri-apps/api/core";

export type ProviderId = "eleven_labs" | "groq";

export type CleanupLevel = "none" | "light" | "medium";

export interface Cleanup {
  level: CleanupLevel;
  model: string;
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

export function updateCleanup(cleanup: Cleanup): Promise<ConfigSnapshot> {
  return invoke<ConfigSnapshot>("update_cleanup", { cleanup });
}

export function updateDictionary(dictionary: string[]): Promise<ConfigSnapshot> {
  return invoke<ConfigSnapshot>("update_dictionary", { dictionary });
}

export function readAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

export interface Timings {
  stopMs: number;
  encodeMs: number;
  uploadMs: number;
  providerMs: number;
  cleanupMs: number;
  totalMs: number;
}

export interface Attempt {
  transcript: string;
  cleaned: string;
  cleanupError: string | null;
  cleanupModel: string | null;
  provider: string;
  model: string;
  durationMs: number;
  timings: Timings;
}

export type Phase =
  | "idle"
  | "listening"
  | "working"
  | "done"
  | "failed"
  | "hint";

/** What the session is doing, as both the window and the pill render it. */
export interface DictationState {
  phase: Phase;
  /** Dictation stays on until the next press. */
  locked: boolean;
  /** Releases never arrived, so the key is a tap rather than a hold. */
  tapMode: boolean;
  message: string | null;
  attempt: Attempt | null;
}

export interface Access {
  hotkey: {
    bound: boolean;
    /** What the system actually assigned, e.g. "Ctrl+Alt+Space". */
    trigger: string | null;
    problem: string | null;
  };
  injection: {
    ready: boolean;
    problem: string | null;
  };
}

export const DICTATION_EVENT = "dictation://state";

/** Starts a dictation, or stops the one running. */
export function toggleDictation(): Promise<void> {
  return invoke<void>("dictation_toggle");
}

/** Drops the dictation in progress and injects nothing. */
export function cancelDictation(): Promise<void> {
  return invoke<void>("dictation_cancel");
}

export function readDictationState(): Promise<DictationState> {
  return invoke<DictationState>("dictation_state");
}

export function readAccess(): Promise<Access> {
  return invoke<Access>("access_status");
}

/** Asks the portal for keyboard access again after a dismissed dialog. */
export function retryKeyboardAccess(): Promise<void> {
  return invoke<void>("retry_keyboard_access");
}

/** Brings the main Settings window to the front. */
export function openMainWindow(): Promise<void> {
  return invoke<void>("open_main_window");
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
