// SPDX-License-Identifier: MIT

use crate::contract::Rect;

/// The card's width in points, Strata's `PEEK_WIDTH`.
pub const WIDTH: f32 = 256.0;

/// The gap between the item and the card in points, Strata's `PEEK_GAP`.
pub const GAP: f32 = 8.0;

pub(super) const MARGIN: f32 = 8.0;

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
    pub(super) dx: f32,
}

/// Where a card goes beside `item` in `viewport` (both in window points).
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
        (Side::Over, viewport.x + viewport.w - MARGIN - WIDTH)
    };
    Some(Placement {
        side,
        x,
        dx: x - item.x,
    })
}
