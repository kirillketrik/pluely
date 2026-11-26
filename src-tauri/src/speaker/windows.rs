// Pluely windows speaker input and stream
use super::{AudioApp, AudioDevice, AudioSource, PlayingAudio, SourceProbe};
use anyhow::{Context, Result};
use futures_util::Stream;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::task::{Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use wasapi::{
    get_default_device, AudioClient, DeviceCollection, Direction, SampleType, StreamMode,
    WaveFormat,
};

/// Every source is captured as 48 kHz stereo f32 (WASAPI converts) and
/// downmixed to mono, so several sources can be mixed sample by sample.
const MIX_SAMPLE_RATE: u32 = 48000;
const MIX_SOURCE_BUFFER_MAX: usize = MIX_SAMPLE_RATE as usize * 5;
const MIX_OUTPUT_BUFFER_MAX: usize = 131072;
/// How long a source gets to start before it is reported as failed.
const SOURCE_START_TIMEOUT: Duration = Duration::from_secs(8);
/// Buffer of polled streams (100 ms), the value Windows loopback recorders use.
const POLLING_BUFFER_HNS: i64 = 1_000_000;

pub fn get_input_devices() -> Result<Vec<AudioDevice>> {
    let mut devices = Vec::new();

    let default_device = get_default_device(&Direction::Capture).ok();
    let default_id = default_device.as_ref().and_then(|d| d.get_id().ok());

    let collection = DeviceCollection::new(&Direction::Capture)?;
    let count = collection.get_nbr_devices()?;

    for i in 0..count {
        if let Ok(device) = collection.get_device_at_index(i) {
            let name = device
                .get_friendlyname()
                .unwrap_or_else(|_| format!("Microphone {}", i));
            let id = device
                .get_id()
                .unwrap_or_else(|_| format!("windows_input_{}", i));
            let is_default = default_id.as_ref().map(|def| def == &id).unwrap_or(false);

            devices.push(AudioDevice {
                id,
                name,
                is_default,
            });
        }
    }

    Ok(devices)
}

pub fn get_output_devices() -> Result<Vec<AudioDevice>> {
    let mut devices = Vec::new();

    let default_device = get_default_device(&Direction::Render).ok();
    let default_id = default_device.as_ref().and_then(|d| d.get_id().ok());

    let collection = DeviceCollection::new(&Direction::Render)?;
    let count = collection.get_nbr_devices()?;

    for i in 0..count {
        if let Ok(device) = collection.get_device_at_index(i) {
            let name = device
                .get_friendlyname()
                .unwrap_or_else(|_| format!("Speaker {}", i));
            let id = device
                .get_id()
                .unwrap_or_else(|_| format!("windows_output_{}", i));
            let is_default = default_id.as_ref().map(|def| def == &id).unwrap_or(false);

            devices.push(AudioDevice {
                id,
                name,
                is_default,
            });
        }
    }

    Ok(devices)
}

fn find_device_by_id(direction: &Direction, device_id: &str) -> Option<wasapi::Device> {
    let collection = match DeviceCollection::new(direction) {
        Ok(c) => c,
        Err(e) => {
            error!(
                "[find_device_by_id] Failed to create device collection: {}",
                e
            );
            return None;
        }
    };

    let count = match collection.get_nbr_devices() {
        Ok(c) => c,
        Err(e) => {
            error!("[find_device_by_id] Failed to get device count: {}", e);
            return None;
        }
    };

    for i in 0..count {
        if let Ok(device) = collection.get_device_at_index(i) {
            if let Ok(id) = device.get_id() {
                if id == device_id {
                    return Some(device);
                }
            }
        }
    }

    error!(
        "[find_device_by_id] No matching device found for ID: {}",
        device_id
    );
    None
}

pub struct SpeakerInput {
    sources: Vec<AudioSource>,
}

impl SpeakerInput {
    /// Loopback of one output device (the default one when `device_id` is empty).
    pub fn new(device_id: Option<String>) -> Result<Self> {
        let id = device_id.filter(|id| !id.is_empty() && id != "default");
        Ok(Self {
            sources: vec![AudioSource::Output { id }],
        })
    }

    /// Capture and mix several sources (devices, microphones, applications).
    pub fn with_sources(sources: Vec<AudioSource>) -> Result<Self> {
        if sources.is_empty() {
            return Self::new(None);
        }
        Ok(Self { sources })
    }

    // Starts the audio stream
    pub fn stream(self) -> SpeakerStream {
        let sample_queue = Arc::new(Mutex::new(VecDeque::new()));
        let waker_state = Arc::new(Mutex::new(WakerState {
            waker: None,
            has_data: false,
            shutdown: false,
        }));
        let (init_tx, init_rx) = mpsc::channel();

        let queue_clone = sample_queue.clone();
        let waker_clone = waker_state.clone();
        let sources = self.sources;

        let capture_thread =
            thread::spawn(move || mixer_loop(sources, queue_clone, waker_clone, init_tx));

        let (actual_sample_rate, init_error) =
            match init_rx.recv_timeout(SOURCE_START_TIMEOUT + Duration::from_secs(4)) {
                Ok(Ok(rate)) => (rate, None),
                Ok(Err(e)) => {
                    error!("Pluely Audio initialization failed: {}", e);
                    (MIX_SAMPLE_RATE, Some(e.to_string()))
                }
                Err(_) => {
                    error!("Pluely Audio initialization timeout");
                    (
                        MIX_SAMPLE_RATE,
                        Some("Audio initialization timed out".to_string()),
                    )
                }
            };

        SpeakerStream {
            sample_queue,
            waker_state,
            capture_thread: Some(capture_thread),
            actual_sample_rate,
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
    actual_sample_rate: u32,
    init_error: Option<String>,
}

impl SpeakerStream {
    pub fn sample_rate(&self) -> u32 {
        self.actual_sample_rate
    }

    /// Why no source could be started, if that is what happened.
    pub fn init_error(&self) -> Option<String> {
        self.init_error.clone()
    }
}

// Drops the audio stream
impl Drop for SpeakerStream {
    fn drop(&mut self) {
        {
            let mut state = self.waker_state.lock().unwrap();
            state.shutdown = true;
        }

        if let Some(thread) = self.capture_thread.take() {
            if let Err(e) = thread.join() {
                error!("Failed to join capture thread: {:?}", e);
            }
        }
    }
}

// Stream of f32 audio samples from the speaker
impl Stream for SpeakerStream {
    type Item = f32;

    // Polls the audio stream
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        {
            let state = self.waker_state.lock().unwrap();
            if state.shutdown {
                return Poll::Ready(None);
            }
        }

        {
            let mut queue = self.sample_queue.lock().unwrap();
            if let Some(sample) = queue.pop_front() {
                return Poll::Ready(Some(sample));
            }
        }

        {
            let mut state = self.waker_state.lock().unwrap();
            if state.shutdown {
                return Poll::Ready(None);
            }
            state.has_data = false;
            state.waker = Some(cx.waker().clone());
            drop(state);
        }

        {
            let mut queue = self.sample_queue.lock().unwrap();
            match queue.pop_front() {
                Some(sample) => Poll::Ready(Some(sample)),
                None => Poll::Pending,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Multi-source capture (devices, microphones, applications) with mixing
// ---------------------------------------------------------------------------

type SampleQueue = Arc<Mutex<VecDeque<f32>>>;

fn pick_device(direction: &Direction, id: Option<&str>) -> Result<wasapi::Device> {
    match id.filter(|id| !id.is_empty() && *id != "default") {
        Some(id) => find_device_by_id(direction, id)
            .ok_or_else(|| anyhow::anyhow!("Audio device not found: {}", id)),
        None => Ok(get_default_device(direction)?),
    }
}

fn describe(source: &AudioSource) -> String {
    match source {
        AudioSource::Output { id: None } => "Output (default)".to_string(),
        AudioSource::Output { id: Some(_) } => "Output device".to_string(),
        AudioSource::Microphone { id: None } => "Microphone (default)".to_string(),
        AudioSource::Microphone { id: Some(_) } => "Microphone".to_string(),
        AudioSource::App { pid } => format!("Application (pid {})", pid),
    }
}

/// How the capture buffer is driven. Some drivers (HDMI/NVIDIA among them)
/// refuse event-driven loopback; polling works almost everywhere.
#[derive(Clone, Copy, Debug)]
enum Timing {
    /// Event driven, default buffer (what per-application capture needs).
    Events,
    /// Event driven, buffer of one device period.
    EventsPeriod,
    /// Polled every few milliseconds.
    Polling,
}

struct OpenedSource {
    client: AudioClient,
    /// `None` when the stream is polled.
    event: Option<wasapi::Handle>,
    capture: wasapi::AudioCaptureClient,
    channels: usize,
    rate: u32,
    /// How the source was opened, for the source test.
    description: String,
}

/// Initialises `client` for capture (loopback when it belongs to an output
/// device or a process) and starts it.
fn start_client(
    mut client: AudioClient,
    format: &WaveFormat,
    mode: &StreamMode,
    channels: usize,
    rate: u32,
    description: String,
) -> Result<OpenedSource> {
    client
        .initialize_client(format, &Direction::Capture, mode)
        .context("initialising the audio stream")?;
    let event = match mode {
        StreamMode::PollingShared { .. } | StreamMode::PollingExclusive { .. } => None,
        _ => Some(
            client
                .set_get_eventhandle()
                .context("creating the stream event")?,
        ),
    };
    let capture = client
        .get_audiocaptureclient()
        .context("getting the capture client")?;
    client.start_stream().context("starting the stream")?;
    Ok(OpenedSource {
        client,
        event,
        capture,
        channels: channels.max(1),
        rate,
        description,
    })
}

/// Opens one source. `native` uses the device's own mix format (no WASAPI
/// conversion); otherwise 48 kHz stereo float is requested and converted by
/// WASAPI.
fn open_source(source: &AudioSource, native: bool, timing: Timing) -> Result<OpenedSource> {
    let client = match source {
        // include_tree: browsers play audio from child processes
        AudioSource::App { pid } => AudioClient::new_application_loopback_client(*pid, true)
            .context("opening per-application capture")?,
        AudioSource::Output { id } => pick_device(&Direction::Render, id.as_deref())?
            .get_iaudioclient()
            .context("opening the output device")?,
        AudioSource::Microphone { id } => pick_device(&Direction::Capture, id.as_deref())?
            .get_iaudioclient()
            .context("opening the microphone")?,
    };

    let (format, channels, rate, autoconvert) = if native {
        let mix = client
            .get_mixformat()
            .context("reading the device format")?;
        if mix.get_bitspersample() != 32 || mix.get_subformat().ok() != Some(SampleType::Float) {
            anyhow::bail!("the device format is not 32-bit float");
        }
        let (channels, rate) = (mix.get_nchannels() as usize, mix.get_samplespersec());
        (mix, channels, rate, false)
    } else {
        (
            WaveFormat::new(32, 32, &SampleType::Float, MIX_SAMPLE_RATE as usize, 2, None),
            2,
            MIX_SAMPLE_RATE,
            true,
        )
    };

    let mode = match timing {
        Timing::Events => StreamMode::EventsShared {
            autoconvert,
            buffer_duration_hns: 0,
        },
        Timing::EventsPeriod => StreamMode::EventsShared {
            autoconvert,
            buffer_duration_hns: client.get_device_period().map(|(_, min)| min).unwrap_or(0),
        },
        Timing::Polling => StreamMode::PollingShared {
            autoconvert,
            buffer_duration_hns: POLLING_BUFFER_HNS,
        },
    };

    let format_text = if native {
        format!("device format, {} Hz {} ch", rate, channels)
    } else {
        "48 kHz stereo".to_string()
    };
    let timing_text = if matches!(timing, Timing::Polling) {
        "polling"
    } else {
        "events"
    };
    start_client(
        client,
        &format,
        &mode,
        channels,
        rate,
        format!("{}, {}", format_text, timing_text),
    )
}

/// Everything playing on this PC except Pluely itself, through the
/// per-process loopback API ("exclude my own process tree"). It does not open
/// the output device, so it also works where device loopback is refused.
fn open_system_wide() -> Result<OpenedSource> {
    let client = AudioClient::new_application_loopback_client(std::process::id(), false)
        .context("opening system-wide capture")?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, MIX_SAMPLE_RATE as usize, 2, None);
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 0,
    };
    start_client(
        client,
        &format,
        &mode,
        2,
        MIX_SAMPLE_RATE,
        "system-wide capture without Pluely's own sound".to_string(),
    )
}

/// Opens a source, trying one capture configuration after another until the
/// driver accepts one. An output device that refuses all of them falls back to
/// system-wide capture. The error lists every distinct reason.
fn open_with_fallbacks(source: &AudioSource) -> std::result::Result<OpenedSource, String> {
    let attempts: &[(bool, Timing)] = match source {
        AudioSource::App { .. } => &[(false, Timing::Events)],
        _ => &[
            (false, Timing::EventsPeriod),
            (true, Timing::EventsPeriod),
            (false, Timing::Events),
            (true, Timing::Events),
            (true, Timing::Polling),
            (false, Timing::Polling),
        ],
    };

    let mut errors: Vec<String> = Vec::new();
    for &(native, timing) in attempts {
        match open_source(source, native, timing) {
            Ok(opened) => return Ok(opened),
            Err(e) => {
                warn!(
                    "{} refused (device format: {}, {:?}): {:#}",
                    describe(source),
                    native,
                    timing,
                    e
                );
                let text = format!("{:#}", e);
                if !errors.contains(&text) {
                    errors.push(text);
                }
            }
        }
    }

    // A healthy Windows never answers a loopback request in the device's own
    // format with "invalid parameter". When every configuration gets that
    // answer, something outside Pluely is refusing the capture.
    let blocked = !errors.is_empty() && errors.iter().all(|e| e.contains("0x80070057"));
    let hint = if blocked { BLOCKED_HINT } else { "" };

    if matches!(source, AudioSource::Output { .. }) {
        match open_system_wide() {
            Ok(mut opened) => {
                opened.description = format!(
                    "{} (the device refused direct capture: {}){}",
                    opened.description,
                    errors.join(" | "),
                    hint
                );
                return Ok(opened);
            }
            Err(e) => errors.push(format!("system-wide capture: {:#}", e)),
        }
    }
    Err(format!("{}{}", errors.join(" | "), hint))
}

/// Shown when Windows rejects every loopback configuration with E_INVALIDARG.
/// Seen with Kaspersky Endpoint Security, whose audio protection answers this
/// way to applications it does not trust and feeds silence to per-application
/// capture.
const BLOCKED_HINT: &str = " — Windows rejected every capture setting, which \
points to security software blocking sound capture rather than to the device. \
In Kaspersky: Intrusion Prevention → Manage applications → move pluely.exe to \
Trusted (or allow it to receive the audio stream), then restart Pluely.";

/// Mixes one interleaved frame down to mono. Layouts of six or more channels
/// follow the WAVEFORMATEXTENSIBLE order (front left, front right, centre,
/// subwoofer, then surround pairs): the subwoofer is dropped and the centre and
/// surrounds count at -3 dB, as in a stereo downmix. Averaging all eight
/// channels instead would make centre-channel dialogue far too quiet.
fn downmix(frame: &[f32]) -> f32 {
    const MINUS_3_DB: f32 = 0.707;
    match frame.len() {
        0 => 0.0,
        1 => frame[0],
        2 => (frame[0] + frame[1]) * 0.5,
        3..=5 => frame.iter().sum::<f32>() / frame.len() as f32,
        _ => {
            let mut left = frame[0] + MINUS_3_DB * frame[2];
            let mut right = frame[1] + MINUS_3_DB * frame[2];
            for (i, sample) in frame.iter().enumerate().skip(4) {
                if i % 2 == 0 {
                    left += MINUS_3_DB * sample;
                } else {
                    right += MINUS_3_DB * sample;
                }
            }
            (left + right) * 0.5
        }
    }
}

/// Captures one source as mono f32 at `MIX_SAMPLE_RATE` into `queue`.
fn source_capture_loop(
    source: AudioSource,
    queue: SampleQueue,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<std::result::Result<String, String>>,
) {
    let _ = wasapi::initialize_mta();

    let opened = match open_with_fallbacks(&source) {
        Ok(opened) => opened,
        Err(message) => {
            warn!("{} failed to start: {}", describe(&source), message);
            let _ = ready.send(Err(message));
            return;
        }
    };
    let OpenedSource {
        client,
        event,
        capture,
        channels,
        rate,
        description,
    } = opened;
    let _ = ready.send(Ok(description));
    drop(ready);

    // Linear resampling to MIX_SAMPLE_RATE when the device runs at another rate
    let step = rate as f64 / MIX_SAMPLE_RATE as f64;
    let mut position = 0.0f64;
    let mut previous = 0.0f32;
    let mut frame = vec![0.0f32; channels];

    while !stop.load(Ordering::Relaxed) {
        // Loopback streams send no events while nothing is playing; always
        // drain afterwards so a missing event cannot stall the capture.
        match &event {
            Some(event) => {
                let _ = event.wait_for_event(50);
            }
            None => thread::sleep(Duration::from_millis(10)),
        }

        let mut mono: Vec<f32> = Vec::new();
        while capture.get_next_packet_size().ok().flatten().unwrap_or(0) > 0 {
            let mut bytes: VecDeque<u8> = VecDeque::new();
            if capture.read_from_device_to_deque(&mut bytes).is_err() {
                break;
            }
            let frame_bytes = channels * 4;
            while bytes.len() >= frame_bytes {
                for sample in frame.iter_mut() {
                    let b = [
                        bytes.pop_front().unwrap_or(0),
                        bytes.pop_front().unwrap_or(0),
                        bytes.pop_front().unwrap_or(0),
                        bytes.pop_front().unwrap_or(0),
                    ];
                    *sample = f32::from_le_bytes(b);
                }
                mono.push(downmix(&frame));
            }
        }

        if mono.is_empty() {
            continue;
        }

        let samples: Vec<f32> = if rate == MIX_SAMPLE_RATE {
            mono
        } else {
            let mut out = Vec::with_capacity((mono.len() as f64 / step) as usize + 1);
            // position is relative to `previous` (index -1) and mono[0..]
            while position < mono.len() as f64 {
                let i = position.floor() as isize;
                let frac = (position - i as f64) as f32;
                let a = if i < 0 { previous } else { mono[i as usize] };
                let b = mono[(i + 1).clamp(0, mono.len() as isize - 1) as usize];
                out.push(a + (b - a) * frac);
                position += step;
            }
            position -= mono.len() as f64;
            previous = *mono.last().unwrap_or(&0.0);
            out
        };

        if let Ok(mut q) = queue.lock() {
            q.extend(samples);
            if q.len() > MIX_SOURCE_BUFFER_MAX {
                let excess = q.len() - MIX_SOURCE_BUFFER_MAX;
                q.drain(..excess);
            }
        }
    }

    let _ = client.stop_stream();
}

/// A capture thread together with the queue it fills and its start-up report.
struct RunningSource {
    source: AudioSource,
    queue: SampleQueue,
    handle: thread::JoinHandle<()>,
    /// How the source was opened, or why it could not be.
    ready: mpsc::Receiver<std::result::Result<String, String>>,
}

impl RunningSource {
    fn spawn(source: AudioSource, stop: &Arc<AtomicBool>) -> Self {
        let queue: SampleQueue = Arc::new(Mutex::new(VecDeque::new()));
        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = {
            let (source, queue, stop) = (source.clone(), queue.clone(), stop.clone());
            thread::spawn(move || source_capture_loop(source, queue, stop, ready_tx))
        };
        Self {
            source,
            queue,
            handle,
            ready: ready_rx,
        }
    }

    fn wait_ready(&self, deadline: Instant) -> std::result::Result<String, String> {
        self.ready
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|_| Err("Timed out while starting".to_string()))
    }
}

fn stop_sources(stop: &AtomicBool, running: Vec<RunningSource>) {
    stop.store(true, Ordering::Relaxed);
    for source in running {
        let _ = source.handle.join();
    }
}

/// Starts one capture thread per source and sums their audio every 10 ms.
fn mixer_loop(
    sources: Vec<AudioSource>,
    sample_queue: Arc<Mutex<VecDeque<f32>>>,
    waker_state: Arc<Mutex<WakerState>>,
    init_tx: mpsc::Sender<Result<u32>>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    let running: Vec<RunningSource> = sources
        .into_iter()
        .map(|source| RunningSource::spawn(source, &stop))
        .collect();

    let deadline = Instant::now() + SOURCE_START_TIMEOUT;
    let mut started = 0;
    let mut failures: Vec<String> = Vec::new();
    for source in &running {
        match source.wait_ready(deadline) {
            Ok(how) => {
                started += 1;
                info!("{} started ({})", describe(&source.source), how);
            }
            Err(e) => failures.push(format!("{}: {}", describe(&source.source), e)),
        }
    }

    if started == 0 {
        let _ = init_tx.send(Err(anyhow::anyhow!("{}", failures.join("; "))));
        stop_sources(&stop, running);
        return;
    }
    if !failures.is_empty() {
        warn!(
            "Some audio sources could not be started: {}",
            failures.join("; ")
        );
    }
    let _ = init_tx.send(Ok(MIX_SAMPLE_RATE));

    loop {
        if waker_state.lock().map(|s| s.shutdown).unwrap_or(true) {
            break;
        }
        thread::sleep(Duration::from_millis(10));

        let mut chunks: Vec<Vec<f32>> = Vec::new();
        for source in &running {
            if let Ok(mut q) = source.queue.lock() {
                if !q.is_empty() {
                    chunks.push(q.drain(..).collect());
                }
            }
        }
        let Some(len) = chunks.iter().map(Vec::len).max() else {
            continue;
        };

        let mut mixed = vec![0.0f32; len];
        for chunk in &chunks {
            for (out, sample) in mixed.iter_mut().zip(chunk) {
                *out += *sample;
            }
        }
        for sample in mixed.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }

        if let Ok(mut queue) = sample_queue.lock() {
            queue.extend(mixed);
            if queue.len() > MIX_OUTPUT_BUFFER_MAX {
                let excess = queue.len() - MIX_OUTPUT_BUFFER_MAX;
                queue.drain(..excess);
            }
        }
        if let Ok(mut state) = waker_state.lock() {
            if !state.has_data {
                state.has_data = true;
                if let Some(waker) = state.waker.take() {
                    drop(state);
                    waker.wake();
                }
            }
        }
    }

    stop_sources(&stop, running);
}

// ---------------------------------------------------------------------------
// Running applications with a window (for per-application capture)
// ---------------------------------------------------------------------------

/// File name of the executable behind a process id.
fn process_name(pid: u32) -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(str::to_string)
    }
}

