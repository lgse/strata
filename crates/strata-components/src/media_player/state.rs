//! The player's state: what the host gave it, what the clock said last, where the pointer is,
//! and the live values that move without a view commit.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use rune::{Live, Pixels, Tx};

use super::audio::layout::Plan;
use super::audio::spectrum::Spectrum;
use super::clock::{PlaybackClock, PlaybackState};
use super::data::{Art, AudioTags, Chapter, Clip, Details, MediaError, MediaKind};
use super::intent::MediaIntent;
use super::keys::{MediaCommand, MediaKeys, Seek};
use super::labels::Labels;
use super::storyboard::Storyboard;
use super::time::{self, VOLUME_STEP};
use super::timeline::levels::Levels;
use crate::viewers::viewport::{Mode, ViewportState};

/// The stand-in fades out over this long once the first frame shows (`frame.rs`, `FADE`).
pub const STAND_IN_FADE: f64 = 0.15;
/// Grow-in frames commit at most this often, in seconds (OWNER ruling: 30 Hz).
pub const GROW_INTERVAL: f64 = 1.0 / 30.0;
/// A seek counts as reported once the clock is this close to its target (a clock that plays
/// on from the target moves a frame or two before the next tick).
const SEEK_REACHED: Duration = Duration::from_millis(250);
/// A seek the clock never reports (a host that dropped it) stops holding the timeline after
/// this long, in seconds on the host's clock.
const SEEK_GIVE_UP: f64 = 1.0;
/// The tick a playing player asks for.
const FRAME: Duration = Duration::from_millis(16);

/// Where the stand-in stands.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StandIn {
    /// No frame yet: the poster (or a plain surface) shows.
    Shown,
    /// The first frame showed at this time: the stand-in fades.
    Fading(f64),
    Gone,
}

/// The live values: positions the view binds and never reads.
#[derive(Clone)]
pub(crate) struct Lives {
    /// The clip over the played copy of the waveform (or the plain bar's fill).
    pub(crate) played: Live<[f32; 4]>,
    /// The clip over the hover-tinted copy.
    pub(crate) hovered: Live<[f32; 4]>,
    /// The playhead (or the plain bar's knob).
    pub(crate) head: Live<[f32; 2]>,
    /// The pointer's line, and whether it shows.
    pub(crate) pointer: Live<[f32; 2]>,
    pub(crate) pointer_shown: Live<f32>,
    /// The hover bubble's offset along the frame.
    pub(crate) bubble: Live<[f32; 2]>,
    /// The stand-in's opacity.
    pub(crate) stand_in: Live<f32>,
}

impl Default for Lives {
    fn default() -> Self {
        Lives {
            played: Live::new([0.0; 4]),
            hovered: Live::new([0.0; 4]),
            head: Live::new([0.0; 2]),
            pointer: Live::new([0.0; 2]),
            pointer_shown: Live::new(0.0),
            bubble: Live::new([0.0; 2]),
            stand_in: Live::new(1.0),
        }
    }
}

/// The bubble's nearest cell (its index and pixels), its time and its chapter.
pub(crate) type Bubble<'a> = (Option<(u32, &'a Pixels)>, Duration, Option<&'a Chapter>);

/// What the built view shows that the state can move: a change commits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    elapsed: u64,
    target: bool,
    value: u64,
    total: Option<u64>,
    playback: PlaybackState,
    seekable: bool,
    stand_in: bool,
    cover: Option<u32>,
    bubble: Option<(Option<u32>, u64, Option<usize>)>,
    levels: u64,
    bars: usize,
    spectrum: usize,
    plan: Plan,
    volume: (u32, bool),
}

/// The player's state, held by the host and reached by accessor.
///
/// The host sets what it knows ([`MediaState::set_clip`] and the setters), calls
/// [`MediaState::tick`] from its poll every frame with its [`PlaybackClock`], and applies the
/// [`MediaIntent`]s the player sends (through `MediaPlayer::on_intent` from the view's
/// handlers, or [`MediaState::drain_intents`] after a tick or a [`MediaState::command`]).
pub struct MediaState {
    clip: Clip,
    /// The video surface's zoom state (C14a): fitted, never zoomed by the user.
    pub viewport: ViewportState,
    labels: Labels,
    keys: MediaKeys,

