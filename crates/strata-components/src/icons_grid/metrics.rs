//! A tile's sizes, from the look and the tile size: pure arithmetic over tokens.
//!
//! After Strata's `icons_cell.rs` and `.icons-card*` rules (lgse/strata `upstream/main`
//! 35ee677ab): a fixed thumbnail slot inside a padded frame, then a gap, a two-line name and one
//! details line, inside a padded card with a margin around it. Strata reserves a fixed height
//! for the name and the details, so nothing a tile shows changes its size.

use std::hash::{Hash, Hasher};

use rune::TextStyle;
use strata_kit::look::Look;
use strata_kit::slots::icons as tokens;
use strata_kit::typography::line_of;

use crate::contract::Rect;

/// The narrowest card, in points: Strata's fallback column (120) less its card spacing.
pub const MIN_CARD: f32 = 116.0;

/// The space Strata leaves between cards across a row (`ICONS_CARD_SPACING`), in points.
pub const COLUMN_SPACING: f32 = 4.0;

/// Text whose average glyph advance stands in for Pango's approximate character width, which
/// sizes Strata's caption (`preferred_caption_width`).
const SAMPLE: &str = "abcdefghijklmnopqrstuvwxyz";

/// A tile's sizes, in points.
///
/// Used by: C3, C20d, hosts (an anchor inside a tile).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TileMetrics {
    /// The thumbnail slot's side: the tile size, within the look's ladder.
    pub slot: f32,
    /// The frame around the slot: the slot and its padding on each side.
    pub frame: f32,
    /// The padding between the slot and the frame's edge.
    pub frame_pad: f32,
    /// The space around each tile.
    pub margin: f32,
    /// The card's padding left and right.
    pub card_x: f32,
    /// The card's padding above and below.
    pub card_y: f32,
    /// The grid's left and right padding.
    pub grid_pad: f32,
    /// The space between the frame and the name.
    pub caption_gap: f32,
    /// The name box's padding left and right (half of it above and below).
    pub caption_pad: f32,
    /// The name's font.
    pub name_font: TextStyle,
    /// A line of the name.
    pub name_line: f32,
    /// How many lines the name takes at most (0: no name).
    pub name_lines: usize,
    /// The details line's font.
    pub details_font: TextStyle,
    /// The details line's height (0 without details).
    pub details_line: f32,
    /// The narrowest cell: a card no narrower than its frame, its caption's characters and
    /// [`MIN_CARD`], and the space between cards.
    pub min_cell: f32,
    /// A tile's height: every line of the grid is this tall.
    pub height: f32,
    /// The most tiles a row holds.
    pub max_columns: usize,
}

impl TileMetrics {
    /// A tile of `size` points (held to the look's ladder), its name over at most `lines`
    /// lines (0: none), with a details line or not.
    pub fn new(look: &Look, size: f32, lines: usize, details: bool) -> Self {
        let kit = &look.kit;
        let size = if size.is_finite() {
            size.clamp(look.tiles.min(), look.tiles.max())
        } else {
            look.tiles.default()
        };
        let frame_pad = tokens::frame_pad(look);
        let frame = size + 2.0 * frame_pad;
        let caption_pad = tokens::caption_pad(look);
        let name_font = kit.ty.ui_compact;
        let name_line = line_of(name_font);
        let details_font = kit.ty.small;
        let details_line = if details { line_of(details_font) } else { 0.0 };
        let (margin, card_x, card_y) = (
            tokens::tile_margin(look),
            tokens::card_pad_x(look),
            tokens::card_pad_y(look),
        );
        let chars = tokens::caption_chars(look);
        let avg = rune::text::advance(SAMPLE, name_font) / SAMPLE.len() as f32;
        let caption_min = if lines > 0 {
            (chars * avg).ceil() + 2.0 * caption_pad
        } else {
            0.0
        };
        let card = (frame.max(caption_min) + 2.0 * card_x).max(MIN_CARD);
        let min_cell = card + COLUMN_SPACING.max(2.0 * margin);
        let caption_gap = if lines > 0 {
            tokens::caption_gap(look)
        } else {
            0.0
        };
        let caption = if lines > 0 {
            lines as f32 * name_line + caption_pad
        } else {
            0.0
        };
        let details_h = if details { details_line + 2.0 } else { 0.0 };
        let height =
            (2.0 * margin + 2.0 * card_y + frame + caption_gap + caption + details_h).ceil();
        let max_columns = (tokens::grid_max_columns(look).round() as usize).max(1);
        Self {
            slot: size,
            frame,
            frame_pad,
            margin,
            card_x,
            card_y,
            grid_pad: tokens::grid_pad(look),
            caption_gap,
            caption_pad,
            name_font,
            name_line,
            name_lines: lines,
            details_font,
            details_line,
            min_cell,
            height,
            max_columns,
        }
    }