pub fn list_audio_apps() -> Result<Vec<AudioApp>> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
    };

    unsafe extern "system" fn on_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let found = &mut *(lparam.0 as *mut Vec<(u32, String)>);

        if !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        // Skip owned windows (dialogs) and tool windows
        if GetWindow(hwnd, GW_OWNER)
            .map(|h| !h.0.is_null())
            .unwrap_or(false)
        {
            return true.into();
        }
        if (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW.0 != 0 {
            return true.into();
        }

        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len <= 0 {
            return true.into();
        }
        let title = String::from_utf16_lossy(&buf[..len as usize]);

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != 0 {
            found.push((pid, title));
        }
        true.into()
    }

    let mut windows: Vec<(u32, String)> = Vec::new();
    unsafe {
        EnumWindows(Some(on_window), LPARAM(&mut windows as *mut _ as isize))
            .map_err(|e| anyhow::anyhow!("Failed to enumerate windows: {}", e))?;
    }

    let own_pid = std::process::id();
    let mut by_pid: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for (pid, title) in windows {
        if pid != own_pid {
            by_pid.entry(pid).or_default().push(title);
        }
    }

    let mut apps: Vec<AudioApp> = by_pid
        .into_iter()
        .map(|(pid, titles)| AudioApp {
            pid,
            name: process_name(pid).unwrap_or_else(|| format!("pid {}", pid)),
            titles,
        })
        .collect();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(apps)
}

