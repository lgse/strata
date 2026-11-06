// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc, time::Instant};

use gtk::{glib, prelude::*};

use super::super::{PreviewState, TRANSITION};

/// One open or close sweep of the divider.
#[derive(Clone, Copy)]
struct Slide {
    id: u64,
    expanded: bool,
    start: i32,
    started: Instant,
    restore_browser_focus: bool,
}

impl PreviewState {
    /// A slot that Columns reserves never slides: content appears in it and leaves
    /// it. `on_settled` runs once, when the panel has finished opening or closing.
    pub(in crate::ui::preview) fn animate_reveal(
        self: &Rc<Self>,
        split: &gtk::Paned,
        expanded: bool,
        on_settled: impl FnOnce(&Rc<Self>) + 'static,
    ) {
        let in_place =
            self.reserves_column_space() || (expanded && !self.geometry(split).can_show_preview());
        if in_place {
            if expanded {
                self.show_panel();
                self.sync_split(split);
            }
            on_settled(self);
            return;
        }
        let slide = self.begin_slide(split, expanded);
        if !super::super::super::motion::animations_enabled() || slide.start <= 0 {
            split.set_position(self.slide_target(split, expanded));
            self.settle_slide(split, slide);
            on_settled(self);
            return;
        }

        self.animating.set(true);
        let slide = Slide {
            started: Instant::now(),
            ..slide
        };
        let weak = Rc::downgrade(self);
        let on_settled = RefCell::new(Some(on_settled));
        split.add_tick_callback(move |split, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.animation_generation.get() != slide.id {
                return glib::ControlFlow::Break;
            }
            if state.step_slide(split, slide) < 1.0 {
                return glib::ControlFlow::Continue;
            }
            state.settle_slide(split, slide);
            if let Some(callback) = on_settled.borrow_mut().take() {
                callback(&state);
            }
            glib::ControlFlow::Break
        });
    }

    fn begin_slide(&self, split: &gtk::Paned, expanded: bool) -> Slide {
        let id = self.animation_generation.get().saturating_add(1);
        self.animation_generation.set(id);
        let (start, restore_browser_focus) = if expanded {
            (self.open_from(split), false)
        } else {
            self.pin_pane_width(split);
            (split.position(), self.hide_intent())
        };
        Slide {
            id,
            expanded,
            start,
            started: Instant::now(),
            restore_browser_focus,
        }
    }

    fn open_from(&self, split: &gtk::Paned) -> i32 {
        // A reopen during the slide out turns back from where the drawer is.
        let start = if self.animating.get() {
            split.position()
        } else {
            split.width()
        };
        self.show_panel();
        // The animation owns the divider, including after a resize while the pane was hidden.
        split.set_resize_start_child(false);
        split.set_resize_end_child(true);
        self.pin_pane_width(split);
        self.preserve_column_positions(start);
        split.set_position(start);
        start
    }

    fn slide_target(&self, split: &gtk::Paned, expanded: bool) -> i32 {
        if expanded {
            self.geometry(split)
                .position(self.sizing.manual_width.get())
        } else {
            split.width()
        }
    }

    /// Moves the divider for this frame and returns the slide's progress.
    fn step_slide(&self, split: &gtk::Paned, slide: Slide) -> f64 {
        let progress =
            (slide.started.elapsed().as_secs_f64() / TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = super::super::super::motion::emphasized_deceleration(progress);
        let end = self.slide_target(split, slide.expanded);
        let position =
            (f64::from(slide.start) + f64::from(end - slide.start) * eased).round() as i32;
        // Resize the scroll range with the viewport, without clamping its retained offset.
        self.preserve_column_positions(position);
        split.set_position(position);
        self.pin_pane_width(split);
        progress
    }

    fn settle_slide(self: &Rc<Self>, split: &gtk::Paned, slide: Slide) {
        self.animating.set(false);
        self.pane.set_width_request(0);
        if slide.expanded {
            self.sync_split(split);
        } else {
            self.finish_hide(slide.restore_browser_focus);
        }
    }
}
