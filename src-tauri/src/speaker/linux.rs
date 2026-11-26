// Pluely linux speaker input and stream
use super::{AudioApp, AudioDevice, AudioSource, PlayingAudio, SourceProbe};
use anyhow::{anyhow, Result};
use futures_util::Stream;
use libpulse_binding as pulse;
use libpulse_simple_binding as psimple;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};
use tracing::error;
use tracing::warn;

use psimple::Simple;
use pulse::context::Context;
use pulse::mainloop::standard::Mainloop;
use pulse::sample::{Format, Spec};
use pulse::stream::Direction;

const DEFAULT_SAMPLE_RATE: u32 = 44_100;

#[derive(Clone, Debug)]
struct SinkRecord {
    index: u32,
    name: String,
    monitor_source: String,
}

#[derive(Clone, Debug)]
struct SinkInputRecord {
    index: u32,
    sink: u32,
    pid: Option<u32>,
    name: String,
    corked: bool,
}

struct PulseConnection {
    context: Context,
    mainloop: Mainloop,
}

impl PulseConnection {
    fn connect(name: &str) -> Result<Self> {
        let mut mainloop =
            Mainloop::new().ok_or_else(|| anyhow!("Failed to create PulseAudio mainloop"))?;
        let mut context = Context::new(&mainloop, name)
            .ok_or_else(|| anyhow!("Failed to create PulseAudio context"))?;
        context
            .connect(None, pulse::context::FlagSet::NOFLAGS, None)
            .map_err(|_| anyhow!("Failed to connect to PulseAudio or PipeWire-Pulse"))?;

        let connect_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match mainloop.iterate(false) {
                pulse::mainloop::standard::IterateResult::Success(_) => {}
                _ => return Err(anyhow!("Failed to iterate the PulseAudio mainloop")),
            }
            match context.get_state() {
                pulse::context::State::Ready => break,
                pulse::context::State::Failed | pulse::context::State::Terminated => {
                    return Err(anyhow!("PulseAudio context failed"));
                }
                _ => {}
            }
            if Instant::now() >= connect_deadline {
                return Err(anyhow!(
                    "Timed out connecting to PulseAudio or PipeWire-Pulse"
                ));
            }
            thread::sleep(Duration::from_millis(2));
        }

