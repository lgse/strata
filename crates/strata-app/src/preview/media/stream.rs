// SPDX-License-Identifier: MIT

//! Sandboxed media playback and A/V sync: GTK's `DecodedMedia` (`src/ui/media.rs`,
//! `src/ui/media/playback.rs`) driven by [`strata_platform::media::PcmOutput`] instead of
//! GStreamer.
//!
//! - **Decoding** runs in the shared `strata_preview::sandbox::media::Session`'s worker. A
//!   [`Pump`] on GLib's main context drains its events every 8 ms and hands them to Rune
//!   through `Later`, bounded both by [`PRESENTATION_QUEUE`] video frames and by the PCM sink's
//!   lookahead capacity so the worker waits for the player as GTK's `can_queue_more` makes it.
//! - **The clock** follows `PcmOutput::position` whenever the clip carries audio (accounting for
//!   output device latency, or paced in real time on the silent fallback when headless), so video
//!   frames stay A/V-synced to heard audio. Silent clips (video without audio, GIFs) pace on the
//!   poll's clock.
//! - **A paused player** pauses and flushes the output device immediately, releases its worker
//!   and audio stream after [`PAUSED_IDLE`], and restarts where it stood on the next play.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
    time::Duration,
};

use rune::Tx;
use strata_components::media_player::{PlaybackClock, PlaybackState};
use strata_fs::services::{MediaPreviewSize, SandboxedMedia};
use strata_platform::media::{OutputConfig, OutputError, PcmOutput, PcmSpec, SampleFormat};
use strata_preview::{
    media::{self, Frame, Header, Packet},
    sandbox::media::{Event, Session},
};

use super::positions;
use crate::{App, slots::Later};

/// Frames decoded ahead of the one shown (GTK's `PRESENTATION_QUEUE`).
pub(super) const PRESENTATION_QUEUE: usize = 3;
/// Interleaved PCM format emitted by the sandboxed decoder (48 kHz stereo S16LE).
const AUDIO_SPEC: PcmSpec = PcmSpec::new(SampleFormat::S16Le, media::SAMPLE_RATE as u32, 2);
/// Lookahead buffered in the PCM sink: covers the first record's lead plus presentation slack.
const AUDIO_LOOKAHEAD: Duration =
    Duration::from_micros((media::AUDIO_LEAD_TICKS as u64 + 12) * 1_000_000 / media::FPS as u64);
/// Sample-boundary tolerance when checking end-of-stream against the audio clock.
const AUDIO_END_TOLERANCE_US: u64 = 1_000_000_u64.div_ceil(media::SAMPLE_RATE) + 1;
/// A selection replaced within this long never starts a decoder (GTK's `START_DWELL`).
const START_DWELL: f64 = 0.05;
/// A start that fails this soon, as one does while the last worker still holds its slot, is
/// tried again (GTK's `start_session`).
const START_RETRY: f64 = 0.25;
/// How long a paused player keeps its worker (GTK's `PAUSED_IDLE`).
const PAUSED_IDLE: f64 = 30.0;
/// How long a resize settles before the decoder restarts at the new size (`RESIZE_DELAY`).
const RESIZE_DELAY: f64 = 0.25;
/// How often the pump drains the worker's events.
const PUMP_INTERVAL: Duration = Duration::from_millis(8);
/// How often a waiting player looks again: before its start, or for a retry.
const WAITING: Duration = Duration::from_millis(16);

/// What a pump hands the app: the events of one session, in order.
pub(super) type Received = Vec<Event>;

/// The GLib side of one decoder session. The app holds it; the timer holds it weakly and
/// stops once the app lets it go, which cancels the session.
pub(super) struct Pump {
    session: Session,
    /// Frames the app can take before the worker must wait.
    room: Cell<usize>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.session.cancel();
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }
}

/// Where the app takes a pump's events: the viewer and session they belong to, and the
/// update that hands them over.
#[derive(Clone)]
pub(super) struct Inlet {
    pub(super) later: Later<App>,
    pub(super) viewer: u64,
    /// Hands over the events of the session for the viewer: `(viewer, session)`.
    pub(super) deliver: fn(&mut App, (u64, u64), Received, &mut Tx) -> bool,
}

