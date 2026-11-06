// SPDX-License-Identifier: MIT

//! The right-click popover shared by sidebar place and device rows.

use gtk::{glib, prelude::*};

/// A popover anchored to `row` that shows `menu`.
pub(super) fn context_popover(row: &gtk::Button, menu: &gtk::Box) -> gtk::Popover {
    let popover = gtk::Popover::builder()
        .child(menu)
        .autohide(true)
        .has_arrow(false)
        .build();
    popover.add_css_class("folder-context-popover");
    popover.set_parent(row);
    popover
}

pub(super) fn popdown(popover: &glib::WeakRef<gtk::Popover>) {
    if let Some(popover) = popover.upgrade() {
        popover.popdown();
    }
}

/// Opens `popover` at the pointer on a secondary click on `row`.
pub(super) fn open_on_secondary_click(row: &gtk::Button, popover: &gtk::Popover) {
    let context = gtk::GestureClick::new();
    context.set_button(3);
    let weak_popover = popover.downgrade();
    context.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(popover) = weak_popover.upgrade() else {
            return;
        };
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            x.round() as i32,
            y.round() as i32,
            1,
            1,
        )));
        popover.popup();
    });
    row.add_controller(context);
}
