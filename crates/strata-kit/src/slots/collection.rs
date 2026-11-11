//! The `collection.*` slot tokens (for example `collection.row`) that the collection element
//! reads, and the views built on it through it.
//!
//! Work package: C1e, `docs/work/phase-2-composites/C1e-collection-element.md`. W1c added the
//! first tokens so C1e can land against them. C1e owns this file: add tokens beside them,
//! following `slots/mod.rs`. Keep their names, since slot names are API.

use rune::Color;

use crate::look::Look;
use crate::tokens::Token;

/// The collection element's slot tokens. `None` inherits, or takes the default its reader
/// names.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Collection {
    pub row_inset: Option<f32>,
    pub row_radius: Option<f32>,
    pub cut_opacity: Option<f32>,
    pub drag_opacity: Option<f32>,
    pub peek_fill: Option<Color>,
}

/// A row's horizontal margin inside the list, on each side: `collection.row.inset`, else 5.
/// That is Strata's `.file-list row` margin (`src/style.css`, `upstream/main` 35ee677ab), which
/// no core token matches.
///
/// Used by: C1e (and C2, C3, C4 through it).
pub fn row_inset(look: &Look) -> f32 {
    look.slots.collection.row_inset.unwrap_or(5.0)
}

/// A row's corner radius: `collection.row.radius`, else `radius.md`.
///
/// Used by: C1e (and C2, C3, C4 through it).
pub fn row_radius(look: &Look) -> f32 {
    look.slots
        .collection
        .row_radius
        .unwrap_or(look.kit.radius.md)
}

/// The opacity of a cut entry's row, 0 to 1: `collection.cut.opacity`, else Strata's 0.65.
///
/// Used by: C1e (and C2, C3, C4 through it).
pub fn cut_opacity(look: &Look) -> f32 {
    look.slots.collection.cut_opacity.unwrap_or(0.65)
}

/// The opacity of a row being dragged, 0 to 1: `collection.drag.opacity`, else Strata's 0.48.
///
/// Used by: C1e (and C2, C3, C4 through it).
pub fn drag_opacity(look: &Look) -> f32 {
    look.slots.collection.drag_opacity.unwrap_or(0.48)
}

/// The fill of the row a folder peek is open from: `collection.peek.fill`, else the accent at
/// Strata's 0.15 of its own opacity (`.peek-open .file-list .file-row.peek-anchor`,
/// `src/style.css`). A selected row keeps the selection's fill.
///
/// Used by: C1e (and C22 through it).
pub fn peek_fill(look: &Look) -> Color {
    look.slots.collection.peek_fill.unwrap_or_else(|| {
        let accent = look.kit.c.bg_accent;
        look.kit.alpha(accent, accent.a * 0.15)
    })
}

pub const TOKENS: &[Token] = &[
    Token::length("collection.row.inset", row_inset, |l, v| {
        l.slots.collection.row_inset = Some(v)
    }),
    Token::length("collection.row.radius", row_radius, |l, v| {
        l.slots.collection.row_radius = Some(v)
    }),
    Token::number("collection.cut.opacity", 0.0, 1.0, cut_opacity, |l, v| {
        l.slots.collection.cut_opacity = Some(v)
    }),
    Token::number("collection.drag.opacity", 0.0, 1.0, drag_opacity, |l, v| {
        l.slots.collection.drag_opacity = Some(v)
    }),
    Token::color("collection.peek.fill", peek_fill, |l, v| {
        l.slots.collection.peek_fill = Some(v)
    }),
];