/// What starts a decoder session at a tick.
#[cfg(test)]
type Start = fn(SandboxedMedia, u32) -> Result<Session, String>;

#[cfg(test)]
thread_local! {
    /// What starts a decoder in tests: a fixture stream in the sandbox's place.
    pub(in crate::preview) static START: Cell<Option<Start>> = const { Cell::new(None) };
}

fn start_session(source: SandboxedMedia, tick: u32) -> Result<Session, String> {
    #[cfg(test)]
    if let Some(start) = START.get() {
        return start(source, tick);
    }
    Session::start(source, tick)
}

impl Pump {
    fn start(
        source: SandboxedMedia,
        tick: u32,
        session: u64,
        inlet: Inlet,
    ) -> Result<Rc<Self>, String> {
        let pump = Rc::new(Self {
            session: start_session(source, tick)?,
            room: Cell::new(PRESENTATION_QUEUE),
            timer: RefCell::new(None),
        });
        let weak = Rc::downgrade(&pump);
        let timer = glib::timeout_add_local(PUMP_INTERVAL, move || drain(&weak, session, &inlet));
        pump.timer.replace(Some(timer));
        Ok(pump)
    }
}

fn drain(pump: &Weak<Pump>, session: u64, inlet: &Inlet) -> glib::ControlFlow {
    let Some(pump) = pump.upgrade() else {
        return glib::ControlFlow::Break;
    };
    let mut events = Vec::new();
    let mut done = false;
    while !done && pump.room.get() > 0 {
        let Some(event) = pump.session.receive() else {
            break;
        };
        match &event {
            Event::Packet(Packet::Frame(_)) => pump.room.set(pump.room.get() - 1),
            Event::Packet(Packet::End(_)) | Event::Failed(_) => done = true,
            Event::Prepared(_) => {}
        }
        events.push(event);
    }
    if !events.is_empty() {
        let (viewer, deliver) = (inlet.viewer, inlet.deliver);
        inlet
            .later
            .then(move |app: &mut App, tx: &mut Tx| deliver(app, (viewer, session), events, tx));
    }
    if done {
        pump.timer.borrow_mut().take();
        glib::ControlFlow::Break
    } else {
        glib::ControlFlow::Continue
    }
}

/// What a frame shown now is: its pixels, in the header's size.
pub(super) struct Shown {
    pub(super) size: [u32; 2],
    pub(super) pixels: Vec<u8>,
}

/// One player's decode, audio output, and playback clock.
pub(super) struct Stream {
    source: SandboxedMedia,
    inlet: Inlet,
    pump: Option<Rc<Pump>>,
    /// The current session, so events from one cancelled are dropped.
    session: u64,
    /// A restart waiting to start its session, at this tick, not before this time.
    restart: Option<(u32, f64)>,
    /// When the current start began, for a retry of a start that failed at once.
    starting: Option<f64>,
    header: Option<Header>,
    frames: VecDeque<Frame>,
    audio: Option<PcmOutput>,
    pending_audio: VecDeque<Vec<u8>>,
    audio_chunks: u32,
    audio_base_us: u64,
    audio_eos_pending: bool,
    /// Frame slots owed back to the pump once the audio sink has room.
    held_room: usize,
    volume: f32,
    muted: bool,
    first_frame: bool,
    /// A seek runs: no frame from its place yet.
    seeking: bool,
    end: Option<u64>,
    position_us: u64,
    /// Playing without audio: the poll clock and the position it played on from.
    anchor: Option<(f64, u64)>,
    playing: bool,
    ended: bool,
    paused_at: Option<f64>,
    dormant: bool,
    resized_at: Option<f64>,
    looped: bool,
    /// The configured start, a percentage of the duration, until the header takes it.
    start_percent: u32,
    /// A position saved when this file was left partway, until the header takes it.
    restore: Option<u64>,
    start_requested: bool,
    error: Option<String>,
    now: f64,
}