    /// How many tiles a row of a viewport `width` points wide holds: at least 1.
    pub fn per_line(&self, width: f32) -> usize {
        let inner = width - 2.0 * self.grid_pad;
        // Written so a NaN width or cell, which fails every comparison, gives 1 too.
        let usable = inner > 0.0 && self.min_cell > 0.0;
        if !usable {
            return 1;
        }
        ((inner / self.min_cell).floor() as usize).clamp(1, self.max_columns)
    }

    /// A cell's width in a viewport `width` wide holding `per` a row: the row's share.
    pub fn cell_width(&self, width: f32, per: usize) -> f32 {
        let inner = (width - 2.0 * self.grid_pad).max(0.0);
        if inner > 0.0 {
            inner / per.max(1) as f32
        } else {
            self.min_cell
        }
    }

    /// The width a name wraps at, in a cell `cell_w` wide: the card's inner width less the
    /// name box's padding.
    pub fn caption_width(&self, cell_w: f32) -> f32 {
        (cell_w - 2.0 * (self.margin + self.card_x + self.caption_pad)).max(1.0)
    }

    /// The frame's rect in a cell `cell_w` wide, in the cell's points: centred, at the top
    /// of the card.
    pub fn frame_rect(&self, cell_w: f32) -> Rect {
        Rect::new(
            (cell_w - self.frame) / 2.0,
            self.margin + self.card_y,
            self.frame,
            self.frame,
        )
    }

    /// The thumbnail slot's rect in a cell `cell_w` wide.
    pub fn slot_rect(&self, cell_w: f32) -> Rect {
        let f = self.frame_rect(cell_w);
        Rect::new(
            f.x + self.frame_pad,
            f.y + self.frame_pad,
            self.slot,
            self.slot,
        )
    }

    /// Where the name's box starts, from the cell's top.
    pub fn caption_top(&self) -> f32 {
        self.margin + self.card_y + self.frame + self.caption_gap
    }

    /// The name area's height: its lines and its box's padding.
    pub fn caption_height(&self) -> f32 {
        if self.name_lines == 0 {
            0.0
        } else {
            self.name_lines as f32 * self.name_line + self.caption_pad
        }
    }

    /// Where the details line starts, from the cell's top.
    pub fn details_top(&self) -> f32 {
        self.caption_top() + self.caption_height()
    }

    /// A hash of every size, for a memo version.
    pub fn key(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        for v in [
            self.slot,
            self.frame,
            self.frame_pad,
            self.margin,
            self.card_x,
            self.card_y,
            self.grid_pad,
            self.caption_gap,
            self.caption_pad,
            self.name_line,
            self.details_line,
            self.min_cell,
            self.height,
        ] {
            v.to_bits().hash(&mut h);
        }
        self.name_lines.hash(&mut h);
        self.name_font.hash(&mut h);
        self.details_font.hash(&mut h);
        self.max_columns.hash(&mut h);
        h.finish()
    }
}
