//! Which part of a tile a point is on, for C1b's click, drag and band decision.
//!
//! After Strata's `hits_item_content` (`src/ui/pointer.rs`, lgse/strata `upstream/main`
//! 35ee677ab): the thumbnail slot and the details text are the entry's content, the drawn box
//! of the name is its name (where a slow second click renames), and the rest is blank space
//! a band starts from: the frame's padding around the slot, the card's padding, the space
//! beside a short centred name, and the margin between tiles. Strata's hover and drag treat
//! the frame padding as content; a band, as here, treats it as blank (lgse/strata#998).

use crate::collection::element::Region;
use crate::contract::{Point, Rect};

use super::caption::Cut;
use super::metrics::TileMetrics;

/// The part of a cell `cell_w` wide that `local` (in the cell's points) is on, for a tile
/// whose name shows as `cut` and whose details line is `details_w` points wide (0: none).
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
            m.details_top(),
            details_w,
            m.details_line,
        );
        if details.contains(local) {
            return Region::Content;
        }
    }
    Region::Gutter
}