        Ok(Self { context, mainloop })
    }

    fn wait_for(&mut self, mut done: impl FnMut() -> bool, action: &str) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            if !matches!(
                self.mainloop.iterate(false),
                pulse::mainloop::standard::IterateResult::Success(_)
            ) {
                return Err(anyhow!("PulseAudio mainloop failed while {}", action));
            }
            match self.context.get_state() {
                pulse::context::State::Failed | pulse::context::State::Terminated => {
                    return Err(anyhow!("PulseAudio context stopped while {}", action));
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err(anyhow!("Timed out while {}", action));
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    fn server_defaults(&mut self) -> Result<(Option<String>, Option<String>)> {
        let result = Rc::new(RefCell::new(None));
        let done = Rc::new(Cell::new(false));
        let result_cb = result.clone();
        let done_cb = done.clone();
        let operation = self.context.introspect().get_server_info(move |info| {
            *result_cb.borrow_mut() = Some((
                info.default_sink_name.as_ref().map(|name| name.to_string()),
                info.default_source_name
                    .as_ref()
                    .map(|name| name.to_string()),
            ));
            done_cb.set(true);
        });
        self.wait_for(|| done.get(), "reading server defaults")?;
        drop(operation);
        let defaults = result
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow!("PulseAudio returned no server information"));
        defaults
    }

    fn sinks(&mut self) -> Result<Vec<SinkRecord>> {
        let result = Rc::new(RefCell::new(Vec::new()));
        let done = Rc::new(Cell::new(false));
        let failed = Rc::new(Cell::new(false));
        let result_cb = result.clone();
        let done_cb = done.clone();
        let failed_cb = failed.clone();
        let operation = self
            .context
            .introspect()
            .get_sink_info_list(move |item| match item {
                pulse::callbacks::ListResult::Item(info) => {
                    if let (Some(name), Some(monitor)) = (&info.name, &info.monitor_source_name) {
                        result_cb.borrow_mut().push(SinkRecord {
                            index: info.index,
                            name: name.to_string(),
                            monitor_source: monitor.to_string(),
                        });
                    }
                }
                pulse::callbacks::ListResult::End => done_cb.set(true),
                pulse::callbacks::ListResult::Error => {
                    failed_cb.set(true);
                    done_cb.set(true);
                }
            });
        self.wait_for(|| done.get(), "listing output devices")?;
        drop(operation);
        if failed.get() {
            return Err(anyhow!("PulseAudio could not list output devices"));
        }
        let sinks = std::mem::take(&mut *result.borrow_mut());
        Ok(sinks)
    }

    fn sink_inputs(&mut self) -> Result<Vec<SinkInputRecord>> {
        let result = Rc::new(RefCell::new(Vec::new()));
        let done = Rc::new(Cell::new(false));
        let failed = Rc::new(Cell::new(false));
        let result_cb = result.clone();
        let done_cb = done.clone();
        let failed_cb = failed.clone();
        let operation =
            self.context
                .introspect()
                .get_sink_input_info_list(move |item| match item {
                    pulse::callbacks::ListResult::Item(info) => {
                        let pid = info
                            .proplist
                            .get_str("application.process.id")
                            .and_then(|value| value.parse::<u32>().ok());
                        let name = info
                            .proplist
                            .get_str("application.name")
                            .or_else(|| info.proplist.get_str("media.name"))
                            .or_else(|| info.proplist.get_str("application.process.binary"))
                            .or_else(|| info.name.as_ref().map(|name| name.to_string()))
                            .unwrap_or_else(|| "Audio application".to_string());
                        result_cb.borrow_mut().push(SinkInputRecord {
                            index: info.index,
                            sink: info.sink,
                            pid,
                            name,
                            corked: info.corked,
                        });
                    }
                    pulse::callbacks::ListResult::End => done_cb.set(true),
                    pulse::callbacks::ListResult::Error => {
                        failed_cb.set(true);
                        done_cb.set(true);
                    }
                });
        self.wait_for(|| done.get(), "listing application audio streams")?;
        drop(operation);
        if failed.get() {
            return Err(anyhow!(
                "PulseAudio could not list application audio streams"
            ));
        }
        let inputs = std::mem::take(&mut *result.borrow_mut());
        Ok(inputs)
    }

    fn load_module(&mut self, name: &str, arguments: &str) -> Result<u32> {
        let index = Rc::new(Cell::new(None));
        let index_cb = index.clone();
        let operation = self
            .context
            .introspect()
            .load_module(name, arguments, move |loaded| index_cb.set(Some(loaded)));
        self.wait_for(|| index.get().is_some(), &format!("loading {}", name))?;
        let module_index = index.get().unwrap_or(pulse::def::INVALID_INDEX);
        drop(operation);
        if module_index == pulse::def::INVALID_INDEX {
            return Err(anyhow!("PulseAudio could not load module {}", name));
        }
        Ok(module_index)
    }

    fn move_sink_input(&mut self, index: u32, sink_name: &str) -> Result<()> {
        let result = Rc::new(Cell::new(None));
        let result_cb = result.clone();
        let operation = self.context.introspect().move_sink_input_by_name(
            index,
            sink_name,
            Some(Box::new(move |success| result_cb.set(Some(success)))),
        );
        self.wait_for(
            || result.get().is_some(),
            &format!("moving audio stream {}", index),
        )?;
        drop(operation);
        if result.get() == Some(true) {
            Ok(())
        } else {
            Err(anyhow!("Could not move application audio stream {}", index))
        }
    }

    fn unload_module(&mut self, index: u32) -> Result<()> {
        let result = Rc::new(Cell::new(None));
        let result_cb = result.clone();
        let operation = self
            .context
            .introspect()
            .unload_module(index, move |success| result_cb.set(Some(success)));
        self.wait_for(
            || result.get().is_some(),
            &format!("unloading module {}", index),
        )?;
        drop(operation);
        if result.get() == Some(true) {
            Ok(())
        } else {
            Err(anyhow!("Could not unload PulseAudio module {}", index))
        }
    }
}

struct AppAudioSession {
    pulse: PulseConnection,
    module_indices: Vec<u32>,
    moved_inputs: Vec<(u32, String)>,
    monitor_source: String,
}

