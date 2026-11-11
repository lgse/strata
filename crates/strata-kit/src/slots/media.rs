//! The `media.*` slot tokens (for example `media.waveform.played`) that MediaPlayer reads.
//!
//! Work package: C13, `docs/work/phase-2-composites/C13-media-player.md`. C13 owns this file.
//! Slot names are API: renaming one breaks users' override files (plan K4).
//!
//! Every default follows the core tokens. Strata's media palette
//! (`src/ui/preview/audio/palette.rs`, lgse/strata `upstream/main` e9a8993) derives its
//! colours from the theme's accent and text: `accent_bright` is the accent mixed 40 % toward
//! the text, `peak` 70 %. Here the accent is `kit.c.text_accent` and the text
//! `kit.c.text_base`. The plain numbers are Strata's `waveform.rs`, `spectrum.rs` and
//! `video/frame.rs` constants, which no core token says.

use rune::Color;

use crate::look::Look;
use crate::tokens::Token;

/// MediaPlayer's slot tokens. `None` inherits.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Media {
    pub waveform_height: Option<f32>,
    pub waveform_bar: Option<f32>,
    pub waveform_gap: Option<f32>,
    pub waveform_played: Option<Color>,
    pub waveform_unplayed: Option<Color>,
    pub waveform_hover: Option<Color>,
    pub waveform_playhead: Option<Color>,
    pub waveform_chapter: Option<Color>,
    pub spectrum_bar: Option<f32>,
    pub spectrum_gap: Option<f32>,
    pub spectrum_base: Option<Color>,
    pub spectrum_bright: Option<Color>,
    pub spectrum_peak: Option<Color>,
    pub spectrum_reflection: Option<Color>,
    pub record_disc: Option<Color>,
    pub record_groove: Option<Color>,
    pub record_label: Option<Color>,
    pub frame_radius: Option<f32>,
    pub frame_fill: Option<Color>,
    pub bubble_fill: Option<Color>,
    pub bubble_border: Option<Color>,
    pub bubble_radius: Option<f32>,
    pub bubble_pad: Option<f32>,
    pub bubble_cell_fill: Option<Color>,
    pub bubble_cell_radius: Option<f32>,
}

fn accent(look: &Look) -> Color {
    look.kit.c.text_accent
}

fn text(look: &Look) -> Color {
    look.kit.c.text_base
}

fn fade(look: &Look, c: Color, x: f32) -> Color {
    look.kit.alpha(c, c.a * x)
}

/// The waveform's height: `media.waveform.height`, else Strata's 40.
pub fn waveform_height(look: &Look) -> f32 {
    look.slots.media.waveform_height.unwrap_or(40.0)
}

/// A waveform bar's width: `media.waveform.bar`, else Strata's 2.
pub fn waveform_bar(look: &Look) -> f32 {
    look.slots.media.waveform_bar.unwrap_or(2.0)
}

/// The gap between waveform bars: `media.waveform.gap`, else Strata's 1.
pub fn waveform_gap(look: &Look) -> f32 {
    look.slots.media.waveform_gap.unwrap_or(1.0)
}

/// The played part of the waveform: `media.waveform.played`, else the accent.
pub fn waveform_played(look: &Look) -> Color {
    look.slots
        .media
        .waveform_played
        .unwrap_or_else(|| accent(look))
}

/// The part not played yet: `media.waveform.unplayed`, else muted text at 45 %.
pub fn waveform_unplayed(look: &Look) -> Color {
    look.slots
        .media
        .waveform_unplayed
        .unwrap_or_else(|| fade(look, look.kit.c.text_muted, 0.45))
}

/// The part between the playhead and the pointer: `media.waveform.hover`, else the accent at
/// 55 %.
pub fn waveform_hover(look: &Look) -> Color {
    look.slots
        .media
        .waveform_hover
        .unwrap_or_else(|| fade(look, accent(look), 0.55))
}

/// The playhead: `media.waveform.playhead`, else Strata's `peak`.
pub fn waveform_playhead(look: &Look) -> Color {
    look.slots
        .media
        .waveform_playhead
        .unwrap_or_else(|| accent(look).mix(text(look), 0.7))
}

