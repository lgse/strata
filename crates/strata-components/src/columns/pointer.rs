// SPDX-License-Identifier: MIT

use std::cell::Cell;
use std::rc::Rc;

use rune::PointerButton;

use crate::collection::element::{
    Ctx, Effect, Flow, Intent, KeyInput, PointerController, PointerInput, PointerPhase,
};
use crate::collection::pointer::PointerSettings;
use crate::contract::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BackgroundClick;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PeekClick(pub(super) Point);

/// Gate armed by `column_press` before [`ColumnPointer`] hears a `Down`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PeekGate {
    #[default]
    Open,
    /// Clipped non-row press: held back until release (emits [`PeekClick`]) or drag threshold.
    Peek,
    /// Second press of a double-click whose first press revealed a clipped column.
    Suppress,
}

#[derive(Clone, Copy, Debug, Default)]
enum HeldPeek {
    #[default]
    Open,
    Deferred(PointerInput),
    Suppress,
}

pub(super) struct ColumnPointer {
    inner: Box<dyn PointerController>,
    gate: Rc<Cell<PeekGate>>,
    held_peek: HeldPeek,
    /// The gesture is a plain primary press on no entry that hasn't moved past the threshold.
    empty: bool,
}

impl ColumnPointer {
    pub(super) fn new(inner: Box<dyn PointerController>, gate: Rc<Cell<PeekGate>>) -> Self {
        Self {
            inner,
            gate,
            held_peek: HeldPeek::Open,
            empty: false,
        }
    }

    fn on_down(&mut self, input: &PointerInput) -> Option<Flow> {
        match self.gate.replace(PeekGate::Open) {
            PeekGate::Suppress => {
                self.held_peek = HeldPeek::Suppress;
                self.empty = false;
                Some(Flow::Handled)
            }
            PeekGate::Peek => {
                self.held_peek = HeldPeek::Deferred(*input);
                self.empty = false;
                Some(Flow::Handled)
            }
            PeekGate::Open => {
                self.held_peek = HeldPeek::Open;
                let m = input.mods;
                self.empty = input.button == PointerButton::Primary
                    && input.hit.index.is_none()
                    && !(m.ctrl || m.shift || m.alt || m.cmd || m.logo);
                None
            }
        }
    }

    fn on_move(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Option<Flow> {
        match self.held_peek {
            HeldPeek::Suppress => Some(Flow::Handled),
            HeldPeek::Deferred(down) => {
                if past(input, cx) {
                    self.held_peek = HeldPeek::Open;
                    let _ = self.inner.pointer(&down, cx);
                    Some(self.inner.pointer(input, cx))
                } else {
                    Some(Flow::Handled)
                }
            }
            HeldPeek::Open => {
                if past(input, cx) {
                    self.empty = false;
                }
                None
            }
        }
    }

    fn on_up(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Option<Flow> {
        match std::mem::take(&mut self.held_peek) {
            HeldPeek::Suppress => Some(Flow::Handled),
            HeldPeek::Deferred(down) => {
                if !input.cancelled && !past(input, cx) {
                    cx.emit(Effect::Intent(Intent::Other(Rc::new(PeekClick(
                        down.window,
                    )))));
                }
                Some(Flow::Handled)
            }
            HeldPeek::Open => None,
        }
    }

    fn completes_empty_click(&mut self, input: &PointerInput, cx: &Ctx<'_>) -> bool {
        if input.phase != PointerPhase::Up || !std::mem::take(&mut self.empty) {
            return false;
        }
        !input.cancelled && !past(input, cx)
    }
}

impl std::fmt::Debug for ColumnPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnPointer")
            .field("empty", &self.empty)
            .finish_non_exhaustive()
    }
}

fn past(input: &PointerInput, cx: &Ctx<'_>) -> bool {
    let threshold = cx
        .settings::<PointerSettings>()
        .map_or(PointerSettings::default().drag_threshold, |s| {
            s.drag_threshold
        });
    input.travel.x.abs() > threshold || input.travel.y.abs() > threshold
}

impl PointerController for ColumnPointer {
    fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
        let early = match input.phase {
            PointerPhase::Down => self.on_down(input),
            PointerPhase::Move => self.on_move(input, cx),
            PointerPhase::Up => self.on_up(input, cx),
        };
        if let Some(flow) = early {
            return flow;
        }
        let flow = self.inner.pointer(input, cx);
        if self.completes_empty_click(input, cx) {
            cx.emit(Effect::Intent(Intent::Other(Rc::new(BackgroundClick))));
        }
        flow
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        self.inner.tick(cx)
    }

    fn key_seen(&mut self, input: &KeyInput<'_>) {
        self.inner.key_seen(input);
    }

    fn reset(&mut self) {
        self.gate.set(PeekGate::Open);
        self.held_peek = HeldPeek::Open;
        self.empty = false;
        self.inner.reset();
    }
}
