//! The gallery page for C14a Pixels viewport: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c14a-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C14a-pixels-viewport.md`.
//!
//! A generated photo with transparent parts (zoom it with Ctrl+wheel, a pinch, `+`, `-` and `0`;
//! pan it by drag, the wheel and the arrows), a G-code-style thumbnail that scales down but never
//! up, and a fake video whose frames `poll` writes into its `Pixels` in place without a commit.
//! The line above the viewport shows the last zoom report, as C12 would hear it.

use rune::{Cx, El, KeyEvent, Pixels, Tx, col, el, text};
use strata_components::viewers::viewport::{
    Interaction, Mode, Viewport, ViewportState, ZoomReport,
};

use crate::{Gallery, Scene};

/// The photo's size in pixels.
pub const PHOTO: [u32; 2] = [1200, 800];
/// The thumbnail's size in pixels.
pub const THUMBNAIL: [u32; 2] = [256, 160];
/// The video's size in pixels.
pub const VIDEO: [u32; 2] = [320, 180];
/// The photo pixel the overlay's marker sits on.
pub const MARKER: [f32; 2] = [840.0, 520.0];

/// What the viewport shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Content {
    /// Nothing yet: `poll` loads the photo, so the page opened from the index shows it.
    #[default]
    None,
    Photo,
    Thumbnail,
    Video,
}

/// A change that needs the viewport laid out first; `poll` applies it once it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Start {
    /// Zoom to `level` about the photo pixel `at`, which stays where fit showed it.
    Zoom { level: f32, at: [f32; 2] },
}

/// C14a's demo state, held in `Gallery::c14a`.
#[derive(Default)]
pub struct Demo {
    pub view: ViewportState,
    pub pixels: Pixels,
    pub content: Content,
    pub start: Option<Start>,
    /// The last report the viewport sent, and how many it has sent.
    pub last: Option<ZoomReport>,
    pub reports: usize,
    /// Video frames written since the page opened.
    pub frames: u64,
    /// Keys the page heard: what the viewport left (an arrow with nothing to pan, say).
    pub passed: usize,
}

impl Demo {
    /// Shows `content` in `mode`.
    pub fn load(&mut self, content: Content, mode: Mode) {
        let (size, rgba) = match content {
            Content::None => return,
            Content::Photo => (PHOTO, photo()),
            Content::Thumbnail => (THUMBNAIL, thumbnail()),
            Content::Video => (VIDEO, video_frame(0)),
        };
        *self = Demo {
            view: ViewportState::new(mode),
            content,
            ..Demo::default()
        };
        self.pixels.set(size[0], size[1], rgba);
        // Before the first layout this reports nothing; the first layout does, via `on_zoom`.
        if let Some(r) = self
            .view
            .set_content(Some([size[0] as f32, size[1] as f32]))
        {
            self.last = Some(r);
            self.reports += 1;
        }
    }
}

fn open(g: &mut Gallery, dark: bool, content: Content, mode: Mode) {
    g.open("c14a", dark);
    g.c14a.load(content, mode);
}

