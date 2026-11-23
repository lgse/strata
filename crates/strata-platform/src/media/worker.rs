// SPDX-License-Identifier: MIT

//! Dedicated PCM audio worker thread, shared playback clock state, and failover loop.

use std::{
    sync::{Arc, Condvar, Mutex, PoisonError},
    time::{Duration, Instant},
};

use super::{
    BackendKind, OutputEvent, PcmSpec,
    backend::{AudioBackend, open_backend},
    ring::{PcmRing, UnheardTail, apply_gain, sanitize_volume},
    silent::SilentBackend,
};

/// Size of each write quantum sent to the audio backend (~10 ms).
const QUANTUM_MS: u64 = 10;
const UNHEARD_TAIL_MS: u64 = 500;
/// Maximum sub-quantum clock extrapolation ahead of the last backend latency reading.
const MAX_EXTRAPOLATION_US: u64 = 100_000;
/// Safety margin added to initial latency when waiting for a backend to finish draining at EOF.
const DRAIN_MARGIN_US: u64 = 20_000;
/// Polling slice while draining final device latency or waiting through an underrun.
const IDLE_SLICE: Duration = Duration::from_millis(10);

pub(super) type EventCallback = Arc<dyn Fn(OutputEvent) + Send + Sync>;

pub(super) struct State {
    pub(super) ring: PcmRing,
    pub(super) tail: UnheardTail,
    pub(super) playing: bool,
    pub(super) finished: bool,
    pub(super) drained: bool,
    pub(super) closed: bool,
    pub(super) flush_pending: bool,
    pub(super) generation: u64,
    pub(super) volume: f32,
    pub(super) muted: bool,
    pub(super) written_frames: u64,
    pub(super) heard_frames: u64,
    pub(super) reported_frames: u64,
    pub(super) anchor_at: Option<Instant>,
    pub(super) backend_kind: BackendKind,
    pub(super) device_lost: bool,
    pub(super) underrun_notified: bool,
}

impl State {
    pub(super) fn new(spec: PcmSpec, capacity_bytes: usize, volume: f32, muted: bool) -> Self {
        let frame_bytes = spec.frame_bytes().max(1);
        let tail_bytes = aligned_ms_bytes(spec, UNHEARD_TAIL_MS, 1);
        Self {
            ring: PcmRing::new(capacity_bytes, frame_bytes),
            tail: UnheardTail::new(tail_bytes),
            playing: false,
            finished: false,
            drained: false,
            closed: false,
            flush_pending: false,
            generation: 0,
            volume: sanitize_volume(volume),
            muted,
            written_frames: 0,
            heard_frames: 0,
            reported_frames: 0,
            anchor_at: None,
            backend_kind: BackendKind::Silent,
            device_lost: false,
            underrun_notified: false,
        }
    }

    pub(super) fn current_heard_frames(&mut self, spec: PcmSpec) -> u64 {
        let mut heard = self.heard_frames;
        if self.playing
            && !self.drained
            && let Some(since) = self.anchor_at
        {
            let elapsed_us = u64::try_from(since.elapsed().as_micros()).unwrap_or(u64::MAX);
            let extra =
                elapsed_us.min(MAX_EXTRAPOLATION_US) * u64::from(spec.sample_rate) / 1_000_000;
            heard = heard.saturating_add(extra).min(self.written_frames);
        }
        self.reported_frames = self.reported_frames.max(heard).min(self.written_frames);
        self.reported_frames
    }

    pub(super) fn record_heard_frames(&mut self, new_heard: u64) {
        let clamped = new_heard.min(self.written_frames);
        if clamped > self.heard_frames {
            self.heard_frames = clamped;
            self.reported_frames = self.reported_frames.max(clamped);
            self.anchor_at = self.playing.then(Instant::now);
        } else if self.playing
            && self.anchor_at.is_none()
            && self.heard_frames < self.written_frames
        {
            self.anchor_at = Some(Instant::now());
        }
    }

    pub(super) fn gain(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }
}

pub(super) struct Shared {
    pub(super) spec: PcmSpec,
    pub(super) mutex: Mutex<State>,
    pub(super) wake: Condvar,
    pub(super) on_event: Option<EventCallback>,
}

