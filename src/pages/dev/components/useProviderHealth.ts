import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import curl2Json from "@bany/curl-to-json";
import { UseSettingsReturn } from "@/types";
import {
  checkAIProvider,
  checkSttProvider,
  HealthResult,
} from "@/lib/functions/provider-health.function";

export type ProviderKind = "ai" | "stt";

const IDLE: HealthResult = { status: "idle" };

export interface ProviderSummary {
  id: string;
  isCustom: boolean;
  host: string;
  method: string;
}

const summarize = (curl: string | undefined): { host: string; method: string } => {
  if (!curl) return { host: "", method: "" };
  try {
    const json = curl2Json(curl);
    const url = (json?.url || "").replace(/\{\{[A-Z_]+\}\}/g, "x");
    let host = url;
    try {
      host = new URL(url).host;
    } catch {
      host = url.replace(/^\w+:\/\//, "").split(/[/?]/)[0];
    }
    return {
      host,
      method: /^wss?:\/\//i.test(url) ? "WS" : json?.method || "POST",
    };
  } catch {
    return { host: "", method: "" };
  }
};

/**
 * Health state for the active AI and STT providers. A result is dropped as
 * soon as the provider or any of its variables change, so a green dot never
 * describes a configuration that is no longer the one on screen.
 */
export const useProviderHealth = (settings: UseSettingsReturn) => {
  const {
    allAiProviders,
    allSttProviders,
    selectedAIProvider,
    selectedSttProvider,
  } = settings;

  const [health, setHealth] = useState<Record<ProviderKind, HealthResult>>({
    ai: IDLE,
    stt: IDLE,
  });
  const runIds = useRef<Record<ProviderKind, number>>({ ai: 0, stt: 0 });

  const aiProvider = allAiProviders?.find(
    (p) => p?.id === selectedAIProvider?.provider
  );
  const sttProvider = allSttProviders?.find(
    (p) => p?.id === selectedSttProvider?.provider
  );

  const aiSignature = JSON.stringify(selectedAIProvider);
  const sttSignature = JSON.stringify(selectedSttProvider);

  useEffect(() => {
    runIds.current.ai += 1;
    setHealth((h) => ({ ...h, ai: IDLE }));
  }, [aiSignature]);

  useEffect(() => {
    runIds.current.stt += 1;
    setHealth((h) => ({ ...h, stt: IDLE }));
  }, [sttSignature]);

  const run = useCallback(
    async (kind: ProviderKind) => {
      const runId = ++runIds.current[kind];
      setHealth((h) => ({ ...h, [kind]: { status: "checking" } }));
      const result =
        kind === "ai"
          ? await checkAIProvider(aiProvider, selectedAIProvider)
          : await checkSttProvider(sttProvider, selectedSttProvider);
      if (runId === runIds.current[kind]) {
        setHealth((h) => ({ ...h, [kind]: result }));
      }
    },
    [aiProvider, sttProvider, selectedAIProvider, selectedSttProvider]
  );

  const runAll = useCallback(() => {
    run("ai");
    run("stt");
  }, [run]);

  const summaries = useMemo<Record<ProviderKind, ProviderSummary | null>>(
    () => ({
      ai: aiProvider
        ? {
            id: aiProvider.id || "",
            isCustom: !!aiProvider.isCustom,
            ...summarize(aiProvider.curl),
          }
        : null,
      stt: sttProvider
        ? {
            id: sttProvider.id || "",
            isCustom: !!sttProvider.isCustom,
            ...summarize(sttProvider.curl),
          }
        : null,
    }),
    [aiProvider, sttProvider]
  );

  return { health, run, runAll, summaries };
};

export type ProviderHealth = ReturnType<typeof useProviderHealth>;
