// SPDX-License-Identifier: MIT

use rune::Color;

use crate::look::Look;
use crate::tokens::Token;

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Collection {
    pub row_inset: Option<f32>,
    pub row_radius: Option<f32>,
    pub cut_opacity: Option<f32>,
    pub drag_opacity: Option<f32>,
    pub file_icon_opacity: Option<f32>,
    pub peek_fill: Option<Color>,
    pub active_path_fill: Option<Color>,
    pub active_parent_fill: Option<Color>,
}

pub fn row_inset(look: &Look) -> f32 {
    look.slots.collection.row_inset.unwrap_or(5.0)
}

pub fn row_radius(look: &Look) -> f32 {
    look.slots
        .collection
        .row_radius
        .unwrap_or(look.kit.radius.md)
}

pub fn cut_opacity(look: &Look) -> f32 {
    look.slots.collection.cut_opacity.unwrap_or(0.65)
}

pub fn drag_opacity(look: &Look) -> f32 {
    look.slots.collection.drag_opacity.unwrap_or(0.48)
}

pub fn file_icon_opacity(look: &Look) -> f32 {
    look.slots.collection.file_icon_opacity.unwrap_or(0.72)
}

pub fn peek_fill(look: &Look) -> Color {
    look.slots.collection.peek_fill.unwrap_or_else(|| {
        let accent = look.kit.c.bg_accent;
        look.kit.alpha(accent, accent.a * 0.15)
    })
}

pub fn active_path_fill(look: &Look) -> Color {
    look.slots.collection.active_path_fill.unwrap_or_else(|| {
        let accent = look.kit.c.bg_accent;
        look.kit.alpha(accent, accent.a * 0.24)
    })
}

pub fn active_parent_fill(look: &Look) -> Color {
    look.slots.collection.active_parent_fill.unwrap_or_else(|| {
        let accent = look.kit.c.bg_accent;
        look.kit.alpha(accent, accent.a * 0.28)
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
    Token::number(
        "collection.file_icon.opacity",
        0.0,
        1.0,
        file_icon_opacity,
        |l, v| l.slots.collection.file_icon_opacity = Some(v),
    ),
    Token::color("collection.peek.fill", peek_fill, |l, v| {
        l.slots.collection.peek_fill = Some(v)
    }),
    Token::color("collection.active_path.fill", active_path_fill, |l, v| {
        l.slots.collection.active_path_fill = Some(v)
    }),
    Token::color(
        "collection.active_parent.fill",
        active_parent_fill,
        |l, v| l.slots.collection.active_parent_fill = Some(v),
    ),
];
