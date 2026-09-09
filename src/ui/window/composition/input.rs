// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::ui::browser::BrowserView;

use super::super::{MouseHistoryAction, mouse_history_action};

pub(super) fn install_mouse_history(root: &gtk::Box, view: &BrowserView) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(0);
    gesture.set_propagation_phase(gtk::PropagationPhase::Bubble);
    let browser = Rc::downgrade(&view.browser());
    gesture.connect_pressed(move |gesture, _, _, _| {
        let Some(browser) = browser.upgrade() else {
            return;
        };
        match mouse_history_action(gesture.current_button()) {
            Some(MouseHistoryAction::Back) if browser.can_go_back() => browser.back(),
            Some(MouseHistoryAction::Forward) if browser.can_go_forward() => browser.forward(),
            _ => return,
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    root.add_controller(gesture);
}

pub(super) fn install_edit_cancellation(window: &gtk::ApplicationWindow, browser: &BrowserView) {
    browser.install_inline_edit_dismissal(window);
    install_location_cancellation(window, browser);
}

fn install_location_cancellation(window: &gtk::ApplicationWindow, browser: &BrowserView) {
    let view = browser.clone();
    let gesture = gtk::GestureClick::new();
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    gesture.connect_pressed(move |gesture, _, x, y| {
        if !view.location_edit_is_active() {
            return;
        }
        let on_location_edit = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|target| view.location_edit_contains(&target));
        if !on_location_edit {
            view.cancel_location_edit();
        }
    });
    window.add_controller(gesture);
}
