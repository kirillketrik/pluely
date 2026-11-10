import { UseSettingsReturn } from "@/types";
import { CustomProviders } from "./CustomProvider";
import { ProviderPanel } from "../ProviderPanel";
import { ProviderHealth } from "../useProviderHealth";

export const STTProviders = ({
  health,
  ...settings
}: UseSettingsReturn & { health: ProviderHealth }) => {
  return (
    <div id="stt-providers" className="space-y-8">
      <ProviderPanel kind="stt" settings={settings} health={health} />
      <div className="border-t border-input/50 pt-6">
        <CustomProviders {...settings} />
      </div>
    </div>
  );
};
