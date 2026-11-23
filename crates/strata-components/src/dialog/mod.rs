// SPDX-License-Identifier: MIT

mod action;
mod keys;
mod limit;
mod shell;
mod text;

#[cfg(test)]
mod tests;

pub use action::{Focus, RESERVED, StrataDialog, Tone};
pub use keys::{newline_on_enter, text_area};
pub use limit::{Measure, fit, limit_on_change, remaining, trim_insertion, trim_to_fit};
pub use shell::DialogShell;
pub use text::keep_words_whole;

pub const BACKDROP_KEY: &str = "strata-dialog-backdrop";

pub const BACKDROP_BLUR: f32 = 1.75;

pub const ENTRANCE_SCALE: f32 = 0.96;

pub const MOTION_SECONDS: f32 = 0.2;

pub const WIDTH: f32 = 480.0;

pub const MESSAGE_WIDTH: f32 = 560.0;

pub const PANEL_RADIUS: f32 = 9.0;

/// The header icon box's side and corner radius before roundness (`.action-dialog-symbol`).
pub const ICON_BOX: f32 = 40.0;
pub const ICON_BOX_RADIUS: f32 = 6.0;

pub const FOCUS_RING_WIDTH: f32 = 3.0;
pub const FOCUS_RING_ALPHA: f32 = 0.28;

pub const KOREAN_WORD_MAX: usize = 12;