/// Chapter ticks and the pointer's line: `media.waveform.chapter`, else text at 50 %.
pub fn waveform_chapter(look: &Look) -> Color {
    look.slots
        .media
        .waveform_chapter
        .unwrap_or_else(|| fade(look, text(look), 0.5))
}

/// The width a spectrum bar aims for: `media.spectrum.bar`, else Strata's 5.
pub fn spectrum_bar(look: &Look) -> f32 {
    look.slots.media.spectrum_bar.unwrap_or(5.0)
}

/// The gap between spectrum bars: `media.spectrum.gap`, else Strata's 2.
pub fn spectrum_gap(look: &Look) -> f32 {
    look.slots.media.spectrum_gap.unwrap_or(2.0)
}

/// A spectrum bar's foot: `media.spectrum.base`, else the accent.
pub fn spectrum_base(look: &Look) -> Color {
    look.slots
        .media
        .spectrum_base
        .unwrap_or_else(|| accent(look))
}

/// The colour a full-height bar reaches: `media.spectrum.bright`, else Strata's
/// `accent_bright`.
pub fn spectrum_bright(look: &Look) -> Color {
    look.slots
        .media
        .spectrum_bright
        .unwrap_or_else(|| accent(look).mix(text(look), 0.4))
}

/// The peak caps: `media.spectrum.peak`, else Strata's `peak`.
pub fn spectrum_peak(look: &Look) -> Color {
    look.slots
        .media
        .spectrum_peak
        .unwrap_or_else(|| accent(look).mix(text(look), 0.7))
}

/// The reflection under the bars at its top: `media.spectrum.reflection`, else the accent at
/// 32 %.
pub fn spectrum_reflection(look: &Look) -> Color {
    look.slots
        .media
        .spectrum_reflection
        .unwrap_or_else(|| fade(look, accent(look), 0.32))
}

/// Strata's record stays dark in light themes too: the darker of the background and the
/// text, then the lighter.
fn shades(look: &Look) -> (Color, Color) {
    let (bg, fg) = (look.kit.c.bg_base, text(look));
    if bg.luminance() <= fg.luminance() {
        (bg, fg)
    } else {
        (fg, bg)
    }
}

/// The record's body, shown when a track has no art: `media.record.disc`, else the darker
/// shade 6 % toward the lighter.
pub fn record_disc(look: &Look) -> Color {
    look.slots.media.record_disc.unwrap_or_else(|| {
        let (dark, light) = shades(look);
        dark.mix(light, 0.06)
    })
}

/// The record's grooves: `media.record.groove`, else the lighter shade at 5 %.
pub fn record_groove(look: &Look) -> Color {
    look.slots.media.record_groove.unwrap_or_else(|| {
        let (_, light) = shades(look);
        fade(look, light, 0.05)
    })
}

/// The record's label: `media.record.label`, else the accent.
pub fn record_label(look: &Look) -> Color {
    look.slots
        .media
        .record_label
        .unwrap_or_else(|| accent(look))
}

/// The video frame's and its stand-in's corner radius: `media.frame.radius`, else Strata's 8
/// at the look's roundness.
pub fn frame_radius(look: &Look) -> f32 {
    look.slots
        .media
        .frame_radius
        .unwrap_or_else(|| look.round(8.0))
}

/// The plain surface that stands in for a video with no poster: `media.frame.fill`, else
/// `layer_02`.
pub fn frame_fill(look: &Look) -> Color {
    look.slots.media.frame_fill.unwrap_or(look.kit.c.layer_02)
}

/// The timeline's hover bubble: `media.bubble.fill`, else the background at 92 %
/// (`.preview-video-bubble`).
pub fn bubble_fill(look: &Look) -> Color {
    look.slots
        .media
        .bubble_fill
        .unwrap_or_else(|| fade(look, look.kit.c.bg_base, 0.92))
}

/// The bubble's border: `media.bubble.border`, else the base border at 50 %.
pub fn bubble_border(look: &Look) -> Color {
    look.slots
        .media
        .bubble_border
        .unwrap_or_else(|| fade(look, look.kit.c.border_base, 0.5))
}

/// The bubble's corner radius: `media.bubble.radius`, else Strata's 6 at the look's roundness.
pub fn bubble_radius(look: &Look) -> f32 {
    look.slots
        .media
        .bubble_radius
        .unwrap_or_else(|| look.round(6.0))
}

