// SPDX-License-Identifier: MIT

//! Linux PCM audio output and playback clock shim, with zero GStreamer dependency.
//!
//! Rune lacks, at the pinned rev (`7a707454`): a Linux PCM audio output with a playback clock.
//! Rune's `Player` takes only a file or URL (`MediaSource`) and uses `video/unsupported.rs` off
//! macOS, so apps that decode audio themselves (for example in a sandboxed helper) have nowhere to
//! push interleaved PCM frames or read back how much audio has actually been heard.
//!
//! Rune issue: upstream [#525](https://github.com/anomalyco/rune/issues/525)
//! (`docs/rune/upstream/rune/26-pcm-output.md`; related: #30, #100, #112, #506).
//!
//! # Backends
//!
//! Every backend is loaded at runtime with `dlopen` (zero build-time audio header dependencies):
//!
//! 1. **PulseAudio simple API (`libpulse-simple.so.0`)** — native on modern PipeWire desktops (via
//!    `pipewire-pulse`) and PulseAudio.
//! 2. **ALSA PCM (`libasound.so.2`)** — fallback on systems without a sound server.
//! 3. **Silent real-time clock** — automatic fallback in headless environments (containers, CI) or
//!    after mid-playback device loss, pacing at the exact sample rate without panics or hangs.
//!
//! # Threading
//!
//! Opening, writing, latency queries, and flushing run exclusively on a dedicated worker thread.
//! Calling [`PcmOutput::open`], [`PcmOutput::push`], [`PcmOutput::play`], [`PcmOutput::pause`],
//! [`PcmOutput::flush`], [`PcmOutput::set_volume`], [`PcmOutput::set_muted`], or
//! [`PcmOutput::position`] from the UI thread never blocks on audio device I/O.

use std::{
    fmt, io,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

mod alsa;
mod backend;
mod pulse;
mod ring;
mod silent;
mod worker;

#[cfg(test)]
mod tests;

pub use ring::MAX_VOLUME;

use ring::sanitize_volume;
use worker::{BackendFactory, EventCallback, Shared, State};

/// Default bounded PCM lookahead capacity when none is explicitly configured.
const DEFAULT_BUFFER_DURATION: Duration = Duration::from_secs(2);
/// Maximum supported sample rate (768 kHz).
const MAX_SAMPLE_RATE: u32 = 768_000;
/// Maximum supported interleaved channel count.
const MAX_CHANNELS: u16 = 32;

/// Sample format of interleaved PCM frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// Signed 16-bit little-endian integer PCM (`i16`).
    S16Le,
    /// 32-bit IEEE 754 little-endian floating-point PCM (`f32`), nominal range `-1.0..=1.0`.
    F32Le,
}

impl SampleFormat {
    /// Number of bytes per single-channel sample.
    #[must_use]
    pub const fn sample_bytes(self) -> usize {
        match self {
            Self::S16Le => 2,
            Self::F32Le => 4,
        }
    }
}

/// PCM stream format specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmSpec {
    pub format: SampleFormat,
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmSpec {
    /// Creates a PCM stream specification.
    #[must_use]
    pub const fn new(format: SampleFormat, sample_rate: u32, channels: u16) -> Self {
        Self {
            format,
            sample_rate,
            channels,
        }
    }

    /// Bytes occupied by one multi-channel PCM frame.
    #[must_use]
    pub const fn frame_bytes(self) -> usize {
        self.format.sample_bytes() * (self.channels as usize)
    }

    /// Bytes consumed per second of playback.
    #[must_use]
    pub const fn bytes_per_second(self) -> u64 {
        (self.sample_rate as u64) * (self.frame_bytes() as u64)
    }

    /// Whether the sample rate and channel count are in supported bounds.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.sample_rate >= 1
            && self.sample_rate <= MAX_SAMPLE_RATE
            && self.channels >= 1
            && self.channels <= MAX_CHANNELS
    }

    /// Converts a frame count to playback duration using exact integer arithmetic.
    #[must_use]
    pub fn frames_to_duration(self, frames: u64) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        let nanos = u128::from(frames) * 1_000_000_000 / u128::from(self.sample_rate);
        let secs = u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX);
        let subsec_nanos = (nanos % 1_000_000_000) as u32;
        Duration::new(secs, subsec_nanos)
    }

    /// Converts a duration to a whole frame count.
    #[must_use]
    pub fn duration_to_frames(self, duration: Duration) -> u64 {
        let frames = duration.as_nanos() * u128::from(self.sample_rate) / 1_000_000_000;
        u64::try_from(frames).unwrap_or(u64::MAX)
    }
}

