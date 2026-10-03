// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib, prelude::*, subclass::prelude::*};

use crate::{
    media::{self, Frame, Header, Packet},
    sandbox::media::{Event, Session},
    services::{MediaPreviewSize, SandboxedMedia},
};

mod audio;
#[cfg(debug_assertions)]
mod diagnostics;
#[cfg(test)]
mod tests;
use audio::PcmOutput;

const PAUSED_IDLE: Duration = Duration::from_secs(30);
const RESIZE_DELAY: Duration = Duration::from_millis(250);
const SEEK_DELAY: Duration = Duration::from_millis(200);
const PRESENTATION_QUEUE: usize = 3;
const MAX_STALL_RECOVERIES: u32 = 3;
const AUDIO_LEAD_CAP_US: u64 = 2_000_000;
const AUDIO_STUCK_TIMEOUT: Duration = Duration::from_secs(3);
const RESTORE_MIN_US: u64 = 1_000_000;
const MAX_REMEMBERED_POSITIONS: usize = 128;

static MEDIA_POSITIONS: LazyLock<Mutex<HashMap<PathBuf, (u64, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn remember_media_position(path: PathBuf, position: u64) {
    if let Ok(mut positions) = MEDIA_POSITIONS.lock() {
        if positions.len() >= MAX_REMEMBERED_POSITIONS && !positions.contains_key(&path) {
            let oldest = positions
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(path, _)| path.clone());
            if let Some(oldest) = oldest {
                positions.remove(&oldest);
            }
        }
        positions.insert(path, (position, Instant::now()));
    }
}

fn forget_media_position(path: &Path) {
    if let Ok(mut positions) = MEDIA_POSITIONS.lock() {
        positions.remove(path);
    }
}

pub(crate) fn recall_media_position(path: &Path) -> Option<u64> {
    MEDIA_POSITIONS.lock().ok().and_then(|mut positions| {
        let entry = positions.get_mut(path)?;
        entry.1 = Instant::now();
        Some(entry.0)
    })
}

