// SPDX-License-Identifier: MIT

use crate::look::{Content, Frame, Look, RowHeights};
use crate::typography;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scale {
    pub compact: u8,
    pub airy: u8,
    pub max: u8,
}

/// A density's named snap points. Any other value is Custom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    Compact,
    Airy,
}

impl Scale {
    /// `px` clamped to the scale: a saved 200 loads as the end.
    pub fn clamp(&self, px: u8) -> u8 {
        px.min(self.max)
    }

    /// `px` moved `delta` px, clamped to the scale, as a slider's arrow keys move it.
    pub fn stepped(&self, px: u8, delta: i32) -> u8 {
        (i32::from(self.clamp(px)) + delta).clamp(0, i32::from(self.max)) as u8
    }

    pub fn preset(&self, px: u8) -> Option<Preset> {
        match self.clamp(px) {
            px if px == self.compact => Some(Preset::Compact),
            px if px == self.airy => Some(Preset::Airy),
            _ => None,
        }
    }
}

/// Frame density: the bars' extra inset, Compact 0 (the default), Airy 4, up to 12 px.
pub const FRAME: Scale = Scale {
    compact: 0,
    airy: 4,
    max: 12,
};

pub const CONTENT: Scale = Scale {
    compact: 4,
    airy: 8,
    max: 16,
};

pub const PALETTE_PAD: f32 = 10.0;
/// The rule under each search palette result (`.search-result`'s bottom border).
pub const PALETTE_RULE: f32 = 1.0;
pub const PALETTE_THUMB: f32 = 32.0;
/// The gap between a search palette result's name and path lines.
pub const PALETTE_LINE_GAP: f32 = 2.0;

pub fn between(pad: f32, compact: f32, airy: f32) -> f32 {
    let (c, a) = (f32::from(CONTENT.compact), f32::from(CONTENT.airy));
    let step = (pad - c) / (a - c);
    (compact + (airy - compact) * step).max(0.0).round()
}

/// IconsGrid's spacing at content pad `pad`, for C3 to read or turn into slot tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconsSpacing {
    /// Around the whole grid.
    pub grid_pad: f32,
    /// Around each tile.
    pub child_margin: f32,
    /// Inside a tile's card, above and below.
    pub card_v: f32,
    /// Inside a tile's card, left and right.
    pub card_h: f32,
}

/// Strata's Icons spacing at content pad `pad`.
pub fn icons_spacing(pad: f32) -> IconsSpacing {
    IconsSpacing {
        grid_pad: between(pad, 0.0, 5.0),
        child_margin: between(pad, 1.0, 3.0),
        card_v: between(pad, 2.0, 5.0),
        card_h: between(pad, 4.0, 7.0),
    }
}

pub fn frame(inset: u8, factor: f32) -> Frame {
    let control = (24.0 * factor).round().max(1.0);
    Frame {
        inset: f32::from(FRAME.clamp(inset)),
        bar: (control + 8.0).max(36.0),
        control,
    }
}

pub fn rows(look: &Look, pad: f32) -> RowHeights {
    let label = typography::line_of(look.kit.ty.ui_compact);
    let path = typography::line_of(look.kit.ty.small);
    let content = label.max(look.icon_size.md);
    let file_row = between(pad, 26.0, 42.0).max(content + 2.0 * pad);
    let sidebar = look.frame.control.max(content) + 2.0 * look.frame.inset;
    let palette =
        PALETTE_THUMB.max(label + PALETTE_LINE_GAP + path) + 2.0 * PALETTE_PAD + PALETTE_RULE;
    RowHeights {
        list: file_row,
        columns: file_row,
        sidebar,
        palette,
    }
}

pub(crate) fn apply(look: &mut Look, frame_px: u8, content_px: u8, factor: f32) {
    look.frame = frame(frame_px, factor);
    let pad = f32::from(CONTENT.clamp(content_px));
    look.content = Content { pad };
    look.row = rows(look, pad);
}
