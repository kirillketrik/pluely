import { useState } from "react";
import { PlayIcon } from "lucide-react";
import {
  AIProviders,
  STTProviders,
  ProviderStatus,
  ProviderKind,
  useProviderHealth,
} from "./components";
import { Button } from "@/components";
import { useSettings } from "@/hooks";
import { PageLayout } from "@/layouts";

const DevSpace = () => {
  const settings = useSettings();
  const health = useProviderHealth(settings);
  const [tab, setTab] = useState<ProviderKind>("ai");

  const checking =
    health.health.ai.status === "checking" ||
    health.health.stt.status === "checking";

  return (
    <PageLayout
      title="Dev Space"
      description="The providers Pluely answers and listens with."
      rightSlot={
        <Button
          size="sm"
          variant="outline"
          disabled={checking}
          onClick={health.runAll}
        >
          <PlayIcon className="size-3.5" />
          Test both
        </Button>
      }
    >
      <ProviderStatus
        settings={settings}
        tab={tab}
        onTab={setTab}
        health={health}
      />

      {tab === "ai" ? (
        <AIProviders {...settings} health={health} />
      ) : (
        <STTProviders {...settings} health={health} />
      )}
    </PageLayout>
  );
};

export default DevSpace;
