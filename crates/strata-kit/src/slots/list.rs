// SPDX-License-Identifier: MIT

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

pub fn heading_height(look: &Look) -> f32 {
    look.slots.list.heading_height.unwrap_or(look.row.list)
}

pub fn heading_pad(look: &Look) -> f32 {
    look.slots.list.heading_pad.unwrap_or(look.space.md)
}

/// The space either side of a cell's text: `list.cell.pad`, else `space.sm`.
pub fn cell_pad(look: &Look) -> f32 {
    look.slots.list.cell_pad.unwrap_or(look.space.sm)
}

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