fn zoomed(g: &mut Gallery, dark: bool) {
    open(g, dark, Content::Photo, Mode::Fit);
    // 250 %, about the marker.
    g.c14a.start = Some(Start::Zoom {
        level: 2.5,
        at: MARKER,
    });
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c14a", dark)`, then fills in `g.c14a`.
pub const SCENES: &[Scene] = &[
    ("c14a-fit-light", |g| {
        open(g, false, Content::Photo, Mode::Fit)
    }),
    ("c14a-fit-dark", |g| {
        open(g, true, Content::Photo, Mode::Fit)
    }),
    ("c14a-actual-light", |g| {
        open(g, false, Content::Photo, Mode::Actual)
    }),
    ("c14a-actual-dark", |g| {
        open(g, true, Content::Photo, Mode::Actual)
    }),
    ("c14a-zoomed-light", |g| zoomed(g, false)),
    ("c14a-zoomed-dark", |g| zoomed(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c14a-scale-down-light", |g| {
        open(g, false, Content::Thumbnail, Mode::ScaleDown)
    }),
    ("c14a-scale-down-dark", |g| {
        open(g, true, Content::Thumbnail, Mode::ScaleDown)
    }),
    ("c14a-fill-light", |g| {
        open(g, false, Content::Photo, Mode::Fill)
    }),
    ("c14a-fill-dark", |g| {
        open(g, true, Content::Photo, Mode::Fill)
    }),
    ("c14a-stream-light", |g| {
        open(g, false, Content::Video, Mode::Fit)
    }),
    ("c14a-stream-dark", |g| {
        open(g, true, Content::Video, Mode::Fit)
    }),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c14a;
    let look = g.look();
    let k = &look.kit;
    let (name, video) = match d.content {
        Content::Thumbnail => ("benchy.gcode", false),
        Content::Video => ("clip.mp4", true),
        _ => ("harbour.png", false),
    };
    let mut viewport = Viewport::new(&d.view, |g: &mut Gallery| &mut g.c14a.view)
        .key("c14a-viewport")
        .pixels(&d.pixels)
        .label(name)
        .on_zoom(|g: &mut Gallery, r, _tx| {
            g.c14a.last = Some(r);
            g.c14a.reports += 1;
        });
    if video {
        // A video frame is opaque, and only shows.
        viewport = viewport.interaction(Interaction::Fixed);
    } else {
        viewport = viewport.transparent(true);
    }
    if d.content == Content::Photo {
        let (size, color) = (look.icon_size.sm, k.c.text_accent);
        let (ring, ring_width) = (k.c.bg_base, look.space.xxs);
        viewport = viewport.overlay(move |map| {
            let [x, y] = map.point(MARKER);
            el().w_full().h_full().child(
                el().key("c14a-marker")
                    .label("Marker")
                    .role("img")
                    .pin(Some(x - size / 2.0), Some(y - size / 2.0), None, None)
                    .size(size, size)
                    .radius(size / 2.0)
                    .fill(color)
                    .border(ring_width, ring),
            )
        });
    }
    let status = match d.last {
        Some(r) => {
            let [w, h] = r.wanted_px(8192);
            let reports = match d.reports {
                1 => "1 report".to_owned(),
                n => format!("{n} reports"),
            };
            format!(
                "{:?} · {:.0}% · {reports} · render at {w} × {h} px",
                r.mode,
                r.level * 100.0,
            )
        }
        None => "No zoom report yet".to_owned(),
    };
    col()
        .key("c14a-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            el().key("c14a-status")
                .w_full()
                .px(look.space.lg)
                .py(look.space.sm)
                .child(
                    text(status)
                        .key("c14a-report")
                        .font(k.ty.small)
                        .color(k.c.text_muted),
                ),
        )
        .child(
            el().key("c14a-stage")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(viewport.build(look, cx)),
        )
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
/// The page counts them, so a test sees which keys the viewport left.
pub fn on_key(g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    g.c14a.passed += 1;
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Loads the photo when the page opens from the index, applies a scene's zoom once the
/// viewport is laid out, and writes the video's next frame in place. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, _tx: &mut Tx) -> bool {
    let d = &mut g.c14a;
    match d.content {
        Content::None => {
            d.load(Content::Photo, Mode::Fit);
            return true;
        }
        Content::Video => {
            // In place: the view doesn't run for a new frame, so this returns false.
            let frame = (t * 30.0) as u64;
            if frame != d.frames {
                d.frames = frame;
                d.pixels.set(VIDEO[0], VIDEO[1], video_frame(frame));
            }
        }
        _ => {}
    }
    if let Some(Start::Zoom { level, at }) = d.start
        && let Some(about) = frame_point(&d.view, at)
    {
        d.start = None;
        if let Some(r) = d.view.zoom_to(level, Some(about)) {
            d.last = Some(r);
            d.reports += 1;
        }
        return true;
    }
    false
}

/// Where content pixel `px` shows in the viewport's frame, once it is laid out.
pub fn frame_point(view: &ViewportState, px: [f32; 2]) -> Option<[f32; 2]> {
    let (frame, drawn, content) = (view.frame()?, view.drawn()?, view.content()?);
    let pan = view.pan();
    Some([
        (frame[0] - drawn[0]) / 2.0 + pan[0] + px[0] / content[0] * drawn[0],
        (frame[1] - drawn[1]) / 2.0 + pan[1] + px[1] / content[1] * drawn[1],
    ])
}

/// RGBA for a `w` × `h` image from `f(x, y)`.
fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            out.extend_from_slice(&f(x, y));
        }
    }
    out
}