impl AppAudioSession {
    fn new(sources: &[AudioSource]) -> Result<Self> {
        static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut pulse = PulseConnection::connect("pluely-app-audio-capture")?;
        let (default_sink, default_source) = pulse.server_defaults()?;
        let sinks = pulse.sinks()?;
        let inputs = pulse.sink_inputs()?;
        let sink_names: BTreeMap<u32, String> = sinks
            .iter()
            .map(|sink| (sink.index, sink.name.clone()))
            .collect();
        let mut selected_outputs = HashSet::new();
        let mut selected_sources = HashSet::new();
        let mut selected_pids = HashSet::new();

        for source in sources {
            match source {
                AudioSource::Output { id } => {
                    let sink = match id
                        .as_deref()
                        .filter(|id| !id.is_empty() && *id != "default")
                    {
                        Some(id) => sinks.iter().find(|sink| sink.name == id),
                        None => default_sink
                            .as_deref()
                            .and_then(|name| sinks.iter().find(|sink| sink.name == name)),
                    }
                    .ok_or_else(|| anyhow!("Selected output device is no longer available"))?;
                    selected_outputs.insert(sink.name.clone());
                    selected_sources.insert(sink.monitor_source.clone());
                }
                AudioSource::Microphone { id } => {
                    let source_name = id
                        .as_deref()
                        .filter(|id| !id.is_empty() && *id != "default")
                        .map(str::to_string)
                        .or_else(|| default_source.clone())
                        .ok_or_else(|| anyhow!("No default microphone is available"))?;
                    selected_sources.insert(source_name);
                }
                AudioSource::App { pid } => {
                    selected_pids.insert(*pid);
                }
            }
        }

        if selected_sources.is_empty() && selected_pids.is_empty() {
            return Err(anyhow!("No Linux audio sources were selected"));
        }

        let app_inputs: Vec<(SinkInputRecord, String)> = inputs
            .into_iter()
            .filter(|input| {
                input
                    .pid
                    .map(|pid| selected_pids.contains(&pid))
                    .unwrap_or(false)
            })
            .filter_map(|input| {
                sink_names
                    .get(&input.sink)
                    .cloned()
                    .map(|sink| (input, sink))
            })
            .collect();
        let matched_pids: HashSet<u32> = app_inputs
            .iter()
            .filter_map(|(input, _)| input.pid)
            .collect();
        let missing_pids: Vec<String> = selected_pids
            .difference(&matched_pids)
            .map(u32::to_string)
            .collect();
        if !missing_pids.is_empty() {
            return Err(anyhow!(
                "No active audio stream found for application process(es): {}. Start playback and refresh the source list.",
                missing_pids.join(", ")
            ));
        }

        let session_id = NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sink_name = format!("pluely_capture_{}_{}", std::process::id(), session_id);
        let mut session = Self {
            pulse,
            module_indices: Vec::new(),
            moved_inputs: Vec::new(),
            monitor_source: format!("{}.monitor", sink_name),
        };

        let module_index = session.pulse.load_module(
            "module-null-sink",
            &format!(
                "sink_name={} rate={} channels=2 channel_map=front-left,front-right sink_properties=device.description=PluelyCapture",
                sink_name, DEFAULT_SAMPLE_RATE
            ),
        )?;
        session.module_indices.push(module_index);

        for source_name in selected_sources {
            let index = session.pulse.load_module(
                "module-loopback",
                &format!("source={} sink={} latency_msec=20", source_name, sink_name),
            )?;
            session.module_indices.push(index);
        }

        let mut app_return_sinks = HashSet::new();
        for (input, original_sink) in app_inputs {
            if selected_outputs.contains(&original_sink) {
                // The selected output's monitor already contains this stream.
                continue;
            }
            session.pulse.move_sink_input(input.index, &sink_name)?;
            session
                .moved_inputs
                .push((input.index, original_sink.clone()));
            app_return_sinks.insert(original_sink);
        }

        for original_sink in app_return_sinks {
            let index = session.pulse.load_module(
                "module-loopback",
                &format!(
                    "source={} sink={} latency_msec=20",
                    session.monitor_source, original_sink
                ),
            )?;
            session.module_indices.push(index);
        }

        Ok(session)
    }
}

impl Drop for AppAudioSession {
    fn drop(&mut self) {
        for (input, sink_name) in self.moved_inputs.iter().rev() {
            if let Err(error) = self.pulse.move_sink_input(*input, sink_name) {
                warn!(
                    "Could not restore application audio stream {}: {}",
                    input, error
                );
            }
        }
        for module_index in self.module_indices.iter().rev() {
            if let Err(error) = self.pulse.unload_module(*module_index) {
                warn!(
                    "Could not unload temporary audio module {}: {}",
                    module_index, error
                );
            }
        }
    }
}

