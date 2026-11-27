// SPDX-License-Identifier: MIT

use rune::Color;

use crate::look::Look;
use crate::tokens::Token;

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Scroll {
    pub bar_width: Option<f32>,
    pub hover_width: Option<f32>,
    pub thumb_color: Option<Color>,
    pub thumb_min_length: Option<f32>,
    pub trough_color: Option<Color>,
    pub trough_opacity: Option<f32>,
    pub trough_hover_opacity: Option<f32>,
    pub listing_width: Option<f32>,
    pub listing_thumb_width: Option<f32>,
    pub listing_trough_opacity: Option<f32>,
    pub listing_hover_opacity: Option<f32>,
    pub listing_end_inset: Option<f32>,
    pub column_right_gap: Option<f32>,
    pub column_top_gap: Option<f32>,
    pub strip_margin: Option<f32>,
    pub strip_ring_opacity: Option<f32>,
    pub strip_ring_width: Option<f32>,
    pub anchor_size: Option<f32>,
    pub trough_radius: Option<f32>,
    pub thumb_radius: Option<f32>,
    pub anchor_border_width: Option<f32>,
    pub anchor_shadow_offset: Option<f32>,
    pub anchor_shadow_blur: Option<f32>,
    pub anchor_fill: Option<Color>,
    pub anchor_border: Option<Color>,
}

/// `c` at `x` of its own opacity, so a translucent theme colour stays translucent.
fn fade(look: &Look, c: Color, x: f32) -> Color {
    look.kit.alpha(c, c.a * x)
}

/// The shared resting scrollbar thickness in points.
const DEFAULT_BAR_WIDTH: f32 = 4.0;

/// How thick the app-wide bar is, trough and thumb alike: `scroll.bar.width`, else 4.
pub fn bar_width(look: &Look) -> f32 {
    look.slots.scroll.bar_width.unwrap_or(DEFAULT_BAR_WIDTH)
}

/// How thick a bar expands to while hovered or dragged: `scroll.bar.hover_width`, if set.
pub fn hover_width(look: &Look) -> Option<f32> {
    look.slots
        .scroll
        .hover_width
        .filter(|&w| w.is_finite() && w > 0.0)
}

/// The thumb's colour: `scroll.thumb.color`, else the accent (`bg_accent`).
pub fn thumb_color(look: &Look) -> Color {
    look.slots
        .scroll
        .thumb_color
        .unwrap_or(look.kit.c.bg_accent)
}

/// The shortest the thumb gets: `scroll.thumb.min_length`, else 32.
pub fn thumb_min_length(look: &Look) -> f32 {
    look.slots.scroll.thumb_min_length.unwrap_or(32.0)
}

/// The trough's colour before its opacity: `scroll.trough.color`, else the border colour.
pub fn trough_color(look: &Look) -> Color {
    look.slots
        .scroll
        .trough_color
        .unwrap_or(look.kit.c.std_border)
}

/// The app-wide trough's opacity at rest: `scroll.trough.opacity`, else 0.25.
pub fn trough_opacity(look: &Look) -> f32 {
    look.slots.scroll.trough_opacity.unwrap_or(0.25)
}

pub fn trough_hover_opacity(look: &Look) -> f32 {
    look.slots.scroll.trough_hover_opacity.unwrap_or(0.45)
}

/// The Icons and List vertical bar's width: `scroll.listing.width`, else 4 (matching `bar_width`).
pub fn listing_width(look: &Look) -> f32 {
    look.slots.scroll.listing_width.unwrap_or(DEFAULT_BAR_WIDTH)
}

/// The Icons and List thumb's width: `scroll.listing.thumb_width`, else 4 (matching `bar_width`).
pub fn listing_thumb_width(look: &Look) -> f32 {
    look.slots
        .scroll
        .listing_thumb_width
        .unwrap_or(DEFAULT_BAR_WIDTH)
}

