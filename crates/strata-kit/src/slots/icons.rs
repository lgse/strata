//! The `icons.*` slot tokens (for example `icons.frame.pad`) that IconsGrid reads.
//!
//! Work package: C3, `docs/work/phase-3-views/C3-icons-grid.md`. C3 owns this file. Slot names
//! are API: renaming one breaks users' override files (plan K4).
//!
//! Every default follows the rest of the [`Look`]: the spacing from Content density through
//! [`density::icons_spacing`], colours from `kit.c` and `fill`. The plain numbers are Strata's
//! `.icons-card*` and `.file-icons` rules (`src/style.css`, `src/ui/icons_cell.rs`,
//! lgse/strata `upstream/main` 35ee677ab), which no core token says. The thumbnail size itself
//! is the look's `tiles` ladder and the host's choice, not a slot token.

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

/// The space at the grid's left and right: `icons.grid.pad`, else Content density's (0 at
/// Compact, 5 at Airy). Strata pads the top and bottom too; C3 leaves those out (a deviation).
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

/// The most tiles a row holds: `icons.grid.max_columns`, else Strata's 20 below Airy Content
/// density and 16 from it on (`density_icons_columns`).
pub fn grid_max_columns(look: &Look) -> f32 {
    look.slots.icons.grid_max_columns.unwrap_or(
        if look.content.pad < f32::from(density::CONTENT.airy) {
            20.0
        } else {
            16.0
        },
    )
}

/// The space between the thumbnail slot and its frame's edge: `icons.frame.pad`, else
/// Strata's 6 (`ICONS_CARD_ICON_PADDING`).
pub fn frame_pad(look: &Look) -> f32 {
    look.slots.icons.frame_pad.unwrap_or(6.0)
}

/// The thumbnail frame's corner radius: `icons.frame.radius`, else Strata's 6 at the current
/// roundness.
pub fn frame_radius(look: &Look) -> f32 {
    look.slots
        .icons
        .frame_radius
        .unwrap_or_else(|| look.round(6.0))
}

/// The placeholder icon's colour: `icons.frame.icon`, else `kit.c.icon_accent` (Strata tints
/// its Icons fallback icons with the accent).
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

/// A selected tile's frame fill under the pointer: `icons.frame.selected_hover_fill`, else
/// the default selected fill (the accent at 22 %) deepened as Strata deepens it, to 30 %. It
/// derives from the accent, not from `icons.frame.selected_fill`: a slot token inherits only
/// core tokens, so setting one slot token moves no other.
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

/// The frame fill of a folder tile a drag would drop on: `icons.frame.drop_fill`, else the
/// highlight fill at 82 %.
pub fn frame_drop_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .icons
        .frame_drop_fill
        .unwrap_or_else(|| k.alpha(look.fill.highlight, look.fill.highlight.a * 0.82))
}

/// The keyboard cursor's outline on its tile's frame, also a drop target's:
/// `icons.frame.focus_ring`, else the accent.
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

/// The name's padding inside its caption box: `icons.caption.pad`, else Strata's 4 left and
/// right (and half of it above and below).
pub fn caption_pad(look: &Look) -> f32 {
    look.slots.icons.caption_pad.unwrap_or(4.0)
}

/// The name box's corner radius: `icons.caption.radius`, else Strata's 4 at the current
/// roundness.
pub fn caption_radius(look: &Look) -> f32 {
    look.slots
        .icons
        .caption_radius
        .unwrap_or_else(|| look.round(4.0))
}

/// How many average characters wide a caption is at least, which widens tiles with the text
/// size: `icons.caption.chars`, else Strata's 12 (`ICONS_CARD_LABEL_CHARS`).
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

/// A selected tile's name colour: `icons.caption.selected_text`, else the base background,
/// as Strata draws `@strata_bg` on the accent.
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