impl Drop for PulseConnection {
    fn drop(&mut self) {
        self.context.disconnect();
        self.mainloop.quit(pulse::def::Retval(0));
    }
}

pub fn get_input_devices() -> Result<Vec<AudioDevice>> {
    let devices = Rc::new(RefCell::new(Vec::new()));
    let done = Rc::new(RefCell::new(false));

    let mut mainloop =
        Mainloop::new().ok_or_else(|| anyhow!("Failed to create PulseAudio mainloop"))?;
    let mut context = Context::new(&mainloop, "pluely-device-enum")
        .ok_or_else(|| anyhow!("Failed to create PulseAudio context"))?;

    context
        .connect(None, pulse::context::FlagSet::NOFLAGS, None)
        .map_err(|_| anyhow!("Failed to connect to PulseAudio"))?;

    loop {
        match mainloop.iterate(true) {
            pulse::mainloop::standard::IterateResult::Success(_) => {}
            _ => return Err(anyhow!("Failed to iterate mainloop")),
        }

        match context.get_state() {
            pulse::context::State::Ready => break,
            pulse::context::State::Failed | pulse::context::State::Terminated => {
                return Err(anyhow!("PulseAudio context failed"));
            }
            _ => {}
        }
    }

    let default_source: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let default_source_clone = default_source.clone();
    let done_clone = done.clone();

    let introspector = context.introspect();

    let op = introspector.get_server_info(move |info| {
        if let Some(name) = &info.default_source_name {
            *default_source_clone.borrow_mut() = Some(name.to_string());
        }
        *done_clone.borrow_mut() = true;
    });

    while !*done.borrow() {
        match mainloop.iterate(true) {
            pulse::mainloop::standard::IterateResult::Success(_) => {}
            _ => break,
        }
    }
    drop(op);

    *done.borrow_mut() = false;
    let devices_clone = devices.clone();
    let done_clone = done.clone();
    let default_source_name = default_source.borrow().clone();

    let op = introspector.get_source_info_list(move |result| match result {
        pulse::callbacks::ListResult::Item(info) => {
            if let Some(name) = &info.name {
                if !name.contains(".monitor") {
                    let device_name = info
                        .description
                        .as_ref()
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| name.to_string());
                    let device_id = name.to_string();
                    let is_default = default_source_name
                        .as_ref()
                        .map(|def| def == &device_id)
                        .unwrap_or(false);

                    devices_clone.borrow_mut().push(AudioDevice {
                        id: device_id,
                        name: device_name,
                        is_default,
                    });
                }
            }
        }
        pulse::callbacks::ListResult::End => {
            *done_clone.borrow_mut() = true;
        }
        pulse::callbacks::ListResult::Error => {
            *done_clone.borrow_mut() = true;
        }
    });

    while !*done.borrow() {
        match mainloop.iterate(true) {
            pulse::mainloop::standard::IterateResult::Success(_) => {}
            _ => break,
        }
    }
    drop(op);

    context.disconnect();
    mainloop.quit(pulse::def::Retval(0));

    Ok(Rc::try_unwrap(devices).unwrap().into_inner())
}