    position: Duration,
    duration: Option<Duration>,
    playback: PlaybackState,
    seekable: bool,
    /// A seek sent but not yet reported by the clock, and the host time it was first seen
    /// pending: the timeline holds its target until the clock reports it.
    pending_seek: Option<(Duration, Option<f64>)>,

    volume: f32,
    muted: bool,
    /// The volume and mute a volume drag started from, to put back if it's cancelled.
    volume_drag: Option<(f32, bool)>,

    error: Option<MediaError>,
    chapters: Vec<Chapter>,
    pub(crate) levels: Levels,
    levels_shown: u64,
    grow_at: f64,
    storyboard: Option<Storyboard>,
    details: Details,
    poster: Option<Pixels>,
    aspect: Option<f32>,
    frame: Option<[u32; 2]>,
    art: Option<Art>,
    tags: AudioTags,
    pub(crate) spectrum: Spectrum,

    hover: Option<f32>,
    drag: Option<f32>,
    timeline_w: f32,
    bars: usize,
    frame_box: [f32; 2],
    bubble_w: Cell<f32>,
    plan: Plan,

    pub(crate) live: Lives,

    stand_in: StandIn,
    asked_storyboard: bool,
    decode_loaded: Option<[u32; 2]>,
    last_tick: Option<f64>,
    wheel: f32,

    outbox: Vec<MediaIntent>,
    built: RefCell<Option<Shown>>,
    pub(crate) motion: Cell<bool>,
    pub(crate) bar_pitch: Cell<f32>,
    pub(crate) wave_h: Cell<f32>,
}

impl Default for MediaState {
    fn default() -> Self {
        MediaState::new(Clip::default())
    }
}

impl MediaState {
    /// A player for `clip`, at its start, paused, at full volume.
    pub fn new(clip: Clip) -> MediaState {
        MediaState {
            clip,
            viewport: ViewportState::new(Mode::Fit),
            labels: Labels::default(),
            keys: MediaKeys::default_map(),
            position: Duration::ZERO,
            duration: None,
            playback: PlaybackState::Loading,
            seekable: false,
            pending_seek: None,
            volume: 1.0,
            muted: false,
            volume_drag: None,
            error: None,
            chapters: Vec::new(),
            levels: Levels::default(),
            levels_shown: 0,
            grow_at: f64::NEG_INFINITY,
            storyboard: None,
            details: Details::Loading,
            poster: None,
            aspect: None,
            frame: None,
            art: None,
            tags: AudioTags::default(),
            spectrum: Spectrum::default(),
            hover: None,
            drag: None,
            timeline_w: 0.0,
            bars: 0,
            frame_box: [0.0, 0.0],
            bubble_w: Cell::new(0.0),
            plan: Plan::default(),
            live: Lives::default(),
            stand_in: StandIn::Shown,
            asked_storyboard: false,
            decode_loaded: None,
            last_tick: None,
            wheel: 0.0,
            outbox: Vec::new(),
            built: RefCell::new(None),
            motion: Cell::new(true),
            bar_pitch: Cell::new(3.0),
            wave_h: Cell::new(40.0),
        }
    }

    // What the host sets.

    /// Shows another file: everything that belonged to the last one goes (its levels lower
    /// first), except the volume, the keys, the labels and the art, which the host replaces
    /// when it differs (so a track with the same art keeps it in place).
    pub fn set_clip(&mut self, clip: Clip) {
        self.clip = clip;
        self.position = Duration::ZERO;
        self.duration = None;
        self.playback = PlaybackState::Loading;
        self.seekable = false;
        self.pending_seek = None;
        self.error = None;
        self.chapters.clear();
        self.levels.clear();
        self.storyboard = None;
        self.details = Details::Loading;
        self.poster = None;
        self.aspect = None;
        self.frame = None;
        self.viewport.set_content(None);
        self.tags = AudioTags::default();
        self.hover = None;
        self.drag = None;
        self.stand_in = StandIn::Shown;
        self.live.stand_in.set(1.0);
        self.asked_storyboard = false;
        self.decode_loaded = None;
        self.wheel = 0.0;
        self.place();
    }

