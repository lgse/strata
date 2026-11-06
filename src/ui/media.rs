// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib, prelude::*, subclass::prelude::*};

use crate::{
    media::{self, Frame, Header},
    sandbox::media::Session,
    services::{MediaPreviewSize, SandboxedMedia},
};

pub(crate) mod ambient;
mod audio;
#[cfg(debug_assertions)]
mod diagnostics;
mod history;
mod playback;
mod positions;
#[cfg(test)]
mod tests;
use audio::PcmOutput;
use history::PcmHistory;
use positions::recall_media_position;
pub(crate) use positions::resume_position;
use positions::{RESTORE_MIN_US, forget_media_position, remember_media_position};

const PAUSED_IDLE: Duration = Duration::from_secs(30);
// A selection replaced within the dwell never spawns a decoder.
pub(crate) const START_DWELL: Duration = Duration::from_millis(50);
const RESIZE_DELAY: Duration = Duration::from_millis(250);
// A pane that grows less than this shows a slight upscale instead of a restart.
const RESIZE_GROWTH: f64 = 0.08;
const SEEK_DELAY: Duration = Duration::from_millis(200);
const PRESENTATION_QUEUE: usize = 3;
const AUDIO_LEAD_CAP_US: u64 = 2_000_000;
const AUDIO_STUCK_TIMEOUT: Duration = Duration::from_secs(3);
const EDGE_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(test)]
thread_local! {
    static TEST_STREAMS: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn use_test_streams(enabled: bool) {
    TEST_STREAMS.with(|streams| streams.set(enabled));
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
        pub(super) seek_origin: Cell<Option<u64>>,
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
        pub(super) start_percent: Cell<u32>,
        pub(super) start_requested: Cell<bool>,
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
                if let Some(audio) = self.audio.borrow().as_ref() {
                    audio.play();
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
            if let Some(audio) = self.audio.borrow().as_ref() {
                audio.pause();
            }
        }

        fn seek(&self, timestamp: i64) {
            let position = timestamp.max(0) as u64;
            let obj = self.obj();
            if self.first_frame.get() && self.seek_origin.get().is_none() {
                obj.capture_position();
                self.seek_origin.set(Some(self.position.get()));
            }
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
        let restore = resume_position(&source.path);
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

    /// A video not previewed earlier this session opens this far into its
    /// duration. One already previewed resumes, at 0:00 when it was left in its
    /// first second or at its end.
    pub(crate) fn set_start_percent(&self, percent: u32) {
        self.imp().start_requested.set(percent > 0);
        let previewed = self
            .imp()
            .source
            .borrow()
            .as_ref()
            .is_some_and(|source| recall_media_position(&source.path).is_some());
        let percent = if previewed { 0 } else { percent };
        self.imp()
            .start_percent
            .set(percent.min(crate::ui::preferences::MAX_PREVIEW_VIDEO_START));
    }

    pub(crate) fn source(&self) -> Option<SandboxedMedia> {
        self.imp().source.borrow().clone()
    }

    pub(crate) fn video_size(&self) -> Option<(u32, u32)> {
        self.imp()
            .header
            .get()
            .filter(|header| header.width > 0)
            .map(|header| (header.width, header.height))
    }

    pub(crate) fn has_frame(&self) -> bool {
        self.imp().texture.borrow().is_some()
    }

    pub(crate) fn edge_grid(&self) -> Option<ambient::EdgeGrid> {
        self.imp().edge_grid.get()
    }

    /// Gain survives sink restarts without changing the saved volume.
    pub(crate) fn set_fade(&self, fade: f64) {
        let fade = fade.clamp(0.0, 1.0);
        self.imp().fade.set(Some(fade));
        if let Some(audio) = self.imp().audio.borrow().as_ref() {
            audio.set_fade(fade);
        }
    }

    #[cfg(test)]
    pub(crate) fn fade(&self) -> f64 {
        self.imp().fade.get().unwrap_or(1.0)
    }

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
    pub(crate) fn use_test_audio_stream(&self) {
        self.use_test_audio_stream_lasting(60_000_000);
    }

    #[cfg(test)]
    pub(crate) fn use_test_audio_stream_lasting(&self, duration_us: u64) {
        self.imp()
            .loader
            .replace(Some(std::rc::Rc::new(move |_, tick| {
                crate::sandbox::media::tests::stream(Header {
                    width: 0,
                    height: 0,
                    audio: true,
                    duration_us,
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
            obj.timer_tick()
        });
        self.imp().timer.replace(Some(timer));
    }

    fn timer_tick(&self) -> glib::ControlFlow {
        if !self.imp().closed.get()
            && let Err(error) = self.tick()
        {
            self.fail(&error);
        }
        if self.timer_done() {
            self.imp().timer.borrow_mut().take();
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    }

    fn timer_done(&self) -> bool {
        let imp = self.imp();
        imp.closed.get() || imp.dormant.get() || self.error().is_some()
    }

    pub fn close(&self) {
        let imp = self.imp();
        if imp.closed.replace(true) {
            return;
        }
        if let Some(source) = imp.source.borrow().as_ref() {
            if self.resumes_later() {
                remember_media_position(source.path.clone(), imp.position.get());
            } else if imp.start_requested.get()
                && imp.header.get().is_some_and(|header| header.width > 0)
            {
                // A video closed in its first second or at its end reopens at
                // 0:00, not at the configured start: remember it as previewed.
                remember_media_position(source.path.clone(), 0);
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

    /// Video stopped partway through comes back at the same position.
    fn resumes_later(&self) -> bool {
        let imp = self.imp();
        let position = imp.position.get();
        let duration = self.duration().max(0) as u64;
        let has_video = imp.header.get().is_some_and(|header| header.width > 0);
        has_video && position > RESTORE_MIN_US && (duration == 0 || position < duration)
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
}
