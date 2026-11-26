import { Header, Button } from "@/components";
import {
  AppWindowIcon,
  AudioLinesIcon,
  HeadphonesIcon,
  MicIcon,
  RefreshCwIcon,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AudioSource,
  getAudioSources,
  saveAudioSources,
} from "@/lib/storage/audio-sources.storage";

type Device = { id: string; name: string; is_default: boolean };
type AudioApp = { pid: number; name: string; titles: string[] };
type Probe = {
  source: AudioSource;
  started: boolean;
  mode: string | null;
  error: string | null;
  samples: number;
  peak: number;
  rms: number;
};
type Playing = {
  pid: number;
  name: string;
  device: string;
  active: boolean;
  peak: number;
  level_known: boolean;
};

const SILENT_PEAK = 0.002;

const sameSource = (a: AudioSource, b: AudioSource) =>
  a.kind === b.kind &&
  (a.kind === "app"
    ? a.pid === (b as typeof a).pid
    : a.id === (b as typeof a).id);

// Square root so that quiet speech still moves the bar
const levelPct = (peak: number) =>
  Math.min(100, Math.round(Math.sqrt(peak) * 100));

const Level = ({
  peak,
  className = "",
}: {
  peak: number;
  className?: string;
}) => (
  <div className={`h-1 rounded-full bg-muted ${className}`}>
    <div
      className="h-1 rounded-full bg-emerald-500"
      style={{ width: `${levelPct(peak)}%` }}
    />
  </div>
);

