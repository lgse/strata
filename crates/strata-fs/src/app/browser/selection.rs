// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::model::{FileEntry, Location};

use super::{Browser, BrowserEvent, CursorToggle, SelectionUpdate, VisualKind, VisualRange};

impl Browser {
    pub fn select(&self, depth: usize, position: usize) {
        let selected = self.state.borrow_mut().select(depth, position);
        self.emit_focus(selected.then_some((depth, Some(position))));
    }

    pub fn toggle_cursor_fill(&self) -> CursorToggle {
        let outcome = self.state.borrow_mut().toggle_cursor_fill();
        if outcome != CursorToggle::Empty
            && let Some((depth, position, _)) = self.focused_item()
        {
            self.emit_fill(depth, position);
        }
        outcome
    }

    pub fn select_visible(&self, depth: usize) -> bool {
        let Some((focused, _)) = self.state.borrow_mut().select_visible(depth) else {
            return false;
        };
        self.emit_fill(depth, focused);
        true
    }

    pub fn invert_visible(&self, depth: usize) -> bool {
        let Some((focused, _)) = self.state.borrow_mut().invert_visible(depth) else {
            return false;
        };
        self.emit_fill(depth, focused);
        true
    }

    pub fn install_pane_fill(&self, depth: usize, positions: &[usize], cursor: usize) -> bool {
        if !self
            .state
            .borrow_mut()
            .install_pane_fill(depth, positions, cursor)
        {
            return false;
        }
        self.emit_fill(depth, cursor);
        true
    }

    /// `order` is the pane's displayed order, which an active visual range walks.
    pub fn set_preserve_fill_on_removal(&self, preserve: bool) {
        self.state
            .borrow_mut()
            .set_preserve_fill_on_removal(preserve);
    }

