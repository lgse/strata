// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

// GTK's own hover delay drops to 60 ms once any tooltip is visible, which would
// flash names across the grid while browsing, so the pointer must rest first.
pub(super) const REST_DELAY: Duration = Duration::from_millis(900);
const REST_SLOP_PX: f64 = 4.0;

#[derive(Default)]
pub(super) struct NameTooltip {
    rest: Cell<Option<(Instant, f64, f64)>>,
    requery: RefCell<Option<glib::SourceId>>,
}

impl NameTooltip {
    /// Returns whether the pointer moved far enough to restart the rest delay.
    pub(super) fn pointer_moved(&self, x: f64, y: f64, now: Instant) -> bool {
        if let Some((_, rest_x, rest_y)) = self.rest.get()
            && (x - rest_x).abs() <= REST_SLOP_PX
            && (y - rest_y).abs() <= REST_SLOP_PX
        {
            return false;
        }
        self.cancel_requery();
        self.rest.set(Some((now, x, y)));
        true
    }

    pub(super) fn reset(&self) {
        self.cancel_requery();
        self.rest.set(None);
    }

    pub(super) fn text(
        &self,
        card: &gtk::Box,
        x: f64,
        y: f64,
        now: Instant,
    ) -> Option<glib::GString> {
        let (since, ..) = self.rest.get()?;
        if now.saturating_duration_since(since) < REST_DELAY {
            return None;
        }
        let (_, label) = super::parts(card)?;
        if !label.is_visible()
            || !crate::ui::pointer::hits_icon_card_content(card.upcast_ref(), x, y)
            || !super::layout::caption_truncated(&label)
        {
            return None;
        }
        label.text().filter(|text| !text.is_empty())
    }

    fn cancel_requery(&self) {
        if let Some(source) = self.requery.take() {
            source.remove();
        }
    }
}

fn schedule_requery(state: &Rc<NameTooltip>, card: &gtk::Box) {
    let pending = state.clone();
    let card = card.downgrade();
    let source = glib::timeout_add_local_once(REST_DELAY, move || {
        pending.requery.take();
        if let Some(card) = card.upgrade() {
            card.trigger_tooltip_query();
        }
    });
    state.requery.replace(Some(source));
}

pub(super) fn install(card: &gtk::Box) {
    let state = Rc::new(NameTooltip::default());
    card.set_has_tooltip(true);

    let motion = gtk::EventControllerMotion::new();
    let moved = {
        let state = state.clone();
        move |controller: &gtk::EventControllerMotion, x: f64, y: f64| {
            let Some(card) = controller.widget().and_downcast::<gtk::Box>() else {
                return;
            };
            if state.pointer_moved(x, y, Instant::now()) {
                schedule_requery(&state, &card);
            }
        }
    };
    motion.connect_enter(moved.clone());
    motion.connect_motion(moved);
    let left = state.clone();
    motion.connect_leave(move |_| left.reset());
    card.add_controller(motion);

    let press = gtk::GestureClick::builder()
        .button(0)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let pressed = state.clone();
    press.connect_pressed(move |_, _, _, _| pressed.reset());
    card.add_controller(press);

    card.connect_query_tooltip(move |card, x, y, keyboard_mode, tooltip| {
        if keyboard_mode {
            return false;
        }
        let Some(text) = state.text(card, f64::from(x), f64::from(y), Instant::now()) else {
            return false;
        };
        tooltip.set_text(Some(&text));
        true
    });
}
