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

pub(super) fn install_edit_cancellation(window: &impl IsA<gtk::Widget>, browser: &BrowserView) {
    browser.install_inline_edit_dismissal(window);
    browser.install_location_cancellation(window);
}
