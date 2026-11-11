//! The `columns.*` slot tokens that ColumnsView (C2) reads: its column headers, the line
//! between columns, its width caption, the wash over the column a file drag would drop into,
//! and the rows' size badge.
//!
//! Work package: C2, `docs/work/phase-3-views/C2-columns-view.md`, and its follow-up C2b (the
//! badge, the title's weight). C2 owns this file: add tokens beside these, following
//! `slots/mod.rs`. Keep their names, since slot names are API.
//!
//! The defaults are Strata's (`src/style.css`, `upstream/main` 35ee677ab): `.column-header`,
//! `.active-column .column-header`, `.column-resize-handle`, `.columns > .drop-destination
//! .column-drop-veil` and `.file-size`. The colours come from the Kit, as K1 maps Strata's:
//! `@strata_accent` is `bg_accent`, `@strata_border` `std_border`, `@strata_surface`
//! `surface_raised`. No token
//! here inherits from a core length, so overriding one moves only the columns.

use rune::{Color, TextStyle};

use crate::look::Look;
use crate::tokens::Token;

/// The column view's slot tokens. `None` takes the default each reader names.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Columns {
    pub header_height: Option<f32>,
    pub header_inset: Option<f32>,
    pub header_indicator: Option<f32>,
    pub header_title_opacity: Option<f32>,
    pub header_title_weight: Option<f32>,
    pub header_fill: Option<Color>,
    pub edge_width: Option<f32>,
    pub edge_opacity: Option<f32>,
    pub drop_opacity: Option<f32>,
    pub drop_ring: Option<f32>,
    pub caption_border: Option<f32>,
    pub badge_radius: Option<f32>,
    pub badge_pad: Option<f32>,
    pub badge_inset: Option<f32>,
}

/// A column header's height: `columns.header.height`, else 41 (Strata's 31 pt minimum, its
/// 2 and 4 pt padding, the 3 pt indicator above and the 1 pt line below).
pub fn header_height(look: &Look) -> f32 {
    look.slots.columns.header_height.unwrap_or(41.0)
}

/// The title's inset from the header's leading edge: `columns.header.inset`, else 12.
pub fn header_inset(look: &Look) -> f32 {
    look.slots.columns.header_inset.unwrap_or(12.0)
}

/// The accent band along the top of the active column's header: `columns.header.indicator`,
/// else 3.
pub fn header_indicator(look: &Look) -> f32 {
    look.slots.columns.header_indicator.unwrap_or(3.0)
}

/// An inactive column's title colour as a share of the accent: `columns.header.title_opacity`,
/// else 0.58. The active column's title takes the text colour.
pub fn header_title_opacity(look: &Look) -> f32 {
    look.slots.columns.header_title_opacity.unwrap_or(0.58)
}

/// The header title's font weight: `columns.header.title_weight`, else 700 (Strata's bold;
/// the kit's type scale stops at medium).
pub fn header_title_weight(look: &Look) -> f32 {
    look.slots.columns.header_title_weight.unwrap_or(700.0)
}

/// The header title's text style: the kit's `ui_medium` at [`header_title_weight`].
pub fn header_title_font(look: &Look) -> TextStyle {
    TextStyle {
        weight: header_title_weight(look),
        ..look.kit.ty.ui_medium
    }
}

/// The active column header's fill: `columns.header.fill`, else the raised surface.
pub fn header_fill(look: &Look) -> Color {
    look.slots
        .columns
        .header_fill
        .unwrap_or(look.kit.c.surface_raised)
}

/// The line along a column's trailing edge, which its resize edge sits on, and under its
/// header: `columns.edge.width`, else 1.
pub fn edge_width(look: &Look) -> f32 {
    look.slots.columns.edge_width.unwrap_or(1.0)
}

/// That line's opacity of the border colour: `columns.edge.opacity`, else 0.6.
pub fn edge_opacity(look: &Look) -> f32 {
    look.slots.columns.edge_opacity.unwrap_or(0.6)
}

/// The wash over the column a held file drag would drop into, as a share of the accent:
/// `columns.drop.opacity`, else 0.18.
pub fn drop_opacity(look: &Look) -> f32 {
    look.slots.columns.drop_opacity.unwrap_or(0.18)
}

/// The accent ring inside that column: `columns.drop.ring`, else 2.
pub fn drop_ring(look: &Look) -> f32 {
    look.slots.columns.drop_ring.unwrap_or(2.0)
}

/// The border of the "Column width" caption over a column's edge: `columns.caption.border`,
/// else 1 (as C9's divider caption draws).
pub fn caption_border(look: &Look) -> f32 {
    look.slots.columns.caption_border.unwrap_or(1.0)
}

/// The corner radius of a row's size badge: `columns.badge.radius`, else 5.
pub fn badge_radius(look: &Look) -> f32 {
    look.slots.columns.badge_radius.unwrap_or(5.0)
}

/// The size badge's padding either side of its text: `columns.badge.pad`, else 7.
pub fn badge_pad(look: &Look) -> f32 {
    look.slots.columns.badge_pad.unwrap_or(7.0)
}

/// The size badge's gap above and below it, inside its row: `columns.badge.inset`, else 2 (as
/// Strata's chip sits inside its row's highlight).
pub fn badge_inset(look: &Look) -> f32 {
    look.slots.columns.badge_inset.unwrap_or(2.0)
}

/// `c` at `x` of its own opacity, so a translucent theme colour stays translucent.
pub fn fade(look: &Look, c: Color, x: f32) -> Color {
    look.kit.alpha(c, c.a * x)
}

/// The slot's entries in the registry.
pub const TOKENS: &[Token] = &[
    Token::length("columns.header.height", header_height, |l, v| {
        l.slots.columns.header_height = Some(v)
    }),
    Token::length("columns.header.inset", header_inset, |l, v| {
        l.slots.columns.header_inset = Some(v)
    }),
    Token::length("columns.header.indicator", header_indicator, |l, v| {
        l.slots.columns.header_indicator = Some(v)
    }),
    Token::number(
        "columns.header.title_opacity",
        0.0,
        1.0,
        header_title_opacity,
        |l, v| l.slots.columns.header_title_opacity = Some(v),
    ),
    Token::number(
        "columns.header.title_weight",
        100.0,
        900.0,
        header_title_weight,
        |l, v| l.slots.columns.header_title_weight = Some(v),
    ),
    Token::color("columns.header.fill", header_fill, |l, v| {
        l.slots.columns.header_fill = Some(v)
    }),
    Token::length("columns.edge.width", edge_width, |l, v| {
        l.slots.columns.edge_width = Some(v)
    }),
    Token::number("columns.edge.opacity", 0.0, 1.0, edge_opacity, |l, v| {
        l.slots.columns.edge_opacity = Some(v)
    }),
    Token::number("columns.drop.opacity", 0.0, 1.0, drop_opacity, |l, v| {
        l.slots.columns.drop_opacity = Some(v)
    }),
    Token::length("columns.drop.ring", drop_ring, |l, v| {
        l.slots.columns.drop_ring = Some(v)
    }),
    Token::length("columns.caption.border", caption_border, |l, v| {
        l.slots.columns.caption_border = Some(v)
    }),
    Token::length("columns.badge.radius", badge_radius, |l, v| {
        l.slots.columns.badge_radius = Some(v)
    }),
    Token::length("columns.badge.pad", badge_pad, |l, v| {
        l.slots.columns.badge_pad = Some(v)
    }),
    Token::length("columns.badge.inset", badge_inset, |l, v| {
        l.slots.columns.badge_inset = Some(v)
    }),
];
