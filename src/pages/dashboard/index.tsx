import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { GetLicense } from "@/components";
import { PluelyApiSetup, Usage } from "./components";
import { PageLayout } from "@/layouts";
import { useApp } from "@/contexts";
import { usePluelyBackend } from "@/hooks/usePluelyBackend";

const Dashboard = () => {
  const { hasActiveLicense } = useApp();
  const pluelyBackend = usePluelyBackend();
  const [activity, setActivity] = useState<any>(null);
  const [loadingActivity, setLoadingActivity] = useState(false);

  const fetchActivity = useCallback(async () => {
    if (!hasActiveLicense || !pluelyBackend) {
      setActivity({ data: [], total_tokens_used: 0 });
      return;
    }
    setLoadingActivity(true);
    try {
      const response = await invoke("get_activity");
      const responseData: any = response;
      if (responseData && responseData.success) {
        setActivity(responseData);
      } else {
        setActivity({ data: [], total_tokens_used: 0 });
      }
    } catch (error) {
      setActivity({ data: [], total_tokens_used: 0 });
    } finally {
      setLoadingActivity(false);
    }
  }, [hasActiveLicense, pluelyBackend]);

  useEffect(() => {
    if (hasActiveLicense && pluelyBackend) {
      fetchActivity();
    } else {
      setActivity(null);
    }
  }, [fetchActivity, hasActiveLicense, pluelyBackend]);

  const activityData =
    activity && Array.isArray(activity.data) ? activity.data : [];
  const totalTokens =
    activity && typeof activity.total_tokens_used === "number"
      ? activity.total_tokens_used
      : 0;

  return (
    <PageLayout
      title="Dashboard"
      description="Pluely license to unlock faster responses, quicker support and premium features."
      rightSlot={!hasActiveLicense && pluelyBackend ? <GetLicense /> : null}
    >
      {pluelyBackend ? (
        <>
          {/* Pluely API Setup */}
          <PluelyApiSetup />

          <Usage
            loading={loadingActivity}
            onRefresh={fetchActivity}
            data={activityData}
            totalTokens={totalTokens}
          />
        </>
      ) : (
        pluelyBackend === false && (
          <div className="text-sm text-muted-foreground bg-muted/40 p-4 rounded-md">
            The hosted Pluely API is not configured in this build. Add your own
            AI and speech-to-text providers in Dev space.
          </div>
        )
      )}
    </PageLayout>
  );
};

export default Dashboard;
