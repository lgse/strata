// SPDX-License-Identifier: MIT

//! Completing a folder double-click after the first click scrolled the strip.

use crate::model::Location;
use crate::ui::browser::ViewState;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// A plain press on a folder row, kept so the next press can complete a
/// double-click on it even after the strip slid another row under the pointer.
pub(in crate::ui::browser) struct FolderPress {
    pub(in crate::ui::browser) at: Instant,
    /// Window coordinates, which stay put while the strip scrolls.
    pub(in crate::ui::browser) point: (f64, f64),
    pub(in crate::ui::browser) depth: usize,
    pub(in crate::ui::browser) position: usize,
    pub(in crate::ui::browser) location: Location,
}

impl FolderPress {
    fn pairs_with(&self, press: (f64, f64), state: &ViewState) -> bool {
        let settings = state.scroller.settings();
        let interval = Duration::from_millis(settings.gtk_double_click_time().max(0) as u64);
        let distance = f64::from(settings.gtk_double_click_distance());
        self.at.elapsed() < interval
            && (press.0 - self.point.0).abs() <= distance
            && (press.1 - self.point.1).abs() <= distance
    }
}

type PairedPress = RefCell<Option<(FolderPress, (f64, f64))>>;

fn claim_second_press(gesture: &gtk::GestureClick, weak: &Weak<ViewState>, paired: &PairedPress) {
    let modified = gesture
        .current_event_state()
        .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK);
    let press = gesture.current_event().and_then(|event| event.position());
    let first = weak.upgrade().and_then(|state| {
        let first = state.folder_press.take()?;
        let opened_on_first = state.columns_click_activation.get().folders
            == crate::ui::browser_modes::ClickCount::One
            && state.browser.is_open_child(first.depth, &first.location);
        let press = press
            .filter(|press| opened_on_first && !modified && first.pairs_with(*press, &state))?;
        state.cancel_click_rename();
        Some((first, press))
    });
    gesture.set_state(if first.is_some() {
        gtk::EventSequenceState::Claimed
    } else {
        gtk::EventSequenceState::Denied
    });
    paired.replace(first);
}

fn open_on_second_release(
    gesture: &gtk::GestureClick,
    weak: &Weak<ViewState>,
    paired: &PairedPress,
) {
    let Some((first, press)) = paired.take() else {
        return;
    };
    let Some(state) = weak.upgrade() else {
        return;
    };
    let threshold = state.scroller.settings().gtk_dnd_drag_threshold();
    let moved = gesture
        .current_event()
        .and_then(|event| event.position())
        .is_none_or(|release| {
            crate::ui::pointer::exceeds_drag_threshold(press, release, threshold)
        });
    if moved {
        return;
    }
    let still_listed = state
        .browser
        .entry_at(first.depth, first.position)
        .is_some_and(|entry| entry.location == first.location);
    if still_listed {
        state.activate_column_entry(first.depth, first.position);
    }
}

/// Swallows the second press of a double-click whose first click opened a folder.
/// Opening scrolls the strip, so that press can land on any row or column.
pub(in crate::ui::browser) fn install_double_click_guard(state: &Rc<ViewState>) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let paired: Rc<PairedPress> = Rc::new(RefCell::new(None));
    let paired_for_press = paired.clone();
    let weak = Rc::downgrade(state);
    click.connect_pressed(move |gesture, _, _, _| {
        claim_second_press(gesture, &weak, &paired_for_press);
    });
    let paired_for_release = paired.clone();
    let weak = Rc::downgrade(state);
    click.connect_released(move |gesture, _, _, _| {
        open_on_second_release(gesture, &weak, &paired_for_release);
    });
    click.connect_cancel(move |_, _| {
        paired.take();
    });
    state.scroller.add_controller(click);
}