#[cfg(test)]
type TestLoader = std::rc::Rc<dyn Fn(SandboxedMedia, u32) -> Result<Session, String>>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DecodedMedia {
        #[cfg(test)]
        pub(super) loader: RefCell<Option<TestLoader>>,
        pub(super) source: RefCell<Option<SandboxedMedia>>,
        pub(super) session: RefCell<Option<Session>>,
        pub(super) header: Cell<Option<Header>>,
        pub(super) loaded_size: Cell<Option<MediaPreviewSize>>,
        pub(super) texture: RefCell<Option<gdk::Texture>>,
        pub(super) frames: RefCell<VecDeque<Frame>>,
        pub(super) audio: RefCell<Option<PcmOutput>>,
        pub(super) timer: RefCell<Option<glib::SourceId>>,
        pub(super) closed: Cell<bool>,
        pub(super) restart: Cell<Option<u32>>,
        pub(super) starting: Cell<Option<Instant>>,
        pub(super) resized: Cell<Option<Instant>>,
        pub(super) paused: Cell<Option<Instant>>,
        pub(super) dormant: Cell<bool>,
        pub(super) seek_pending: Cell<Option<(u64, Instant)>>,
        pub(super) first_frame: Cell<bool>,
        pub(super) clock: Cell<Option<Instant>>,
        pub(super) clock_base: Cell<u64>,
        pub(super) position: Cell<u64>,
        pub(super) end: Cell<Option<u64>>,
        pub(super) last_progress: Cell<Option<Instant>>,
        pub(super) recoveries: Cell<u32>,
        pub(super) audio_stuck_since: Cell<Option<Instant>>,
        pub(super) restore: Cell<Option<u64>>,
        pub(super) history: RefCell<Option<PcmHistory>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DecodedMedia {
        const NAME: &'static str = "StrataDecodedMedia";
        type Type = super::DecodedMedia;
        type ParentType = gtk::MediaStream;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for DecodedMedia {
        fn dispose(&self) {
            self.obj().close();
        }
    }

    impl MediaStreamImpl for DecodedMedia {
        fn play(&self) -> bool {
            if self.closed.get() {
                return false;
            }
            let obj = self.obj();
            obj.ensure_timer();
            self.paused.set(None);
            if obj.is_ended() {
                obj.restart_at(0);
            } else if self.dormant.replace(false) {
                obj.restart_at(self.position.get());
            } else if self.first_frame.get() {
                self.clock.set(Some(Instant::now()));
                let result = self
                    .audio
                    .borrow()
                    .as_ref()
                    .map(PcmOutput::play)
                    .transpose();
                if let Err(error) = result {
                    obj.fail(&error);
                    return false;
                }
            }
            self.last_progress.set(Some(Instant::now()));
            self.paused.set(None);
            true
        }

        fn pause(&self) {
            self.obj().capture_position();
            self.clock.set(None);
            self.audio_stuck_since.set(None);
            self.paused.set(Some(Instant::now()));
            let result = self
                .audio
                .borrow()
                .as_ref()
                .map(PcmOutput::pause)
                .transpose();
            if let Err(error) = result {
                self.obj().fail(&error);
            }
        }

        fn seek(&self, timestamp: i64) {
            let position = timestamp.max(0) as u64;
            let obj = self.obj();
            if self
                .starting
                .get()
                .is_some_and(|time| time.elapsed() < SEEK_DELAY)
            {
                self.seek_pending.set(Some((position, Instant::now())));
                if obj.is_prepared() {
                    obj.update(position as i64);
                }
            } else {
                self.seek_pending.set(None);
                obj.restart_at(position);
                if obj.is_prepared() {
                    obj.update(self.position.get() as i64);
                }
            }
            self.dormant.set(false);
            obj.ensure_timer();
        }
        fn realize(&self, _: gdk::Surface) {}
        fn unrealize(&self, _: gdk::Surface) {}
        fn update_audio(&self, muted: bool, volume: f64) {
            if let Some(audio) = self.audio.borrow().as_ref() {
                audio.set_audio(muted, volume);
            }
        }
    }

    impl gdk::subclass::prelude::PaintableImpl for DecodedMedia {
        fn intrinsic_width(&self) -> i32 {
            self.texture
                .borrow()
                .as_ref()
                .map_or(0, |texture| texture.width())
        }
        fn intrinsic_height(&self) -> i32 {
            self.texture
                .borrow()
                .as_ref()
                .map_or(0, |texture| texture.height())
        }
        fn intrinsic_aspect_ratio(&self) -> f64 {
            self.texture.borrow().as_ref().map_or(0.0, |texture| {
                f64::from(texture.width()) / f64::from(texture.height())
            })
        }
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            if let Some(texture) = self.texture.borrow().as_ref() {
                texture.snapshot(snapshot, width, height);
            }
        }
    }
}

glib::wrapper! {
    pub struct DecodedMedia(ObjectSubclass<imp::DecodedMedia>) @extends gtk::MediaStream, @implements gdk::Paintable;
}

impl DecodedMedia {
    /// Keeps recently decoded PCM so `played_samples` can follow the sink.
    pub(crate) fn retain_played_audio(&self) {
        self.imp()
            .history
            .borrow_mut()
            .get_or_insert_with(PcmHistory::default);
    }

    /// Fills `window` with the mono samples that ended at the audible playhead.
    pub(crate) fn played_samples(&self, window: &mut [f32]) -> bool {
        let imp = self.imp();
        if !self.is_playing() || !imp.first_frame.get() {
            return false;
        }
        let elapsed = imp
            .clock
            .get()
            .map_or(0, |clock| clock.elapsed().as_micros() as u64);
        let played = (imp.clock_base.get() + elapsed) * media::SAMPLE_RATE / 1_000_000;
        imp.history
            .borrow()
            .as_ref()
            .is_some_and(|history| history.window_ending_at(played, window))
    }

    pub fn new(source: SandboxedMedia) -> Self {
        let obj: Self = glib::Object::new();
        // Songs start from the top like in a music player; videos reopen where they closed.
        let restore =
            recall_media_position(&source.path).filter(|&position| position > RESTORE_MIN_US);
        obj.imp().source.replace(Some(source));
        if let Some(position) = restore {
            obj.imp().restore.set(Some(position));
        }
        obj.restart_at(0);
        obj.ensure_timer();
        obj
    }