/// The Icons and List trough's opacity at rest: `scroll.listing.trough_opacity`, else 0.08.
pub fn listing_trough_opacity(look: &Look) -> f32 {
    look.slots.scroll.listing_trough_opacity.unwrap_or(0.08)
}

pub fn listing_hover_opacity(look: &Look) -> f32 {
    look.slots.scroll.listing_hover_opacity.unwrap_or(0.12)
}

pub fn listing_end_inset(look: &Look) -> f32 {
    look.slots.scroll.listing_end_inset.unwrap_or(4.0)
}

/// A Columns column's bar gap from its right edge: `scroll.column.right_gap`, else 6.
pub fn column_right_gap(look: &Look) -> f32 {
    look.slots.scroll.column_right_gap.unwrap_or(6.0)
}

/// A Columns column's bar gap from its top: `scroll.column.top_gap`, else 5.
pub fn column_top_gap(look: &Look) -> f32 {
    look.slots.scroll.column_top_gap.unwrap_or(5.0)
}

/// The column strip's sideways trough's gap above and below: `scroll.strip.margin`, else 3.
pub fn strip_margin(look: &Look) -> f32 {
    look.slots.scroll.strip_margin.unwrap_or(3.0)
}

pub fn strip_ring_width(look: &Look) -> f32 {
    look.slots.scroll.strip_ring_width.unwrap_or(1.0)
}

pub fn strip_ring_opacity(look: &Look) -> f32 {
    look.slots.scroll.strip_ring_opacity.unwrap_or(0.45)
}

/// The autoscroll anchor's diameter: `scroll.anchor.size`, else 22.
pub fn anchor_size(look: &Look) -> f32 {
    look.slots.scroll.anchor_size.unwrap_or(22.0)
}

/// The trough's corner radius: `scroll.trough.radius`, else 2 (Strata's `border-radius: 2px`).
pub fn trough_radius(look: &Look) -> f32 {
    look.slots.scroll.trough_radius.unwrap_or(2.0)
}

pub fn thumb_radius(look: &Look) -> f32 {
    look.slots.scroll.thumb_radius.unwrap_or(2.0)
}

/// The autoscroll anchor's border width: `scroll.anchor.border_width`, else 1.
pub fn anchor_border_width(look: &Look) -> f32 {
    look.slots.scroll.anchor_border_width.unwrap_or(1.0)
}

pub fn anchor_shadow_offset(look: &Look) -> f32 {
    look.slots.scroll.anchor_shadow_offset.unwrap_or(1.0)
}

/// The autoscroll anchor's shadow blur: `scroll.anchor.shadow_blur`, else 4.
pub fn anchor_shadow_blur(look: &Look) -> f32 {
    look.slots.scroll.anchor_shadow_blur.unwrap_or(4.0)
}

/// The autoscroll anchor's fill: `scroll.anchor.fill`, else the surface at 92 %.
pub fn anchor_fill(look: &Look) -> Color {
    look.slots
        .scroll
        .anchor_fill
        .unwrap_or_else(|| fade(look, look.kit.c.surface_raised, 0.92))
}

/// The autoscroll anchor's border: `scroll.anchor.border`, else the border colour at 90 %.
pub fn anchor_border(look: &Look) -> Color {
    look.slots
        .scroll
        .anchor_border
        .unwrap_or_else(|| fade(look, look.kit.c.std_border, 0.9))
}

/// The autoscroll anchor's shadow colour: the border colour at 35 %, as Strata's.
pub fn anchor_shadow(look: &Look) -> Color {
    fade(look, look.kit.c.std_border, 0.35)
}

/// The trough's colour at opacity `x` of [`trough_color`].
pub fn trough_at(look: &Look, x: f32) -> Color {
    fade(look, trough_color(look), x)
}