    /// Updates the listing facts (position, previous and next) without resetting playback.
    pub fn set_listing(
        &mut self,
        position: Option<crate::viewers::layout::Position>,
        has_previous: bool,
        has_next: bool,
    ) {
        self.clip.position = position;
        self.clip.has_previous = has_previous;
        self.clip.has_next = has_next;
    }

    pub fn clip(&self) -> &Clip {
        &self.clip
    }

    /// The saved volume and mute, as the host's preference holds them.
    pub fn set_volume(&mut self, volume: f32, muted: bool) {
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = muted;
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn set_error(&mut self, error: Option<MediaError>) {
        self.error = error;
    }

    pub fn error(&self) -> Option<&MediaError> {
        self.error.as_ref()
    }

    pub fn set_chapters(&mut self, chapters: Vec<Chapter>) {
        self.chapters = chapters;
    }

    pub fn chapters(&self) -> &[Chapter] {
        &self.chapters
    }

    /// Waveform levels from bucket `start`, RMS from 0 to 1 (Strata: `rms(level)`), out of
    /// `buckets` (Strata's 1,024). They grow in.
    pub fn set_levels(&mut self, buckets: usize, start: usize, levels: &[f32]) {
        self.levels.set(buckets, start, levels);
    }

    /// The levels land at once, without growing in: a fixture's starting point, or a host
    /// showing a waveform it had cached.
    pub fn settle_levels(&mut self) {
        self.levels.step(0.0, false);
        self.levels_shown = self.levels.generation();
    }

    /// The levels lower to the plain line.
    pub fn clear_levels(&mut self) {
        self.levels.clear();
    }

    pub fn set_storyboard(&mut self, board: Option<Storyboard>) {
        self.storyboard = board;
    }

    /// The board, to hand cells over as they land.
    pub fn storyboard_mut(&mut self) -> Option<&mut Storyboard> {
        self.storyboard.as_mut()
    }

    pub fn storyboard(&self) -> Option<&Storyboard> {
        self.storyboard.as_ref()
    }

    pub fn set_details(&mut self, details: Details) {
        self.details = details;
    }

    pub fn details(&self) -> &Details {
        &self.details
    }

    /// What stands in before the first frame: the listing's cached thumbnail (dimmed), or,
    /// with none, a plain surface at `aspect` (width over height, from the probe).
    pub fn set_poster(&mut self, poster: Option<Pixels>, aspect: Option<f32>) {
        self.poster = poster;
        self.aspect = aspect;
    }

    /// The decoder's frames are `size` pixels: the first frame shows (the stand-in fades),
    /// or the size changed. `None` before any frame.
    pub fn set_frame(&mut self, size: Option<[u32; 2]>) {
        if self.frame == size {
            return;
        }
        self.frame = size;
        self.viewport
            .set_content(size.map(|[w, h]| [w as f32, h as f32]));
        if let Some(s) = size {
            self.aspect = Some(s[0] as f32 / s[1].max(1) as f32);
        }
    }

    pub fn has_frame(&self) -> bool {
        self.frame.is_some()
    }

    pub fn set_art(&mut self, art: Option<Art>) {
        self.art = art;
    }

    pub fn set_tags(&mut self, tags: AudioTags) {
        self.tags = tags;
    }

    pub fn tags(&self) -> &AudioTags {
        &self.tags
    }

    /// Band levels heard since the last tick, for the spectrum.
    pub fn feed_bands(&mut self, bands: &[f32]) {
        self.spectrum.feed(bands);
    }

    pub fn spectrum(&self) -> &Spectrum {
        &self.spectrum
    }

    pub fn set_keys(&mut self, keys: MediaKeys) {
        self.keys = keys;
    }

    pub fn keys(&self) -> &MediaKeys {
        &self.keys
    }

    pub fn set_labels(&mut self, labels: Labels) {
        self.labels = labels;
    }

    pub fn labels(&self) -> &Labels {
        &self.labels
    }

    /// The decode size to ask for now, in device pixels, from the frame's laid-out size:
    /// `None` before the first layout. The player then asks again only when the frame grows
    /// enough ([`MediaIntent::DecodeSize`]).
    pub fn decode_size(&mut self) -> Option<[u32; 2]> {
        let frame = self.viewport.frame()?;
        let wanted = time::decode_request(frame, self.viewport.scale());
        self.decode_loaded = Some(wanted);
        Some(wanted)
    }

    // What the clock says.

    pub fn position(&self) -> Duration {
        self.position
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn playback(&self) -> PlaybackState {
        self.playback
    }

    pub fn playing(&self) -> bool {
        self.playback == PlaybackState::Playing
    }

    /// Whether a seek can run now.
    pub fn seekable(&self) -> bool {
        self.seekable && self.duration.is_some_and(|d| !d.is_zero())
    }

    /// The time under the pointer (or the drag), if any.
    pub fn pointer_time(&self) -> Option<Duration> {
        let f = self.drag.or(self.hover)?;
        let d = self.duration?;
        Some(d.mul_f64(f64::from(f)))
    }

    /// One frame: reads the clock, moves the live values, runs the timers, and returns true
    /// when the view should commit (what it shows changed). While anything moves it asks for
    /// the next frame. `now` is the host's clock in seconds.
    pub fn tick(&mut self, clock: &dyn PlaybackClock, now: f64, tx: &mut Tx) -> bool {
        let elapsed = self.last_tick.map_or(1.0 / 60.0, |t| (now - t).max(0.0)) as f32;
        self.last_tick = Some(now);
        self.read(clock, now);
        let motion = self.motion.get();

        if self.frame.is_some() {
            if self.stand_in == StandIn::Shown {
                self.stand_in = if motion {
                    StandIn::Fading(now)
                } else {
                    StandIn::Gone
                };
                if self.clip.kind == MediaKind::Video && !self.asked_storyboard {
                    self.asked_storyboard = true;
                    self.outbox.push(MediaIntent::StoryboardWanted);
                }
            }
            if let StandIn::Fading(since) = self.stand_in {
                let t = (now - since) / STAND_IN_FADE;
                if t >= 1.0 || !motion {
                    self.stand_in = StandIn::Gone;
                }
                self.live.stand_in.set((1.0 - t).clamp(0.0, 1.0) as f32);
            }
        }

        self.levels.step(elapsed, motion);
        let generation = self.levels.generation();
        if generation != self.levels_shown
            && (now - self.grow_at >= GROW_INTERVAL || !self.levels.moving())
        {
            self.levels_shown = generation;
            self.grow_at = now;
        }

        let spectrum = self.clip.kind == MediaKind::Audio && self.spectrum.tick(elapsed);
        self.check_decode();
        self.place();

        let fading = matches!(self.stand_in, StandIn::Fading(_));
        if self.playing() || spectrum || self.levels.moving() || fading {
            tx.wake_after(FRAME);
        }
        self.stale()
    }

    /// Reads the clock. A seek the host hasn't applied yet (its poll drains intents after the
    /// tick) keeps its target on the timeline until the clock reports it, so no frame shows
    /// the old position.
    fn read(&mut self, clock: &dyn PlaybackClock, now: f64) {
        let position = clock.position();
        self.duration = clock.duration();
        self.playback = clock.state();
        self.seekable = clock.seekable();
        self.position = match &mut self.pending_seek {
            Some((target, since)) => {
                let since = *since.get_or_insert(now);
                let reached = position >= *target && position - *target <= SEEK_REACHED;
                if reached || now - since > SEEK_GIVE_UP {
                    self.pending_seek = None;
                    position
                } else {
                    *target
                }
            }
            None => position,
        };
    }

    fn check_decode(&mut self) {
        if self.clip.kind == MediaKind::Audio {
            return;
        }
        let (Some(decoded), Some(frame)) = (self.frame, self.viewport.frame()) else {
            return;
        };
        let wanted = time::decode_request(frame, self.viewport.scale());
        let loaded = *self.decode_loaded.get_or_insert(decoded);
        if time::needs_restart(decoded, loaded, wanted) {
            self.decode_loaded = Some(wanted);
            self.outbox.push(MediaIntent::DecodeSize(wanted));
        }
    }

    pub(crate) fn outbox_is_empty(&self) -> bool {
        self.outbox.is_empty()
    }

    /// What the player asked since the last drain, oldest first.
    pub fn drain_intents(&mut self) -> Vec<MediaIntent> {
        std::mem::take(&mut self.outbox)
    }

    // Commands.

    /// Runs a media key's command (from [`MediaKeys::window_command`] or the player's own
    /// keys). False when it does nothing for this kind of clip (Return on audio): the key
    /// goes on.
    pub fn command(&mut self, command: MediaCommand) -> bool {
        match command {
            MediaCommand::PlayPause => self.toggle(),
            MediaCommand::Seek(seek) => self.seek_by(seek),
            MediaCommand::VolumeUp => self.step_volume(1.0),
            MediaCommand::VolumeDown => self.step_volume(-1.0),
            MediaCommand::Mute => self.toggle_mute(),
            MediaCommand::Previous => self.step(-1),
            MediaCommand::Next => self.step(1),
            MediaCommand::HandOff => return self.hand_off(),
        }
        true
    }

    /// Play, or pause. Off while an error shows.
    pub fn toggle(&mut self) {
        if self.error.is_some() {
            return;
        }
        let intent = if self.playing() {
            MediaIntent::Pause
        } else {
            MediaIntent::Play
        };
        self.outbox.push(intent);
    }

    /// A seek by key or step: clamped to the start and the last frame, and only when the
    /// clip can seek.
    pub fn seek_by(&mut self, seek: Seek) {
        let Some(d) = self.duration else { return };
        let to = match seek {
            Seek::Back(by) => self.position.saturating_sub(by),
            Seek::Forward(by) => self.position + by,
            Seek::Start => Duration::ZERO,
            Seek::End => time::last_frame(d),
        };
        self.seek_to(to);
    }

    /// A seek to `to`, clamped. The timeline shows the target at once.
    pub fn seek_to(&mut self, to: Duration) {
        if !self.seekable() {
            return;
        }
        let d = self.duration.unwrap_or_default();
        let to = to.min(time::last_frame(d));
        self.position = to;
        self.pending_seek = Some((to, None));
        self.outbox.push(MediaIntent::Seek(to));
        self.place();
    }

    /// The volume `steps` grid steps on (0.1 each), from 0 while muted; reaching 0 mutes.
    pub fn step_volume(&mut self, steps: f32) {
        let from = if self.muted { 0.0 } else { self.volume };
        self.set_user_volume(time::snapped_volume(from + steps * VOLUME_STEP));
    }

    /// A volume the person set (a step, a drag, Home, End). 0 mutes and keeps the volume
    /// before it, which unmuting restores (`step_preview_volume`).
    pub fn set_user_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.muted = volume == 0.0;
        if !self.muted || self.volume == 0.0 {
            self.volume = volume;
        }
        self.outbox.push(MediaIntent::SetVolume {
            volume: self.volume,
            muted: self.muted,
        });
    }

    /// A volume drag moved to `volume`: the first move keeps where it started from.
    pub(crate) fn drag_volume(&mut self, volume: f32) {
        self.volume_drag.get_or_insert((self.volume, self.muted));
        self.set_user_volume(volume);
    }

    /// A volume drag ended. A cancelled one (Escape, the window losing focus) puts the volume
    /// it started from back.
    pub(crate) fn end_volume_drag(&mut self, cancelled: bool) {
        if let Some((volume, muted)) = self.volume_drag.take()
            && cancelled
        {
            self.volume = volume;
            self.muted = muted;
            self.outbox.push(MediaIntent::SetVolume { volume, muted });
        }
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.outbox.push(MediaIntent::SetVolume {
            volume: self.volume,
            muted: self.muted,
        });
    }

    fn step(&mut self, by: i8) {
        let enabled = if by < 0 {
            self.clip.has_previous
        } else {
            self.clip.has_next
        };
        if enabled {
            self.outbox.push(MediaIntent::Step(by));
        }
    }

    /// Pauses and asks for the default player at the position (video only). Strata's Open
    /// button runs this too.
    pub fn hand_off(&mut self) -> bool {
        if self.clip.kind != MediaKind::Video {
            return false;
        }
        if self.playing() {
            self.outbox.push(MediaIntent::Pause);
        }
        self.outbox.push(MediaIntent::HandOff {
            at: time::hand_off_at(self.position, self.duration),
        });
        true
    }

    // The pointer.

    /// The pointer moved over the timeline at `x` points from its start, or left it
    /// (`None`). True when what the view shows changed (commit), else only live values moved.
    pub fn hover_at(&mut self, x: Option<f32>) -> bool {
        let f = x.map(|x| self.fraction(x));
        if self.hover == f {
            return false;
        }
        self.hover = f;
        self.place();
        self.stale()
    }

    /// A drag on the timeline: `Some(x)` as it presses and moves, `None` when it ends;
    /// `seek` says whether it ends with a seek (released) or not (cancelled). A clip that
    /// can't seek takes no drag.
    pub fn drag_at(&mut self, x: Option<f32>, seek: bool) -> bool {
        match x {
            Some(x) if self.seekable() => {
                self.drag = Some(self.fraction(x));
            }
            Some(_) => return false,
            None => {
                if let (Some(f), true) = (self.drag.take(), seek)
                    && let Some(d) = self.duration
                {
                    self.seek_to(d.mul_f64(f64::from(f)));
                }
            }
        }
        self.place();
        self.stale()
    }

    /// Puts the pointer over the timeline at `fraction` of its width, as a hover would: a
    /// fixture's starting point (a golden of the bubble).
    pub fn preset_hover(&mut self, fraction: f32) {
        self.hover = Some(fraction.clamp(0.0, 1.0));
        self.place();
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// A wheel over the plain bar's seek slider: `notches` (a mouse) or `precise` points (a
    /// touchpad, 40 points a step). Seeks at once, a step of 5 s each; moves under one tick
    /// are ignored, as Strata ignores touchpad jumps within the current frame.
    pub fn wheel(&mut self, dy: f32, precise: bool) -> bool {
        if !self.seekable() {
            return false;
        }
        let steps = if precise {
            self.wheel += dy / 40.0;
            std::mem::take(&mut self.wheel)
        } else {
            dy.signum()
        };
        let by = time::SEEK_STEP.mul_f32(steps.abs());
        if by < time::tick() {
            self.wheel = steps;
            return false;
        }
        let seek = if steps > 0.0 {
            Seek::Forward(by)
        } else {
            Seek::Back(by)
        };
        self.seek_by(seek);
        true
    }

    fn fraction(&self, x: f32) -> f32 {
        if self.timeline_w <= 0.0 {
            0.0
        } else {
            (x / self.timeline_w).clamp(0.0, 1.0)
        }
    }

    /// The timeline's laid-out width. True when the bar count changed (the view rebuilds).
    pub(crate) fn set_timeline_width(&mut self, w: f32) -> bool {
        self.timeline_w = w;
        let pitch = self.bar_pitch.get().max(1.0);
        let bars = (((w + 1.0) / pitch).floor().max(1.0)) as usize;
        let changed = self.bars != bars;
        self.bars = bars;
        self.place();
        changed
    }

    pub(crate) fn timeline_width(&self) -> f32 {
        self.timeline_w
    }

    pub(crate) fn bars(&self) -> usize {
        self.bars
    }

    /// The frame's laid-out size, for the bubble.
    pub(crate) fn set_frame_box(&mut self, size: [f32; 2]) -> bool {
        let changed = self.frame_box != size;
        self.frame_box = size;
        self.place();
        changed
    }

    pub(crate) fn set_bubble_width(&self, w: f32) {
        self.bubble_w.set(w);
    }

    pub(crate) fn set_audio_box(&mut self, plan: Plan) -> bool {
        let changed = self.plan != plan;
        self.plan = plan;
        changed
    }

    pub(crate) fn plan(&self) -> Plan {
        self.plan
    }

    pub(crate) fn was_wide(&self) -> bool {
        self.plan.wide
    }

    /// Writes the live values from the state.
    pub(crate) fn place(&self) {
        let w = self.timeline_w;
        let played = self.drag.unwrap_or_else(|| self.played_fraction());
        let px = played * w;
        let h = self.wave_h.get();
        self.live.played.set([0.0, 0.0, px, h]);
        let pointer = self.drag.or(self.hover);
        let hx = pointer.map(|f| f * w);
        self.live.hovered.set([0.0, 0.0, hx.unwrap_or(0.0), h]);
        let head = px.clamp(1.0, (w - 1.0).max(1.0));
        self.live.head.set([head - 1.0, 0.0]);
        match hx {
            Some(x) if (x - head).abs() > 2.0 => {
                self.live
                    .pointer
                    .set([x.clamp(0.0, (w - 1.0).max(0.0)), 0.0]);
                self.live.pointer_shown.set(1.0);
            }
            _ => {
                self.live.pointer_shown.set(0.0);
            }
        }
        if let Some(f) = pointer {
            let fw = self.frame_box[0];
            let bw = self.bubble_w.get();
            let x = (f * fw - bw / 2.0).clamp(0.0, (fw - bw).max(0.0));
            self.live.bubble.set([x, 0.0]);
        }
    }

    fn played_fraction(&self) -> f32 {
        match self.duration {
            Some(d) if !d.is_zero() => {
                (self.position.as_secs_f64() / d.as_secs_f64()).clamp(0.0, 1.0) as f32
            }
            _ => 0.0,
        }
    }

    // What the view reads.

    pub(crate) fn poster(&self) -> Option<&Pixels> {
        self.poster.as_ref()
    }

    pub(crate) fn aspect(&self) -> Option<f32> {
        self.aspect
    }

    pub(crate) fn art(&self) -> Option<&Art> {
        self.art.as_ref()
    }

    pub(crate) fn stand_in_shown(&self) -> bool {
        self.stand_in != StandIn::Gone
    }

    /// The storyboard cell that covers the frame while a seek runs.
    pub(crate) fn cover(&self) -> Option<(u32, &Pixels)> {
        if self.playback != PlaybackState::Seeking || self.frame.is_none() {
            return None;
        }
        self.storyboard.as_ref()?.nearest(self.position)
    }

    /// The bubble's cell, time and chapter, while the pointer is over the timeline.
    pub(crate) fn bubble(&self) -> Option<Bubble<'_>> {
        if self.clip.kind != MediaKind::Video {
            return None;
        }
        let t = self.pointer_time()?;
        let cell = self.storyboard.as_ref().and_then(|b| b.nearest(t));
        Some((cell, t, self.chapter_at(t)))
    }

    /// The chapter `t` falls in.
    pub fn chapter_at(&self, t: Duration) -> Option<&Chapter> {
        self.chapters.iter().rev().find(|c| c.start <= t)
    }

    /// The elapsed label's time: the pointer's while it is over the timeline.
    pub(crate) fn shown_time(&self) -> Duration {
        self.pointer_time().unwrap_or(self.position)
    }

    /// The timeline's spoken value: "1:05 of 3:20".
    pub(crate) fn value_text(&self) -> String {
        let total = self
            .duration
            .map_or_else(|| self.labels.unknown_total.clone(), time::clock);
        self.labels
            .value
            .replace("{elapsed}", &time::clock(self.position))
            .replace("{duration}", &total)
    }

    fn shown(&self) -> Shown {
        Shown {
            elapsed: self.shown_time().as_secs(),
            target: self.pointer_time().is_some(),
            value: self.position.as_secs(),
            total: self.duration.map(|d| d.as_secs()),
            playback: self.playback,
            seekable: self.seekable(),
            stand_in: self.stand_in_shown(),
            cover: self.cover().map(|(i, _)| i),
            bubble: self.bubble().map(|(c, t, ch)| {
                (
                    c.map(|(i, _)| i),
                    t.as_secs(),
                    ch.map(|c| c.start.as_millis() as usize),
                )
            }),
            levels: self.levels_shown,
            bars: self.bars,
            spectrum: self.spectrum.count(),
            plan: self.plan,
            volume: ((self.volume * 1000.0).round() as u32, self.muted),
        }
    }

    /// The view built what the state shows now: `MediaPlayer::build` calls it.
    pub(crate) fn note_built(&self) {
        *self.built.borrow_mut() = Some(self.shown());
    }

    /// What the view shows is behind the state.
    pub(crate) fn stale(&self) -> bool {
        self.built.borrow().as_ref() != Some(&self.shown())
    }

    /// Commits only when what the view shows changed; else says `live_only`.
    pub(crate) fn finish(&self, tx: &mut Tx) {
        if !self.stale() {
            tx.live_only();
        }
    }
}
