import { TYPE_PROVIDER } from "@/types";
import { extractVariables } from "./common.function";
import { fetchAIResponse } from "./ai-response.function";
import { fetchSTT } from "./stt.function";

export type HealthStatus = "idle" | "checking" | "ok" | "fail";

export interface HealthResult {
  status: HealthStatus;
  latencyMs?: number;
  detail?: string;
  checkedAt?: number;
}

type SelectedProvider = { provider: string; variables: Record<string, string> };

const AI_TIMEOUT_MS = 20000;
const STT_TIMEOUT_MS = 20000;

const AI_ERROR_PREFIXES = [
  "API request failed",
  "Network error",
  "Failed to parse",
  "Streaming not supported",
  "Error reading stream",
  "Pluely API Error",
];

const missingVariables = (
  provider: TYPE_PROVIDER,
  selected: SelectedProvider,
  ignore: string[]
) =>
  extractVariables(provider.curl, true)
    .filter(({ value }) => !ignore.includes(value))
    .filter(({ key }) => !selected.variables?.[key]?.trim())
    .map(({ value }) => value);

const withTimeout = <T>(promise: Promise<T>, ms: number): Promise<T> =>
  new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`No response within ${ms / 1000} s`)),
      ms
    );
    promise.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      }
    );
  });

const failure = (detail: string): HealthResult => ({
  status: "fail",
  detail,
  checkedAt: Date.now(),
});

const cleanMessage = (error: unknown): string => {
  const raw = error instanceof Error ? error.message : String(error);
  return raw.replace(/^Error in fetchAIResponse:\s*/i, "").slice(0, 400);
};

/**
 * Sends a one-word prompt through the real request path used by the chat,
 * so a pass means the key, URL, model and response path all work.
 */
export async function checkAIProvider(
  provider: TYPE_PROVIDER | undefined,
  selected: SelectedProvider
): Promise<HealthResult> {
  if (!provider) return failure("No provider selected");
  const missing = missingVariables(provider, selected, [
    "SYSTEM_PROMPT",
    "TEXT",
    "IMAGE",
  ]);
  if (missing.length) {
    return failure(`Missing ${missing.join(", ").replace(/_/g, " ")}`);
  }

  const started = performance.now();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), AI_TIMEOUT_MS);

  try {
    let reply = "";
    for await (const chunk of fetchAIResponse({
      provider,
      selectedProvider: selected,
      userMessage: "Reply with the single word: ok",
      signal: controller.signal,
    })) {
      reply += chunk;
      if (reply.trim()) break;
    }

    if (controller.signal.aborted && !reply.trim()) {
      return failure(`No response within ${AI_TIMEOUT_MS / 1000} s`);
    }
    if (!reply.trim()) {
      return failure("Empty reply. Check the response path of this provider");
    }
    if (AI_ERROR_PREFIXES.some((p) => reply.startsWith(p))) {
      return failure(reply.slice(0, 400));
    }
    return {
      status: "ok",
      latencyMs: Math.round(performance.now() - started),
      checkedAt: Date.now(),
    };
  } catch (error) {
    return failure(cleanMessage(error));
  } finally {
    clearTimeout(timer);
  }
}

/** Half a second of 16 kHz mono silence as a WAV file. */
function silentWav(seconds = 0.5, sampleRate = 16000): Blob {
  const samples = Math.floor(seconds * sampleRate);
  const buffer = new ArrayBuffer(44 + samples * 2);
  const view = new DataView(buffer);
  const write = (offset: number, text: string) =>
    [...text].forEach((c, i) => view.setUint8(offset + i, c.charCodeAt(0)));
  write(0, "RIFF");
  view.setUint32(4, 36 + samples * 2, true);
  write(8, "WAVE");
  write(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  write(36, "data");
  view.setUint32(40, samples * 2, true);
  return new Blob([buffer], { type: "audio/wav" });
}

/**
 * Uploads a short silent clip. Any reply, even "No transcription found",
 * proves the endpoint, key and request format are accepted.
 */
export async function checkSttProvider(
  provider: TYPE_PROVIDER | undefined,
  selected: SelectedProvider
): Promise<HealthResult> {
  if (!provider) return failure("No provider selected");
  const missing = missingVariables(provider, selected, ["AUDIO"]);
  if (missing.length) {
    return failure(`Missing ${missing.join(", ").replace(/_/g, " ")}`);
  }

  const started = performance.now();
  try {
    await withTimeout(
      fetchSTT({ provider, selectedProvider: selected, audio: silentWav() }),
      STT_TIMEOUT_MS
    );
    return {
      status: "ok",
      latencyMs: Math.round(performance.now() - started),
      checkedAt: Date.now(),
    };
  } catch (error) {
    return failure(cleanMessage(error));
  }
}