pub const TOKENS: &[Token] = &[
    Token::length("scroll.bar.width", bar_width, |l, v| {
        l.slots.scroll.bar_width = Some(v)
    }),
    Token::length(
        "scroll.bar.hover_width",
        |l| hover_width(l).unwrap_or(0.0),
        |l, v| l.slots.scroll.hover_width = (v > 0.0).then_some(v),
    ),
    Token::color("scroll.thumb.color", thumb_color, |l, v| {
        l.slots.scroll.thumb_color = Some(v)
    }),
    Token::length("scroll.thumb.min_length", thumb_min_length, |l, v| {
        l.slots.scroll.thumb_min_length = Some(v)
    }),
    Token::color("scroll.trough.color", trough_color, |l, v| {
        l.slots.scroll.trough_color = Some(v)
    }),
    Token::number("scroll.trough.opacity", 0.0, 1.0, trough_opacity, |l, v| {
        l.slots.scroll.trough_opacity = Some(v)
    }),
    Token::number(
        "scroll.trough.hover_opacity",
        0.0,
        1.0,
        trough_hover_opacity,
        |l, v| l.slots.scroll.trough_hover_opacity = Some(v),
    ),
    Token::length("scroll.listing.width", listing_width, |l, v| {
        l.slots.scroll.listing_width = Some(v)
    }),
    Token::length("scroll.listing.thumb_width", listing_thumb_width, |l, v| {
        l.slots.scroll.listing_thumb_width = Some(v)
    }),
    Token::number(
        "scroll.listing.trough_opacity",
        0.0,
        1.0,
        listing_trough_opacity,
        |l, v| l.slots.scroll.listing_trough_opacity = Some(v),
    ),
    Token::number(
        "scroll.listing.hover_opacity",
        0.0,
        1.0,
        listing_hover_opacity,
        |l, v| l.slots.scroll.listing_hover_opacity = Some(v),
    ),
    Token::length("scroll.listing.end_inset", listing_end_inset, |l, v| {
        l.slots.scroll.listing_end_inset = Some(v)
    }),
    Token::length("scroll.column.right_gap", column_right_gap, |l, v| {
        l.slots.scroll.column_right_gap = Some(v)
    }),
    Token::length("scroll.column.top_gap", column_top_gap, |l, v| {
        l.slots.scroll.column_top_gap = Some(v)
    }),
    Token::length("scroll.strip.margin", strip_margin, |l, v| {
        l.slots.scroll.strip_margin = Some(v)
    }),
    Token::number(
        "scroll.strip.ring_opacity",
        0.0,
        1.0,
        strip_ring_opacity,
        |l, v| l.slots.scroll.strip_ring_opacity = Some(v),
    ),
    Token::length("scroll.strip.ring_width", strip_ring_width, |l, v| {
        l.slots.scroll.strip_ring_width = Some(v)
    }),
    Token::length("scroll.anchor.size", anchor_size, |l, v| {
        l.slots.scroll.anchor_size = Some(v)
    }),
    Token::length("scroll.trough.radius", trough_radius, |l, v| {
        l.slots.scroll.trough_radius = Some(v)
    }),
    Token::length("scroll.thumb.radius", thumb_radius, |l, v| {
        l.slots.scroll.thumb_radius = Some(v)
    }),
    Token::length("scroll.anchor.border_width", anchor_border_width, |l, v| {
        l.slots.scroll.anchor_border_width = Some(v)
    }),
    Token::length(
        "scroll.anchor.shadow_offset",
        anchor_shadow_offset,
        |l, v| l.slots.scroll.anchor_shadow_offset = Some(v),
    ),
    Token::length("scroll.anchor.shadow_blur", anchor_shadow_blur, |l, v| {
        l.slots.scroll.anchor_shadow_blur = Some(v)
    }),
    Token::color("scroll.anchor.fill", anchor_fill, |l, v| {
        l.slots.scroll.anchor_fill = Some(v)
    }),
    Token::color("scroll.anchor.border", anchor_border, |l, v| {
        l.slots.scroll.anchor_border = Some(v)
    }),
];
