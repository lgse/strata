// SPDX-License-Identifier: MIT

//! [`RowDrag`]: a row dragged to a new place in its block (the built-in places or the pins),
//! in the full sidebar and the rail alike.
//!
//! Where it lands is Strata's rule, and the host hears it as Strata's sidebar takes a drop
//! ([`Landing`]): on the row under the pointer, before or after it by which half of it the
//! pointer is in (the middle itself is after), or in a gap between two rows, or in the gap
//! before the first row or after the last (`ReorderSlot` and `sidebar_reorder_feedback` in
//! Strata's `src/ui/window/sidebar_reorder.rs`). Anywhere else (past the gaps at either end,
//! over the other block, or off the sidebar's side) is no drop at all: [`landing`]. The host
//! turns a landing into a move as Strata does, which differs from row to row where its order
//! holds rows the sidebar does not show. rune-kit's `Reorder`, which the sidebar used before,
//! rounded the rows passed instead, and clamped to the list's ends, with its drag's offset
//! private, so the sidebar could not correct it (Rune packet entry 102).
//!
//! It is drawn as rune-kit draws its reorder: the dragged row follows the pointer, raised and
//! elevated on an opaque fill, and the rows between its place and its landing step a row toward
//! the place it left. The move is reported when the button comes up, with the handler's `Tx`:
//! nothing is parked for a later press (U21).

use rune::{Anim, Drag, El, Phase, Tx};
use strata_kit::look::Look;

use super::data::ItemId;
use super::state::SidebarState;

/// Points the pointer travels before a press on a row becomes a drag: GTK's drag threshold,
/// 8 pixels by default (`gtk-dnd-drag-threshold`), which Strata's sidebar rows take.
const SLOP: f32 = 8.0;

/// A block's drag to reorder, held in [`super::SidebarState`]. Rows are taken to share one
/// height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowDrag {
    lift: Option<Lift>,
    gap: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lift {
    Pressed {
        index: usize,
    },
    /// Carried from `from` to where it would land now, if anywhere, `dy` from where it was
    /// pressed, rows `pitch` apart.
    Dragging {
        from: usize,
        landing: Option<Landing>,
        dy: f32,
        pitch: f32,
    },
}

/// Where a dropped row lands, as Strata's sidebar takes a drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// On row `index`, before it or after it.
    Row { index: usize, after: bool },
    /// In the gap before row `index`: `0` above the first row, the block's length below the
    /// last.
    Gap(usize),
}

impl Landing {
    /// The place in the block the row would take, by the indices before it moves: what the
    /// rows step aside for.
    pub fn place(self, from: usize, count: usize) -> usize {
        let insertion = match self {
            Landing::Row { index, after } => index + usize::from(after),
            Landing::Gap(index) => index,
        };
        let place = if insertion > from {
            insertion - 1
        } else {
            insertion
        };
        place.min(count.saturating_sub(1))
    }
}

/// The rows of a block as the drag measures them: how tall a row is, and how far one row's
/// middle is from the next one's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rows {
    pub height: f32,
    pub pitch: f32,
}

/// Where a drop of row `from`, one of `count`, lands with the pointer `along` points past the
/// row's own middle (negative toward the start), or None where Strata's sidebar takes no drop:
/// past the gap before the first row or after the last.
pub fn landing(from: usize, count: usize, along: f32, rows: Rows) -> Option<Landing> {
    if count == 0 || rows.pitch <= 0.0 {
        return None;
    }
    // The pointer, in rows from the first row's middle.
    let at = from as f32 + along / rows.pitch;
    let half = rows.height / 2.0 / rows.pitch;
    let gap = (rows.pitch - rows.height).max(0.0) / rows.pitch;
    let last = (count - 1) as f32;
    if at < -half - gap || at > last + half + gap {
        return None;
    }
    let index = at.round().clamp(0.0, last) as usize;
    let off = at - index as f32;
    Some(if off < -half {
        Landing::Gap(index)
    } else if off > half {
        Landing::Gap(index + 1)
    } else {
        Landing::Row {
            index,
            after: off >= 0.0,
        }
    })
}

impl RowDrag {
    pub fn new() -> RowDrag {
        RowDrag::default()
    }

    /// Points between the rows (the list's gap).
    pub fn gap(mut self, points: f32) -> Self {
        self.gap = points.max(0.0);
        self
    }

    /// The row being dragged, by its index before the drag.
    pub fn dragging(&self) -> Option<usize> {
        match self.lift {
            Some(Lift::Dragging { from, .. }) => Some(from),
            _ => None,
        }
    }

