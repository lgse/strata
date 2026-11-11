//! Presses in the strip: which column a press targets and activates, the second press of a
//! click that opened a column, and plain clicks on a column's empty space.

use std::rc::Rc;

use rune::PointerButton;

use crate::collection::element::{
    Ctx, Effect, Flow, Intent, KeyInput, PointerController, PointerInput, PointerPhase,
};
use crate::collection::pointer::PointerSettings;

/// What [`ColumnPointer`] sends as `Intent::Other` for a plain click on no entry: the view
/// closes deeper columns and picks what the column selects (`browser/view-modes/columns`,
/// "Background clicks").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BackgroundClick;

/// A column's pointer controller: the one inside (C24's autoscroll over C1b) hears everything,
/// and a plain primary click on no entry, released where it was pressed, also sends a
/// [`BackgroundClick`]. A press that becomes a band hands its gesture to the marquee, so its
/// release never comes here: a band is no click.
pub(super) struct ColumnPointer {
    inner: Box<dyn PointerController>,
    /// The gesture is a plain primary press on no entry that hasn't moved past the threshold.
    empty: bool,
}

impl ColumnPointer {
    pub(super) fn new(inner: Box<dyn PointerController>) -> Self {
        Self {
            inner,
            empty: false,
        }
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
        let m = input.mods;
        match input.phase {
            PointerPhase::Down => {
                self.empty = input.button == PointerButton::Primary
                    && input.hit.index.is_none()
                    && !(m.ctrl || m.shift || m.alt || m.cmd || m.logo);
            }
            PointerPhase::Move => {
                if past(input, cx) {
                    self.empty = false;
                }
            }
            PointerPhase::Up => {}
        }
        let flow = self.inner.pointer(input, cx);
        if input.phase == PointerPhase::Up
            && std::mem::take(&mut self.empty)
            && !input.cancelled
            && !past(input, cx)
        {
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
        self.empty = false;
        self.inner.reset();
    }
}
