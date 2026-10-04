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

pub(crate) mod ambient;
mod audio;
#[cfg(debug_assertions)]
mod diagnostics;
#[cfg(test)]
mod tests;
use audio::PcmOutput;

const PAUSED_IDLE: Duration = Duration::from_secs(30);
// A selection replaced within the dwell never spawns a decoder.
pub(crate) const START_DWELL: Duration = Duration::from_millis(50);
const RESIZE_DELAY: Duration = Duration::from_millis(250);
const SEEK_DELAY: Duration = Duration::from_millis(200);
const PRESENTATION_QUEUE: usize = 3;
// PCM appsrc may hold: the lead the first record carries plus slack, so the
// sink's own buffer fills before the playhead has to move.
const AUDIO_LOOKAHEAD: Duration =
    Duration::from_micros((media::AUDIO_LEAD_TICKS as u64 + 6) * 1_000_000 / media::FPS as u64);
const MAX_STALL_RECOVERIES: u32 = 3;
const AUDIO_LEAD_CAP_US: u64 = 2_000_000;
const AUDIO_STUCK_TIMEOUT: Duration = Duration::from_secs(3);
const RESTORE_MIN_US: u64 = 1_000_000;
const MAX_REMEMBERED_POSITIONS: usize = 128;
// Edge colours for the glow are refreshed at most this often.
const EDGE_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

static MEDIA_POSITIONS: LazyLock<Mutex<HashMap<PathBuf, (u64, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(test)]
thread_local! {
    static TEST_STREAMS: Cell<bool> = const { Cell::new(false) };
}