/// Active audio output backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// PulseAudio simple client (`libpulse-simple.so.0`, including PipeWire's `pipewire-pulse`).
    PulseAudio,
    /// ALSA PCM (`libasound.so.2`).
    Alsa,
    /// Real-time silent fallback clock (headless environment or after device loss).
    Silent,
}

/// Asynchronous lifecycle notifications emitted from the audio worker thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputEvent {
    /// The ring buffer ran dry while playing before [`PcmOutput::finish`] was called.
    Underrun,
    /// All queued PCM frames finished playing after [`PcmOutput::finish`].
    Drained,
    /// A hardware or daemon audio backend failed mid-playback and degraded to [`BackendKind::Silent`].
    DeviceLost,
}

/// Errors returned by [`PcmOutput`].
#[derive(Debug)]
pub enum OutputError {
    /// Sample rate or channel count was zero or outside supported limits.
    InvalidSpec,
    /// Pushed slice length was not aligned to [`PcmSpec::frame_bytes`], or did not match the format.
    UnalignedChunk,
    /// Bounded ring buffer does not have enough free space for the pushed chunk.
    BufferFull,
    /// [`PcmOutput::finish`] was already called on the current stream generation.
    Finished,
    /// Failed to spawn the dedicated audio worker thread.
    SpawnFailed(io::Error),
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpec => f.write_str("invalid PCM specification"),
            Self::UnalignedChunk => f.write_str("PCM chunk is not aligned to whole frames"),
            Self::BufferFull => f.write_str("PCM output buffer is full"),
            Self::Finished => f.write_str("PCM output already reached end of stream"),
            Self::SpawnFailed(err) => write!(f, "failed to spawn audio worker thread: {err}"),
        }
    }
}

impl std::error::Error for OutputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SpawnFailed(err) => Some(err),
            _ => None,
        }
    }
}

/// Configuration for opening a [`PcmOutput`].
pub struct OutputConfig {
    pub spec: PcmSpec,
    pub app_name: String,
    pub stream_name: String,
    pub buffer_duration: Duration,
    pub volume: f32,
    pub muted: bool,
    pub silent_only: bool,
    pub on_event: Option<EventCallback>,
}

impl OutputConfig {
    /// Creates a configuration with default buffer sizing (2 s), unity volume, unmuted.
    #[must_use]
    pub fn new(spec: PcmSpec) -> Self {
        Self {
            spec,
            app_name: String::from("audio-app"),
            stream_name: String::from("playback"),
            buffer_duration: DEFAULT_BUFFER_DURATION,
            volume: 1.0,
            muted: false,
            silent_only: false,
            on_event: None,
        }
    }

    #[must_use]
    pub fn app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = name.into();
        self
    }

    #[must_use]
    pub fn stream_name(mut self, name: impl Into<String>) -> Self {
        self.stream_name = name.into();
        self
    }

    #[must_use]
    pub fn buffer_duration(mut self, duration: Duration) -> Self {
        self.buffer_duration = duration;
        self
    }

    #[must_use]
    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    #[must_use]
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// Forces the silent real-time clock fallback without probing PulseAudio or ALSA.
    #[must_use]
    pub fn silent_only(mut self, silent_only: bool) -> Self {
        self.silent_only = silent_only;
        self
    }

    /// Registers an optional callback invoked from the audio worker thread on [`OutputEvent`].
    #[must_use]
    pub fn on_event(mut self, callback: impl Fn(OutputEvent) + Send + Sync + 'static) -> Self {
        self.on_event = Some(Arc::new(callback));
        self
    }
}