    fn ensure_timer(&self) {
        if self.imp().timer.borrow().is_some() || self.imp().closed.get() {
            return;
        }
        let weak = self.downgrade();
        let timer = glib::timeout_add_local(Duration::from_millis(8), move || {
            let Some(obj) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !obj.imp().closed.get()
                && let Err(error) = obj.tick()
            {
                obj.fail(&error);
            }
            if obj.imp().closed.get() || obj.imp().dormant.get() || obj.error().is_some() {
                obj.imp().timer.borrow_mut().take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.imp().timer.replace(Some(timer));
    }

    pub fn close(&self) {
        let imp = self.imp();
        if imp.closed.replace(true) {
            return;
        }
        if let Some(source) = imp.source.borrow().as_ref() {
            let position = imp.position.get();
            let duration = self.duration().max(0) as u64;
            if imp.header.get().is_some_and(|header| header.width > 0)
                && position > RESTORE_MIN_US
                && (duration == 0 || position < duration)
            {
                remember_media_position(source.path.clone(), position);
            } else {
                forget_media_position(&source.path);
            }
        }
        if let Some(timer) = imp.timer.borrow_mut().take() {
            timer.remove();
        }
        imp.session.borrow_mut().take();
        imp.audio.borrow_mut().take();
        imp.frames.borrow_mut().clear();
        imp.texture.borrow_mut().take();
        imp.history.borrow_mut().take();
        imp.source.borrow_mut().take();
        imp.restart.set(None);
        imp.seek_pending.set(None);
        self.invalidate_contents();
    }

    pub fn resize(&self, size: MediaPreviewSize) {
        let imp = self.imp();
        if let Some(source) = imp.source.borrow_mut().as_mut()
            && source.size != size
        {
            source.size = size;
            imp.resized.set(Some(Instant::now()));
        }
    }

    fn restart_at(&self, position: u64) {
        let imp = self.imp();
        let duration = if self.duration() > 0 {
            self.duration() as u64
        } else {
            imp.header.get().map_or(1, |header| header.duration_us)
        };
        let tick = media::seek_tick(position, duration);
        if let Some(session) = imp.session.borrow().as_ref() {
            session.cancel();
        }
        imp.audio.borrow_mut().take();
        imp.frames.borrow_mut().clear();
        if let Some(history) = imp.history.borrow_mut().as_mut() {
            history.clear();
        }
        imp.restart.set(Some(tick));
        imp.starting.set(Some(Instant::now()));
        imp.position.set(media::timestamp(tick));
        imp.clock.set(None);
        imp.clock_base.set(0);
        imp.audio_stuck_since.set(None);
        imp.first_frame.set(false);
        imp.end.set(None);
        imp.dormant.set(false);
        if !self.is_playing() {
            imp.paused.set(Some(Instant::now()));
        }
    }

    fn fail(&self, message: &str) {
        let imp = self.imp();
        imp.session.borrow_mut().take();
        imp.audio.borrow_mut().take();
        imp.frames.borrow_mut().clear();
        if self.is_seeking() {
            self.seek_failed();
        }
        if self.error().is_none() {
            self.set_error(glib::Error::new(gio::IOErrorEnum::Failed, message));
        }
    }

    fn recover(&self, reason: &str) -> Result<(), String> {
        let imp = self.imp();
        let recoveries = imp.recoveries.get();
        if recoveries >= MAX_STALL_RECOVERIES {
            return Err(reason.into());
        }
        imp.recoveries.set(recoveries + 1);
        tracing::warn!(
            recoveries = recoveries + 1,
            reason,
            "restarting stalled media playback"
        );
        self.capture_position_progress(false);
        let position = imp.position.get();
        self.restart_at(position);
        imp.last_progress.set(Some(Instant::now()));
        Ok(())
    }

    fn capture_position(&self) {
        self.capture_position_progress(true);
    }

    // A failure snapshot must not clear the strike even if the playhead moved.
    fn capture_position_progress(&self, clears_recoveries: bool) {
        let imp = self.imp();
        if !imp.first_frame.get() {
            return;
        }
        let base = imp.clock_base.get();
        let elapsed = imp
            .clock
            .get()
            .map_or(0, |clock| clock.elapsed().as_micros() as u64);
        let mut relative = base + elapsed;
        let mut audio_flowing = imp.audio.borrow().is_none();
        if let Some(audio) = imp.audio.borrow().as_ref() {
            match audio.position_us() {
                Some(position) if position > base => {
                    imp.clock_base.set(position);
                    imp.clock.set(self.is_playing().then(Instant::now));
                    imp.audio_stuck_since.set(None);
                    audio_flowing = true;
                    relative = position;
                }
                // Keep the audio anchor so a recovered sink can take over again.
                _ => {
                    relative = relative.min(base + AUDIO_LEAD_CAP_US);
                    if self.is_playing() && imp.audio_stuck_since.get().is_none() {
                        imp.audio_stuck_since.set(Some(Instant::now()));
                    }
                }
            }
        }
        if let Some(header) = imp.header.get() {
            let position = (media::timestamp(header.start_tick) + relative)
                .min(imp.end.get().unwrap_or(header.duration_us));
            if position != imp.position.get() {
                imp.last_progress.set(Some(Instant::now()));
                // Wall-clock drift over stuck audio must not reset the strike cap.
                if clears_recoveries && audio_flowing {
                    imp.recoveries.set(0);
                }
            }
            imp.position.set(position);
        }
    }

    fn tick(&self) -> Result<(), String> {
        let imp = self.imp();
        if imp
            .starting
            .get()
            .is_some_and(|time| time.elapsed() > media::STARTUP_TIMEOUT)
        {
            return Err("Media startup or seek timed out".into());
        }
        if imp
            .resized
            .get()
            .is_some_and(|time| time.elapsed() >= RESIZE_DELAY)
        {
            imp.resized.set(None);
            if !imp.dormant.get() && self.needs_resize() {
                self.capture_position();
                self.restart_at(imp.position.get());
            }
        }
        // A settled seek must wake a dormant, paused worker before idle handling.
        if let Some((position, since)) = imp.seek_pending.get()
            && since.elapsed() >= SEEK_DELAY
        {
            imp.seek_pending.take();
            self.restart_at(position);
            if self.is_prepared() {
                self.update(imp.position.get() as i64);
            }
        }
        if !self.is_playing()
            && imp
                .paused
                .get()
                .is_some_and(|time| time.elapsed() >= PAUSED_IDLE)
            && !imp.dormant.get()
            && imp.seek_pending.get().is_none()
        {
            imp.session.borrow_mut().take();
            imp.audio.borrow_mut().take();
            imp.frames.borrow_mut().clear();
            imp.restart.set(None);
            imp.dormant.set(true);
            tracing::debug!("sandboxed media paused worker released");
        }
        if imp.dormant.get() {
            return Ok(());
        }
        if let Some(tick) = imp.restart.get() {
            if imp
                .session
                .borrow()
                .as_ref()
                .is_some_and(|session| !session.finished())
            {
                return Ok(());
            }
            imp.session.borrow_mut().take();
            let source = imp
                .source
                .borrow()
                .as_ref()
                .cloned()
                .ok_or("Preview closed")?;
            let size = source.size;
            #[cfg(test)]
            let started = match imp.loader.borrow().as_ref() {
                Some(loader) => loader(source, tick),
                None => Session::start(source, tick),
            };
            #[cfg(not(test))]
            let started = Session::start(source, tick);
            match started {
                Ok(session) => {
                    imp.session.replace(Some(session));
                    imp.loaded_size.set(Some(size));
                    imp.restart.set(None);
                }
                Err(error)
                    if imp
                        .starting
                        .get()
                        .is_some_and(|time| time.elapsed() < Duration::from_millis(250)) =>
                {
                    let _ = error;
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
        while imp.end.get().is_none() && imp.frames.borrow().len() < PRESENTATION_QUEUE {
            if imp
                .audio
                .borrow()
                .as_ref()
                .is_some_and(|audio| !audio.has_capacity())
            {
                break;
            }
            let event = imp.session.borrow().as_ref().and_then(Session::receive);
            match event {
                Some(Event::Prepared(header)) => {
                    if let Some(saved) = imp.restore.take()
                        && header.width > 0
                        && saved < header.duration_us
                    {
                        imp.header.set(Some(header));
                        tracing::debug!(position_us = saved, "resuming media preview");
                        self.restart_at(saved);
                        break;
                    }
                    let audio = header
                        .audio
                        .then(|| PcmOutput::new(self.is_muted(), self.volume()))
                        .transpose()?;
                    imp.audio.replace(audio);
                    imp.header.set(Some(header));
                    if imp.source.borrow().as_ref().map(|source| source.size)
                        != imp.loaded_size.get()
                    {
                        imp.resized.set(Some(Instant::now()));
                    }
                    if !self.is_prepared() {
                        let known_duration = header.duration_us != media::MAX_DURATION_US;
                        self.stream_prepared(
                            header.audio,
                            header.width > 0,
                            known_duration,
                            if known_duration {
                                header.duration_us as i64
                            } else {
                                0
                            },
                        );
                    }
                }
                Some(Event::Packet(Packet::Frame(mut frame))) => {
                    let header = imp.header.get().ok_or("Missing media header")?;
                    if let Some(audio) = imp.audio.borrow().as_ref() {
                        let samples_left = (header.duration_us - media::timestamp(frame.tick))
                            * media::SAMPLE_RATE
                            / 1_000_000;
                        frame
                            .samples
                            .truncate(samples_left.min(media::AUDIO_BYTES as u64 / 4) as usize * 4);
                        if !frame.samples.is_empty() {
                            if let Some(history) = imp.history.borrow_mut().as_mut() {
                                history.push(&frame.samples);
                            }
                            audio.push(
                                std::mem::take(&mut frame.samples),
                                media::timestamp(frame.tick - header.start_tick),
                            )?;
                        }
                    }
                    if !imp.first_frame.get() {
                        let latency_ms = imp
                            .starting
                            .get()
                            .map_or(0, |time| time.elapsed().as_millis())
                            as u64;
                        imp.first_frame.set(true);
                        imp.starting.set(None);
                        imp.last_progress.set(Some(Instant::now()));
                        if self.is_playing() {
                            imp.clock.set(Some(Instant::now()));
                            if let Some(audio) = imp.audio.borrow().as_ref() {
                                audio.play()?;
                            }
                        }
                        self.present(frame);
                        tracing::debug!(
                            latency_ms,
                            position_us = media::timestamp(header.start_tick),
                            width = header.width,
                            height = header.height,
                            "sandboxed media first frame"
                        );
                        if self.is_seeking() {
                            self.seek_success();
                        }
                        self.update(imp.position.get() as i64);
                    } else {
                        imp.frames.borrow_mut().push_back(frame);
                    }
                }
                Some(Event::Packet(Packet::End(duration))) => {
                    imp.end.set(Some(duration));
                    if let Some(audio) = imp.audio.borrow().as_ref() {
                        audio.finish()?;
                    }
                    if self.duration() != duration as i64 {
                        let playing = self.is_playing();
                        self.capture_position();
                        let header = imp.header.get().ok_or("Missing media header")?;
                        let _notifications = self.freeze_notify();
                        self.stream_unprepared();
                        self.stream_prepared(header.audio, header.width > 0, true, duration as i64);
                        self.update(imp.position.get().min(duration) as i64);
                        if playing {
                            self.play();
                        }
                    }
                    break;
                }
                Some(Event::Failed(error)) => {
                    if imp.first_frame.get() {
                        self.recover(&error)?;
                        return Ok(());
                    }
                    return Err(error);
                }
                None => break,
            }
        }
        let audio_error = imp.audio.borrow().as_ref().and_then(PcmOutput::error);
        if let Some(error) = audio_error {
            if imp.first_frame.get() {
                self.recover(&error)?;
                return Ok(());
            }
            return Err(error);
        }
        if self.is_playing() && imp.first_frame.get() {
            self.capture_position();
            if imp
                .audio_stuck_since
                .get()
                .is_some_and(|since| since.elapsed() > AUDIO_STUCK_TIMEOUT)
            {
                self.recover("Audio output stopped advancing")?;
                return Ok(());
            }
            let ended = imp.end.get().is_some_and(|end| {
                // The advertised duration can fall between 48-kHz sample boundaries.
                let tolerance = if imp.header.get().is_some_and(|header| header.audio) {
                    1_000_000_u64.div_ceil(media::SAMPLE_RATE) + 1
                } else {
                    0
                };
                if imp.position.get().saturating_add(tolerance) >= end {
                    imp.position.set(end);
                    true
                } else {
                    false
                }
            });
            loop {
                let frame = {
                    let mut frames = imp.frames.borrow_mut();
                    if frames
                        .front()
                        .is_some_and(|frame| media::timestamp(frame.tick) <= imp.position.get())
                    {
                        frames.pop_front()
                    } else {
                        None
                    }
                };
                let Some(frame) = frame else {
                    break;
                };
                self.present(frame);
            }
            self.update(imp.position.get() as i64);
            if ended {
                if self.is_loop() {
                    self.restart_at(0);
                } else {
                    imp.session.borrow_mut().take();
                    imp.audio.borrow_mut().take();
                    self.stream_ended();
                }
            } else if imp
                .last_progress
                .get()
                .is_some_and(|time| time.elapsed() > media::FRAME_TIMEOUT)
            {
                self.recover("Media playback clock stopped making progress")?;
            }
        }
        Ok(())
    }

    fn needs_resize(&self) -> bool {
        let imp = self.imp();
        let (Some(header), Some(loaded)) = (imp.header.get(), imp.loaded_size.get()) else {
            return false;
        };
        if header.width == 0 {
            return false;
        }
        let Some(size) = imp.source.borrow().as_ref().map(|source| source.size) else {
            return false;
        };
        let mut scale = (f64::from(size.width) / f64::from(header.width))
            .min(f64::from(size.height) / f64::from(header.height));
        if header.width + 1 < loaded.width as u32 && header.height + 1 < loaded.height as u32 {
            scale = scale.min(1.0);
        }
        (f64::from(header.width) * (scale - 1.0)).abs() >= 2.0
            || (f64::from(header.height) * (scale - 1.0)).abs() >= 2.0
    }

    fn present(&self, frame: Frame) {
        let imp = self.imp();
        if let Some(header) = imp.header.get()
            && header.width > 0
        {
            #[cfg(debug_assertions)]
            let pixels = diagnostics::Pixels::new(frame.pixels);
            #[cfg(not(debug_assertions))]
            let pixels = frame.pixels;
            let texture = gdk::MemoryTexture::new(
                header.width as i32,
                header.height as i32,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_owned(pixels),
                header.width as usize * 4,
            )
            .upcast::<gdk::Texture>();
            let size_changed = imp.texture.borrow().as_ref().is_none_or(|old| {
                old.width() != texture.width() || old.height() != texture.height()
            });
            imp.texture.replace(Some(texture));
            if size_changed {
                self.invalidate_size();
            }
            self.invalidate_contents();
        }
    }
}

const HISTORY_SAMPLES: usize = 48_000;

/// Mono PCM indexed by samples since the last restart, matching the sink clock.
#[derive(Default)]
pub(crate) struct PcmHistory {
    samples: VecDeque<f32>,
    end: u64,
}

impl PcmHistory {
    fn clear(&mut self) {
        self.samples.clear();
        self.end = 0;
    }

    fn push(&mut self, interleaved: &[u8]) {
        for frame in interleaved.as_chunks::<4>().0 {
            let left = i16::from_le_bytes([frame[0], frame[1]]);
            let right = i16::from_le_bytes([frame[2], frame[3]]);
            self.samples
                .push_back((f32::from(left) + f32::from(right)) / 65_536.0);
        }
        self.end += (interleaved.len() / 4) as u64;
        let excess = self.samples.len().saturating_sub(HISTORY_SAMPLES);
        self.samples.drain(..excess);
    }

    fn window_ending_at(&self, played: u64, window: &mut [f32]) -> bool {
        let played = played.min(self.end);
        let start = self.end - self.samples.len() as u64;
        if played <= start {
            return false;
        }
        let available = (played - start) as usize;
        let copied = available.min(window.len());
        let missing = window.len() - copied;
        window[..missing].fill(0.0);
        for (target, source) in window[missing..]
            .iter_mut()
            .zip(self.samples.range(available - copied..available))
        {
            *target = *source;
        }
        true
    }
}
