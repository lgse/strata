// SPDX-License-Identifier: MIT

use crate::collection::element::Region;
use crate::contract::{Point, Rect};

use super::caption::Cut;
use super::metrics::TileMetrics;

pub fn region(m: &TileMetrics, cell_w: f32, cut: &Cut, details_w: f32, local: Point) -> Region {
    if m.slot_rect(cell_w).contains(local) {
        return Region::Content;
    }
    if m.name_lines > 0 {
        let (w, h) = cut.size;
        let bw = w + 2.0 * m.caption_pad;
        let name = Rect::new((cell_w - bw) / 2.0, m.caption_top(), bw, h + m.caption_pad);
        if name.contains(local) {
            return Region::Name;
        }
    }
    // The line draws no wider than the card's inside (`tile::content_slot`'s `max_w`).
    let details_w = details_w.min(cell_w - 2.0 * (m.margin + m.card_x));
    if details_w > 0.0 && m.details_line > 0.0 {
        let details = Rect::new(
            (cell_w - details_w) / 2.0,
            m.details_top_for(cut),
            details_w,
            m.details_line,
        );
        if details.contains(local) {
            return Region::Content;
        }
    }
    Region::Gutter
}
