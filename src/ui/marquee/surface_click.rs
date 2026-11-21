// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk::ModifierType;
use gtk::prelude::*;

use super::hit_test::{self, ItemHit};
use super::{MarqueeState, surface_origin};
use crate::ui::pointer::exceeds_drag_threshold;

/// A second press on item selection space that never became a marquee.
struct PendingActivation {
    item: ItemHit,
    press: (f64, f64),
    moved: bool,
}

type Pending = Rc<RefCell<Option<PendingActivation>>>;

/// Claiming a second press on item selection space keeps a held tap free to
/// drag a band, which cancels the row's own double-click. A double-click that
/// stays put therefore activates the item here, on release as rows do.
pub(super) fn install(surface: &gtk::Widget, drag: &gtk::GestureDrag, state: &Rc<MarqueeState>) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let pending = Pending::default();
    connect_press(&click, state, &pending);
    connect_motion(&click, &pending);
    connect_release(&click, state, &pending);
    click.connect_cancel(move |_, _| {
        pending.take();
    });
    surface.add_controller(click.clone());
    drag.group_with(&click);
}

fn connect_press(click: &gtk::GestureClick, state: &Rc<MarqueeState>, pending: &Pending) {
    let state = state.clone();
    let pending = pending.clone();
    click.connect_pressed(move |click, count, x, y| {
        pending.take();
        let modified = click
            .current_event_state()
            .intersects(ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK);
        if count != 2 || modified || !state.allow_drag.get() {
            return;
        }
        let Some(origin) = surface_origin(&state, click.upcast_ref(), (x, y)) else {
            return;
        };
        // The row claims second presses for its own release activation, which
        // would deny the band; claiming here first keeps it.
        click.set_state(gtk::EventSequenceState::Claimed);
        pending.replace(origin.item.map(|item| PendingActivation {
            item,
            press: (x, y),
            moved: false,
        }));
    });
}

fn connect_motion(click: &gtk::GestureClick, pending: &Pending) {
    let pending = pending.clone();
    click.connect_update(move |click, sequence| {
        if let (Some(pending), Some(point), Some(surface)) = (
            pending.borrow_mut().as_mut(),
            click.point(sequence),
            click.widget(),
        ) {
            pending.moved |= exceeds_drag_threshold(
                pending.press,
                point,
                surface.settings().gtk_dnd_drag_threshold(),
            );
        }
    });
}

fn connect_release(click: &gtk::GestureClick, state: &Rc<MarqueeState>, pending: &Pending) {
    let state = state.clone();
    let pending = pending.clone();
    click.connect_released(move |click, count, x, y| {
        let (Some(pending), Some(surface)) = (pending.take(), click.widget()) else {
            return;
        };
        let stayed = !pending.moved
            && !exceeds_drag_threshold(
                pending.press,
                (x, y),
                surface.settings().gtk_dnd_drag_threshold(),
            );
        let same_item = hit_test::item_at(&state.targets, &surface, pending.press)
            .is_some_and(|item| item.is_same(&pending.item));
        if count == 2 && stayed && same_item {
            pending.item.activate();
        }
    });
}
