import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { HealthResult } from "@/lib/functions/provider-health.function";

const time = (ts?: number) =>
  ts
    ? new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : "";

/** One status mark: a 8px dot and a short line. Colour carries the state only. */
export const HealthBadge = ({
  result,
  configured,
  className,
}: {
  result: HealthResult;
  configured: boolean;
  className?: string;
}) => {
  let dot = "bg-muted-foreground/40";
  let label = configured ? "Not checked" : "Not set up";
  let tone = "text-muted-foreground";

  if (result.status === "checking") {
    label = "Checking…";
  } else if (result.status === "ok") {
    dot = "bg-emerald-500";
    tone = "text-emerald-700 dark:text-emerald-400";
    label = `Working · ${result.latencyMs?.toLocaleString("en-US")} ms`;
  } else if (result.status === "fail") {
    dot = "bg-destructive";
    tone = "text-destructive";
    label = "Not working";
  }

  return (
    <div className={cn("min-w-0", className)}>
      <div className={cn("flex items-center gap-2 text-sm font-medium", tone)}>
        {result.status === "checking" ? (
          <Loader2 className="size-3.5 shrink-0 animate-spin" />
        ) : (
          <span className={cn("size-2 shrink-0 rounded-full", dot)} />
        )}
        <span className="truncate tabular-nums">{label}</span>
        {result.checkedAt && result.status !== "checking" ? (
          <span className="shrink-0 text-xs font-normal text-muted-foreground tabular-nums">
            {time(result.checkedAt)}
          </span>
        ) : null}
      </div>
      {result.status === "fail" && result.detail ? (
        <p
          title={result.detail}
          className="mt-1 line-clamp-2 break-words text-xs text-muted-foreground"
        >
          {result.detail}
        </p>
      ) : null}
    </div>
  );
};