/// The space inside the bubble's border: `media.bubble.pad`, else Strata's 3.
pub fn bubble_pad(look: &Look) -> f32 {
    look.slots.media.bubble_pad.unwrap_or(3.0)
}

/// The storyboard cell's backing, shown until a cell lands: `media.bubble.cell_fill`, else
/// muted text at 14 %.
pub fn bubble_cell_fill(look: &Look) -> Color {
    look.slots
        .media
        .bubble_cell_fill
        .unwrap_or_else(|| fade(look, look.kit.c.text_muted, 0.14))
}

/// The storyboard cell's corner radius: `media.bubble.cell_radius`, else Strata's 4 at the
/// look's roundness.
pub fn bubble_cell_radius(look: &Look) -> f32 {
    look.slots
        .media
        .bubble_cell_radius
        .unwrap_or_else(|| look.round(4.0))
}

pub const TOKENS: &[Token] = &[
    Token::length("media.waveform.height", waveform_height, |l, v| {
        l.slots.media.waveform_height = Some(v)
    }),
    Token::length("media.waveform.bar", waveform_bar, |l, v| {
        l.slots.media.waveform_bar = Some(v)
    }),
    Token::length("media.waveform.gap", waveform_gap, |l, v| {
        l.slots.media.waveform_gap = Some(v)
    }),
    Token::color("media.waveform.played", waveform_played, |l, v| {
        l.slots.media.waveform_played = Some(v)
    }),
    Token::color("media.waveform.unplayed", waveform_unplayed, |l, v| {
        l.slots.media.waveform_unplayed = Some(v)
    }),
    Token::color("media.waveform.hover", waveform_hover, |l, v| {
        l.slots.media.waveform_hover = Some(v)
    }),
    Token::color("media.waveform.playhead", waveform_playhead, |l, v| {
        l.slots.media.waveform_playhead = Some(v)
    }),
    Token::color("media.waveform.chapter", waveform_chapter, |l, v| {
        l.slots.media.waveform_chapter = Some(v)
    }),
    Token::length("media.spectrum.bar", spectrum_bar, |l, v| {
        l.slots.media.spectrum_bar = Some(v)
    }),
    Token::length("media.spectrum.gap", spectrum_gap, |l, v| {
        l.slots.media.spectrum_gap = Some(v)
    }),
    Token::color("media.spectrum.base", spectrum_base, |l, v| {
        l.slots.media.spectrum_base = Some(v)
    }),
    Token::color("media.spectrum.bright", spectrum_bright, |l, v| {
        l.slots.media.spectrum_bright = Some(v)
    }),
    Token::color("media.spectrum.peak", spectrum_peak, |l, v| {
        l.slots.media.spectrum_peak = Some(v)
    }),
    Token::color("media.spectrum.reflection", spectrum_reflection, |l, v| {
        l.slots.media.spectrum_reflection = Some(v)
    }),
    Token::color("media.record.disc", record_disc, |l, v| {
        l.slots.media.record_disc = Some(v)
    }),
    Token::color("media.record.groove", record_groove, |l, v| {
        l.slots.media.record_groove = Some(v)
    }),
    Token::color("media.record.label", record_label, |l, v| {
        l.slots.media.record_label = Some(v)
    }),
    Token::length("media.frame.radius", frame_radius, |l, v| {
        l.slots.media.frame_radius = Some(v)
    }),
    Token::color("media.frame.fill", frame_fill, |l, v| {
        l.slots.media.frame_fill = Some(v)
    }),
    Token::color("media.bubble.fill", bubble_fill, |l, v| {
        l.slots.media.bubble_fill = Some(v)
    }),
    Token::color("media.bubble.border", bubble_border, |l, v| {
        l.slots.media.bubble_border = Some(v)
    }),
    Token::length("media.bubble.radius", bubble_radius, |l, v| {
        l.slots.media.bubble_radius = Some(v)
    }),
    Token::length("media.bubble.pad", bubble_pad, |l, v| {
        l.slots.media.bubble_pad = Some(v)
    }),
    Token::color("media.bubble.cell_fill", bubble_cell_fill, |l, v| {
        l.slots.media.bubble_cell_fill = Some(v)
    }),
    Token::length("media.bubble.cell_radius", bubble_cell_radius, |l, v| {
        l.slots.media.bubble_cell_radius = Some(v)
    }),
];
