import { Button } from "@/components";
import { cn } from "@/lib/utils";
import { UseSettingsReturn } from "@/types";
import { MicIcon, PlayIcon, SparklesIcon } from "lucide-react";
import { HealthBadge } from "./HealthBadge";
import { ProviderHealth, ProviderKind } from "./useProviderHealth";

const CELLS: {
  kind: ProviderKind;
  label: string;
  icon: typeof MicIcon;
  empty: string;
}[] = [
  {
    kind: "ai",
    label: "Answers",
    icon: SparklesIcon,
    empty: "No AI provider chosen",
  },
  {
    kind: "stt",
    label: "Transcription",
    icon: MicIcon,
    empty: "No speech provider chosen",
  },
];

/**
 * The page's question: can Pluely answer and listen right now?
 * Two cells split by one hairline; the selected cell carries a 2px rule and
 * switches the editor below. Each cell has its own test.
 */
export const ProviderStatus = ({
  settings,
  tab,
  onTab,
  health,
}: {
  settings: UseSettingsReturn;
  tab: ProviderKind;
  onTab: (kind: ProviderKind) => void;
  health: ProviderHealth;
}) => {
  const isConfigured = (kind: ProviderKind) => {
    const selected =
      kind === "ai" ? settings.selectedAIProvider : settings.selectedSttProvider;
    const vars = kind === "ai" ? settings.variables : settings.sttVariables;
    if (!selected?.provider) return false;
    return (vars ?? []).every((v) => selected.variables?.[v.key]?.trim());
  };

  return (
    <div className="grid grid-cols-1 overflow-hidden rounded-xl border border-input/50 sm:grid-cols-2 sm:divide-x sm:divide-input/50">
      {CELLS.map(({ kind, label, icon: Icon, empty }) => {
        const summary = health.summaries[kind];
        const selected = tab === kind;
        const configured = isConfigured(kind);
        const checking = health.health[kind].status === "checking";

        return (
          <div
            key={kind}
            className={cn(
              "relative flex flex-col gap-4 p-4 transition-colors max-sm:border-b max-sm:border-input/50 max-sm:last:border-b-0",
              selected ? "bg-muted/40" : "hover:bg-muted/20"
            )}
          >
            <span
              aria-hidden
              className={cn(
                "absolute inset-x-0 bottom-0 h-0.5 transition-colors",
                selected ? "bg-primary" : "bg-transparent"
              )}
            />
            <button
              type="button"
              aria-pressed={selected}
              onClick={() => onTab(kind)}
              className="flex min-w-0 flex-col items-start gap-1 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/50 rounded-md"
            >
              <span className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                <Icon className="size-3.5" />
                {label}
              </span>
              <span className="max-w-full truncate text-lg font-semibold leading-tight">
                {summary?.isCustom
                  ? "Custom provider"
                  : summary?.id || empty}
              </span>
              <span className="max-w-full truncate font-mono text-xs text-muted-foreground">
                {summary?.host || "—"}
              </span>
            </button>

            <div className="flex items-end justify-between gap-3">
              <HealthBadge
                result={health.health[kind]}
                configured={configured}
                className="flex-1"
              />
              <Button
                size="sm"
                variant={selected ? "default" : "outline"}
                className="shrink-0"
                disabled={!summary || checking}
                onClick={() => health.run(kind)}
              >
                <PlayIcon className="size-3.5" />
                Test
              </Button>
            </div>
          </div>
        );
      })}
    </div>
  );
};
