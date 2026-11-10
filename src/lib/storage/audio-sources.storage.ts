import { safeLocalStorage } from "./helper";

export const AUDIO_SOURCES_STORAGE_KEY = "selected_audio_sources";

/** Matches the Rust `AudioSource` enum (serde tag = "kind"). */
export type AudioSource =
  | { kind: "output"; id: string | null }
  | { kind: "microphone"; id: string | null }
  | { kind: "app"; pid: number };

export const getAudioSources = (): AudioSource[] => {
  try {
    const raw = safeLocalStorage.getItem(AUDIO_SOURCES_STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
};

export const saveAudioSources = (sources: AudioSource[]): void => {
  safeLocalStorage.setItem(
    AUDIO_SOURCES_STORAGE_KEY,
    JSON.stringify(sources)
  );
};

/** Value for the `sources` argument of `start_system_audio_capture`. */
export const getAudioSourcesForCapture = (): AudioSource[] | null => {
  const sources = getAudioSources();
  return sources.length > 0 ? sources : null;
};
