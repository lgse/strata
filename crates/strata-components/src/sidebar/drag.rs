// SPDX-License-Identifier: MIT

use rune::{Anim, Drag, El, Phase, Tx};
use strata_kit::look::Look;

use super::Emit;
use super::data::ItemId;
use super::intent::SidebarIntent;
use super::state::SidebarState;

const SLOP: f32 = 8.0;

pub(crate) fn past_threshold(dx: f32, dy: f32) -> bool {
    dx.abs().max(dy.abs()) > SLOP
}

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
    Dragging {
        from: usize,
        landing: Option<Landing>,
        dy: f32,
        pitch: f32,
    },
    LetGo {
        from: usize,
    },
}

/// Where a dropped row lands, as Strata's sidebar takes a drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// On row `index`, before it or after it.
    Row {
        index: usize,
        after: bool,
    },
    Gap(usize),
}

impl Landing {
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rows {
    pub height: f32,
    pub pitch: f32,
}

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

    pub fn cancel(&mut self) {
        self.lift = match self.lift {
            Some(Lift::Pressed { index }) => Some(Lift::LetGo { from: index }),
            Some(Lift::Dragging { from, .. }) => Some(Lift::LetGo { from }),
            held => held,
        };
    }

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

    pub(crate) fn press(&mut self, index: usize) {
        self.lift = Some(Lift::Pressed { index });
    }

    /// The pointer moved during a drag of row `index`, one of `count`: `d` as Rune gives it.
    pub(crate) fn follow(&mut self, index: usize, count: usize, d: &Drag) {
        let (from, pitch) = match self.lift {
            Some(Lift::Dragging { from, pitch, .. }) => (from, pitch),
            Some(Lift::LetGo { .. }) => return,
            _ if !past_threshold(d.dx, d.dy) => return,
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

    pub(crate) fn end(&mut self, cancelled: bool) -> Option<(usize, Option<Landing>)> {
        match self.lift.take()? {
            Lift::Dragging { from, landing, .. } if !cancelled => Some((from, landing)),
            Lift::LetGo { from } if !cancelled => Some((from, None)),
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

pub(crate) struct AttachSpec<'a, S> {
    pub look: &'a Look,
    pub state: &'a RowDrag,
    pub block: Block,
    pub access: fn(&mut S) -> &mut SidebarState,
    pub emit: Emit<S>,
    pub item: ItemId,
    pub index: usize,
    pub count: usize,
    pub intent: std::rc::Rc<dyn Fn(usize, Landing) -> SidebarIntent>,
}

pub(crate) fn attach<S: 'static>(el: El, spec: AttachSpec<'_, S>) -> El {
    let AttachSpec {
        look,
        state,
        block,
        access,
        emit,
        item,
        index,
        count,
        intent,
    } = spec;
    let c = &look.kit.c;
    let mut el = el.offset(0.0, state.shift(index, count));
    if state.dragging() == Some(index) {
        let lifted = el
            .raise()
            .offset_motion(Anim::INSTANT)
            .fill(super::rows::over(c.pressed, c.bg_deep));
        el = look.kit.elevate(lifted, &look.kit.elevation.raised);
    }
    el.on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
        handle_row_drag(
            s,
            d,
            tx,
            &RowDragCtx {
                block,
                access,
                emit: &emit,
                item: &item,
                index,
                count,
                intent: &intent,
            },
        );
    })
}

struct RowDragCtx<'a, S> {
    block: Block,
    access: fn(&mut S) -> &mut SidebarState,
    emit: &'a Emit<S>,
    item: &'a ItemId,
    index: usize,
    count: usize,
    intent: &'a std::rc::Rc<dyn Fn(usize, Landing) -> SidebarIntent>,
}

fn handle_row_drag<S: 'static>(s: &mut S, d: &mut Drag, tx: &mut Tx, ctx: &RowDragCtx<'_, S>) {
    match d.phase {
        Phase::Down => {
            let sidebar = (ctx.access)(s);
            sidebar.dropped = None;
            ctx.block.of(sidebar).press(ctx.index);
        }
        Phase::Move => {
            tx.animated();
            let (started, dragging, landing) = {
                let drag = ctx.block.of((ctx.access)(s));
                let was = drag.dragging().is_some();
                drag.follow(ctx.index, ctx.count, d);
                let now = drag.dragging().is_some();
                (!was && now, now, drag.target())
            };
            if started {
                (ctx.emit)(
                    s,
                    SidebarIntent::RowDragStart {
                        item: ctx.item.clone(),
                    },
                    tx,
                );
            }
            if dragging {
                (ctx.emit)(
                    s,
                    SidebarIntent::RowDragMove {
                        item: ctx.item.clone(),
                        landing,
                        at: d.window_point(),
                    },
                    tx,
                );
            }
        }
        Phase::Up => finish_row_drag(s, d, tx, ctx),
    }
}

fn finish_row_drag<S: 'static>(s: &mut S, d: &Drag, tx: &mut Tx, ctx: &RowDragCtx<'_, S>) {
    let (was_dragging, ended) = {
        let sidebar = (ctx.access)(s);
        let drag = ctx.block.of(sidebar);
        let was = drag.dragging().is_some();
        let ended = drag.end(d.cancelled());
        if ended.is_some() {
            sidebar.dropped = Some(ctx.item.clone());
        }
        (was, ended)
    };
    match ended {
        Some((from, Some(landing))) => {
            (ctx.emit)(
                s,
                SidebarIntent::RowDragEnd {
                    item: ctx.item.clone(),
                    dropped_outside: None,
                },
                tx,
            );
            (ctx.emit)(s, (ctx.intent)(from, landing), tx);
        }
        Some((_, None)) => {
            (ctx.emit)(
                s,
                SidebarIntent::RowDragEnd {
                    item: ctx.item.clone(),
                    dropped_outside: Some(d.window_point()),
                },
                tx,
            );
        }
        None if was_dragging => {
            (ctx.emit)(
                s,
                SidebarIntent::RowDragEnd {
                    item: ctx.item.clone(),
                    dropped_outside: None,
                },
                tx,
            );
        }
        None => {}
    }
}

#[cfg(test)]
mod tests;
