import { useState } from "react";
import { EyeIcon, EyeOffIcon, PlayIcon, TrashIcon } from "lucide-react";
import { Button, Input, Selection, TextInput } from "@/components";
import { UseSettingsReturn } from "@/types";
import curl2Json from "@bany/curl-to-json";
import { HealthBadge } from "./HealthBadge";
import { ProviderHealth, ProviderKind } from "./useProviderHealth";

const Field = ({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) => (
  <div className="space-y-1.5">
    <div className="flex items-baseline justify-between gap-3">
      <label className="text-sm font-medium">{label}</label>
      {hint ? (
        <span className="text-xs text-muted-foreground">{hint}</span>
      ) : null}
    </div>
    {children}
  </div>
);

/**
 * Active provider editor for either AI or STT: pick, see the endpoint,
 * fill the key and the other variables, test. One component for both kinds.
 */
export const ProviderPanel = ({
  kind,
  settings,
  health,
}: {
  kind: ProviderKind;
  settings: UseSettingsReturn;
  health: ProviderHealth;
}) => {
  const [reveal, setReveal] = useState(false);

  const all = kind === "ai" ? settings.allAiProviders : settings.allSttProviders;
  const selected =
    kind === "ai" ? settings.selectedAIProvider : settings.selectedSttProvider;
  const setSelected =
    kind === "ai"
      ? settings.onSetSelectedAIProvider
      : settings.onSetSelectedSttProvider;
  const vars = (kind === "ai" ? settings.variables : settings.sttVariables) ?? [];

  const apiKeyVar = vars.find((v) => v.key === "api_key");
  const otherVars = vars.filter((v) => v.key !== "api_key");
  const summary = health.summaries[kind];
  const result = health.health[kind];

  const setVariable = (key: string, value: string) =>
    setSelected({
      provider: selected?.provider,
      variables: { ...selected?.variables, [key]: value },
    });

  const wsHint = kind === "stt" && summary?.method === "WS";

  return (
    <div className="space-y-5">
      <Field
        label={kind === "ai" ? "Provider" : "Speech provider"}
        hint={wsHint ? "Streams over WebSocket" : undefined}
      >
        <Selection
          selected={selected?.provider}
          options={all?.map((provider) => {
            const json = (() => {
              try {
                return curl2Json(provider?.curl);
              } catch {
                return null;
              }
            })();
            return {
              label: provider?.isCustom
                ? json?.url || "Custom Provider"
                : provider?.id || "Custom Provider",
              value: provider?.id || "Custom Provider",
              isCustom: provider?.isCustom,
            };
          })}
          placeholder={
            kind === "ai" ? "Choose your AI provider" : "Choose your STT provider"
          }
          onChange={(value) => setSelected({ provider: value, variables: {} })}
        />
        {summary?.host ? (
          <p className="flex items-center gap-2 pt-0.5 text-xs text-muted-foreground">
            <span className="rounded border border-input/60 px-1.5 py-px font-mono text-[11px] font-medium text-foreground">
              {summary.method}
            </span>
            <span className="truncate font-mono">{summary.host}</span>
          </p>
        ) : null}
      </Field>

      {apiKeyVar ? (
        <Field label="API key" hint="Stored on this device only">
          <div className="flex gap-2">
            <Input
              type={reveal ? "text" : "password"}
              placeholder="Paste your key"
              value={selected?.variables?.[apiKeyVar.key] || ""}
              onChange={(value) =>
                setVariable(
                  apiKeyVar.key,
                  typeof value === "string" ? value : value.target.value
                )
              }
              className="h-10 flex-1 font-mono text-sm"
            />
            <Button
              type="button"
              size="icon"
              variant="outline"
              className="size-10 shrink-0"
              title={reveal ? "Hide key" : "Show key"}
              onClick={() => setReveal((r) => !r)}
            >
              {reveal ? (
                <EyeOffIcon className="size-4" />
              ) : (
                <EyeIcon className="size-4" />
              )}
            </Button>
            <Button
              type="button"
              size="icon"
              variant="outline"
              className="size-10 shrink-0 text-destructive hover:text-destructive"
              title="Clear key"
              disabled={!selected?.variables?.[apiKeyVar.key]}
              onClick={() => setVariable(apiKeyVar.key, "")}
            >
              <TrashIcon className="size-4" />
            </Button>
          </div>
        </Field>
      ) : null}

      {otherVars.length > 0 ? (
        <div className="grid gap-4 sm:grid-cols-2">
          {otherVars.map((variable) => (
            <Field
              key={variable.key}
              label={variable.key
                .replace(/_/g, " ")
                .replace(/^\w/, (c) => c.toUpperCase())}
            >
              <TextInput
                placeholder={`Enter ${variable.key.replace(/_/g, " ")}`}
                value={selected?.variables?.[variable.key] || ""}
                onChange={(value) => setVariable(variable.key, value)}
              />
            </Field>
          ))}
        </div>
      ) : null}

      <div className="flex items-center justify-between gap-4 border-t border-input/50 pt-4">
        <HealthBadge result={result} configured={!!summary} className="flex-1" />
        <Button
          variant="outline"
          disabled={!summary || result.status === "checking"}
          onClick={() => health.run(kind)}
        >
          <PlayIcon className="size-3.5" />
          {kind === "ai" ? "Send test message" : "Send test audio"}
        </Button>
      </div>
    </div>
  );
};