impl Shared {
    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn emit(&self, event: OutputEvent) {
        if let Some(callback) = &self.on_event {
            callback(event);
        }
    }
}

pub(super) enum BackendFactory {
    Auto {
        app_name: String,
        stream_name: String,
        silent_only: bool,
    },
    #[cfg(test)]
    Custom(Box<dyn FnOnce() -> Box<dyn AudioBackend> + Send>),
}

impl BackendFactory {
    fn open(self, spec: PcmSpec) -> Box<dyn AudioBackend> {
        match self {
            Self::Auto {
                app_name,
                stream_name,
                silent_only,
            } => open_backend(spec, &app_name, &stream_name, silent_only),
            #[cfg(test)]
            Self::Custom(build) => build(),
        }
    }
}

struct Worker {
    shared: Arc<Shared>,
    backend: Box<dyn AudioBackend>,
    scratch: Vec<u8>,
    was_playing: bool,
    drain_deadline: Option<Instant>,
}

pub(super) fn run(shared: Arc<Shared>, factory: BackendFactory) {
    let backend = factory.open(shared.spec);
    let kind = backend.kind();
    {
        let mut state = shared.lock();
        state.backend_kind = kind;
    }
    let quantum = aligned_ms_bytes(shared.spec, QUANTUM_MS, 1);
    let mut worker = Worker {
        shared,
        backend,
        scratch: vec![0; quantum],
        was_playing: false,
        drain_deadline: None,
    };
    worker.drive();
}

enum Action {
    Exit,
    Flush,
    PauseFlush,
    Write {
        len: usize,
        gain: f32,
        generation: u64,
    },
    Drain {
        generation: u64,
        written: u64,
        heard: u64,
    },
    Underrun {
        notify: bool,
    },
    Wait,
}

impl Worker {
    fn drive(&mut self) {
        loop {
            match self.next_action() {
                Action::Exit => break,
                Action::Flush => self.flush_backend(),
                Action::PauseFlush => self.flush_backend(),
                Action::Write {
                    len,
                    gain,
                    generation,
                } => self.write_quantum(len, gain, generation),
                Action::Drain {
                    generation,
                    written,
                    heard,
                } => self.step_drain(generation, written, heard),
                Action::Underrun { notify } => self.handle_underrun(notify),
                Action::Wait => self.wait_for_wakeup(),
            }
        }
    }

    fn next_action(&mut self) -> Action {
        let mut state = self.shared.lock();
        if state.closed {
            return Action::Exit;
        }
        if state.flush_pending {
            state.flush_pending = false;
            self.was_playing = false;
            self.drain_deadline = None;
            return Action::Flush;
        }
        if !state.playing {
            self.drain_deadline = None;
            return handle_paused(&mut state, self.shared.spec, &mut self.was_playing);
        }
        self.was_playing = true;
        let frame_bytes = self.shared.spec.frame_bytes().max(1);
        let len = state.ring.pop_into(&mut self.scratch, frame_bytes);
        if len > 0 {
            self.drain_deadline = None;
            state.tail.record(&self.scratch[..len]);
            return Action::Write {
                len,
                gain: state.gain(),
                generation: state.generation,
            };
        }
        if state.finished && !state.drained {
            return Action::Drain {
                generation: state.generation,
                written: state.written_frames,
                heard: state.heard_frames,
            };
        }
        if state.finished {
            return Action::Wait;
        }
        let notify = !state.underrun_notified;
        state.underrun_notified = true;
        Action::Underrun { notify }
    }

    fn flush_backend(&mut self) {
        if self.backend.flush().is_err() {
            self.degrade_to_silent();
        }
    }

    fn write_quantum(&mut self, len: usize, gain: f32, generation: u64) {
        apply_gain(self.shared.spec.format, &mut self.scratch[..len], gain);
        if self.backend.write(&self.scratch[..len]).is_err() {
            self.degrade_to_silent();
            let _ = self.backend.write(&self.scratch[..len]);
        }
        let latency = self.backend.latency_us();
        let frames = (len / self.shared.spec.frame_bytes().max(1)) as u64;
        let mut state = self.shared.lock();
        if state.generation != generation || state.flush_pending || state.closed {
            return;
        }
        state.written_frames = state.written_frames.saturating_add(frames);
        let latency_frames = latency.map_or(0, |us| us_to_frames(us, self.shared.spec));
        let heard = state.written_frames.saturating_sub(latency_frames);
        state.record_heard_frames(heard);
        state.underrun_notified = false;
    }

