// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent};
use strata_kit::look::Ladder;

pub const PINCH_WHEEL: f32 = 200.0;

pub fn wheel_factor(dy: f32) -> f32 {
    wheel_factor_per(dy, 1.0 / PINCH_WHEEL)
}

pub fn wheel_factor_per(dy: f32, per_point: f32) -> f32 {
    (dy * per_point).exp()
}

pub fn magnify_factor(delta: f32) -> f32 {
    delta.exp()
}

pub fn zoom_about_scroll(offset: f32, anchor: f32, k: f32) -> f32 {
    (offset + anchor) * k - anchor
}

/// Strata's zoom steps for `+` and `-`: from 10 % to 3200 %, with 100 % (1:1) the default.
pub fn default_steps() -> Ladder {
    let steps = vec![
        0.1,
        0.25,
        1.0 / 3.0,
        0.5,
        2.0 / 3.0,
        1.0,
        1.5,
        2.0,
        3.0,
        4.0,
        6.0,
        8.0,
        12.0,
        16.0,
        24.0,
        32.0,
    ];
    Ladder::new(steps, 1.0).unwrap_or_else(|| unreachable!("the default steps increase"))
}

/// The range a level is held in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomLimits {
    pub min: f32,
    pub max: f32,
}

impl ZoomLimits {
    /// Limits from `min` to `max`, swapped if given the wrong way round.
    pub fn new(min: f32, max: f32) -> ZoomLimits {
        ZoomLimits {
            min: min.min(max),
            max: max.max(min),
        }
    }

    /// The range `steps` spans, from its first step to its last.
    pub fn of(steps: &Ladder) -> ZoomLimits {
        ZoomLimits::new(steps.min(), steps.max())
    }

    pub fn clamp(&self, level: f32) -> f32 {
        level.clamp(self.min, self.max)
    }

    pub fn widened_to(&self, level: f32) -> ZoomLimits {
        ZoomLimits::new(self.min.min(level), self.max.max(level))
    }
}

impl Default for ZoomLimits {
    fn default() -> ZoomLimits {
        ZoomLimits::of(&default_steps())
    }
}

/// How the content sits in the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The whole content, as large as fits, letterboxed.
    #[default]
    Fit,
    /// As [`Mode::Fit`], but never larger than 1:1 (still images, animations, G-code thumbnails).
    ScaleDown,
    /// Covers the frame; the overflow is cropped and can be panned.
    Fill,
    /// 1:1: one content pixel per device pixel.
    Actual,
    /// A level the user zoomed to.
    Free,
}

impl Mode {
    /// Whether the level follows the frame's size.
    pub fn follows_frame(self) -> bool {
        matches!(self, Mode::Fit | Mode::ScaleDown | Mode::Fill)
    }
}

pub fn mode_level(mode: Mode, frame: [f32; 2], content: [f32; 2], scale: f32, free: f32) -> f32 {
    let ratio = |pick: fn(f32, f32) -> f32| {
        if content[0] <= 0.0 || content[1] <= 0.0 || frame[0] <= 0.0 || frame[1] <= 0.0 {
            1.0
        } else {
            pick(frame[0] / content[0], frame[1] / content[1]) * scale
        }
    };
    match mode {
        Mode::Fit => ratio(f32::min),
        Mode::ScaleDown => ratio(f32::min).min(1.0),
        Mode::Fill => ratio(f32::max),
        Mode::Actual => 1.0,
        Mode::Free => free,
    }
}

/// The size `content` draws at, in points, at `level` and display `scale`.
pub fn drawn_size(content: [f32; 2], level: f32, scale: f32) -> [f32; 2] {
    let k = level / scale.max(f32::EPSILON);
    [content[0] * k, content[1] * k]
}

pub fn clamp_pan(pan: [f32; 2], drawn: [f32; 2], frame: [f32; 2]) -> [f32; 2] {
    let axis = |p: f32, d: f32, f: f32| {
        let reach = ((d - f) / 2.0).max(0.0);
        p.clamp(-reach, reach)
    };
    [
        axis(pan[0], drawn[0], frame[0]),
        axis(pan[1], drawn[1], frame[1]),
    ]
}

/// Whether the drawn content overflows the frame, per axis (beyond a hundredth of a point).
pub fn overflows(drawn: [f32; 2], frame: [f32; 2]) -> [bool; 2] {
    [drawn[0] > frame[0] + 0.01, drawn[1] > frame[1] + 0.01]
}

pub fn zoom_about(pan: [f32; 2], about: [f32; 2], k: f32) -> [f32; 2] {
    [
        about[0] - (about[0] - pan[0]) * k,
        about[1] - (about[1] - pan[1]) * k,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomReport {
    /// Device pixels per content pixel.
    pub level: f32,
    pub mode: Mode,
    /// The content's size as drawn, in device pixels.
    pub drawn_px: [f32; 2],
    /// The content's own size, in content pixels.
    pub content: [f32; 2],
}

impl ZoomReport {
    /// A report for `content` at `level` in `mode`.
    pub fn new(level: f32, mode: Mode, content: [f32; 2]) -> ZoomReport {
        ZoomReport {
            level,
            mode,
            drawn_px: [content[0] * level, content[1] * level],
            content,
        }
    }

    pub fn wanted_px(&self, cap: u32) -> [u32; 2] {
        let w = self.drawn_px[0].min(self.content[0]).max(0.0);
        let h = self.drawn_px[1].min(self.content[1]).max(0.0);
        let cap = cap.max(1) as f32;
        let k = (cap / w.max(h).max(f32::EPSILON)).min(1.0);
        [
            ((w * k).ceil() as u32).min(cap as u32),
            ((h * k).ceil() as u32).min(cap as u32),
        ]
    }
}

/// What a key asks of the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ZoomKey {
    /// One step up the zoom steps.
    In,
    /// One step down.
    Out,
    Reset,
    Pan(i8, i8),
}

pub fn zoom_key(key: &KeyEvent) -> Option<ZoomKey> {
    if let Some(reset) = ctrl_zero_key(key) {
        return Some(reset);
    }
    let m = key.mods;
    if m.cmd || m.ctrl || m.alt || m.logo {
        return None;
    }
    match &key.key {
        Key::Text(t) => match t.as_str() {
            "+" | "=" => Some(ZoomKey::In),
            "-" => Some(ZoomKey::Out),
            // Shift+0 types ")" on most layouts; a "0" with Shift is a keypad 0 at most.
            "0" => Some(ZoomKey::Reset),
            _ => None,
        },
        Key::Left if !m.shift => Some(ZoomKey::Pan(-1, 0)),
        Key::Right if !m.shift => Some(ZoomKey::Pan(1, 0)),
        Key::Up if !m.shift => Some(ZoomKey::Pan(0, -1)),
        Key::Down if !m.shift => Some(ZoomKey::Pan(0, 1)),
        _ => None,
    }
}

pub fn ctrl_zero_key(key: &KeyEvent) -> Option<ZoomKey> {
    let m = key.mods;
    let zero = match &key.key {
        // Rune sends a character typed with the primary modifier as `Command`.
        Key::Command(c) => *c == '0',
        Key::Text(t) => t == "0",
        _ => false,
    };
    let primary = m.cmd || m.ctrl;
    let super_with_ctrl = m.ctrl && m.logo;
    (zero && primary && !m.alt && !m.shift && !super_with_ctrl).then_some(ZoomKey::Reset)
}

/// No keys at all: for a host that routes every key itself.
pub fn no_keys(_key: &KeyEvent) -> Option<ZoomKey> {
    None
}
