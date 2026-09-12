// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::ui::browser) const COLUMN_PEEK_WIDTH: f64 = 48.0;

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::browser) struct ColumnSpan {
    pub left: f64,
    pub right: f64,
    pub total: f64,
}

impl ColumnSpan {
    pub fn width(self) -> f64 {
        self.right - self.left
    }

    pub fn peek_space(self, available: f64) -> f64 {
        let neighbors = usize::from(self.left > 0.0) + usize::from(self.right < self.total);
        ((available - self.width()).max(0.0) / COLUMN_PEEK_WIDTH)
            .floor()
            .min(neighbors as f64)
            * COLUMN_PEEK_WIDTH
    }

    pub fn reveal_target(self, current: f64, page_size: f64, lower: f64, upper: f64) -> f64 {
        let budget = self.peek_space(page_size);
        let left_first = self.left < current || self.right >= self.total;
        let left = if self.left > lower && (left_first || budget >= COLUMN_PEEK_WIDTH * 2.0) {
            budget.min(COLUMN_PEEK_WIDTH)
        } else {
            0.0
        };
        let right = if self.right < self.total {
            (budget - left).min(COLUMN_PEEK_WIDTH)
        } else {
            0.0
        };
        let reveal_left = self.left - left;
        let reveal_right = self.right + right;
        let target = if reveal_right > current + page_size {
            reveal_right - page_size
        } else if reveal_left < current {
            reveal_left
        } else {
            current
        };
        target.clamp(lower, (upper - page_size).max(lower))
    }
}

impl ViewState {
    pub(super) fn reveal_column_only(self: &Rc<Self>, depth: usize, location: &Location) {
        if self.browser.location_at(depth).as_ref() != Some(location) {
            return;
        }
        self.browser.set_active_column(depth);
        self.browser.focus_active();
        let shell = self
            .columns
            .borrow()
            .get(depth)
            .map(|column| column.shell.clone());
        if let Some(shell) = shell {
            self.reveal_column(shell);
        }
    }

    fn clipped_column(&self, x: f64, y: f64) -> Option<(usize, Location)> {
        let page = self.scroller.width() as f32;
        self.columns
            .borrow()
            .iter()
            .enumerate()
            .find_map(|(depth, column)| {
                let bounds = column.shell.compute_bounds(&self.scroller)?;
                let partial = bounds.width() <= page + 1.0
                    && (bounds.x() < -1.0 || bounds.x() + bounds.width() > page + 1.0);
                (partial && bounds.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)))
                    .then(|| Some((depth, self.browser.location_at(depth)?)))
                    .flatten()
            })
    }

    pub(in crate::ui::browser) fn install_column_peek_targets(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.scroller.add_tick_callback(move |scroller, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let page = scroller.width() as f32;
            for column in state.columns.borrow().iter() {
                let Some(bounds) = column.shell.compute_bounds(scroller) else {
                    continue;
                };
                let visible =
                    ((bounds.x() + bounds.width()).min(page) - bounds.x().max(0.0)).max(0.0);
                let partial =
                    bounds.width() <= page + 1.0 && visible > 1.0 && visible < bounds.width() - 1.0;
                column.reveal_button.set_visible(partial);
                if partial {
                    column.reveal_button.set_halign(if bounds.x() < 0.0 {
                        gtk::Align::End
                    } else {
                        gtk::Align::Start
                    });
                    column
                        .reveal_button
                        .set_width_request(visible.floor() as i32);
                }
            }
            glib::ControlFlow::Continue
        });
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let pending = Rc::new(RefCell::new(None));
        let peek_sequence = Rc::new(Cell::new(false));
        let weak = Rc::downgrade(self);
        let pressed = pending.clone();
        let sequence = peek_sequence.clone();
        click.connect_pressed(move |gesture, count, x, y| {
            if count > 1 && sequence.get() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                return;
            }
            let target = weak.upgrade().and_then(|state| state.clipped_column(x, y));
            sequence.set(target.is_some());
            *pressed.borrow_mut() = target;
            if sequence.get() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        let weak = Rc::downgrade(self);
        let released = pending.clone();
        click.connect_released(move |_, count, _, _| {
            let target = released.borrow_mut().take();
            if count == 1
                && let Some((depth, location)) = target
                && let Some(state) = weak.upgrade()
            {
                state.reveal_column_only(depth, &location);
            }
        });
        click.connect_stopped(move |_| {
            pending.borrow_mut().take();
        });
        self.scroller.add_controller(click);
    }
}
