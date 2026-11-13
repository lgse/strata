// SPDX-License-Identifier: MIT

//! [`RowDrag`]: a row dragged to a new place in its block (the built-in places or the pins),
//! in the full sidebar and the rail alike.
//!
//! Where it lands is Strata's rule: before or after the row under the pointer, by which half
//! of it the pointer is in, and next to the row on either side of a gap the pointer is in
//! (`ReorderSlot` and `sidebar_reorder_feedback` in Strata's `src/ui/window/sidebar_reorder.rs`).
//! Measured from the dragged row's middle in rows, that is the whole rows the pointer has gone
//! past, toward the start or the end: [`landing`]. rune-kit's `Reorder`, which the sidebar used
//! before, rounds instead, so a drop past half a row lands a row further on, and its drag's
//! offset is private, so the sidebar could not correct it (Rune packet entry 102).
//!
//! It is drawn as rune-kit draws its reorder: the dragged row follows the pointer, raised and
//! elevated on an opaque fill, and the rows between its place and its landing step a row toward
//! the place it left. The move is reported when the button comes up, with the handler's `Tx`:
//! nothing is parked for a later press (U21).

use rune::{Anim, Drag, El, Phase, Tx};
use strata_kit::look::Look;

use super::state::SidebarState;

/// Points the pointer travels before a press on a row becomes a drag (rune-kit's slop).
const SLOP: f32 = 4.0;

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
    /// Carried from `from` to `to`, `dy` from where it was pressed, rows `pitch` apart.
    Dragging {
        from: usize,
        to: usize,
        dy: f32,
        pitch: f32,
    },
}

/// Where a row from `from` lands among `count`, with the pointer `along` points past the
/// row's own middle (negative toward the start) and rows `pitch` apart: the rows it has gone
/// past whole, toward the start or the end, as Strata's half-row and gap rule has it.
pub fn landing(from: usize, count: usize, along: f32, pitch: f32) -> usize {
    if count == 0 || pitch <= 0.0 {
        return from;
    }
    let past = (along / pitch).trunc();
    (from as f32 + past).clamp(0.0, (count - 1) as f32) as usize
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

    /// Where the dragged row would land if dropped now.
    pub fn target(&self) -> Option<usize> {
        match self.lift {
            Some(Lift::Dragging { to, .. }) => Some(to),
            _ => None,
        }
    }

    /// How far row `index` sits from its place: the dragged row with the pointer, and the rows
    /// between its place and its landing a row toward the place it left.
    pub fn shift(&self, index: usize) -> f32 {
        let Some(Lift::Dragging {
            from,
            to,
            dy,
            pitch,
        }) = self.lift
        else {
            return 0.0;
        };
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
            _ if d.dy.abs() < SLOP => return,
            Some(Lift::Pressed { index }) => (index, (d.h + self.gap).max(1.0)),
            None => (index, (d.h + self.gap).max(1.0)),
        };
        // The pointer, from the row's middle: its travel, plus where on the row it was pressed.
        let along = d.dy + d.start_y - d.h / 2.0;
        let to = landing(from, count, along, pitch);
        self.lift = Some(Lift::Dragging {
            from,
            to,
            dy: d.dy,
            pitch,
        });
    }

    /// The drag ended: `(from, to)` for a drop that moves a row, `None` for a cancel, a drop in
    /// place or a press that never became a drag.
    pub(crate) fn end(&mut self, cancelled: bool) -> Option<(usize, usize)> {
        match self.lift.take()? {
            Lift::Dragging { from, to, .. } if !cancelled && from != to => Some((from, to)),
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
/// drag puts it, and `moved` called with the handler's `Tx` when a drop moves it.
pub(crate) fn attach<S: 'static>(
    el: El,
    look: &Look,
    (state, block, access): (&RowDrag, Block, fn(&mut S) -> &mut SidebarState),
    (index, count): (usize, usize),
    moved: impl Fn(&mut S, usize, usize, &mut Tx) + 'static,
) -> El {
    let c = &look.kit.c;
    let mut el = el.offset(0.0, state.shift(index));
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
                if let Some((from, to)) = block.of(sidebar).end(d.cancelled()) {
                    moved(s, from, to, tx);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests;
