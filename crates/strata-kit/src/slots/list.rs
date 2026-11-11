//! The `list.*` slot tokens (for example `list.row`) that ListTable reads.
//!
//! Work package: C4, `docs/work/phase-3-views/C4-list-table.md`.
//!
//! W2 seeded `list.row.height` so K4 has a slot to override before C4 lands. C4 owns this
//! file: add tokens beside it, following `slots/mod.rs`. Keep the seed's name, since slot
//! names are API.

use crate::look::Look;
use crate::tokens::Token;

/// ListTable's slot tokens. `None` inherits.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct List {
    pub row_height: Option<f32>,
    pub heading_height: Option<f32>,
    pub heading_pad: Option<f32>,
    pub cell_pad: Option<f32>,
    pub reorder_width: Option<f32>,
}

/// A ListTable row's height: `list.row.height`, else `row.list`.
pub fn row_height(look: &Look) -> f32 {
    look.slots.list.row_height.unwrap_or(look.row.list)
}

/// The heading strip's height: `list.heading.height`, else `row.list` (Strata's
/// `.list-headings` is 28 px, its rows' height at the default size). Slot tokens inherit from
/// core tokens only, so it doesn't follow `list.row.height`.
pub fn heading_height(look: &Look) -> f32 {
    look.slots.list.heading_height.unwrap_or(look.row.list)
}

/// The space either side of a heading's title: `list.heading.pad`, else `space.md` (Strata's
/// `.list-heading-button` padding, 12 px).
pub fn heading_pad(look: &Look) -> f32 {
    look.slots.list.heading_pad.unwrap_or(look.space.md)
}

/// The space either side of a cell's text: `list.cell.pad`, else `space.sm`.
pub fn cell_pad(look: &Look) -> f32 {
    look.slots.list.cell_pad.unwrap_or(look.space.sm)
}

/// The width of the line that marks where a dragged heading drops: `list.reorder.width`, else
/// Strata's 3 px inset.
pub fn reorder_width(look: &Look) -> f32 {
    look.slots.list.reorder_width.unwrap_or(3.0)
}

pub const TOKENS: &[Token] = &[
    Token::length("list.row.height", row_height, |l, v| {
        l.slots.list.row_height = Some(v)
    }),
    Token::length("list.heading.height", heading_height, |l, v| {
        l.slots.list.heading_height = Some(v)
    }),
    Token::length("list.heading.pad", heading_pad, |l, v| {
        l.slots.list.heading_pad = Some(v)
    }),
    Token::length("list.cell.pad", cell_pad, |l, v| {
        l.slots.list.cell_pad = Some(v)
    }),
    Token::length("list.reorder.width", reorder_width, |l, v| {
        l.slots.list.reorder_width = Some(v)
    }),
];
