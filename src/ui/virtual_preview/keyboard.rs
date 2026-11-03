// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{gdk::ModifierType as Modifiers, glib, prelude::*};

use super::highlight::{selection_text, update_all_bound_selection};
use super::selection::{self, Motion};
use super::{DocumentSelection, SelectionPoint, VirtualPreviewState};
use crate::ui::keys::{
    self, KeyPress, Match,
    bindings::{COPY, DOWN, END_KEY, HOME_KEY, LEFT, RIGHT, SELECT_ALL, UP},
};

pub(super) fn install_keyboard_selection(list: &gtk::ListView, state: Rc<VirtualPreviewState>) {
    let keys = gtk::EventControllerKey::new();
    let weak_list = list.downgrade();
    keys::on_press(&keys, move |_, press| {
        let Some(list) = weak_list.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if press.is(SELECT_ALL) {
            let Some(last) = state.units.len().checked_sub(1) else {
                return glib::Propagation::Stop;
            };
            state.selection.set(Some(DocumentSelection {
                anchor: SelectionPoint { unit: 0, offset: 0 },
                focus: SelectionPoint {
                    unit: last,
                    offset: state.units[last].selection_len(),
                },
            }));
            update_all_bound_selection(&state);
            return glib::Propagation::Stop;
        }
        if press.is(COPY) {
            if let Some(text) = selection_text(&state) {
                list.clipboard().set_text(&text);
            }
            return glib::Propagation::Stop;
        }
        let held = press.modifiers();
        if let Some(motion) = caret_motion(&press.untyped())
            && let Some(selection) = selection::move_selection(
                &state.units,
                state.selection.get(),
                motion,
                held.contains(Modifiers::CONTROL_MASK),
                held.contains(Modifiers::SHIFT_MASK),
            )
        {
            state.select_and_reveal(selection);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    list.add_controller(keys);
}

/// Ctrl moves by word or to the document's ends; Shift extends.
pub(super) fn caret_motion(press: &KeyPress) -> Option<Motion> {
    const HELD: Modifiers = Modifiers::CONTROL_MASK.union(Modifiers::SHIFT_MASK);
    let motions: [(&dyn Match, Motion); 6] = [
        (&LEFT.ignoring(HELD), Motion::Left),
        (&RIGHT.ignoring(HELD), Motion::Right),
        (&UP.ignoring(HELD), Motion::Up),
        (&DOWN.ignoring(HELD), Motion::Down),
        (&HOME_KEY.ignoring(HELD), Motion::Home),
        (&END_KEY.ignoring(HELD), Motion::End),
    ];
    motions
        .into_iter()
        .find_map(|(key, motion)| press.is(key).then_some(motion))
}