export const AudioSources = () => {
  const [selected, setSelected] = useState<AudioSource[]>(getAudioSources);
  const [outputs, setOutputs] = useState<Device[]>([]);
  const [mics, setMics] = useState<Device[]>([]);
  const [apps, setApps] = useState<AudioApp[]>([]);
  const [isLoading, setIsLoading] = useState(false);

  const load = useCallback(async () => {
    setIsLoading(true);
    try {
      const [outputDevices, inputDevices, runningApps] = await Promise.all([
        invoke<Device[]>("get_output_devices"),
        invoke<Device[]>("get_input_devices"),
        invoke<AudioApp[]>("get_audio_apps"),
      ]);
      setOutputs(outputDevices);
      setMics(inputDevices);
      setApps(runningApps);
    } catch (error) {
      console.error("Failed to load audio sources:", error);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const isSelected = (source: AudioSource) =>
    selected.some((s) => sameSource(s, source));

  const toggle = (source: AudioSource) => {
    const next = isSelected(source)
      ? selected.filter((s) => !sameSource(s, source))
      : [...selected, source];
    setSelected(next);
    saveAudioSources(next);
  };

  const [probing, setProbing] = useState(false);
  const [probes, setProbes] = useState<Probe[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [playing, setPlaying] = useState<Playing[] | null>(null);

  const labelOf = (source: AudioSource) => {
    if (source.kind === "app") {
      const known =
        apps.find((a) => a.pid === source.pid) ??
        playing?.find((p) => p.pid === source.pid);
      return known
        ? `${known.name} (pid ${source.pid})`
        : `App pid ${source.pid}`;
    }
    const list = source.kind === "output" ? outputs : mics;
    const device = source.id
      ? list.find((d) => d.id === source.id)
      : list.find((d) => d.is_default);
    return (
      (source.kind === "output" ? "Output: " : "Mic: ") +
      (device?.name ?? "Default")
    );
  };

  const runProbe = async (targets?: AudioSource[]) => {
    setProbing(true);
    setProbes(null);
    try {
      const sources: AudioSource[] =
        targets ??
        (selected.length > 0 ? selected : [{ kind: "output", id: null }]);
      setProbes(
        await invoke<Probe[]>("probe_audio_sources", { sources, seconds: 3 })
      );
    } catch (error) {
      console.error("Source test failed:", error);
      setProbes([]);
    } finally {
      setProbing(false);
    }
  };

  const scanPlaying = async () => {
    setScanning(true);
    try {
      setPlaying(await invoke<Playing[]>("get_playing_audio"));
    } catch (error) {
      console.error("Failed to list what is playing:", error);
      setPlaying([]);
    } finally {
      setScanning(false);
    }
  };

  const clear = () => {
    setSelected([]);
    saveAudioSources([]);
  };

  const renderRow = (
    key: string,
    source: AudioSource,
    icon: React.ReactNode,
    label: string,
    detail?: string,
    trailing?: React.ReactNode
  ) => (
    <label
      key={key}
      className="flex items-center gap-3 rounded-md border border-input/50 px-3 py-2 text-sm cursor-pointer hover:bg-accent/50 transition-colors"
    >
      <input
        type="checkbox"
        className="size-4 accent-primary"
        checked={isSelected(source)}
        onChange={() => toggle(source)}
      />
      {icon}
      <div className="min-w-0 flex-1">
        <div className="font-medium truncate">{label}</div>
        {detail && (
          <div className="text-xs text-muted-foreground truncate">{detail}</div>
        )}
      </div>
      {trailing}
    </label>
  );

  const playingState = (p: Playing) =>
    !p.level_known
      ? p.active
        ? "active stream"
        : "paused"
      : p.peak >= SILENT_PEAK
      ? "making sound"
      : p.active
      ? "open but silent"
      : "idle";

  return (
    <div id="audio-sources" className="space-y-3">
      <Header
        title="Audio Sources"
        description="Choose what Pluely listens to: a specific application or window, an output device, your microphone, or several at once. Selected sources are mixed into one stream. With nothing selected, the output device above is used."
      />

      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="outline"
          onClick={load}
          disabled={isLoading}
          title="Refresh sources"
        >
          <RefreshCwIcon
            className={`size-4 ${isLoading ? "animate-spin" : ""}`}
          />
          Refresh
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={clear}
          disabled={selected.length === 0}
        >
          Clear selection
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => runProbe()}
          disabled={probing}
          title="Listen to the selected sources for 3 seconds and show their level"
        >
          {probing ? "Listening…" : "Test sources (3 s)"}
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            runProbe(
              outputs.map((d): AudioSource => ({ kind: "output", id: d.id }))
            )
          }
          disabled={probing || outputs.length === 0}
          title="Listen to every output device for 3 seconds"
        >
          Test all outputs
        </Button>
        <span className="text-xs text-muted-foreground ml-auto">
          {selected.length === 0
            ? "Using default output"
            : `${selected.length} selected`}
        </span>
      </div>

      {probes && (
        <div className="space-y-2 rounded-md border border-input/50 p-3">
          <div className="text-xs text-muted-foreground">
            Play something now, then press the test. Level is the loudest
            moment in 3 seconds.
          </div>
          {probes.length === 0 && (
            <div className="text-sm text-destructive">Test failed to run.</div>
          )}
          {probes.map((p, i) => {
            const silent = p.started && p.peak < SILENT_PEAK;
            return (
              <div key={i} className="space-y-1">
                <div className="flex items-center justify-between gap-2 text-sm">
                  <span className="truncate font-medium">
                    {labelOf(p.source)}
                  </span>
                  <span
                    className={`shrink-0 text-xs ${
                      !p.started || silent
                        ? "text-destructive"
                        : "text-emerald-600 dark:text-emerald-400"
                    }`}
                  >
                    {!p.started
                      ? "Could not start"
                      : silent
                      ? p.samples === 0
                        ? "No audio data"
                        : "Silent"
                      : `Sound detected · ${levelPct(p.peak)}%`}
                  </span>
                </div>
                <Level peak={p.peak} className="w-full" />
                {p.started && (
                  <div className="text-xs text-muted-foreground tabular-nums">
                    {p.samples.toLocaleString("en-US")} samples · peak{" "}
                    {p.peak.toFixed(3)} · rms {p.rms.toFixed(4)}
                  </div>
                )}
                {p.mode && (
                  <div className="text-xs text-muted-foreground break-words">
                    Opened as: {p.mode}
                  </div>
                )}
                {p.error && (
                  <div className="text-xs text-muted-foreground break-words">
                    {p.error}
                  </div>
                )}
              </div>
            );
          })}
          {probes.some((p) => p.started && p.peak < SILENT_PEAK) && (
            <div className="text-xs text-muted-foreground">
              Silent means the source opened but no sound went through it. Use
              “Find what is playing” to see which process has an audio stream,
              then refresh and select that process. On Linux, app capture uses
              the PulseAudio-compatible interface exposed by PulseAudio or
              PipeWire.
            </div>
          )}
        </div>
      )}

      <div className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <div className="text-xs font-medium text-muted-foreground uppercase">
            Playing now
          </div>
          <Button
            size="sm"
            variant="outline"
            onClick={scanPlaying}
            disabled={scanning}
            title="List the audio streams reported by the sound server"
          >
            {scanning ? "Listening…" : "Find what is playing"}
          </Button>
        </div>
        {playing === null && (
          <div className="text-xs text-muted-foreground">
            Start the sound, then press the button. It shows audio streams and
            their output devices. Select a process to capture its application
            audio.
          </div>
        )}
        {playing !== null && playing.length === 0 && (
          <div className="text-xs text-destructive">
            No process has an audio session on any output, so nothing is
            playing sound right now.
          </div>
        )}
        {playing?.map((p) =>
          renderRow(
            `play-${p.pid}-${p.device}`,
            { kind: "app", pid: p.pid },
            <AudioLinesIcon
              className={`size-4 shrink-0 ${
                p.peak >= SILENT_PEAK
                  ? "text-emerald-600 dark:text-emerald-400"
                  : "text-muted-foreground"
              }`}
            />,
            `${p.name} (pid ${p.pid})`,
            `${p.device} · ${playingState(p)}`,
            p.level_known ? (
              <Level peak={p.peak} className="w-16 shrink-0" />
            ) : undefined
          )
        )}
      </div>

      <div className="space-y-2">
        <div className="text-xs font-medium text-muted-foreground uppercase">
          Applications &amp; windows
        </div>
        {apps.length === 0 && !isLoading && (
          <div className="text-xs text-muted-foreground">
            No application windows found.
          </div>
        )}
        {apps.map((app) =>
          renderRow(
            `app-${app.pid}`,
            { kind: "app", pid: app.pid },
            <AppWindowIcon className="size-4 shrink-0" />,
            app.name,
            app.titles.slice(0, 3).join(" · ")
          )
        )}
      </div>

      <div className="space-y-2">
        <div className="text-xs font-medium text-muted-foreground uppercase">
          Output devices (everything you hear)
        </div>
        {outputs.map((device) =>
          renderRow(
            `out-${device.id}`,
            { kind: "output", id: device.id },
            <HeadphonesIcon className="size-4 shrink-0" />,
            device.name + (device.is_default ? " (Default)" : "")
          )
        )}
      </div>

      <div className="space-y-2">
        <div className="text-xs font-medium text-muted-foreground uppercase">
          Microphones
        </div>
        {mics.map((device) =>
          renderRow(
            `mic-${device.id}`,
            { kind: "microphone", id: device.id },
            <MicIcon className="size-4 shrink-0" />,
            device.name + (device.is_default ? " (Default)" : "")
          )
        )}
      </div>

      <div className="text-xs text-muted-foreground/70 space-y-1">
        <p>
          💡 <strong>Browsers:</strong> Audio capture is per process, not per
          tab. Selecting a browser process captures all audio streams from that
          process. Browser helper processes may appear separately.
        </p>
        <p>
          On Linux, selected application streams are routed through a temporary
          virtual output while their sound is sent back to the original device.
          Changes apply the next time capture starts.
        </p>
      </div>
    </div>
  );
};