    fn step_drain(&mut self, generation: u64, written: u64, heard: u64) {
        let latency = if heard >= written {
            Some(0)
        } else {
            self.backend.latency_us()
        };
        let latency_us = latency.unwrap_or(0);
        let deadline = *self.drain_deadline.get_or_insert_with(|| {
            Instant::now() + Duration::from_micros(latency_us.saturating_add(DRAIN_MARGIN_US))
        });
        let finished = heard >= written || latency_us == 0 || Instant::now() >= deadline;
        if finished {
            let emitted = {
                let mut state = self.shared.lock();
                if state.generation == generation && !state.flush_pending && state.finished {
                    state.heard_frames = state.written_frames;
                    state.reported_frames = state.written_frames;
                    state.drained = true;
                    state.anchor_at = None;
                    true
                } else {
                    false
                }
            };
            self.drain_deadline = None;
            if emitted {
                self.shared.emit(OutputEvent::Drained);
            }
            return;
        }
        let updated_heard = written.saturating_sub(us_to_frames(latency_us, self.shared.spec));
        let mut state = self.shared.lock();
        if state.generation == generation && !state.flush_pending && state.playing {
            state.record_heard_frames(updated_heard);
            drop(self.shared.wake.wait_timeout(state, IDLE_SLICE));
        }
    }

    fn handle_underrun(&mut self, notify: bool) {
        if notify {
            self.shared.emit(OutputEvent::Underrun);
        }
        let state = self.shared.lock();
        if !state.closed
            && !state.flush_pending
            && state.playing
            && state.ring.is_empty()
            && !state.finished
        {
            drop(self.shared.wake.wait(state));
        }
    }

    fn wait_for_wakeup(&mut self) {
        let state = self.shared.lock();
        if !state.closed && !state.flush_pending && (!state.playing || state.drained) {
            drop(self.shared.wake.wait(state));
        }
    }

    fn degrade_to_silent(&mut self) {
        if self.backend.kind() == BackendKind::Silent {
            return;
        }
        self.backend = Box::new(SilentBackend::new(self.shared.spec));
        {
            let mut state = self.shared.lock();
            state.backend_kind = BackendKind::Silent;
            state.device_lost = true;
        }
        self.shared.emit(OutputEvent::DeviceLost);
    }
}

fn handle_paused(state: &mut State, spec: PcmSpec, was_playing: &mut bool) -> Action {
    if !*was_playing {
        return Action::Wait;
    }
    *was_playing = false;
    let frame_bytes = spec.frame_bytes().max(1);
    let unheard_frames = state.written_frames.saturating_sub(state.heard_frames);
    let unheard_bytes = usize::try_from(unheard_frames)
        .unwrap_or(usize::MAX)
        .saturating_mul(frame_bytes);
    let restored = state.tail.take_trailing(unheard_bytes, frame_bytes);
    let restored_frames = (restored.len() / frame_bytes) as u64;
    if !restored.is_empty() {
        state.ring.push_front(&restored);
    }
    state.written_frames = state.written_frames.saturating_sub(restored_frames);
    state.heard_frames = state.heard_frames.min(state.written_frames);
    state.reported_frames = state.reported_frames.min(state.written_frames);
    state.anchor_at = None;
    Action::PauseFlush
}

fn us_to_frames(us: u64, spec: PcmSpec) -> u64 {
    let numerator = u128::from(us) * u128::from(spec.sample_rate);
    u64::try_from(numerator / 1_000_000).unwrap_or(u64::MAX)
}

fn aligned_ms_bytes(spec: PcmSpec, ms: u64, min_frames: usize) -> usize {
    let frame = spec.frame_bytes().max(1);
    let raw = (spec.bytes_per_second() * ms / 1_000) as usize;
    (raw / frame).max(min_frames) * frame
}