impl Stream {
    pub(super) fn new(source: SandboxedMedia, inlet: Inlet, start_percent: u32, now: f64) -> Self {
        let previewed = positions::recall(&source.path).is_some();
        let restore = positions::resume(&source.path);
        Self {
            source,
            inlet,
            pump: None,
            session: 0,
            restart: Some((0, now + START_DWELL)),
            starting: Some(now),
            header: None,
            frames: VecDeque::new(),
            audio: None,
            pending_audio: VecDeque::new(),
            audio_chunks: 0,
            audio_base_us: 0,
            audio_eos_pending: false,
            held_room: 0,
            volume: 1.0,
            muted: false,
            first_frame: false,
            seeking: false,
            end: None,
            position_us: 0,
            anchor: None,
            playing: false,
            ended: false,
            paused_at: Some(now),
            dormant: false,
            resized_at: None,
            looped: false,
            start_percent: if previewed { 0 } else { start_percent },
            restore,
            start_requested: start_percent > 0,
            error: None,
            now,
        }
    }

    /// Plays from the start again at the end, as a GIF does.
    pub(super) fn set_loop(&mut self, looped: bool) {
        self.looped = looped;
    }

    /// Applies linear output volume (`0.0..=1.0+`) and mute immediately to live audio.
    pub(super) fn set_audio(&mut self, volume: f32, muted: bool) {
        self.volume = volume;
        self.muted = muted;
        if let Some(audio) = &self.audio {
            audio.set_volume(volume);
            audio.set_muted(muted);
        }
    }