pub fn get_output_devices() -> Result<Vec<AudioDevice>> {
    let devices = Rc::new(RefCell::new(Vec::new()));
    let done = Rc::new(RefCell::new(false));

    let mut mainloop =
        Mainloop::new().ok_or_else(|| anyhow!("Failed to create PulseAudio mainloop"))?;
    let mut context = Context::new(&mainloop, "pluely-device-enum")
        .ok_or_else(|| anyhow!("Failed to create PulseAudio context"))?;

    context
        .connect(None, pulse::context::FlagSet::NOFLAGS, None)
        .map_err(|_| anyhow!("Failed to connect to PulseAudio"))?;

    loop {
        match mainloop.iterate(true) {
            pulse::mainloop::standard::IterateResult::Success(_) => {}
            _ => return Err(anyhow!("Failed to iterate mainloop")),
        }

        match context.get_state() {
            pulse::context::State::Ready => break,
            pulse::context::State::Failed | pulse::context::State::Terminated => {
                return Err(anyhow!("PulseAudio context failed"));
            }
            _ => {}
        }
    }

    // Get default sink for comparison
    let default_sink: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let default_sink_clone = default_sink.clone();
    let done_clone = done.clone();

    let introspector = context.introspect();

    let op = introspector.get_server_info(move |info| {
        if let Some(name) = &info.default_sink_name {
            *default_sink_clone.borrow_mut() = Some(name.to_string());
        }
        *done_clone.borrow_mut() = true;
    });

    while !*done.borrow() {
        match mainloop.iterate(true) {
            pulse::mainloop::standard::IterateResult::Success(_) => {}
            _ => break,
        }
    }
    drop(op);

    *done.borrow_mut() = false;
    let devices_clone = devices.clone();
    let done_clone = done.clone();
    let default_sink_name = default_sink.borrow().clone();

    let op = introspector.get_sink_info_list(move |result| match result {
        pulse::callbacks::ListResult::Item(info) => {
            if let Some(name) = &info.name {
                let device_name = info
                    .description
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| name.to_string());
                let device_id = name.to_string();
                let is_default = default_sink_name
                    .as_ref()
                    .map(|def| def == &device_id)
                    .unwrap_or(false);

                devices_clone.borrow_mut().push(AudioDevice {
                    id: device_id,
                    name: device_name,
                    is_default,
                });
            }
        }
        pulse::callbacks::ListResult::End => {
            *done_clone.borrow_mut() = true;
        }
        pulse::callbacks::ListResult::Error => {
            *done_clone.borrow_mut() = true;
        }
    });

    while !*done.borrow() {
        match mainloop.iterate(true) {
            pulse::mainloop::standard::IterateResult::Success(_) => {}
            _ => break,
        }
    }
    drop(op);

    context.disconnect();
    mainloop.quit(pulse::def::Retval(0));

    Ok(Rc::try_unwrap(devices).unwrap().into_inner())
}

pub fn list_audio_apps() -> Result<Vec<AudioApp>> {
    let mut pulse = PulseConnection::connect("pluely-audio-app-list")?;
    let own_pid = std::process::id();
    let mut apps = BTreeMap::<u32, String>::new();

    for input in pulse.sink_inputs()? {
        if let Some(pid) = input.pid.filter(|pid| *pid != own_pid) {
            apps.entry(pid).or_insert(input.name);
        }
    }

    Ok(apps
        .into_iter()
        .map(|(pid, name)| AudioApp {
            pid,
            name,
            titles: Vec::new(),
        })
        .collect())
}

pub fn list_playing_audio() -> Result<Vec<PlayingAudio>> {
    let mut pulse = PulseConnection::connect("pluely-audio-session-list")?;
    let own_pid = std::process::id();
    let sinks: BTreeMap<u32, String> = pulse
        .sinks()?
        .into_iter()
        .map(|sink| (sink.index, sink.name))
        .collect();
    let mut sessions = BTreeMap::<(u32, String), PlayingAudio>::new();

    for input in pulse.sink_inputs()? {
        let Some(pid) = input.pid.filter(|pid| *pid != own_pid) else {
            continue;
        };
        let Some(device) = sinks.get(&input.sink) else {
            continue;
        };
        let session = sessions
            .entry((pid, device.clone()))
            .or_insert_with(|| PlayingAudio {
                pid,
                name: input.name.clone(),
                device: device.clone(),
                active: false,
                peak: 0.0,
                level_known: false,
            });
        // PulseAudio exposes whether an output stream is corked, but not its
        // instantaneous level. The source probe reports the actual level.
        session.active |= !input.corked;
    }

    let mut sessions: Vec<_> = sessions.into_values().collect();
    sessions.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(sessions)
}

struct ProbeWake;

impl Wake for ProbeWake {
    fn wake(self: Arc<Self>) {}
}

