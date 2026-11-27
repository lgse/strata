// SPDX-License-Identifier: MIT

use rune::Color;

use crate::density;
use crate::look::Look;
use crate::tokens::Token;

/// IconsGrid's slot tokens. `None` inherits.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Icons {
    pub grid_pad: Option<f32>,
    pub tile_margin: Option<f32>,
    pub card_pad_x: Option<f32>,
    pub card_pad_y: Option<f32>,
    pub grid_max_columns: Option<f32>,
    pub frame_pad: Option<f32>,
    pub frame_radius: Option<f32>,
    pub frame_icon: Option<Color>,
    pub frame_selected_fill: Option<Color>,
    pub frame_selected_ring: Option<Color>,
    pub frame_selected_hover_fill: Option<Color>,
    pub frame_hover_fill: Option<Color>,
    pub frame_drop_fill: Option<Color>,
    pub frame_focus_ring: Option<Color>,
    pub caption_gap: Option<f32>,
    pub caption_pad: Option<f32>,
    pub caption_radius: Option<f32>,
    pub caption_chars: Option<f32>,
    pub caption_hover_fill: Option<Color>,
    pub caption_selected_fill: Option<Color>,
    pub caption_selected_text: Option<Color>,
}

pub fn grid_pad(look: &Look) -> f32 {
    let s = &look.slots.icons;
    s.grid_pad
        .unwrap_or_else(|| density::icons_spacing(look.content.pad).grid_pad)
}

/// The space around each tile: `icons.tile.margin`, else Content density's (1 to 3).
pub fn tile_margin(look: &Look) -> f32 {
    let s = &look.slots.icons;
    s.tile_margin
        .unwrap_or_else(|| density::icons_spacing(look.content.pad).child_margin)
}

/// A card's padding left and right: `icons.card.pad_x`, else Content density's (4 to 7).
pub fn card_pad_x(look: &Look) -> f32 {
    let s = &look.slots.icons;
    s.card_pad_x
        .unwrap_or_else(|| density::icons_spacing(look.content.pad).card_h)
}

/// A card's padding above and below: `icons.card.pad_y`, else Content density's (2 to 5).
pub fn card_pad_y(look: &Look) -> f32 {
    let s = &look.slots.icons;
    s.card_pad_y
        .unwrap_or_else(|| density::icons_spacing(look.content.pad).card_v)
}

pub fn grid_max_columns(look: &Look) -> f32 {
    look.slots.icons.grid_max_columns.unwrap_or(
        if look.content.pad < f32::from(density::CONTENT.airy) {
            20.0
        } else {
            16.0
        },
    )
}

pub fn frame_pad(look: &Look) -> f32 {
    look.slots.icons.frame_pad.unwrap_or(6.0)
}

pub fn frame_radius(look: &Look) -> f32 {
    look.slots
        .icons
        .frame_radius
        .unwrap_or_else(|| look.round(6.0))
}

pub fn frame_icon(look: &Look) -> Color {
    look.slots
        .icons
        .frame_icon
        .unwrap_or(look.kit.c.icon_accent)
}

/// A selected tile's frame fill: `icons.frame.selected_fill`, else the accent at 22 %.
pub fn frame_selected_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .icons
        .frame_selected_fill
        .unwrap_or_else(|| k.alpha(k.c.bg_accent, 0.22))
}

/// A selected tile's frame outline: `icons.frame.selected_ring`, else the accent at 60 %.
pub fn frame_selected_ring(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .icons
        .frame_selected_ring
        .unwrap_or_else(|| k.alpha(k.c.bg_accent, 0.6))
}

pub fn frame_selected_hover_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .icons
        .frame_selected_hover_fill
        .unwrap_or_else(|| {
            let fill = k.alpha(k.c.bg_accent, 0.22);
            k.alpha(fill, (fill.a * 30.0 / 22.0).min(1.0))
        })
}

/// A hovered tile's frame fill: `icons.frame.hover_fill`, else the accent at 15 %.
pub fn frame_hover_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .icons
        .frame_hover_fill
        .unwrap_or_else(|| k.alpha(k.c.bg_accent, 0.15))
}

pub fn frame_drop_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .icons
        .frame_drop_fill
        .unwrap_or_else(|| k.alpha(look.fill.highlight, look.fill.highlight.a * 0.82))
}

pub fn frame_focus_ring(look: &Look) -> Color {
    look.slots
        .icons
        .frame_focus_ring
        .unwrap_or(look.kit.c.bg_accent)
}