    #[cfg(test)]
    pub(in crate::preview) fn audio_levels(&self) -> Option<(f32, bool)> {
        self.audio
            .as_ref()
            .map(|audio| (audio.volume(), audio.is_muted()))
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn has_video(&self) -> bool {
        self.header.is_some_and(|header| header.width > 0)
    }

    pub(super) fn playing(&self) -> bool {
        self.playing
    }

    pub(super) fn play(&mut self) {
        if self.error.is_some() {
            return;
        }
        if self.ended {
            self.ended = false;
            self.restart_at(0);
        } else if std::mem::take(&mut self.dormant) {
            self.restart_at(self.position_us);
        }
        self.playing = true;
        self.paused_at = None;
        if self.first_frame {
            if let Some(audio) = &self.audio {
                audio.play();
            } else {
                self.anchor = Some((self.now, self.position_us));
            }
        }
    }

    pub(super) fn pause(&mut self) {
        self.capture();
        self.playing = false;
        self.anchor = None;
        self.paused_at = Some(self.now);
        if let Some(audio) = &self.audio {
            audio.pause();
        }
    }

    pub(super) fn seek(&mut self, to: Duration) {
        self.ended = false;
        self.dormant = false;
        self.restart_at(to.as_micros() as u64);
        self.seeking = true;
    }

    /// The pane asks for frames this big: the decoder restarts at the new size once the
    /// resize settles.
    pub(super) fn resize(&mut self, [width, height]: [u32; 2]) {
        let size = MediaPreviewSize::new(width as i32, height as i32);
        if self.source.size != size {
            self.source.size = size;
            self.resized_at = Some(self.now);
        }
    }

    fn restart_at(&mut self, position_us: u64) {
        let duration = self
            .header
            .map_or(1, |header| self.end.unwrap_or(header.duration_us));
        let tick = media::seek_tick(position_us, duration);
        self.pump = None;
        self.session += 1;
        self.frames.clear();
        self.pending_audio.clear();
        self.audio_chunks = 0;
        self.audio_eos_pending = false;
        self.held_room = 0;
        self.first_frame = false;
        self.end = None;
        self.position_us = media::timestamp(tick);
        self.audio_base_us = self.position_us;
        self.anchor = None;
        if let Some(audio) = &self.audio {
            audio.pause();
            audio.flush();
        }
        self.restart = Some((tick, self.now));
        self.starting = Some(self.now);
        if !self.playing {
            self.paused_at = Some(self.now);
        }
    }

    /// The poll's clock moved to `now`: a waiting start starts, playback moves on, and the
    /// frame whose time has come is returned to show.
    pub(super) fn advance(&mut self, now: f64, tx: &mut Tx) -> Option<Shown> {
        self.now = now;
        if self.error.is_some() {
            return None;
        }
        self.settle_resize();
        self.release_idle();
        self.start_waiting(tx);
        self.drain_pending_audio();
        self.flush_room();
        if !self.playing || !self.first_frame {
            return None;
        }
        self.capture();
        let ended = self.reached_end();
        let shown = self.due_frame();
        self.flush_room();
        if ended {
            self.finish();
        }
        shown
    }

    fn settle_resize(&mut self) {
        if self
            .resized_at
            .is_some_and(|at| self.now - at >= RESIZE_DELAY)
        {
            self.resized_at = None;
            if !self.dormant && self.has_video() {
                self.capture();
                self.restart_at(self.position_us);
            }
        }
    }

    /// Paused long enough, with a worker that has shown its frame.
    fn idle(&self) -> bool {
        let paused_long = self
            .paused_at
            .is_some_and(|at| self.now - at >= PAUSED_IDLE);
        let holds_worker = self.first_frame && !self.dormant;
        paused_long && holds_worker && !self.playing
    }

    fn release_idle(&mut self) {
        if self.idle() {
            self.pump = None;
            self.audio = None;
            self.restart = None;
            self.frames.clear();
            self.pending_audio.clear();
            self.audio_eos_pending = false;
            self.held_room = 0;
            self.dormant = true;
            tracing::debug!("sandboxed media paused worker released");
        }
    }

    fn start_waiting(&mut self, tx: &mut Tx) {
        let Some((tick, after)) = self.restart else {
            return;
        };
        if self.now < after {
            tx.wake_after(WAITING);
            return;
        }
        let inlet = self.inlet.clone();
        match Pump::start(self.source.clone(), tick, self.session, inlet) {
            Ok(pump) => {
                self.pump = Some(pump);
                self.restart = None;
            }
            Err(error) if self.starting.is_some_and(|at| self.now - at < START_RETRY) => {
                tracing::debug!(error, "sandboxed media start deferred");
                self.restart = Some((tick, self.now + START_DWELL));
                tx.wake_after(WAITING);
            }
            Err(error) => self.fail(error),
        }
    }

    /// The events a pump handed over, and the first frame to show if they hold it. Events of a
    /// session that has gone are dropped.
    pub(super) fn receive(&mut self, session: u64, events: Received) -> Option<Shown> {
        if session != self.session || self.error.is_some() {
            return None;
        }
        let mut shown = None;
        for event in events {
            match event {
                Event::Prepared(header) => self.prepared(header),
                Event::Packet(Packet::Frame(frame)) => {
                    if let Some(first) = self.frame(frame) {
                        shown = Some(first);
                    }
                }
                Event::Packet(Packet::End(duration)) => self.on_end(duration),
                Event::Failed(error) => self.fail(error),
            }
            if session != self.session {
                break;
            }
        }
        shown
    }

    fn prepared(&mut self, header: Header) {
        self.header = Some(header);
        let known = header.duration_us != media::MAX_DURATION_US;
        let video = header.width > 0;
        if let Some(saved) = self
            .restore
            .take()
            .filter(|&saved| video && saved < header.duration_us)
        {
            tracing::debug!(position_us = saved, "resuming media preview");
            self.restart_at(saved);
            return;
        }
        let percent = std::mem::take(&mut self.start_percent);
        let start = (video && known)
            .then(|| header.duration_us / 100 * u64::from(percent))
            .filter(|&position| media::seek_tick(position, header.duration_us) > 0);
        if let Some(position) = start {
            tracing::debug!(position_us = position, "starting video preview");
            self.restart_at(position);
            return;
        }
        self.ensure_audio(header);
    }

    fn ensure_audio(&mut self, header: Header) {
        self.audio_base_us = media::timestamp(header.start_tick);
        self.audio_chunks = 0;
        self.pending_audio.clear();
        self.audio_eos_pending = false;
        if !header.audio {
            self.audio = None;
            return;
        }
        if let Some(audio) = &self.audio {
            audio.pause();
            audio.flush();
            audio.set_volume(self.volume);
            audio.set_muted(self.muted);
            return;
        }
        let config = OutputConfig::new(AUDIO_SPEC)
            .app_name("Strata")
            .stream_name("Media preview")
            .buffer_duration(AUDIO_LOOKAHEAD)
            .volume(self.volume)
            .muted(self.muted)
            .silent_only(cfg!(test));
        match PcmOutput::open(config) {
            Ok(output) => self.audio = Some(output),
            Err(error) => {
                tracing::warn!(%error, "unable to open PCM audio output; using silent clock");
            }
        }
    }

    fn on_end(&mut self, duration: u64) {
        self.end = Some(duration);
        self.audio_eos_pending = self.audio.is_some();
        self.drain_pending_audio();
    }

    /// Queues a decoded frame, or returns the first one to show at once.
    fn frame(&mut self, mut frame: Frame) -> Option<Shown> {
        if let Some(header) = self.header.filter(|header| header.audio) {
            let samples = std::mem::take(&mut frame.samples);
            self.queue_audio(&header, samples);
            self.drain_pending_audio();
        }
        if self.first_frame {
            if self.has_video() {
                self.frames.push_back(frame);
            } else {
                self.held_room += 1;
                self.flush_room();
            }
            return None;
        }
        self.first_frame = true;
        self.seeking = false;
        self.starting = None;
        self.held_room += 1;
        self.flush_room();
        if self.playing {
            if let Some(audio) = &self.audio {
                audio.play();
            } else {
                self.anchor = Some((self.now, self.position_us));
            }
        }
        self.show(frame)
    }

    /// Splits a record's PCM into tick-sized blocks, trims any trailing pad past `duration_us`,
    /// and pushes them to the PCM sink (holding overflow in `pending_audio` for backpressure).
    fn queue_audio(&mut self, header: &Header, samples: Vec<u8>) {
        if self.audio.is_none() || samples.is_empty() {
            return;
        }
        for block in samples.chunks(media::AUDIO_BYTES) {
            let chunk = self.audio_chunks;
            let tick = header.start_tick.saturating_add(chunk);
            let samples_left = header.duration_us.saturating_sub(media::timestamp(tick))
                * media::SAMPLE_RATE
                / 1_000_000;
            let max_frames = samples_left.min(block.len() as u64 / 4) as usize;
            let trimmed = &block[..max_frames * 4];
            if !trimmed.is_empty() {
                self.push_or_pend_audio(trimmed);
            }
            self.audio_chunks = chunk + 1;
        }
    }

    fn push_or_pend_audio(&mut self, block: &[u8]) {
        if self.pending_audio.is_empty()
            && let Some(audio) = &self.audio
        {
            match audio.push(block) {
                Ok(()) => return,
                Err(OutputError::BufferFull) => {}
                Err(_) => return,
            }
        }
        self.pending_audio.push_back(block.to_vec());
    }

    fn drain_pending_audio(&mut self) {
        while let Some(front) = self.pending_audio.front() {
            let Some(audio) = &self.audio else {
                self.pending_audio.clear();
                break;
            };
            match audio.push(front) {
                Ok(()) => {
                    self.pending_audio.pop_front();
                }
                Err(OutputError::BufferFull) => break,
                Err(_) => {
                    self.pending_audio.pop_front();
                }
            }
        }
        if self.audio_eos_pending && self.pending_audio.is_empty() {
            self.audio_eos_pending = false;
            if let Some(audio) = &self.audio {
                audio.finish();
            }
        }
    }

    fn can_accept_more_audio(&self) -> bool {
        if !self.pending_audio.is_empty() {
            return false;
        }
        self.audio
            .as_ref()
            .is_none_or(|audio| audio.has_capacity(media::AUDIO_BYTES))
    }

    fn flush_room(&mut self) {
        while self.held_room > 0 && self.can_accept_more_audio() {
            self.held_room -= 1;
            self.give_room();
        }
    }

    fn due_frame(&mut self) -> Option<Shown> {
        let mut due = None;
        while self
            .frames
            .front()
            .is_some_and(|frame| media::timestamp(frame.tick) <= self.position_us)
        {
            due = self.frames.pop_front();
            self.held_room += 1;
        }
        due.and_then(|frame| self.show(frame))
    }

    fn give_room(&self) {
        if let Some(pump) = &self.pump {
            pump.room.set((pump.room.get() + 1).min(PRESENTATION_QUEUE));
        }
    }

    fn show(&self, frame: Frame) -> Option<Shown> {
        let header = self.header.filter(|header| header.width > 0)?;
        if frame.pixels.len() != header.video_bytes() {
            return None;
        }
        Some(Shown {
            size: [header.width, header.height],
            pixels: frame.pixels,
        })
    }

    fn capture(&mut self) {
        let limit = self
            .end
            .or(self.header.map(|header| header.duration_us))
            .unwrap_or(u64::MAX);
        if let Some(audio) = &self.audio {
            if !self.first_frame {
                return;
            }
            let heard_us = u64::try_from(audio.position().as_micros()).unwrap_or(u64::MAX);
            self.position_us = self.audio_base_us.saturating_add(heard_us).min(limit);
            return;
        }
        let Some((since, from)) = self.anchor else {
            return;
        };
        let played = ((self.now - since).max(0.0) * 1_000_000.0) as u64;
        self.position_us = from.saturating_add(played).min(limit);
    }

    fn reached_end(&mut self) -> bool {
        let Some(end) = self.end else {
            return false;
        };
        if let Some(audio) = &self.audio {
            let by_time = self.position_us.saturating_add(AUDIO_END_TOLERANCE_US) >= end;
            let by_drain = self.pending_audio.is_empty() && audio.is_drained();
            if by_time || by_drain {
                self.position_us = end;
                return true;
            }
            return false;
        }
        self.position_us >= end
    }

    fn finish(&mut self) {
        if self.looped {
            self.restart_at(0);
            return;
        }
        self.pump = None;
        self.audio = None;
        self.pending_audio.clear();
        self.audio_eos_pending = false;
        self.held_room = 0;
        self.playing = false;
        self.anchor = None;
        self.ended = true;
        self.paused_at = Some(self.now);
    }

    fn fail(&mut self, error: String) {
        tracing::debug!(error, "sandboxed media preview failed");
        self.pump = None;
        self.audio = None;
        self.restart = None;
        self.frames.clear();
        self.pending_audio.clear();
        self.audio_eos_pending = false;
        self.held_room = 0;
        self.playing = false;
        self.anchor = None;
        self.error = Some(error);
    }

    /// The file is left: a video stopped partway reopens where it stood (GTK's `close`).
    pub(super) fn close(&mut self) {
        self.capture();
        self.pump = None;
        self.audio = None;
        self.pending_audio.clear();
        let path = self.source.path.clone();
        let duration = self.end.or(self.header.map(|header| header.duration_us));
        let partway = self.position_us > positions::RESTORE_MIN_US
            && duration.is_none_or(|duration| duration == 0 || self.position_us < duration);
        if self.has_video() && partway {
            positions::remember(path, self.position_us);
        } else if self.start_requested && self.has_video() {
            // A video closed in its first second or at its end reopens at 0:00, not at the
            // configured start.
            positions::remember(path, 0);
        } else {
            positions::forget(&path);
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.close();
    }
}

impl PlaybackClock for Stream {
    fn position(&self) -> Duration {
        Duration::from_micros(self.position_us)
    }

    fn duration(&self) -> Option<Duration> {
        let header = self.header?;
        let duration = self.end.unwrap_or(header.duration_us);
        (duration != media::MAX_DURATION_US).then(|| Duration::from_micros(duration))
    }

    fn state(&self) -> PlaybackState {
        let settled = self.first_frame || self.dormant || self.error.is_some();
        if self.ended {
            PlaybackState::Ended
        } else if settled && self.playing {
            PlaybackState::Playing
        } else if settled {
            PlaybackState::Paused
        } else if self.seeking {
            PlaybackState::Seeking
        } else {
            PlaybackState::Loading
        }
    }
}
