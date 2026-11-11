//! The gallery page for C13 MediaPlayer: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c13-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C13-media-player.md`.
//!
//! The player sits beside a fake listing, on a [`FakeClock`] that plays on the page's virtual
//! clock. Everything the host would bring is generated here and handed over by `poll` when it
//! comes due, as a fake worker: the video frames (a pacer stand-in writing a `Pixels` in
//! place), the waveform levels in chunks, the storyboard cells in Strata's subdivision order,
//! the probe's details, and the spectrum's bands. Goldens and scripts see the same results
//! every run.
//!
//! The page plays the host: it applies each intent (play, pause and seeks to the clock, the
//! volume to its "preference", steps to its fake listing), and says what a host would do with
//! the rest on its status line (`c13-status`): the hand-off, decode sizes. The window's media
//! keys (Ctrl+Alt+Space and the rest) go through the page's `on_key` to
//! [`MediaKeys::window_command`], as Strata's window key handler would.

use std::collections::VecDeque;
use std::time::Duration;

use rune::{Chord, ChordKey, Cx, El, KeyEvent, Pixels, Tx, col, row, text};
use strata_components::media_player::{
    Art, AudioTags, Chapter, Clip, Details, FakeClock, KeyScope, MediaCommand, MediaError,
    MediaIntent, MediaKeys, MediaKind, MediaPlayer, MediaState, PlaybackClock, Seek, Sheet,
    Storyboard, badges::VideoDetails, time,
};
use strata_components::viewers::layout::Position;

use crate::{Gallery, Scene};

/// The waveform's bucket count: Strata's `peaks::BUCKETS`.
pub const BUCKETS: usize = 1024;
/// The fake frames' size in pixels.
pub const FRAME: [u32; 2] = [320, 180];
/// The perf scene's frame size.
pub const PERF_FRAME: [u32; 2] = [1280, 720];
/// A storyboard cell's size in pixels (Strata's 128-edge cells of a 16:9 clip).
const CELL: [u32; 2] = [128, 72];

/// A fake worker's result, due at a time on the page's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Job {
    /// Waveform levels from this bucket, 128 of them.
    Levels(usize),
    /// The storyboard's next cell, in subdivision order.
    Cell,
    /// The probe answers.
    Details,
    /// The first frame lands.
    FirstFrame,
}

/// A fake file of the listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct File {
    pub name: &'static str,
    pub kind: MediaKind,
    pub seconds: u64,
}

pub const VIDEOS: [File; 3] = [
    File {
        name: "harbour.mp4",
        kind: MediaKind::Video,
        seconds: 200,
    },
    File {
        name: "ferry.mkv",
        kind: MediaKind::Video,
        seconds: 95,
    },
    File {
        name: "gulls.webm",
        kind: MediaKind::Video,
        seconds: 42,
    },
];

pub const SONGS: [File; 3] = [
    File {
        name: "01 Low Tide.flac",
        kind: MediaKind::Audio,
        seconds: 245,
    },
    File {
        name: "02 Breakwater.flac",
        kind: MediaKind::Audio,
        seconds: 188,
    },
    File {
        name: "03 Lantern.ogg",
        kind: MediaKind::Audio,
        seconds: 301,
    },
];

pub const ANIMATION: File = File {
    name: "waves.gif",
    kind: MediaKind::Plain,
    seconds: 75,
};

/// C13's demo state, held in `Gallery::c13`.
pub struct Demo {
    pub media: MediaState,
    pub clock: FakeClock,
    pub frames: Pixels,
    pub frame_px: [u32; 2],
    pub now: f64,
    pub started: bool,
    /// Which list the listing shows, and which file is open.
    pub files: &'static [File],
    pub file: usize,
    pub jobs: VecDeque<(f64, Job)>,
    /// The storyboard cells still to come, in the helper's order.
    pub cells: VecDeque<u32>,
    /// The player's area beside the listing, in points.
    pub area: [f32; 2],
    /// Whether the frames move (the pacer).
    pub last_frame_at: f64,
    /// Whether the first frame comes on its own (false holds it, for the loading scene).
    pub first_frame_at: Option<f64>,
    /// What a host would do, for scripts to read.
    pub status: String,
    pub intents: Vec<MediaIntent>,
    /// Seeks the host applied, for scripts to count.
    pub seeks: usize,
    /// The fake "preference" for the volume.
    pub volume: (f32, bool),
    /// The storyboard lands whole when asked (scenes that start mid-clip).
    pub instant: bool,
    /// Frames made ahead, which the pacer cycles through (the perf scene measures the player,
    /// not this page's pixel loop).
    pub made: Vec<Vec<u8>>,
    pub frame_count: usize,
}