    pub fn place_cursor(&self, depth: usize, position: usize, order: Option<&[usize]>) {
        let Some(cleared) = self.state.borrow_mut().place_cursor(depth, position) else {
            return;
        };
        if self.emit_visual_fill(order) {
            return;
        }
        if cleared {
            let focused = self
                .focused_item()
                .filter(|(item_depth, _, _)| *item_depth == depth)
                .map(|(_, cursor, _)| cursor)
                .unwrap_or(position);
            self.emit_fill(depth, focused);
        } else {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: Some(position),
                triggered_by_removal: false,
            });
        }
    }

    pub fn page_cursor(&self, direction: i32, page: usize, order: Option<&[usize]>) {
        let moved = self.state.borrow_mut().page_cursor(direction, page, order);
        if let Some((depth, position, cleared)) = moved {
            if self.emit_visual_fill(order) {
                return;
            }
            if cleared {
                self.emit_fill(depth, position);
            } else {
                self.emit(BrowserEvent::FocusChanged {
                    depth,
                    position: Some(position),
                    triggered_by_removal: false,
                });
            }
        }
    }

    pub fn toggle_visual(&self, kind: VisualKind, order: Option<&[usize]>) -> bool {
        if self.visual_kind() == Some(kind) {
            self.leave_visual();
            return true;
        }
        let started = self.state.borrow_mut().start_visual(kind, order);
        let Some((depth, focused, _)) = started else {
            return false;
        };
        self.emit_fill(depth, focused);
        true
    }

    pub fn toggle_visual_cursor(&self, order: Option<&[usize]>) -> bool {
        let toggled = self.state.borrow_mut().toggle_visual_cursor(order);
        let Some((depth, focused, _)) = toggled else {
            return false;
        };
        self.emit_fill(depth, focused);
        true
    }

    pub fn refresh_visual(&self, order: Option<&[usize]>) -> bool {
        self.emit_visual_fill(order)
    }

    pub fn leave_visual(&self) -> bool {
        let valid = self.visual_kind().is_some();
        if !self.state.borrow_mut().leave_visual() || !valid {
            return false;
        }
        if let Some((depth, position, _)) = self.focused_item() {
            self.emit_fill(depth, position);
        }
        true
    }

    pub fn take_visual(&self) -> Option<VisualRange> {
        self.state.borrow_mut().take_visual()
    }

    pub fn restore_visual(&self, range: Option<VisualRange>) {
        self.state.borrow_mut().restore_visual(range);
    }

    pub fn visual_kind(&self) -> Option<VisualKind> {
        self.state.borrow().visual_kind()
    }

    pub(super) fn emit_visual_fill(&self, order: Option<&[usize]>) -> bool {
        let refreshed = self.state.borrow_mut().refresh_visual(order);
        let Some((depth, focused, _)) = refreshed else {
            return false;
        };
        self.emit_fill(depth, focused);
        true
    }

    pub(super) fn emit_fill(&self, depth: usize, focused: usize) {
        let positions = self.selected_positions(depth);
        self.emit(BrowserEvent::SelectionSetChanged {
            depth,
            selection: SelectionUpdate::Positions(positions),
            focused,
            take_focus: true,
        });
    }

    pub fn set_selection(&self, depth: usize, positions: &[usize], focused: Option<usize>) {
        let mut state = self.state.borrow_mut();
        if state.set_selection(depth, positions, focused) {
            tracing::debug!(
                depth,
                selected = state.selected_count(),
                "selection changed"
            );
            drop(state);
            self.emit(BrowserEvent::SelectionSynced { depth, focused });
        }
    }

    pub fn select_all(&self, depth: usize) {
        let Some(focused) = self.state.borrow_mut().select_all(depth) else {
            return;
        };
        self.emit(BrowserEvent::SelectionSetChanged {
            depth,
            selection: SelectionUpdate::All,
            focused,
            take_focus: true,
        });
    }

    pub fn move_selection(&self, direction: i32) {
        let moved = self.state.borrow_mut().move_selection(direction);
        self.emit_focus(moved.map(|(depth, position)| (depth, Some(position))));
    }

    /// Moves the focus by `page` visible entries, for `Page Up` and `Page Down`.
    pub fn page_along(&self, direction: i32, page: usize, order: Option<&[usize]>) {
        let moved = self.state.borrow_mut().page_along(direction, page, order);
        self.emit_focus(moved.map(|(depth, position)| (depth, Some(position))));
    }

    pub fn extend_page_selection(&self, direction: i32, page: usize, order: Option<&[usize]>) {
        let extended = self
            .state
            .borrow_mut()
            .extend_page_selection(direction, page, order);
        self.emit_extended_fill(extended);
    }

    pub fn extend_selection(&self, direction: i32) {
        let extended = self.state.borrow_mut().extend_selection(direction);
        self.emit_extended_fill(extended);
    }

    pub fn extend_visual_selection(&self, depth: usize, focused: usize, order: &[usize]) {
        let positions = self
            .state
            .borrow_mut()
            .extend_visual_selection(depth, focused, order);
        self.emit_extended_fill(positions.map(|positions| (depth, focused, positions)));
    }

    pub fn focus_parent(&self) {
        let focus = self.state.borrow_mut().focus_parent();
        self.emit_focus(focus);
    }

    pub(super) fn focus_child(&self) {
        let focus = self.state.borrow_mut().focus_child();
        self.emit_focus(focus);
    }

    fn emit_focus(&self, focus: Option<(usize, Option<usize>)>) {
        if let Some((depth, position)) = focus {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position,
                triggered_by_removal: false,
            });
        }
    }

    fn emit_extended_fill(&self, extended: Option<(usize, usize, Vec<usize>)>) {
        if let Some((depth, focused, positions)) = extended {
            self.emit(BrowserEvent::SelectionSetChanged {
                depth,
                selection: SelectionUpdate::Positions(positions),
                focused,
                take_focus: true,
            });
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn select_entries_by_name(self: &Rc<Self>, names: &[String]) {
        let Some(depth) = self.active_depth() else {
            return;
        };
        self.select_entries_by_name_at(depth, names);
    }

    pub fn select_entries_by_name_at(self: &Rc<Self>, depth: usize, names: &[String]) -> bool {
        let requested: HashSet<&str> = names.iter().map(String::as_str).collect();
        self.select_entries_matching_at(depth, |entry| {
            requested.contains(entry.display_name.as_str()).then_some(0)
        })
    }

    /// The first requested location takes the cursor, regardless of sort order.
    pub fn select_entries_by_location_at(
        self: &Rc<Self>,
        depth: usize,
        locations: &[Location],
    ) -> bool {
        let requested = location_ranks(locations);
        self.select_entries_matching_at(depth, |entry| requested.get(&entry.location).copied())
    }

    /// [`Self::select_entries_by_location_at`] among the items shown, never
    /// showing hidden files for a hidden one.
    pub fn select_shown_entries_by_location_at(
        &self,
        depth: usize,
        locations: &[Location],
    ) -> bool {
        let requested = location_ranks(locations);
        let show_hidden = self.preferences.get().show_hidden;
        self.select_ranked_at(depth, |entry| {
            requested
                .get(&entry.location)
                .copied()
                .filter(|_| show_hidden || !entry.is_hidden)
        })
    }

    pub(super) fn select_entries_matching_at(
        self: &Rc<Self>,
        depth: usize,
        rank: impl Fn(&FileEntry) -> Option<usize>,
    ) -> bool {
        let has_hidden_match = self
            .state
            .borrow()
            .columns
            .get(depth)
            .is_some_and(|column| {
                column
                    .entries
                    .iter()
                    .any(|entry| entry.is_hidden && rank(entry).is_some())
            });
        if has_hidden_match && !self.preferences.get().show_hidden {
            self.toggle_hidden();
        }
        self.select_ranked_at(depth, rank)
    }

    /// Selects the entries `rank` ranks, the lowest rank taking the cursor.
    fn select_ranked_at(&self, depth: usize, rank: impl Fn(&FileEntry) -> Option<usize>) -> bool {
        let state = self.state.borrow();
        let Some(column) = state.columns.get(depth) else {
            return false;
        };
        let ranked: Vec<(usize, usize)> = column
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| rank(entry).map(|rank| (rank, position)))
            .collect();
        drop(state);
        let Some(&(_, focused)) = ranked.iter().min() else {
            return false;
        };
        let positions: Vec<usize> = ranked.into_iter().map(|(_, position)| position).collect();
        self.commit_selection();
        self.set_selection(depth, &positions, Some(focused));
        self.emit(BrowserEvent::SelectionSetChanged {
            depth,
            selection: SelectionUpdate::Positions(positions),
            focused,
            take_focus: true,
        });
        true
    }
}

fn location_ranks(locations: &[Location]) -> HashMap<&Location, usize> {
    let mut requested = HashMap::new();
    for (rank, location) in locations.iter().enumerate() {
        requested.entry(location).or_insert(rank);
    }
    requested
}
