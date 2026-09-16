// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

use super::*;

const HISTORY_LIMIT: usize = 128;

#[derive(Clone)]
struct ListPosition {
    selected: Vec<Location>,
    focused: Option<Location>,
    anchor: Option<Location>,
    vertical: f64,
    horizontal: f64,
}

#[derive(Default)]
pub(super) struct ListNavigation {
    history: VecDeque<(Location, ListPosition)>,
    pending: Option<ListPosition>,
    restoring: Rc<Cell<bool>>,
}

impl ListNavigation {
    pub(super) fn is_restoring(&self) -> bool {
        self.restoring.get()
    }

    pub(super) fn cancel(&mut self) {
        self.restoring.set(false);
        self.pending = None;
    }

    pub(super) fn capture(&mut self, pane: &Pane, browser: &Browser) {
        // Leaving during a load/layout must not replace a complete saved visit.
        if self.is_restoring() {
            return;
        }
        let Some(snapshot) = browser.column_snapshot(pane.depth) else {
            return;
        };
        if snapshot.loading || snapshot.error.is_some() {
            return;
        }
        let Some((vertical, horizontal)) = adjustments(pane) else {
            return;
        };
        let position = ListPosition {
            selected: browser
                .selected_entries()
                .into_iter()
                .map(|entry| entry.location)
                .collect(),
            focused: browser.focused_item().map(|(_, _, entry)| entry.location),
            anchor: browser
                .selection_anchor_position(pane.depth)
                .and_then(|position| browser.entry_at(pane.depth, position))
                .map(|entry| entry.location),
            vertical: vertical.value(),
            horizontal: horizontal.value(),
        };
        self.history
            .retain(|(location, _)| *location != snapshot.location);
        self.history.push_back((snapshot.location, position));
        if self.history.len() > HISTORY_LIMIT {
            self.history.pop_front();
        }
    }

    pub(super) fn prepare(&mut self, pane: &Pane, snapshot: &BrowserColumnSnapshot) {
        if !snapshot.loading {
            return;
        }
        self.pending = self
            .history
            .iter()
            .find(|(location, _)| *location == snapshot.location)
            .map(|(_, position)| position.clone());
        if self.pending.is_none() {
            return;
        }
        self.restoring = Rc::new(Cell::new(true));
        let restoring = self.restoring.clone();
        let input = gtk::EventControllerLegacy::new();
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        input.connect_event(move |_, event| {
            if matches!(
                event.event_type(),
                gtk::gdk::EventType::KeyPress
                    | gtk::gdk::EventType::ButtonPress
                    | gtk::gdk::EventType::Scroll
                    | gtk::gdk::EventType::TouchBegin
            ) {
                restoring.set(false);
            }
            glib::Propagation::Proceed
        });
        pane.shell.add_controller(input);
    }

    pub(super) fn restore(&mut self, pane: &Pane, browser: &Browser) {
        let Some(saved) = self.pending.take().filter(|_| self.is_restoring()) else {
            return;
        };
        let Some((vertical, horizontal)) = adjustments(pane) else {
            self.cancel();
            return;
        };
        let selected: HashSet<_> = saved.selected.iter().collect();
        let (positions, focused, anchor) = browser
            .with_entries(pane.depth, 0..pane.model.n_items() as usize, |entries| {
                let visible = |position: usize| {
                    view_position_for_source(&pane.model, Some(&pane.section.view_model), position)
                        .is_some()
                };
                let find = |location: &Option<Location>| {
                    location.as_ref().and_then(|location| {
                        entries
                            .iter()
                            .position(|entry| entry.location == *location)
                            .filter(|&position| visible(position))
                    })
                };
                let positions = entries
                    .iter()
                    .enumerate()
                    .filter_map(|(position, entry)| {
                        (selected.contains(&entry.location) && visible(position))
                            .then_some(position)
                    })
                    .collect::<Vec<_>>();
                (positions, find(&saved.focused), find(&saved.anchor))
            })
            .unwrap_or_default();
        let focused = focused.or_else(|| positions.first().copied());
        if let Some(anchor) = anchor.or(focused) {
            reset_native_range_origin(pane, anchor);
        }
        browser.set_selection(pane.depth, &positions, focused);
        if let Some(anchor) = anchor.or(focused) {
            browser.set_selection_anchor(pane.depth, anchor);
        }
        set_selections(pane, &positions);

        let view = &pane.section.view;
        let cursor = focused.and_then(|source| {
            view_position_for_source(&pane.model, Some(&pane.section.view_model), source)
        });
        // Bind the cursor before restoring the viewport; focusing it afterwards
        // would otherwise reveal it at a different vertical offset.
        if !view.grab_focus() {
            pane.stack.grab_focus();
        }
        if let Some(cursor) = cursor {
            focus_collection_item(view, cursor);
        }
        let restoring = self.restoring.clone();
        let items = pane.section.bound_items.clone();
        let frames = Cell::new(0u8);
        let settled = Cell::new(0u8);
        let last_upper = Cell::new(-1.0);
        view.add_tick_callback(move |view, _| {
            if !restoring.get() || !view.is_mapped() || !collection_keeps_cursor(view) {
                restoring.set(false);
                return glib::ControlFlow::Break;
            }
            frames.set(frames.get() + 1);
            if let Some(cursor) = cursor {
                focus_bound_cursor(&items, cursor);
            }
            vertical.set_value(saved.vertical);
            horizontal.set_value(saved.horizontal);
            if vertical.page_size() > 0.0
                && vertical.upper() == last_upper.replace(vertical.upper())
            {
                settled.set(settled.get() + 1);
            } else {
                settled.set(0);
            }
            if settled.get() >= 3 || frames.get() >= 20 {
                restoring.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

fn adjustments(pane: &Pane) -> Option<(gtk::Adjustment, gtk::Adjustment)> {
    let vertical = pane
        .section
        .view
        .ancestor(gtk::ScrolledWindow::static_type())?
        .downcast::<gtk::ScrolledWindow>()
        .ok()?;
    let horizontal = vertical
        .parent()?
        .ancestor(gtk::ScrolledWindow::static_type())?
        .downcast::<gtk::ScrolledWindow>()
        .ok()?;
    Some((vertical.vadjustment(), horizontal.hadjustment()))
}

#[cfg(test)]
mod tests;
