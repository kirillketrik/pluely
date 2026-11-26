use anyhow::Result;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::{SpeakerInput as PlatformSpeakerInput, SpeakerStream as PlatformSpeakerStream};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::{SpeakerInput as PlatformSpeakerInput, SpeakerStream as PlatformSpeakerStream};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::{SpeakerInput as PlatformSpeakerInput, SpeakerStream as PlatformSpeakerStream};

mod commands;

// Re-export commands for tauri handler
pub use commands::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// One audio source that can be captured and mixed with others.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioSource {
    /// Everything played through an output device (system loopback).
    Output { id: Option<String> },
    /// A microphone / capture device.
    Microphone { id: Option<String> },
    /// Audio of one application (and its child processes), by process id.
    App { pid: u32 },
}

/// A running application with a visible window that can be captured.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioApp {
    pub pid: u32,
    pub name: String,
    pub titles: Vec<String>,
}

#[cfg(target_os = "windows")]
pub(crate) fn list_audio_apps() -> Result<Vec<AudioApp>> {
    windows::list_audio_apps()
}

#[cfg(target_os = "linux")]
pub(crate) fn list_audio_apps() -> Result<Vec<AudioApp>> {
    linux::list_audio_apps()
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) fn list_audio_apps() -> Result<Vec<AudioApp>> {
    Ok(vec![])
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn list_input_devices() -> Result<Vec<AudioDevice>> {
    #[cfg(target_os = "macos")]
    return macos::get_input_devices();

    #[cfg(target_os = "windows")]
    return windows::get_input_devices();

    #[cfg(target_os = "linux")]
    return linux::get_input_devices();
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn list_output_devices() -> Result<Vec<AudioDevice>> {
    #[cfg(target_os = "macos")]
    return macos::get_output_devices();

    #[cfg(target_os = "windows")]
    return windows::get_output_devices();

    #[cfg(target_os = "linux")]
    return linux::get_output_devices();
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) fn list_input_devices() -> Result<Vec<AudioDevice>> {
    Ok(vec![])
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) fn list_output_devices() -> Result<Vec<AudioDevice>> {
    Ok(vec![])
}

// Pluely speaker input and stream
pub struct SpeakerInput {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    inner: PlatformSpeakerInput,
}

impl SpeakerInput {
    // Creates a new speaker input. Fails on unsupported platforms.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    pub fn new() -> Result<Self> {
        let inner = PlatformSpeakerInput::new(None)?;
        Ok(Self { inner })
    }

    // Creates a new speaker input with a specific device ID
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    pub fn new_with_device(device_id: Option<String>) -> Result<Self> {
        let inner = PlatformSpeakerInput::new(device_id)?;
        Ok(Self { inner })
    }

    // Creates a speaker input that mixes several sources on platforms that
    // support source selection.
    #[cfg(target_os = "windows")]
    pub fn new_with_sources(sources: Vec<AudioSource>) -> Result<Self> {
        let inner = PlatformSpeakerInput::with_sources(sources)?;
        Ok(Self { inner })
    }

    #[cfg(target_os = "linux")]
    pub fn new_with_sources(sources: Vec<AudioSource>) -> Result<Self> {
        let inner = PlatformSpeakerInput::with_sources(sources)?;
        Ok(Self { inner })
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    pub fn new_with_sources(_sources: Vec<AudioSource>) -> Result<Self> {
        Self::new_with_device(None)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    pub fn new() -> Result<Self> {
        Err(anyhow::anyhow!(
            "SpeakerInput::new is not supported on this platform"
        ))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    pub fn new_with_device(_device_id: Option<String>) -> Result<Self> {
        Err(anyhow::anyhow!(
            "SpeakerInput::new_with_device is not supported on this platform"
        ))
    }

    // Starts the audio stream.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    pub fn stream(self) -> SpeakerStream {
        let inner = self.inner.stream();
        SpeakerStream { inner }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    pub fn stream(self) -> SpeakerStream {
        unimplemented!("SpeakerInput::stream is not supported on this platform")
    }
}

// Stream of f32 audio samples from the speaker.
pub struct SpeakerStream {
    inner: PlatformSpeakerStream,
}

impl Stream for SpeakerStream {
    type Item = f32;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            Pin::new(&mut self.inner).poll_next(cx)
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            std::task::Poll::Pending
        }
    }
}

impl SpeakerStream {
    // Gets the sample rate (e.g., 16000 Hz on stub, variable on real impls).
    pub fn sample_rate(&self) -> u32 {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        return self.inner.sample_rate();

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        0
    }

    // Why capture could not start, if that is what happened (Windows only).
    pub fn init_error(&self) -> Option<String> {
        #[cfg(target_os = "windows")]
        return self.inner.init_error();

        #[cfg(not(target_os = "windows"))]
        None
    }
}

/// Result of listening to one source for a moment.
#[derive(Debug, Clone, Serialize)]
pub struct SourceProbe {
    pub source: AudioSource,
    pub started: bool,
    /// How the source was opened (format, timing, or a fallback).
    pub mode: Option<String>,
    pub error: Option<String>,
    pub samples: usize,
    pub peak: f32,
    pub rms: f32,
}

/// A process that has an audio session on an output device.
#[derive(Debug, Clone, Serialize)]
pub struct PlayingAudio {
    pub pid: u32,
    pub name: String,
    pub device: String,
    /// The session is rendering sound right now.
    pub active: bool,
    /// Loudest level seen while listening, 0.0 to 1.0.
    pub peak: f32,
    /// Whether `peak` is a measured level rather than an unavailable value.
    pub level_known: bool,
}

#[cfg(target_os = "windows")]
pub(crate) fn list_playing_audio() -> Result<Vec<PlayingAudio>> {
    windows::list_playing_audio()
}

#[cfg(target_os = "linux")]
pub(crate) fn list_playing_audio() -> Result<Vec<PlayingAudio>> {
    linux::list_playing_audio()
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) fn list_playing_audio() -> Result<Vec<PlayingAudio>> {
    Ok(vec![])
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_sources(sources: Vec<AudioSource>, seconds: u64) -> Vec<SourceProbe> {
    windows::probe_sources(sources, seconds)
}

#[cfg(target_os = "linux")]
pub(crate) fn probe_sources(sources: Vec<AudioSource>, seconds: u64) -> Vec<SourceProbe> {
    linux::probe_sources(sources, seconds)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) fn probe_sources(sources: Vec<AudioSource>, _seconds: u64) -> Vec<SourceProbe> {
    sources
        .into_iter()
        .map(|source| SourceProbe {
            source,
            started: false,
            mode: None,
            error: Some("Source test is only available on Windows".to_string()),
            samples: 0,
            peak: 0.0,
            rms: 0.0,
        })
        .collect()
}