pub fn probe_sources(sources: Vec<AudioSource>, seconds: u64) -> Vec<SourceProbe> {
    sources
        .into_iter()
        .map(|source| {
            let input = match SpeakerInput::with_sources(vec![source.clone()]) {
                Ok(input) => input,
                Err(error) => {
                    return SourceProbe {
                        source,
                        started: false,
                        mode: None,
                        error: Some(error.to_string()),
                        samples: 0,
                        peak: 0.0,
                        rms: 0.0,
                    };
                }
            };
            let mut stream = input.stream();
            if let Some(error) = stream.init_error() {
                return SourceProbe {
                    source,
                    started: false,
                    mode: None,
                    error: Some(error),
                    samples: 0,
                    peak: 0.0,
                    rms: 0.0,
                };
            }

            let waker = Waker::from(Arc::new(ProbeWake));
            let mut task_context = TaskContext::from_waker(&waker);
            let deadline = Instant::now() + Duration::from_secs(seconds.clamp(1, 10));
            let (mut count, mut peak, mut sum_squares) = (0usize, 0.0f32, 0.0f64);
            while Instant::now() < deadline {
                match Pin::new(&mut stream).poll_next(&mut task_context) {
                    Poll::Ready(Some(sample)) => {
                        let sample = sample.clamp(-1.0, 1.0);
                        peak = peak.max(sample.abs());
                        sum_squares += f64::from(sample * sample);
                        count += 1;
                    }
                    Poll::Ready(None) | Poll::Pending => {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            }
            drop(stream);
            let mode = match &source {
                AudioSource::App { .. } => {
                    "PulseAudio app stream via temporary virtual sink".to_string()
                }
                _ => "PulseAudio source via temporary virtual sink".to_string(),
            };

            SourceProbe {
                source,
                started: true,
                mode: Some(mode),
                error: None,
                samples: count,
                peak,
                rms: if count == 0 {
                    0.0
                } else {
                    (sum_squares / count as f64).sqrt() as f32
                },
            }
        })
        .collect()
}

pub struct SpeakerInput {
    source_name: Option<String>,
    sources: Option<Vec<AudioSource>>,
}

impl SpeakerInput {
    pub fn new(device_id: Option<String>) -> Result<Self> {
        // For Linux, device_id is the PulseAudio sink name for output devices
        let source_name = match device_id {
            Some(ref id) if !id.is_empty() && id != "default" => {
                let monitor = format!("{}.monitor", id);
                Some(monitor)
            }
            _ => None,
        };
        Ok(Self {
            source_name,
            sources: None,
        })
    }

    pub fn with_sources(sources: Vec<AudioSource>) -> Result<Self> {
        if sources.is_empty() {
            return Self::new(None);
        }
        Ok(Self {
            source_name: None,
            sources: Some(sources),
        })
    }

    pub fn stream(self) -> SpeakerStream {
        let sample_queue = Arc::new(Mutex::new(VecDeque::new()));
        let waker_state = Arc::new(Mutex::new(WakerState {
            waker: None,
            has_data: false,
            shutdown: false,
        }));
        let (init_tx, init_rx) = std::sync::mpsc::channel();

        let queue_clone = sample_queue.clone();
        let waker_clone = waker_state.clone();
        let source_name = self.source_name;
        let sources = self.sources;

        let mut capture_thread = Some(thread::spawn(move || {
            if let Err(e) = SpeakerStream::capture_audio_loop(
                queue_clone,
                waker_clone,
                source_name.as_deref(),
                sources.as_deref(),
                init_tx,
            ) {
                eprintln!("Audio capture loop failed: {}", e);
            }
        }));

        let (sample_rate, init_error) = match init_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(sr)) => (sr, None),
            Ok(Err(e)) => {
                eprintln!("Audio initialization failed: {}", e);
                (DEFAULT_SAMPLE_RATE, Some(e.to_string()))
            }
            Err(e) => {
                eprintln!("Failed to receive audio init signal: {}", e);
                (
                    DEFAULT_SAMPLE_RATE,
                    Some(format!("Audio initialization timed out: {}", e)),
                )
            }
        };

        if init_error.is_some() {
            {
                let mut state = waker_state.lock().unwrap();
                state.shutdown = true;
                if let Some(waker) = state.waker.take() {
                    drop(state);
                    waker.wake();
                }
            }

            if let Some(handle) = capture_thread.take() {
                let _ = handle.join();
            }
        }

        SpeakerStream {
            sample_queue,
            waker_state,
            capture_thread,
            sample_rate,
            init_error,
        }
    }
}

struct WakerState {
    waker: Option<Waker>,
    has_data: bool,
    shutdown: bool,
}

pub struct SpeakerStream {
    sample_queue: Arc<Mutex<VecDeque<f32>>>,
    waker_state: Arc<Mutex<WakerState>>,
    capture_thread: Option<thread::JoinHandle<()>>,
    sample_rate: u32,
    init_error: Option<String>,
}

impl SpeakerStream {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn init_error(&self) -> Option<String> {
        self.init_error.clone()
    }

