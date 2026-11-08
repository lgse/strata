// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::PointerSequence;

impl PointerSequence {
    /// Keep GTK's item widget from activating and claiming the second press before a
    /// grouped file drag can start. Editors retain their own word-selection gesture.
    pub(in crate::ui) fn claim_double_press(
        &self,
        gesture: &gtk::GestureClick,
        count: i32,
        point: (f64, f64),
    ) {
        if count != 2 || self.activation() != Some(true) {
            return;
        }
        if !picks_editable(gesture, point.0, point.1) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    }
}

pub(in crate::ui) fn picks_editable(gesture: &gtk::GestureClick, x: f64, y: f64) -> bool {
    gesture
        .widget()
        .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
        .is_some_and(|target| crate::ui::focus_navigation::editable(&target))
}