fn mix(a: f32, b: f32, t: f32) -> u8 {
    (a + (b - a) * t).clamp(0.0, 255.0) as u8
}

/// A harbour at dusk: a sky gradient, a sun, a sea of stripes and a grid every 100 pixels (so
/// 1:1 is easy to see), with a clear window cut through it and a half-transparent band.
fn photo() -> Vec<u8> {
    let [w, h] = PHOTO;
    image(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let horizon = 0.62;
        let mut c = if fy < horizon {
            let t = fy / horizon;
            [
                mix(255.0, 70.0, t),
                mix(170.0, 90.0, t),
                mix(90.0, 170.0, t),
            ]
        } else {
            let t = (fy - horizon) / (1.0 - horizon);
            let wave = if (y / 6 + x / 40) % 2 == 0 { 18.0 } else { 0.0 };
            [
                mix(30.0 + wave, 10.0, t),
                mix(80.0 + wave, 40.0, t),
                mix(120.0 + wave, 80.0, t),
            ]
        };
        let (dx, dy) = (fx - 0.7, (fy - 0.45) * h as f32 / w as f32);
        if dx * dx + dy * dy < 0.006 {
            c = [255, 236, 170];
        }
        if x % 100 == 0 || y % 100 == 0 {
            c = [c[0] / 2, c[1] / 2, c[2] / 2];
        }
        // The clear window and the half-transparent band show the checkerboard.
        let (cx, cy) = (fx - 0.25, (fy - 0.35) * h as f32 / w as f32);
        let alpha = if cx * cx + cy * cy < 0.012 {
            0
        } else if (0.80..0.88).contains(&fy) {
            110
        } else {
            255
        };
        [c[0], c[1], c[2], alpha]
    })
}

/// A print preview on a clear background: a plate and a box on it.
fn thumbnail() -> Vec<u8> {
    let [w, h] = THUMBNAIL;
    image(w, h, |x, y| {
        let plate = y > 110 && y < 140 && x > 30 && x < 226;
        let part = x > 88 && x < 168 && y > 40 && y <= 110;
        match (plate, part) {
            (_, true) => [240, 140, 40, 255],
            (true, _) => [90, 90, 100, 255],
            _ => [0, 0, 0, 0],
        }
    })
}

/// Video frame `n`: colour bars sliding left.
fn video_frame(n: u64) -> Vec<u8> {
    let [w, h] = VIDEO;
    const BARS: [[u8; 3]; 7] = [
        [192, 192, 192],
        [192, 192, 0],
        [0, 192, 192],
        [0, 192, 0],
        [192, 0, 192],
        [192, 0, 0],
        [0, 0, 192],
    ];
    image(w, h, |x, y| {
        let bar = ((x as u64 + n * 4) * 7 / w as u64 % 7) as usize;
        let c = BARS[bar];
        let shade = if y > h * 3 / 4 { 2 } else { 1 };
        [c[0] / shade, c[1] / shade, c[2] / shade, 255]
    })
}