/// The space between the frame and the caption: `icons.caption.gap`, else Strata's 4.
pub fn caption_gap(look: &Look) -> f32 {
    look.slots.icons.caption_gap.unwrap_or(4.0)
}

pub fn caption_pad(look: &Look) -> f32 {
    look.slots.icons.caption_pad.unwrap_or(4.0)
}

pub fn caption_radius(look: &Look) -> f32 {
    look.slots
        .icons
        .caption_radius
        .unwrap_or_else(|| look.round(4.0))
}

pub fn caption_chars(look: &Look) -> f32 {
    look.slots.icons.caption_chars.unwrap_or(12.0)
}

/// A hovered tile's name fill: `icons.caption.hover_fill`, else the accent at 10 %.
pub fn caption_hover_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .icons
        .caption_hover_fill
        .unwrap_or_else(|| k.alpha(k.c.bg_accent, 0.10))
}

/// A selected tile's name fill: `icons.caption.selected_fill`, else the accent.
pub fn caption_selected_fill(look: &Look) -> Color {
    look.slots
        .icons
        .caption_selected_fill
        .unwrap_or(look.kit.c.bg_accent)
}

pub fn caption_selected_text(look: &Look) -> Color {
    look.slots
        .icons
        .caption_selected_text
        .unwrap_or(look.kit.c.bg_base)
}

pub const TOKENS: &[Token] = &[
    Token::length("icons.grid.pad", grid_pad, |l, v| {
        l.slots.icons.grid_pad = Some(v)
    }),
    Token::number(
        "icons.grid.max_columns",
        1.0,
        64.0,
        grid_max_columns,
        |l, v| l.slots.icons.grid_max_columns = Some(v),
    ),
    Token::length("icons.tile.margin", tile_margin, |l, v| {
        l.slots.icons.tile_margin = Some(v)
    }),
    Token::length("icons.card.pad_x", card_pad_x, |l, v| {
        l.slots.icons.card_pad_x = Some(v)
    }),
    Token::length("icons.card.pad_y", card_pad_y, |l, v| {
        l.slots.icons.card_pad_y = Some(v)
    }),
    Token::length("icons.frame.pad", frame_pad, |l, v| {
        l.slots.icons.frame_pad = Some(v)
    }),
    Token::length("icons.frame.radius", frame_radius, |l, v| {
        l.slots.icons.frame_radius = Some(v)
    }),
    Token::color("icons.frame.icon", frame_icon, |l, v| {
        l.slots.icons.frame_icon = Some(v)
    }),
    Token::color("icons.frame.selected_fill", frame_selected_fill, |l, v| {
        l.slots.icons.frame_selected_fill = Some(v)
    }),
    Token::color("icons.frame.selected_ring", frame_selected_ring, |l, v| {
        l.slots.icons.frame_selected_ring = Some(v)
    }),
    Token::color(
        "icons.frame.selected_hover_fill",
        frame_selected_hover_fill,
        |l, v| l.slots.icons.frame_selected_hover_fill = Some(v),
    ),
    Token::color("icons.frame.hover_fill", frame_hover_fill, |l, v| {
        l.slots.icons.frame_hover_fill = Some(v)
    }),
    Token::color("icons.frame.drop_fill", frame_drop_fill, |l, v| {
        l.slots.icons.frame_drop_fill = Some(v)
    }),
    Token::color("icons.frame.focus_ring", frame_focus_ring, |l, v| {
        l.slots.icons.frame_focus_ring = Some(v)
    }),
    Token::length("icons.caption.gap", caption_gap, |l, v| {
        l.slots.icons.caption_gap = Some(v)
    }),
    Token::length("icons.caption.pad", caption_pad, |l, v| {
        l.slots.icons.caption_pad = Some(v)
    }),
    Token::length("icons.caption.radius", caption_radius, |l, v| {
        l.slots.icons.caption_radius = Some(v)
    }),
    Token::number("icons.caption.chars", 1.0, 64.0, caption_chars, |l, v| {
        l.slots.icons.caption_chars = Some(v)
    }),
    Token::color("icons.caption.hover_fill", caption_hover_fill, |l, v| {
        l.slots.icons.caption_hover_fill = Some(v)
    }),
    Token::color(
        "icons.caption.selected_fill",
        caption_selected_fill,
        |l, v| l.slots.icons.caption_selected_fill = Some(v),
    ),
    Token::color(
        "icons.caption.selected_text",
        caption_selected_text,
        |l, v| l.slots.icons.caption_selected_text = Some(v),
    ),
];
