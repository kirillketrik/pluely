import {
  deepVariableReplacer,
  getByPath,
  blobToBase64,
} from "./common.function";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { invoke } from "@tauri-apps/api/core";

import { TYPE_PROVIDER } from "@/types";
import curl2Json from "@bany/curl-to-json";
import { shouldUsePluelyAPI } from "./pluely.api";

// Pluely STT function
async function fetchPluelySTT(audio: File | Blob): Promise<string> {
  try {
    // Convert audio to base64
    const audioBase64 = await blobToBase64(audio);

    // Call Tauri command
    const response = await invoke<{
      success: boolean;
      transcription?: string;
      error?: string;
    }>("transcribe_audio", {
      audioBase64,
    });

    if (response.success && response.transcription) {
      return response.transcription;
    } else {
      return response.error || "Transcription failed";
    }
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    return `Pluely STT Error: ${errorMessage}`;
  }
}

const WS_IDLE_TIMEOUT_MS = 15000;
const WS_MAX_TIMEOUT_MS = 60000;
const WS_FRAME_BYTES = 32 * 1024;
const WS_MAX_BUFFERED = 1024 * 1024;

export interface STTProgress {
  /** Everything heard so far: confirmed text plus the current tentative phrase. */
  text: string;
  /** Confirmed text only. */
  finalText: string;
}

/**
 * Transcribes an audio chunk over a WebSocket (Deepgram-style).
 * Sends the audio in 32 KB frames, then the optional curl `-d` payload
 * (e.g. {"type":"CloseStream"}) and collects every message until the server
 * closes the socket. Browser WebSockets cannot set headers, so a
 * `Sec-WebSocket-Protocol` header is passed as the subprotocol list
 * (e.g. "token, <API_KEY>") and auth can also go in the URL query.
 * Interim results are reported through `onProgress` as they arrive.
 */
function fetchWebSocketSTT(opts: {
  url: string;
  headers: Record<string, string>;
  audio: Blob;
  closePayload?: unknown;
  responsePath: string;
  onProgress?: (progress: STTProgress) => void;
}): Promise<string> {
  const { url, headers, audio, closePayload, responsePath, onProgress } = opts;

  const protocolHeader = Object.entries(headers).find(
    ([k]) => k.toLowerCase() === "sec-websocket-protocol"
  )?.[1];
  const protocols = protocolHeader
    ? protocolHeader
        .split(",")
        .map((p) => p.trim())
        .filter(Boolean)
    : undefined;

  return new Promise<string>((resolve, reject) => {
    const finals: string[] = [];
    let interim = "";
    let serverError = "";
    let settled = false;
    let opened = false;
    let idleTimer: ReturnType<typeof setTimeout>;
    const ws = new WebSocket(url, protocols);
    ws.binaryType = "arraybuffer";

    const report = () =>
      onProgress?.({
        text: [...finals, interim].filter(Boolean).join(" ").trim(),
        finalText: finals.join(" ").trim(),
      });

    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(idleTimer);
      clearTimeout(maxTimer);
      try {
        ws.close();
      } catch {}
      // A phrase that never got confirmed is still better than nothing
      const text = (finals.length ? finals.join(" ") : interim).trim();
      if (text) return resolve(text);
      if (error) return reject(error);
      if (serverError) return reject(new Error(serverError));
      resolve("No transcription found");
    };

    const resetIdle = () => {
      clearTimeout(idleTimer);
      idleTimer = setTimeout(
        () => finish(new Error("Transcription server stopped responding")),
        WS_IDLE_TIMEOUT_MS
      );
    };
    const maxTimer = setTimeout(
      () => finish(new Error("Transcription took too long")),
      WS_MAX_TIMEOUT_MS
    );

    ws.onopen = async () => {
      opened = true;
      try {
        const bytes = await audio.arrayBuffer();
        for (let i = 0; i < bytes.byteLength; i += WS_FRAME_BYTES) {
          if (settled) return;
          while (ws.bufferedAmount > WS_MAX_BUFFERED && !settled) {
            await new Promise((r) => setTimeout(r, 20));
          }
          ws.send(bytes.slice(i, i + WS_FRAME_BYTES));
        }
        if (closePayload && Object.keys(closePayload as object).length > 0) {
          ws.send(
            typeof closePayload === "string"
              ? closePayload
              : JSON.stringify(closePayload)
          );
        }
        resetIdle();
      } catch (e) {
        finish(e instanceof Error ? e : new Error(String(e)));
      }
    };

    ws.onmessage = (event) => {
      resetIdle();
      if (typeof event.data !== "string") return;
      console.debug("[stt-ws]", event.data.slice(0, 300));
      let data: any;
      try {
        data = JSON.parse(event.data);
      } catch {
        const raw = event.data.trim();
        if (raw) {
          finals.push(raw);
          report();
        }
        return;
      }

      const errMsg = data?.err_msg || data?.error || data?.message;
      if (data?.type === "Error" || data?.err_code || data?.error) {
        serverError = `${data?.err_code ? data.err_code + ": " : ""}${
          typeof errMsg === "string" ? errMsg : JSON.stringify(data)
        }`;
        return;
      }

      const value = getByPath(data, responsePath);
      if (typeof value !== "string" || !value.trim()) return;
      if (data?.is_final === false) {
        interim = value.trim();
      } else {
        finals.push(value.trim());
        interim = "";
      }
      report();
    };

    ws.onerror = () =>
      finish(
        new Error(
          opened
            ? "WebSocket connection error"
            : "Could not connect to the transcription server (check the API key and URL)"
        )
      );
    ws.onclose = (e) => {
      if (!e.wasClean && e.code !== 1005 && e.code !== 1000) {
        finish(
          new Error(
            `Connection closed (${e.code}${e.reason ? ": " + e.reason : ""})`
          )
        );
      } else {
        finish();
      }
    };
  });
}