/// Non-blocking Linux PCM audio sink and monotonic playback clock.
pub struct PcmOutput {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl PcmOutput {
    /// Opens a PCM output in the paused state and spawns its dedicated audio worker thread.
    pub fn open(config: OutputConfig) -> Result<Self, OutputError> {
        let factory = BackendFactory::Auto {
            app_name: config.app_name,
            stream_name: config.stream_name,
            silent_only: config.silent_only,
        };
        Self::open_inner(
            config.spec,
            config.buffer_duration,
            config.volume,
            config.muted,
            config.on_event,
            factory,
        )
    }

    #[cfg(test)]
    fn open_with_backend(
        config: OutputConfig,
        build: impl FnOnce() -> Box<dyn backend::AudioBackend> + Send + 'static,
    ) -> Result<Self, OutputError> {
        Self::open_inner(
            config.spec,
            config.buffer_duration,
            config.volume,
            config.muted,
            config.on_event,
            BackendFactory::Custom(Box::new(build)),
        )
    }

    fn open_inner(
        spec: PcmSpec,
        buffer_duration: Duration,
        volume: f32,
        muted: bool,
        on_event: Option<EventCallback>,
        factory: BackendFactory,
    ) -> Result<Self, OutputError> {
        if !spec.is_valid() {
            return Err(OutputError::InvalidSpec);
        }
        let frames = spec.duration_to_frames(buffer_duration).max(1);
        let capacity_bytes = usize::try_from(frames)
            .unwrap_or(usize::MAX)
            .saturating_mul(spec.frame_bytes());
        let shared = Arc::new(Shared {
            spec,
            mutex: Mutex::new(State::new(spec, capacity_bytes, volume, muted)),
            wake: Condvar::new(),
            on_event,
        });
        let worker_shared = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name(String::from("pcm-audio-out"))
            .spawn(move || worker::run(worker_shared, factory))
            .map_err(OutputError::SpawnFailed)?;
        Ok(Self {
            shared,
            worker: Some(handle),
        })
    }

    /// Stream format specification.
    #[must_use]
    pub fn spec(&self) -> PcmSpec {
        self.shared.spec
    }

    /// Backend currently driving playback.
    #[must_use]
    pub fn backend(&self) -> BackendKind {
        self.shared.lock().backend_kind
    }

    /// Whether a hardware or daemon audio backend failed mid-playback and degraded to silent pacing.
    #[must_use]
    pub fn device_lost(&self) -> bool {
        self.shared.lock().device_lost
    }

    /// Queues interleaved PCM bytes without blocking. Returns [`OutputError::BufferFull`] if the
    /// bounded ring does not have room for the entire slice.
    pub fn push(&self, bytes: &[u8]) -> Result<(), OutputError> {
        let frame = self.shared.spec.frame_bytes();
        if !bytes.len().is_multiple_of(frame) {
            return Err(OutputError::UnalignedChunk);
        }
        let mut state = self.shared.lock();
        if state.finished {
            return Err(OutputError::Finished);
        }
        if bytes.is_empty() {
            return Ok(());
        }
        if !state.ring.push(bytes) {
            return Err(OutputError::BufferFull);
        }
        state.underrun_notified = false;
        self.shared.wake.notify_all();
        Ok(())
    }

    /// Convenience helper for pushing interleaved `i16` samples to an [`SampleFormat::S16Le`] output.
    pub fn push_s16(&self, samples: &[i16]) -> Result<(), OutputError> {
        if self.shared.spec.format != SampleFormat::S16Le
            || !samples
                .len()
                .is_multiple_of(usize::from(self.shared.spec.channels))
        {
            return Err(OutputError::UnalignedChunk);
        }
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.push(&bytes)
    }

    /// Convenience helper for pushing interleaved `f32` samples to an [`SampleFormat::F32Le`] output.
    pub fn push_f32(&self, samples: &[f32]) -> Result<(), OutputError> {
        if self.shared.spec.format != SampleFormat::F32Le
            || !samples
                .len()
                .is_multiple_of(usize::from(self.shared.spec.channels))
        {
            return Err(OutputError::UnalignedChunk);
        }
        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for &sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.push(&bytes)
    }