/// Every stream created afterwards on this thread decodes the synthetic test
/// clip, for tests that reach the player only through the window.
#[cfg(test)]
pub(crate) fn use_test_streams(enabled: bool) {
    TEST_STREAMS.with(|streams| streams.set(enabled));
}

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
        #[cfg(test)]
        pub(super) audio_sink: RefCell<Option<String>>,
        #[cfg(test)]
        pub(super) audio_clock: RefCell<Option<gstreamer::Clock>>,
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
        pub(super) start_after: Cell<Option<Instant>>,
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
        pub(super) audio_anchored: Cell<bool>,
        pub(super) audio_chunks: Cell<u32>,
        pub(super) restore: Cell<Option<u64>>,
        pub(super) history: RefCell<Option<PcmHistory>>,
        pub(super) edge_grid: Cell<Option<ambient::EdgeGrid>>,
        pub(super) edge_sampled: Cell<Option<Instant>>,
        pub(super) fade: Cell<Option<f64>>,
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
                self.clock.set(obj.wall_clock_allowed().then(Instant::now));
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
            // Resuming waits for the sink again, like a fresh start.
            self.audio_anchored.set(false);
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

    impl DecodedMedia {
        // The probed frame size, so the layout settles before the first frame.
        fn frame_size(&self) -> (i32, i32) {
            if let Some(texture) = self.texture.borrow().as_ref() {
                return (texture.width(), texture.height());
            }
            self.header
                .get()
                .filter(|header| header.width > 0)
                .map_or((0, 0), |header| (header.width as i32, header.height as i32))
        }
    }

    impl gdk::subclass::prelude::PaintableImpl for DecodedMedia {
        fn intrinsic_width(&self) -> i32 {
            self.frame_size().0
        }
        fn intrinsic_height(&self) -> i32 {
            self.frame_size().1
        }
        fn intrinsic_aspect_ratio(&self) -> f64 {
            match self.frame_size() {
                (_, 0) => 0.0,
                (width, height) => f64::from(width) / f64::from(height),
            }
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
        imp.history
            .borrow()
            .as_ref()
            .is_some_and(|history| history.window_ending_at(self.played_sample(), window))
    }

    // Mirrors the playhead: wall time past the last sink position, capped like
    // the scrubber while the sink is stalled.
    fn played_sample(&self) -> u64 {
        let imp = self.imp();
        let elapsed = imp
            .clock
            .get()
            .map_or(0, |clock| clock.elapsed().as_micros() as u64);
        (imp.clock_base.get() + elapsed.min(AUDIO_LEAD_CAP_US)) * media::SAMPLE_RATE / 1_000_000
    }

    pub fn new(source: SandboxedMedia) -> Self {
        let obj: Self = glib::Object::new();
        let restore =
            recall_media_position(&source.path).filter(|&position| position > RESTORE_MIN_US);
        obj.imp().source.replace(Some(source));
        if let Some(position) = restore {
            obj.imp().restore.set(Some(position));
        }
        obj.imp()
            .start_after
            .set(Some(Instant::now() + START_DWELL));
        #[cfg(test)]
        if TEST_STREAMS.with(Cell::get) {
            obj.use_test_stream();
        }
        obj.restart_at(0);
        obj.ensure_timer();
        obj
    }

    pub(crate) fn source(&self) -> Option<SandboxedMedia> {
        self.imp().source.borrow().clone()
    }

    /// The probed video frame size, known once the stream is prepared.
    pub(crate) fn video_size(&self) -> Option<(u32, u32)> {
        self.imp()
            .header
            .get()
            .filter(|header| header.width > 0)
            .map(|header| (header.width, header.height))
    }

    /// Whether a decoded frame is on screen.
    pub(crate) fn has_frame(&self) -> bool {
        self.imp().texture.borrow().is_some()
    }

    /// Border colours of a recent frame, refreshed at most ten times a second.
    pub(crate) fn edge_grid(&self) -> Option<ambient::EdgeGrid> {
        self.imp().edge_grid.get()
    }

    /// Scales the audio output by `fade` on top of the user's volume, across
    /// restarts of the sink. It never touches the stream's volume property.
    pub(crate) fn set_fade(&self, fade: f64) {
        let fade = fade.clamp(0.0, 1.0);
        self.imp().fade.set(Some(fade));
        if let Some(audio) = self.imp().audio.borrow().as_ref() {
            audio.set_fade(fade);
        }
    }

    pub(crate) fn fade(&self) -> f64 {
        self.imp().fade.get().unwrap_or(1.0)
    }

    /// Decodes a synthetic minute-long clip instead of the sandbox, so a
    /// test's stream survives without a real file.
    #[cfg(test)]
    pub(crate) fn use_test_stream(&self) {
        self.imp().loader.replace(Some(std::rc::Rc::new(|_, tick| {
            crate::sandbox::media::tests::stream(Header {
                width: 16,
                height: 16,
                audio: false,
                duration_us: 60_000_000,
                start_tick: tick,
            })
        })));
    }

    #[cfg(test)]
    pub(crate) fn present_test_frame(&self, width: u32, height: u32) {
        let imp = self.imp();
        let header = imp.header.get().unwrap_or(Header {
            width,
            height,
            audio: false,
            duration_us: 10_000_000,
            start_tick: 0,
        });
        imp.header.set(Some(header));
        if !self.is_prepared() {
            self.stream_prepared(false, true, true, header.duration_us as i64);
        }
        imp.first_frame.set(true);
        self.present(Frame {
            tick: header.start_tick,
            pixels: vec![0; header.video_bytes()],
            samples: Vec::new(),
        });
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
        imp.edge_grid.set(None);
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
        imp.audio_anchored.set(false);
        imp.audio_chunks.set(0);
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
                    imp.audio_anchored.set(true);
                    audio_flowing = true;
                    relative = position;
                }
                // Keep the audio anchor so a recovered sink can take over again.
                // The stuck timer only watches a clock that has run; a sink that is
                // still starting is bounded by the progress watchdog instead.
                _ => {
                    relative = relative.min(base + AUDIO_LEAD_CAP_US);
                    if self.is_playing()
                        && imp.audio_anchored.get()
                        && imp.audio_stuck_since.get().is_none()
                    {
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

    // Until the sink has advanced since the generation started or playback
    // resumed, the playhead waits for it rather than leading on wall time, which
    // would later snap back by the sink's delay. Afterwards a stalled clock
    // still lets video lead.
    fn wall_clock_allowed(&self) -> bool {
        let imp = self.imp();
        imp.audio.borrow().is_none() || imp.audio_anchored.get()
    }

    // Audio files read ahead until appsrc is full, so a sink that only starts
    // once its own buffer fills is fed regardless of the playhead. Video keeps
    // the three-frame presentation queue; its audio runs ahead inside the
    // records instead of pinning frames.
    fn can_queue_more(&self) -> bool {
        let imp = self.imp();
        let frames = imp.frames.borrow().len();
        let Some(header) = imp.header.get() else {
            return true;
        };
        match imp.audio.borrow().as_ref() {
            Some(audio) => {
                audio.has_capacity() && (header.width == 0 || frames < PRESENTATION_QUEUE)
            }
            None => frames < PRESENTATION_QUEUE,
        }
    }

    fn pcm_output(&self) -> Result<PcmOutput, String> {
        #[cfg(test)]
        let output = if let Some(description) = self.imp().audio_sink.borrow().as_deref() {
            PcmOutput::test_sink(
                self.is_muted(),
                self.volume(),
                AUDIO_LOOKAHEAD,
                description,
                self.imp().audio_clock.borrow().as_ref(),
            )?
        } else {
            PcmOutput::new(self.is_muted(), self.volume(), AUDIO_LOOKAHEAD)?
        };
        #[cfg(not(test))]
        let output = PcmOutput::new(self.is_muted(), self.volume(), AUDIO_LOOKAHEAD)?;
        if let Some(fade) = self.imp().fade.get() {
            output.set_fade(fade);
        }
        Ok(output)
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
            if imp
                .start_after
                .get()
                .is_some_and(|after| Instant::now() < after)
            {
                return Ok(());
            }
            imp.start_after.set(None);
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
        while imp.end.get().is_none() && self.can_queue_more() {
            let event = imp.session.borrow().as_ref().and_then(Session::receive);
            match event {
                Some(Event::Prepared(header)) => {
                    if let Some(saved) = imp.restore.take()
                        && header.width > 0
                        && saved < header.duration_us
                    {
                        imp.header.set(Some(header));
                        self.invalidate_size();
                        tracing::debug!(position_us = saved, "resuming media preview");
                        self.restart_at(saved);
                        break;
                    }
                    let audio = header.audio.then(|| self.pcm_output()).transpose()?;
                    imp.audio.replace(audio);
                    imp.header.set(Some(header));
                    self.invalidate_size();
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
                        // Blocks belong to successive audio ticks, running ahead of the frame.
                        for block in std::mem::take(&mut frame.samples).chunks(media::AUDIO_BYTES) {
                            let chunk = imp.audio_chunks.get();
                            let tick = header.start_tick.saturating_add(chunk);
                            let samples_left =
                                header.duration_us.saturating_sub(media::timestamp(tick))
                                    * media::SAMPLE_RATE
                                    / 1_000_000;
                            let block =
                                &block[..samples_left.min(block.len() as u64 / 4) as usize * 4];
                            if !block.is_empty() {
                                if let Some(history) = imp.history.borrow_mut().as_mut() {
                                    history.push(block);
                                }
                                audio.push(block.to_vec(), media::timestamp(chunk))?;
                            }
                            imp.audio_chunks.set(chunk + 1);
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
                            imp.clock.set(self.wall_clock_allowed().then(Instant::now));
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
        // Trim behind the sink itself, not the extrapolated playhead: a stalled
        // sink resumes from its last position and still needs those samples.
        if let Some(history) = imp.history.borrow_mut().as_mut() {
            history.trim_behind(imp.clock_base.get() * media::SAMPLE_RATE / 1_000_000);
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
            if imp
                .edge_sampled
                .get()
                .is_none_or(|sampled| sampled.elapsed() >= EDGE_SAMPLE_INTERVAL)
            {
                imp.edge_sampled.set(Some(Instant::now()));
                imp.edge_grid
                    .set(ambient::sample(&frame.pixels, header.width, header.height));
            }
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

// Samples kept behind the playhead; everything decoded ahead of it is kept too,
// however far the sink's buffer and device delay push that lead.
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
    }

    fn trim_behind(&mut self, played: u64) {
        let keep_from = played.min(self.end).saturating_sub(HISTORY_SAMPLES as u64);
        let start = self.end - self.samples.len() as u64;
        if keep_from > start {
            self.samples.drain(..(keep_from - start) as usize);
        }
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