// ---------------------------------------------------------------------------
// Processes that are playing sound right now (audio sessions)
// ---------------------------------------------------------------------------

/// Lists the audio sessions of every output device with the process that owns
/// them and the loudest level seen over about a second. A browser reports its
/// audio-service child process here, which is the pid that really makes the
/// sound.
pub fn list_playing_audio() -> Result<Vec<PlayingAudio>> {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
    use windows::Win32::Media::Audio::{
        eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
        IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

    struct Session {
        pid: u32,
        device: String,
        active: bool,
        peak: f32,
        meter: Option<IAudioMeterInformation>,
    }

    let _ = wasapi::initialize_mta();

    let names: HashMap<String, String> = get_output_devices()
        .unwrap_or_default()
        .into_iter()
        .map(|d| (d.id, d.name))
        .collect();

    let mut sessions: Vec<Session> = Vec::new();
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("creating the audio device enumerator")?;
        let devices = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .context("listing the output devices")?;

        for i in 0..devices.GetCount().unwrap_or(0) {
            let Ok(device) = devices.Item(i) else {
                continue;
            };
            let device_id = device
                .GetId()
                .ok()
                .map(|p| {
                    let id = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
                    id
                })
                .unwrap_or_default();
            let device_name = names
                .get(&device_id)
                .cloned()
                .unwrap_or_else(|| "Output device".to_string());

            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(list) = manager.GetSessionEnumerator() else {
                continue;
            };

            for s in 0..list.GetCount().unwrap_or(0) {
                let Ok(control) = list.GetSession(s) else {
                    continue;
                };
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let pid = control2.GetProcessId().unwrap_or(0);
                if pid == 0 {
                    continue; // the system-sounds session
                }
                sessions.push(Session {
                    pid,
                    device: device_name.clone(),
                    active: control
                        .GetState()
                        .map(|state| state == AudioSessionStateActive)
                        .unwrap_or(false),
                    peak: 0.0,
                    meter: control.cast::<IAudioMeterInformation>().ok(),
                });
            }
        }

        // One reading is only a snapshot: keep the loudest of several.
        for _ in 0..8 {
            thread::sleep(Duration::from_millis(100));
            for session in sessions.iter_mut() {
                if let Some(meter) = &session.meter {
                    if let Ok(peak) = meter.GetPeakValue() {
                        session.peak = session.peak.max(peak);
                    }
                }
            }
        }
    }

    let mut found: Vec<PlayingAudio> = Vec::new();
    for session in sessions {
        match found
            .iter_mut()
            .find(|p| p.pid == session.pid && p.device == session.device)
        {
            Some(existing) => {
                existing.active |= session.active;
                existing.peak = existing.peak.max(session.peak);
            }
            None => found.push(PlayingAudio {
                pid: session.pid,
                name: process_name(session.pid).unwrap_or_else(|| format!("pid {}", session.pid)),
                device: session.device,
                active: session.active,
                peak: session.peak,
                level_known: true,
            }),
        }
    }
    found.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(
                b.peak
                    .partial_cmp(&a.peak)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(found)
}

// ---------------------------------------------------------------------------
// Probe: listen to each source for a moment and report its level
// ---------------------------------------------------------------------------

/// Captures every source in parallel for `seconds` and reports whether it
/// started, how it was opened and how loud it was. Lets the user see which
/// source really delivers sound.
pub fn probe_sources(sources: Vec<AudioSource>, seconds: u64) -> Vec<SourceProbe> {
    let stop = Arc::new(AtomicBool::new(false));
    let running: Vec<RunningSource> = sources
        .into_iter()
        .map(|source| RunningSource::spawn(source, &stop))
        .collect();

    let deadline = Instant::now() + SOURCE_START_TIMEOUT;
    let started: Vec<std::result::Result<String, String>> = running
        .iter()
        .map(|source| source.wait_ready(deadline))
        .collect();

    thread::sleep(Duration::from_secs(seconds));
    stop.store(true, Ordering::Relaxed);

    running
        .into_iter()
        .zip(started)
        .map(|(running, started)| {
            let RunningSource {
                source,
                queue,
                handle,
                ..
            } = running;
            let _ = handle.join();
            let samples: Vec<f32> = queue
                .lock()
                .map(|q| q.iter().copied().collect())
                .unwrap_or_default();
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            let rms = if samples.is_empty() {
                0.0
            } else {
                (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
            };
            let (mode, error) = match started {
                Ok(how) => (Some(how), None),
                Err(e) => (None, Some(e)),
            };
            SourceProbe {
                source,
                started: mode.is_some(),
                mode,
                error,
                samples: samples.len(),
                peak,
                rms,
            }
        })
        .collect()
}