export interface STTParams {
  provider: TYPE_PROVIDER | undefined;
  selectedProvider: {
    provider: string;
    variables: Record<string, string>;
  };
  audio: File | Blob;
  /** Called while a WebSocket provider streams back partial text. */
  onProgress?: (progress: STTProgress) => void;
}

/**
 * Transcribes audio and returns either the transcription or an error/warning message as a single string.
 */
export async function fetchSTT(params: STTParams): Promise<string> {
  let warnings: string[] = [];

  try {
    const { provider, selectedProvider, audio, onProgress } = params;

    // Check if we should use Pluely API instead
    const usePluelyAPI = await shouldUsePluelyAPI();
    if (usePluelyAPI) {
      return await fetchPluelySTT(audio);
    }

    if (!provider) throw new Error("Provider not provided");
    if (!selectedProvider) throw new Error("Selected provider not provided");
    if (!audio) throw new Error("Audio file is required");

    let curlJson: any;
    try {
      curlJson = curl2Json(provider.curl);
    } catch (error) {
      throw new Error(
        `Failed to parse curl: ${
          error instanceof Error ? error.message : "Unknown error"
        }`
      );
    }

    // Validate audio file
    const file = audio as File;
    if (file.size === 0) throw new Error("Audio file is empty");
    // maximum size of 10MB
    // const maxSize = 10 * 1024 * 1024;
    // if (file.size > maxSize) {
    //   warnings.push("Audio exceeds 10MB limit");
    // }

    // Build variable map
    const allVariables = {
      ...Object.fromEntries(
        Object.entries(selectedProvider.variables).map(([key, value]) => [
          key.toUpperCase(),
          value,
        ])
      ),
    };

    // Prepare request
    let url = deepVariableReplacer(curlJson.url || "", allVariables);
    const headers = deepVariableReplacer(curlJson.header || {}, allVariables);
    const formData = deepVariableReplacer(curlJson.form || {}, allVariables);

    // To Check if API accepts Binary Data
    const isBinaryUpload = provider.curl.includes("--data-binary");
    // Fetch URL Params
    const rawParams = curlJson.params || {};
    // Decode Them
    const decodedParams = Object.fromEntries(
      Object.entries(rawParams).map(([key, value]) => [
        key,
        typeof value === "string" ? decodeURIComponent(value) : "",
      ])
    );
    // Get the Parameters from allVariables
    const replacedParams = deepVariableReplacer(decodedParams, allVariables);

    // Add query parameters to URL
    const queryString = new URLSearchParams(replacedParams).toString();
    if (queryString) {
      url += (url.includes("?") ? "&" : "?") + queryString;
    }

    // WebSocket providers: wss:// or ws:// URL in the curl command
    if (/^wss?:\/\//i.test(url)) {
      const rawPath = provider.responseContentPath || "text";
      const wsText = await fetchWebSocketSTT({
        url,
        headers,
        audio,
        closePayload: deepVariableReplacer(curlJson.data || {}, allVariables),
        responsePath: rawPath.charAt(0).toLowerCase() + rawPath.slice(1),
        onProgress,
      });
      return [...warnings, wsText].filter(Boolean).join("; ");
    }

    let finalHeaders = { ...headers };
    let body: FormData | string | Blob;

    const isForm =
      provider.curl.includes("-F ") || provider.curl.includes("--form");
    if (isForm) {
      const form = new FormData();
      const freshBlob = new Blob([await audio.arrayBuffer()], {
        type: audio.type,
      });
      form.append("file", freshBlob, "audio.wav");
      const headerKeys = Object.keys(headers).map((k) =>
        k.toUpperCase().replace(/[-_]/g, "")
      );

      for (const [key, val] of Object.entries(formData)) {
        if (typeof val !== "string") {
          if (
            !val ||
            headerKeys.includes(key.toUpperCase()) ||
            key.toUpperCase() === "AUDIO"
          )
            continue;
          form.append(key.toLowerCase(), val as string | Blob);
          continue;
        }

        // Check if key is a number, which indicates array-like parsing from curl2json
        if (!isNaN(parseInt(key, 10))) {
          const [formKey, ...formValueParts] = val.split("=");
          const formValue = formValueParts.join("=");

          if (formKey.toLowerCase() === "file") continue; // Already handled by form.append('file', audio)

          if (
            !formValue ||
            headerKeys.includes(formKey.toUpperCase().replace(/[-_]/g, ""))
          )
            continue;

          form.append(formKey, formValue);
        } else {
          if (key.toLowerCase() === "file") continue; // Already handled by form.append('file', audio)
          if (
            !val ||
            headerKeys.includes(key.toUpperCase()) ||
            key.toUpperCase() === "AUDIO"
          )
            continue;
          form.append(key.toLowerCase(), val as string | Blob);
        }
      }
      delete finalHeaders["Content-Type"];
      body = form;
    } else if (isBinaryUpload) {
      // Deepgram-style: raw binary body
      body = new Blob([await audio.arrayBuffer()], {
        type: audio.type,
      });
    } else {
      // Google-style: JSON payload with base64
      allVariables.AUDIO = await blobToBase64(audio);
      const dataObj = curlJson.data ? { ...curlJson.data } : {};
      body = JSON.stringify(deepVariableReplacer(dataObj, allVariables));
    }

    const fetchFunction = url?.includes("http") ? fetch : tauriFetch;

    // Send request
    let response: Response;
    try {
      response = await fetchFunction(url, {
        method: curlJson.method || "POST",
        headers: finalHeaders,
        body: curlJson.method === "GET" ? undefined : body,
      });
    } catch (e) {
      throw new Error(`Network error: ${e instanceof Error ? e.message : e}`);
    }

    if (!response.ok) {
      let errText = "";
      try {
        errText = await response.text();
      } catch {}
      let errMsg: string;
      try {
        const errObj = JSON.parse(errText);
        errMsg = errObj.message || errText;
      } catch {
        errMsg = errText || response.statusText;
      }
      throw new Error(`HTTP ${response.status}: ${errMsg}`);
    }

    const responseText = await response.text();
    let data: any;
    try {
      data = JSON.parse(responseText);
    } catch {
      return [...warnings, responseText.trim()].filter(Boolean).join("; ");
    }

    // Extract transcription
    const rawPath = provider.responseContentPath || "text";
    const path = rawPath.charAt(0).toLowerCase() + rawPath.slice(1);
    const transcription = (getByPath(data, path) || "").trim();

    if (!transcription) {
      return [...warnings, "No transcription found"].join("; ");
    }

    // Return transcription with any warnings
    return [...warnings, transcription].filter(Boolean).join("; ");
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    throw new Error(msg);
  }
}