    /// Bytes currently queued ahead of the audio device.
    #[must_use]
    pub fn buffered_bytes(&self) -> usize {
        self.shared.lock().ring.len()
    }

    /// Bytes of free space currently available for [`PcmOutput::push`].
    #[must_use]
    pub fn free_bytes(&self) -> usize {
        self.shared.lock().ring.free()
    }

    /// Total bounded capacity of the PCM ring buffer in bytes.
    #[must_use]
    pub fn capacity_bytes(&self) -> usize {
        self.shared.lock().ring.capacity()
    }

    /// Whether `bytes` would currently fit into the bounded ring buffer.
    #[must_use]
    pub fn has_capacity(&self, bytes: usize) -> bool {
        let state = self.shared.lock();
        !state.finished && state.ring.free() >= bytes
    }

    /// Starts or resumes audio playback.
    pub fn play(&self) {
        let mut state = self.shared.lock();
        if !state.playing {
            state.playing = true;
            state.anchor_at = None;
            self.shared.wake.notify_all();
        }
    }

    /// Pauses playback immediately, freezing the playback clock and flushing buffered device latency
    /// without losing unheard PCM frames.
    pub fn pause(&self) {
        let mut state = self.shared.lock();
        if state.playing {
            let heard = state.current_heard_frames(self.shared.spec);
            state.heard_frames = heard;
            state.playing = false;
            state.anchor_at = None;
            self.shared.wake.notify_all();
        }
    }

    /// Whether playback is currently active.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.shared.lock().playing
    }

    /// Clears all buffered PCM, resets the playback clock to zero, and flushes the audio device
    /// (typically called on seek).
    pub fn flush(&self) {
        let mut state = self.shared.lock();
        state.ring.clear();
        state.tail.clear();
        state.finished = false;
        state.drained = false;
        state.underrun_notified = false;
        state.written_frames = 0;
        state.heard_frames = 0;
        state.anchor_at = None;
        state.generation = state.generation.wrapping_add(1);
        state.flush_pending = true;
        self.shared.wake.notify_all();
    }

    /// Marks end-of-stream so the remaining queued PCM drains cleanly and sets [`Self::is_drained`].
    pub fn finish(&self) {
        let mut state = self.shared.lock();
        if !state.finished {
            state.finished = true;
            self.shared.wake.notify_all();
        }
    }

    /// Whether [`Self::finish`] has been called on the current stream generation.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.shared.lock().finished
    }

    /// Whether all queued PCM has finished playing out after [`Self::finish`].
    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.shared.lock().drained
    }

    /// Sets linear output volume (`0.0` = silent, `1.0` = unity, clamped to `0.0..=MAX_VOLUME`).
    pub fn set_volume(&self, volume: f32) {
        let mut state = self.shared.lock();
        state.volume = sanitize_volume(volume);
    }

    /// Mutes or unmutes output immediately on subsequent audio quanta.
    pub fn set_muted(&self, muted: bool) {
        let mut state = self.shared.lock();
        state.muted = muted;
    }

    /// Current linear volume factor.
    #[must_use]
    pub fn volume(&self) -> f32 {
        self.shared.lock().volume
    }

    /// Whether output is currently muted.
    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.shared.lock().muted
    }

    /// Monotonic playback position in PCM frames heard since open or last [`Self::flush`].
    #[must_use]
    pub fn position_frames(&self) -> u64 {
        let state = self.shared.lock();
        state.current_heard_frames(self.shared.spec)
    }

    /// Monotonic playback position as a [`Duration`] heard since open or last [`Self::flush`].
    #[must_use]
    pub fn position(&self) -> Duration {
        self.shared.spec.frames_to_duration(self.position_frames())
    }
}

impl Drop for PcmOutput {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            state.closed = true;
            state.playing = false;
            self.shared.wake.notify_all();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}
