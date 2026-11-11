//! Where the card goes: Strata's rule (`peek_horizontal_placement`, `src/ui/browser/peek.rs`),
//! measured against the collection's viewport, with Rune's anchored margin taken at its edges.

use crate::contract::Rect;

/// The card's width in points, Strata's `PEEK_WIDTH`. A constant until a host needs to
/// override it (owner ruling), when it can move to a `peek.*` slot.
pub const WIDTH: f32 = 256.0;

/// The gap between the item and the card in points, Strata's `PEEK_GAP`. A constant for now,
/// as [`WIDTH`] is.
pub const GAP: f32 = 8.0;

/// How far an anchored overlay stays inside the window at the pinned rev
/// (`runtime/anchor.rs`, `MARGIN`). A card that came closer to the window's trailing edge
/// would be flipped to its anchor's other side, so the rule keeps it this far inside the
/// viewport, which lies within the window.
pub(super) const MARGIN: f32 = 8.0;

/// The gap rune-kit's `HoverCard` puts between its anchor and the card (`overlay.rs`).
pub(super) const CARD_GAP: f32 = 4.0;

/// Which side of the item the card opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// To its right: there was room.
    Right,
    /// To its left: there was room there and not on the right.
    Left,
    /// Over the viewport's trailing edge: no room on either side (a List row, a narrow window).
    Over,
}

/// Where a card goes, worked out once as it opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The side.
    pub side: Side,
    /// The card's left edge in window points, as it opened.
    pub x: f32,
    /// The anchor's offset from the item's left edge, which the item's own part carries, so
    /// the card rides the item as it scrolls.
    pub(super) dx: f32,
}

/// Where a card goes beside `item` in `viewport` (both in window points), or `None` when the
/// viewport is narrower than the card.
pub fn place(item: Rect, viewport: Rect) -> Option<Placement> {
    if viewport.w < WIDTH {
        return None;
    }
    let right = item.x + item.w + GAP;
    let left = item.x - GAP - WIDTH;
    let (side, x) = if right + WIDTH <= viewport.x + viewport.w - MARGIN {
        (Side::Right, right)
    } else if left >= viewport.x && left >= MARGIN {
        (Side::Left, left)
    } else {
        // Ends `MARGIN` short of the trailing edge, so Rune never flips it; in a viewport
        // less than `MARGIN` wider than the card (256 to 263 pt), it overhangs by the rest: at
        // the leading edge, or at the trailing edge where Rune holds it 8 pt inside the window
        // (a pane at x 0).
        (Side::Over, viewport.x + viewport.w - MARGIN - WIDTH)
    };
    Some(Placement {
        side,
        x,
        dx: x - item.x - CARD_GAP,
    })
}