impl Default for Demo {
    fn default() -> Self {
        Demo {
            media: MediaState::default(),
            clock: FakeClock::new(None),
            frames: Pixels::new(),
            frame_px: FRAME,
            now: 0.0,
            started: false,
            files: &VIDEOS,
            file: 0,
            jobs: VecDeque::new(),
            cells: VecDeque::new(),
            area: [660.0, 640.0],
            last_frame_at: f64::NEG_INFINITY,
            first_frame_at: Some(0.1),
            status: String::new(),
            intents: Vec::new(),
            seeks: 0,
            volume: (0.6, false),
            instant: false,
            made: Vec::new(),
            frame_count: 0,
        }
    }
}

/// Strata's storyboard order: the middle first, then quarters, eighths and so on.
fn subdivision_order(count: u32) -> Vec<u32> {
    let mut order = Vec::new();
    let mut ranges = VecDeque::from([(0, count)]);
    while let Some((low, high)) = ranges.pop_front() {
        if low >= high {
            continue;
        }
        let middle = low + (high - low) / 2;
        order.push(middle);
        ranges.push_back((low, middle));
        ranges.push_back((middle + 1, high));
    }
    order
}

/// A level for bucket `i` of a clip: a few sines, so the shape is stable and varied.
fn level(i: usize, seed: f32) -> f32 {
    let x = i as f32 / BUCKETS as f32;
    let v = 0.45
        + 0.3 * (x * 23.0 + seed).sin()
        + 0.18 * (x * 71.0 + seed * 2.0).sin()
        + 0.07 * (x * 199.0).sin();
    (v.clamp(0.02, 1.0)).powi(2)
}