    /// Where the dragged row would land if dropped now, if anywhere.
    pub fn target(&self) -> Option<Landing> {
        match self.lift {
            Some(Lift::Dragging { landing, .. }) => landing,
            _ => None,
        }
    }

    /// Lets go of a held row without a move: the block it was in changed under it.
    pub fn cancel(&mut self) {
        self.lift = None;
    }

    /// How far row `index` sits from its place: the dragged row with the pointer, and the rows
    /// between its place and its landing a row toward the place it left.
    pub fn shift(&self, index: usize, count: usize) -> f32 {
        let Some(Lift::Dragging {
            from,
            landing,
            dy,
            pitch,
        }) = self.lift
        else {
            return 0.0;
        };
        let to = landing.map_or(from, |landing| landing.place(from, count));
        if index == from {
            dy
        } else if from < index && index <= to {
            -pitch
        } else if to <= index && index < from {
            pitch
        } else {
            0.0
        }
    }

    /// The primary button went down on row `index`. A drag whose release was lost ends here,
    /// moving nothing.
    pub(crate) fn press(&mut self, index: usize) {
        self.lift = Some(Lift::Pressed { index });
    }

    /// The pointer moved during a drag of row `index`, one of `count`: `d` as Rune gives it.
    pub(crate) fn follow(&mut self, index: usize, count: usize, d: &Drag) {
        let (from, pitch) = match self.lift {
            Some(Lift::Dragging { from, pitch, .. }) => (from, pitch),
            _ if d.dx.hypot(d.dy) < SLOP => return,
            Some(Lift::Pressed { index }) => (index, (d.h + self.gap).max(1.0)),
            None => (index, (d.h + self.gap).max(1.0)),
        };
        // Off the side of the row, and so of the sidebar, a drop lands nowhere.
        let x = d.start_x + d.dx;
        let beside = (0.0..=d.w).contains(&x);
        // The pointer, from the row's middle: its travel, plus where on the row it was pressed.
        let along = d.dy + d.start_y - d.h / 2.0;
        let rows = Rows { height: d.h, pitch };
        let landing = beside.then(|| landing(from, count, along, rows)).flatten();
        self.lift = Some(Lift::Dragging {
            from,
            landing,
            dy: d.dy,
            pitch,
        });
    }

    /// The button came up. A drag that was not cancelled is a drop, the row it carried and where
    /// it lands, if anywhere; a cancel or a press that never became a drag is none.
    pub(crate) fn end(&mut self, cancelled: bool) -> Option<(usize, Option<Landing>)> {
        match self.lift.take()? {
            Lift::Dragging { from, landing, .. } if !cancelled => Some((from, landing)),
            _ => None,
        }
    }
}

/// Which of the sidebar's blocks a row drags in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Places,
    Pins,
}

impl Block {
    fn of(self, state: &mut SidebarState) -> &mut RowDrag {
        match self {
            Block::Places => &mut state.places_reorder,
            Block::Pins => &mut state.pins_reorder,
        }
    }
}

/// Makes `el`, row `index` of `count` in its block, draggable to a new place: drawn where the
/// drag puts it, and `moved` called with the handler's `Tx` when a drop lands somewhere. Every
/// drop keeps the row as the sidebar's last dropped one, so the press Rune sends on the same
/// release (U21) opens nothing, as Strata's drag source claims its gesture.
pub(crate) fn attach<S: 'static>(
    el: El,
    look: &Look,
    (state, block, access): (&RowDrag, Block, fn(&mut S) -> &mut SidebarState),
    (item, index, count): (ItemId, usize, usize),
    moved: impl Fn(&mut S, usize, Landing, &mut Tx) + 'static,
) -> El {
    let c = &look.kit.c;
    let mut el = el.offset(0.0, state.shift(index, count));
    if state.dragging() == Some(index) {
        // Follows the pointer exactly; the rest of the block springs around it. Opaque, on the
        // window's deep background, so the rows it passes don't show through.
        let lifted = el
            .raise()
            .offset_motion(Anim::INSTANT)
            .fill(super::rows::over(c.pressed, c.bg_deep));
        el = look.kit.elevate(lifted, &look.kit.elevation.raised);
    }
    el.on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
        let sidebar = access(s);
        match d.phase {
            Phase::Down => {
                sidebar.dropped = None;
                block.of(sidebar).press(index);
            }
            Phase::Move => {
                tx.animated();
                block.of(sidebar).follow(index, count, d);
            }
            Phase::Up => {
                let Some((from, landing)) = block.of(sidebar).end(d.cancelled()) else {
                    return;
                };
                sidebar.dropped = Some(item.clone());
                if let Some(landing) = landing {
                    moved(s, from, landing, tx);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests;