    fn capture_audio_loop(
        sample_queue: Arc<Mutex<VecDeque<f32>>>,
        waker_state: Arc<Mutex<WakerState>>,
        source_name: Option<&str>,
        sources: Option<&[AudioSource]>,
        init_tx: std::sync::mpsc::Sender<Result<u32>>,
    ) -> Result<()> {
        let spec = Spec {
            format: Format::F32le,
            channels: 1,
            rate: 44100, // Fixed: Use 44100 Hz to match macOS/Windows
        };

        if !spec.is_valid() {
            error!("[capture_audio_loop] Invalid audio specification");
            return Err(anyhow!("Invalid audio specification"));
        }

        let mut app_audio_session = match sources {
            Some(sources) => match AppAudioSession::new(sources) {
                Ok(session) => Some(session),
                Err(error) => {
                    let _ = init_tx.send(Err(anyhow!(
                        "Failed to prepare selected audio sources: {}",
                        error
                    )));
                    return Ok(());
                }
            },
            None => None,
        };

        let final_source = app_audio_session
            .as_ref()
            .map(|session| session.monitor_source.clone())
            .or_else(|| source_name.map(str::to_string))
            .or_else(get_default_monitor_source);

        let init_result: Result<(Simple, u32)> = (|| {
            let simple = Simple::new(
                None,                    // Use default server
                "pluely",                // Application name
                Direction::Record,       // Record direction
                final_source.as_deref(), // Source name (monitor)
                "System Audio Capture",  // Stream description
                &spec,                   // Sample specification
                None,                    // Channel map (use default)
                None,                    // Buffer attributes (use default)
            )
            .map_err(|e| {
                error!(
                    "[capture_audio_loop] Failed to create PulseAudio connection: {}",
                    e
                );
                anyhow!("Failed to create PulseAudio simple connection: {}", e)
            })?;

            Ok((simple, spec.rate))
        })();

        match init_result {
            Ok((simple, sample_rate)) => {
                let _ = init_tx.send(Ok(sample_rate));

                // Buffer for reading audio data (1024 samples * 4 bytes/sample)
                let mut buffer = vec![0u8; 4096];

                loop {
                    if waker_state.lock().unwrap().shutdown {
                        break;
                    }

                    match simple.read(&mut buffer) {
                        Ok(_) => {
                            // Convert byte buffer to f32 samples
                            let samples: Vec<f32> = buffer
                                .chunks_exact(4)
                                .map(|chunk| {
                                    f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                                })
                                .collect();

                            if !samples.is_empty() {
                                // Consistent buffer overflow handling
                                let dropped = {
                                    let mut queue = sample_queue.lock().unwrap();
                                    let max_buffer_size = 131072; // 128KB buffer (matching macOS/Windows)

                                    queue.extend(samples.iter());

                                    // If buffer exceeds maximum, drop oldest samples
                                    if queue.len() > max_buffer_size {
                                        let to_drop = queue.len() - max_buffer_size;
                                        queue.drain(0..to_drop);
                                        to_drop
                                    } else {
                                        0
                                    }
                                };

                                if dropped > 0 {
                                    warn!("[capture_audio_loop] Linux buffer overflow - dropped {} samples", dropped);
                                }

                                // Wake up consumer
                                {
                                    let mut state = waker_state.lock().unwrap();
                                    if !state.has_data {
                                        state.has_data = true;
                                        if let Some(waker) = state.waker.take() {
                                            drop(state);
                                            waker.wake();
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("[capture_audio_loop] PulseAudio read error: {}", e);
                            thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                }
            }
            Err(e) => {
                error!("[capture_audio_loop] PulseAudio init failed: {}", e);
                let _ = init_tx.send(Err(e));
            }
        }
        drop(app_audio_session.take());
        Ok(())
    }
}

fn get_default_monitor_source() -> Option<String> {
    Some("@DEFAULT_MONITOR@".to_string())
}

impl Drop for SpeakerStream {
    fn drop(&mut self) {
        {
            let mut state = self.waker_state.lock().unwrap();
            state.shutdown = true;
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
        if let Some(thread) = self.capture_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Stream for SpeakerStream {
    type Item = f32;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut queue = self.sample_queue.lock().unwrap();
        if let Some(sample) = queue.pop_front() {
            return Poll::Ready(Some(sample));
        }

        let mut state = self.waker_state.lock().unwrap();
        if state.shutdown {
            return Poll::Ready(None);
        }

        state.has_data = false;
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}