/// A frame of the fake video at `t` seconds: a sky gradient whose hue follows the clip and a
/// bar that crosses with time, so playback, seeks and steps show.
pub fn frame_rgba(size: [u32; 2], t: f64, seed: u8) -> Vec<u8> {
    let [w, h] = size;
    let mut px = vec![0u8; (w * h * 4) as usize];
    let bar = ((t * 40.0) as u32) % w.max(1);
    for y in 0..h {
        let v = y * 255 / h.max(1);
        let row = (y * w * 4) as usize;
        for x in 0..w {
            let i = row + (x * 4) as usize;
            let on_bar = x.abs_diff(bar) < w / 40 + 1;
            let (r, g, b) = if on_bar {
                (240, 236, 220)
            } else {
                (
                    (30 + v / 3) as u8 ^ seed,
                    (60 + v / 2) as u8,
                    (120 + (x * 100 / w.max(1))) as u8,
                )
            };
            px[i..i + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    px
}

/// A small RGBA picture: a storyboard cell, a poster or art.
fn picture(size: [u32; 2], tint: u8) -> Pixels {
    let p = Pixels::new();
    p.set(
        size[0],
        size[1],
        frame_rgba(size, f64::from(tint) * 0.37, tint),
    );
    p
}

impl Demo {
    fn current(&self) -> File {
        self.files[self.file]
    }

    /// Opens file `index` of `files`: the host's new clip, paused at its start, loading.
    pub fn open(&mut self, files: &'static [File], index: usize, now: f64) {
        self.files = files;
        self.file = index;
        let f = self.current();
        let n = files.len();
        let position = (n > 1).then_some(Position {
            index: index + 1,
            count: n,
            results: false,
        });
        self.media.set_clip(Clip {
            kind: f.kind,
            name: f.name.into(),
            position,
            has_previous: index > 0,
            has_next: index + 1 < n,
        });
        self.media.set_volume(self.volume.0, self.volume.1);
        self.clock = FakeClock::new(Some(Duration::from_secs(f.seconds)));
        self.jobs.clear();
        self.cells.clear();
        self.started = true;
        self.last_frame_at = f64::NEG_INFINITY;
        match f.kind {
            MediaKind::Video => {
                self.media
                    .set_poster(Some(picture([160, 90], 3)), Some(16.0 / 9.0));
                if f.seconds >= 4 {
                    self.media.set_chapters(vec![
                        Chapter {
                            start: Duration::ZERO,
                            title: "Opening".into(),
                        },
                        Chapter {
                            start: Duration::from_secs(f.seconds * 9 / 40),
                            title: "The harbour".into(),
                        },
                        Chapter {
                            start: Duration::from_secs(f.seconds * 3 / 5),
                            title: "Night".into(),
                        },
                    ]);
                }
                self.jobs.push_back((now + 0.2, Job::Details));
                self.schedule_levels(now + 0.3);
            }
            MediaKind::Audio => {
                self.media.set_tags(AudioTags {
                    title: (index != 1).then(|| time::stem(&f.name[3..]).into()),
                    artist: Some("The Moorings".into()),
                    album: Some("Tidelines".into()),
                    track: (index != 2).then_some(index as u32 + 1),
                    track_total: (index == 0).then_some(12),
                });
                self.schedule_levels(now + 0.1);
            }
            MediaKind::Plain => {}
        }
        if let Some(at) = self.first_frame_at
            && f.kind != MediaKind::Audio
        {
            self.jobs.push_back((now + at, Job::FirstFrame));
        }
        self.sort_jobs();
    }

    fn schedule_levels(&mut self, from: f64) {
        for (i, start) in (0..BUCKETS).step_by(128).enumerate() {
            self.jobs
                .push_back((from + i as f64 * 0.04, Job::Levels(start)));
        }
    }

    fn sort_jobs(&mut self) {
        self.jobs
            .make_contiguous()
            .sort_by(|a, b| a.0.total_cmp(&b.0));
    }

    /// Everything due by `now` lands at once: for scenes that start mid-way.
    pub fn land_all(&mut self) {
        while let Some((_, job)) = self.jobs.pop_front() {
            self.run(job);
        }
        while !self.cells.is_empty() {
            self.run(Job::Cell);
        }
        self.media.settle_levels();
    }

    fn run(&mut self, job: Job) {
        let seed = self.file as f32 * 1.7
            + if self.current().kind == MediaKind::Audio {
                3.0
            } else {
                0.0
            };
        match job {
            Job::Levels(start) => {
                let levels: Vec<f32> = (start..start + 128).map(|i| level(i, seed)).collect();
                self.media.set_levels(BUCKETS, start, &levels);
            }
            Job::Cell => {
                if let Some(i) = self.cells.pop_front()
                    && let Some(b) = self.media.storyboard_mut()
                {
                    b.set_cell(i, picture(CELL, (i * 5) as u8));
                }
            }
            Job::Details => self.media.set_details(Details::Ready(VideoDetails {
                dimensions: Some((1920, 1080)),
                color_transfer: Some("smpte2084".into()),
                pixel_format: Some("yuv420p10le".into()),
                frame_rate: Some(29.97),
                video_codec: Some("hevc".into()),
                audio_codec: Some("aac".into()),
                channels: Some(2),
                channel_layout: Some("stereo".into()),
                subtitle_tracks: 1,
                sidecar_captions: 1,
            })),
            Job::FirstFrame => {
                self.write_frame(true);
                self.media.set_frame(Some(self.frame_px));
            }
        }
    }

    /// The pacer stand-in: writes the frame for the clock's position, in place.
    fn write_frame(&mut self, force: bool) {
        if !force && !self.media.has_frame() {
            return;
        }
        self.frame_count += 1;
        if !self.made.is_empty() {
            let rgba = self.made[self.frame_count % self.made.len()].clone();
            self.frames.set(self.frame_px[0], self.frame_px[1], rgba);
            return;
        }
        let t = self.clock.position().as_secs_f64();
        self.frames.set(
            self.frame_px[0],
            self.frame_px[1],
            frame_rgba(self.frame_px, t, self.file as u8 * 40),
        );
    }

    /// The host's answer to an intent.
    pub fn apply(&mut self, intent: MediaIntent) {
        self.intents.push(intent);
        self.clock.apply(&intent);
        match intent {
            MediaIntent::SetVolume { volume, muted } => {
                self.volume = (volume, muted);
                self.media.set_volume(volume, muted);
                self.status = if muted {
                    "Muted".into()
                } else {
                    format!("Volume {}%", (volume * 100.0).round() as u32)
                };
            }
            MediaIntent::Step(by) => {
                let next = (self.file as i64 + i64::from(by)).clamp(0, self.files.len() as i64 - 1);
                let files = self.files;
                let now = self.now;
                self.open(files, next as usize, now);
                self.status = format!("Showing {}", self.current().name);
            }
            MediaIntent::HandOff { at } => {
                self.status = match at {
                    Some(t) => format!("Opened in the default player at {}", time::clock(t)),
                    None => "Opened in the default player".into(),
                };
            }
            MediaIntent::StoryboardWanted => {
                let f = self.current();
                // Strata's rule: no storyboard under 4 s.
                if f.seconds >= 4 {
                    let count = ((f.seconds / 2) as u32).clamp(8, 48);
                    self.media.set_storyboard(Some(Storyboard::new(Sheet {
                        cell: CELL,
                        count,
                        duration: Duration::from_secs(f.seconds),
                    })));
                    self.cells = subdivision_order(count).into();
                    if self.instant {
                        while !self.cells.is_empty() {
                            self.run(Job::Cell);
                        }
                    } else {
                        let now = self.now;
                        for i in 0..count {
                            self.jobs
                                .push_back((now + 0.05 + f64::from(i) * 0.02, Job::Cell));
                        }
                        self.sort_jobs();
                    }
                }
            }
            MediaIntent::DecodeSize([w, h]) => {
                self.status = format!("Decoding at {w} × {h}");
                // The decoder restarts at that size, fitting the 16:9 source and never past
                // its 1920 × 1080 (the perf scene keeps its fixed frames).
                if self.frame_px != PERF_FRAME {
                    let s = (w as f32 / 16.0).min(h as f32 / 9.0).min(120.0);
                    self.frame_px = [(16.0 * s).round() as u32, (9.0 * s).round() as u32];
                    self.write_frame(true);
                    self.media.set_frame(Some(self.frame_px));
                }
            }
            MediaIntent::Play => self.status = "Playing".into(),
            MediaIntent::Pause => self.status = "Paused".into(),
            MediaIntent::Seek(_) => self.seeks += 1,
        }
    }

    fn drain(&mut self) {
        for i in self.media.drain_intents() {
            self.apply(i);
        }
    }
}

// Scenes.

/// A video mid-clip: everything landed, at 1:05.
fn video_ready(g: &mut Gallery, dark: bool) {
    g.open("c13", dark);
    let d = &mut g.c13;
    d.instant = true;
    d.open(&VIDEOS, 0, 0.0);
    d.land_all();
    d.write_frame(true);
    d.media.set_frame(Some(d.frame_px));
    d.clock.set_position(Duration::from_secs(65));
}

fn video_playing(g: &mut Gallery, dark: bool) {
    video_ready(g, dark);
    g.c13.clock.play_now();
}

fn video_loading(g: &mut Gallery, dark: bool) {
    g.open("c13", dark);
    let d = &mut g.c13;
    d.first_frame_at = None;
    d.open(&VIDEOS, 0, 0.0);
    d.clock = FakeClock::new(Some(Duration::from_secs(200))).loading();
    d.jobs
        .retain(|(_, j)| !matches!(j, Job::Details | Job::Levels(_)));
}

fn video_hover(g: &mut Gallery, dark: bool) {
    video_ready(g, dark);
    g.c13.media.preset_hover(0.62);
}

fn video_seeking(g: &mut Gallery, dark: bool) {
    video_ready(g, dark);
    let d = &mut g.c13;
    d.clock = FakeClock::new(Some(Duration::from_secs(200))).latency(0.0, 60.0);
    d.clock.set_position(Duration::from_secs(65));
    d.clock.apply(&MediaIntent::Seek(Duration::from_secs(150)));
}

fn video_error(g: &mut Gallery, dark: bool) {
    g.open("c13", dark);
    let d = &mut g.c13;
    d.first_frame_at = None;
    d.open(&VIDEOS, 1, 0.0);
    d.jobs.clear();
    d.media.set_details(Details::Failed);
    d.media.set_error(Some(MediaError {
        title: "Can't play this video".into(),
        detail: "The decoder stopped: the file ends early.".into(),
    }));
    d.clock = FakeClock::new(None);
}

fn audio(g: &mut Gallery, dark: bool, area: [f32; 2], art: bool) {
    g.open("c13", dark);
    let d = &mut g.c13;
    d.area = area;
    d.open(&SONGS, 0, 0.0);
    d.land_all();
    if art {
        d.media.set_art(Some(Art {
            id: 1,
            pixels: picture([240, 240], 9),
            size: [240, 240],
        }));
    }
    d.clock.set_position(Duration::from_secs(52));
    d.clock.play_now();
}

fn plain(g: &mut Gallery, dark: bool) {
    g.open("c13", dark);
    let d = &mut g.c13;
    d.open(std::slice::from_ref(&ANIMATION), 0, 0.0);
    d.land_all();
    d.clock.set_position(Duration::from_secs(4));
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c13", dark)`, then fills in `g.c13`.
pub const SCENES: &[Scene] = &[
    ("c13-video-playing-light", |g| video_playing(g, false)),
    ("c13-video-playing-dark", |g| video_playing(g, true)),
    ("c13-video-loading-light", |g| video_loading(g, false)),
    ("c13-video-loading-dark", |g| video_loading(g, true)),
    ("c13-video-hover-light", |g| video_hover(g, false)),
    ("c13-video-hover-dark", |g| video_hover(g, true)),
    ("c13-video-seeking-light", |g| video_seeking(g, false)),
    ("c13-video-seeking-dark", |g| video_seeking(g, true)),
    ("c13-video-error-light", |g| video_error(g, false)),
    ("c13-video-error-dark", |g| video_error(g, true)),
    ("c13-audio-wide-light", |g| {
        audio(g, false, [760.0, 640.0], true)
    }),
    ("c13-audio-wide-dark", |g| {
        audio(g, true, [760.0, 640.0], true)
    }),
    ("c13-audio-record-light", |g| {
        audio(g, false, [520.0, 640.0], false)
    }),
    ("c13-audio-record-dark", |g| {
        audio(g, true, [520.0, 640.0], false)
    }),
    // Stacked and short: the spectrum hides first.
    ("c13-audio-short-light", |g| {
        audio(g, false, [300.0, 320.0], false)
    }),
    ("c13-audio-short-dark", |g| {
        audio(g, true, [300.0, 320.0], false)
    }),
    ("c13-plain-light", |g| plain(g, false)),
    ("c13-plain-dark", |g| plain(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    // A paused video at its start, everything landed: most scripts start here.
    ("c13-video-paused-light", |g| video_paused(g, false)),
    ("c13-video-paused-dark", |g| video_paused(g, true)),
    // A fresh video: the first frame lands at 0.1 s and the storyboard streams in.
    ("c13-video-fresh-light", |g| video_fresh(g, false)),
    ("c13-video-fresh-dark", |g| video_fresh(g, true)),
    // A video whose duration isn't known yet: it learns it at 1.5 s.
    ("c13-video-unknown-light", |g| video_unknown(g, false)),
    ("c13-video-unknown-dark", |g| video_unknown(g, true)),
    // The 10xer key map.
    ("c13-tenxer-light", |g| tenxer(g, false)),
    ("c13-tenxer-dark", |g| tenxer(g, true)),
    ("c13-audio-tenxer-light", |g| audio_tenxer(g, false)),
    ("c13-audio-tenxer-dark", |g| audio_tenxer(g, true)),
    // A host's own bindings: F9 seeks to the end, and `+` is unbound so `=` alone raises the
    // volume (Strata's `[keymap.*]` overrides).
    ("c13-rebound-light", |g| rebound(g, false)),
    ("c13-rebound-dark", |g| rebound(g, true)),
    // A paused song with the waveform landed.
    ("c13-audio-paused-light", |g| audio_paused(g, false)),
    ("c13-audio-paused-dark", |g| audio_paused(g, true)),
    // The second song: no title tag, a track number without a total.
    ("c13-audio-untitled-light", |g| audio_untitled(g, false)),
    ("c13-audio-untitled-dark", |g| audio_untitled(g, true)),
    // Stacked and too short for art or spectrum.
    ("c13-audio-tiny-light", |g| {
        audio(g, false, [300.0, 270.0], false)
    }),
    ("c13-audio-tiny-dark", |g| {
        audio(g, true, [300.0, 270.0], false)
    }),
    ("c13-audio-error-light", |g| audio_error(g, false)),
    ("c13-audio-error-dark", |g| audio_error(g, true)),
    ("c13-plain-paused-light", |g| plain_paused(g, false)),
    ("c13-plain-paused-dark", |g| plain_paused(g, true)),
    // A 1280 × 720 video at 60 fps, for perf.
    ("c13-perf-light", |g| perf(g, false)),
    ("c13-perf-dark", |g| perf(g, true)),
];

fn video_paused(g: &mut Gallery, dark: bool) {
    video_ready(g, dark);
    g.c13.clock.set_position(Duration::ZERO);
}

fn video_fresh(g: &mut Gallery, dark: bool) {
    g.open("c13", dark);
    g.c13.open(&VIDEOS, 0, 0.0);
}

fn video_unknown(g: &mut Gallery, dark: bool) {
    video_ready(g, dark);
    g.c13.clock = FakeClock::new(None);
}

fn tenxer(g: &mut Gallery, dark: bool) {
    video_ready(g, dark);
    g.c13.clock.set_position(Duration::from_secs(65));
    g.c13.media.set_keys(MediaKeys::tenxer());
}

fn audio_tenxer(g: &mut Gallery, dark: bool) {
    audio(g, dark, [520.0, 640.0], false);
    g.c13.clock = FakeClock::new(Some(Duration::from_secs(245)));
    g.c13.media.set_keys(MediaKeys::tenxer());
}

fn rebound(g: &mut Gallery, dark: bool) {
    video_ready(g, dark);
    let mut keys = MediaKeys::default_map();
    keys.bind(
        KeyScope::Focused,
        Chord::new(ChordKey::Function(9)),
        MediaCommand::Seek(Seek::End),
    )
    .unbind_chord(KeyScope::Focused, Chord::new(ChordKey::Char('+')));
    g.c13.media.set_keys(keys);
}

fn audio_paused(g: &mut Gallery, dark: bool) {
    audio(g, dark, [520.0, 640.0], true);
    g.c13.clock = FakeClock::new(Some(Duration::from_secs(245)));
}

fn audio_untitled(g: &mut Gallery, dark: bool) {
    audio(g, dark, [520.0, 640.0], false);
    let d = &mut g.c13;
    d.open(&SONGS, 1, 0.0);
    d.land_all();
}

fn audio_error(g: &mut Gallery, dark: bool) {
    audio(g, dark, [520.0, 640.0], false);
    let d = &mut g.c13;
    d.open(&SONGS, 1, 0.0);
    d.media.set_error(Some(MediaError {
        title: "Can't play this file".into(),
        detail: "The audio stream is damaged.".into(),
    }));
}

fn plain_paused(g: &mut Gallery, dark: bool) {
    plain(g, dark);
    g.c13.clock.set_position(Duration::ZERO);
}

fn perf(g: &mut Gallery, dark: bool) {
    g.open("c13", dark);
    let d = &mut g.c13;
    d.frame_px = PERF_FRAME;
    d.made = (0..4)
        .map(|i| frame_rgba(PERF_FRAME, f64::from(i) * 0.25, 0))
        .collect();
    video_playing_inner(d);
}

fn video_playing_inner(d: &mut Demo) {
    d.instant = true;
    d.open(&VIDEOS, 0, 0.0);
    d.land_all();
    d.write_frame(true);
    d.media.set_frame(Some(d.frame_px));
    d.clock.play_now();
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c13;
    let look = g.look();
    let k = &look.kit;
    let player = MediaPlayer::new(
        &d.media,
        |g: &mut Gallery| &mut g.c13.media,
        |g: &mut Gallery| &mut g.c13.media.viewport,
    )
    .key("media")
    .video(&d.frames)
    .on_intent(|g: &mut Gallery, intent, _tx| g.c13.apply(intent))
    .build(look, cx);
    let names = d.files.iter().enumerate().map(|(i, f)| {
        text(f.name)
            .font(k.ty.ui)
            .color(if i == d.file {
                k.c.text_base
            } else {
                k.c.text_muted
            })
            .h(look.row.list)
    });
    let listing_w = (crate::SIZE.0 - d.area[0]).max(0.0);
    row()
        .key("c13")
        .w_full()
        .h_full()
        .child(
            col()
                .key("files")
                .role("listbox")
                .label("Files")
                .focusable(true)
                .w(listing_w)
                .h_full()
                .padding(look.space.md)
                .gap(look.space.xs)
                .children(names)
                .child(
                    text(d.status.clone())
                        .key("c13-status")
                        .font(k.ty.small)
                        .color(k.c.text_accent),
                )
                .child(
                    text(format!("{} seeks", d.seeks))
                        .key("c13-seeks")
                        .font(k.ty.small)
                        .color(k.c.text_muted),
                ),
        )
        .child(
            col()
                .key("c13-area")
                .w(d.area[0])
                .h(d.area[1])
                .fill(k.c.bg_deep)
                .child(player),
        )
}

/// The window's media keys, as Strata's window key handler hands them over.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c13;
    let Some(command) = d.media.keys().window_command(key, tx) else {
        return false;
    };
    let taken = d.media.command(command);
    d.drain();
    taken
}

/// The fake host: the clock, the workers, the pacer, the spectrum's bands, then the player's
/// tick.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = &mut g.c13;
    d.now = t;
    let mut changed = false;
    if !d.started {
        // Opened from the index: the paused video.
        d.open(&VIDEOS, 0, t);
        changed = true;
    }
    d.clock.advance(t);
    while let Some(&(due, job)) = d.jobs.front() {
        if due > t {
            break;
        }
        d.jobs.pop_front();
        d.run(job);
        // The player commits for its own data (levels grow in, a cell shows in the bubble).
        changed |= matches!(job, Job::Details | Job::FirstFrame);
    }
    // The unknown-duration scene learns its duration at 1.5 s.
    if d.clock.duration().is_none() && d.media.error().is_none() && t >= 1.5 {
        d.clock
            .set_duration(Some(Duration::from_secs(d.current().seconds)));
    }
    let playing = d.clock.state() == strata_components::media_player::PlaybackState::Playing;
    if playing && d.current().kind != MediaKind::Audio && t - d.last_frame_at >= 1.0 / 61.0 {
        d.last_frame_at = t;
        d.write_frame(false);
    }
    if playing && d.current().kind == MediaKind::Audio {
        let bands: Vec<f32> = (0..64)
            .map(|i| {
                let x = i as f64;
                (0.55 + 0.35 * (t * 3.1 + x * 0.37).sin() * (t * 0.7 + x * 0.11).cos() - x / 160.0)
                    .clamp(0.0, 1.0) as f32
            })
            .collect();
        d.media.feed_bands(&bands);
    }
    changed |= d.media.tick(&d.clock, t, tx);
    d.drain();
    if let Some(due) = d.clock.next_due() {
        tx.wake_after(Duration::from_secs_f64((due - t).max(0.0)));
    }
    if let Some(&(due, _)) = d.jobs.front() {
        tx.wake_after(Duration::from_secs_f64((due - t).max(0.0)));
    }
    if d.clock.duration().is_none() && t < 1.5 {
        tx.wake_after(Duration::from_secs_f64(1.5 - t));
    }
    changed
}
