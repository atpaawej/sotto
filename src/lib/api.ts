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
  cleanupFailure: Failure | null;
  cleanupModel: string | null;
  provider: string;
  model: string;
  durationMs: number;
  timings: Timings;
}

/** The one failure vocabulary, as the Rust side serializes it. */
export interface Failure {
  kind: FailureKind;
  /** The short sentence the Pill shows, derived once in Rust. */
  reason: string;
  /** The full sentence Settings shows. */
  detail: string;
  /** The cause chain for the log, when it differs from the detail. */
  cause: string | null;
}

export type FailureKind =
  | "key_rejected"
  | "rate_limited"
  | "provider_error"
  | "network"
  | "timeout"
  | "mic_denied"
  | "shortcut_refused"
  | "injection_blocked"
  | "storage_failed";

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
  failure: Failure | null;
  /** The Recording Retry would send again, when there is one on disk. */
  retryId: string | null;
}

export type InjectionOutcome = "not_attempted" | "inserted" | "blocked";

/** One Dictation as History shows it. */
export interface HistoryEntry {
  id: string;
  createdAtMs: number;
  finishedAtMs: number | null;
  durationMs: number;
  provider: string;
  model: string;
  transcript: string;
  cleaned: string;
  injection: InjectionOutcome;
  failure: Failure | null;
  recordingPath: string;
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

/** Re-transcribes the last failed Recording. The user does not speak again. */
export function retryDictation(): Promise<void> {
  return invoke<void>("dictation_retry");
}

/** Newest first. An empty query lists everything. */
export function readHistory(query = ""): Promise<HistoryEntry[]> {
  return invoke<HistoryEntry[]>("history_list", { query });
}

export function retryHistory(id: string): Promise<void> {
  return invoke<void>("history_retry", { id });
}

export function copyHistory(id: string): Promise<void> {
  return invoke<void>("history_copy", { id });
}

/** The WAV bytes for one entry, for the Play button. */
export function readRecording(id: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("history_recording", { id });
}

export function deleteHistory(id: string): Promise<void> {
  return invoke<void>("history_delete", { id });
}

export function deleteAllHistory(): Promise<void> {
  return invoke<void>("history_delete_all");
}

/** Saves the limit and deletes anything already older than it. */
export function updateRetention(days: number | null): Promise<ConfigSnapshot> {
  return invoke<ConfigSnapshot>("update_retention", { days });
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
