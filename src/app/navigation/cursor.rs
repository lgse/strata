// SPDX-License-Identifier: MIT

use std::{cmp::Ordering, collections::HashSet};

use crate::model::FileEntry;

use super::{
    ColumnState, LoadState, NavigationState, adopt_selected_locations, adopt_selected_positions,
};

impl ColumnState {
    pub(super) fn shows(&self, entry: &FileEntry) -> bool {
        self.preferences.show_hidden || !entry.is_hidden
    }

    pub(super) fn first_visible(&self) -> Option<usize> {
        self.entries.iter().position(|entry| self.shows(entry))
    }

    pub(super) fn last_visible(&self) -> Option<usize> {
        self.entries.iter().rposition(|entry| self.shows(entry))
    }

    fn clear_cursor(&mut self) {
        self.selected = None;
        self.selected_locations.clear();
        self.selection_anchor = None;
        self.load_cursor = None;
    }

    /// The next visible entry in `direction`, or the cursor itself when no
    /// visible entry lies that way.
    fn stepped_cursor(&self, direction: i32) -> Option<usize> {
        match (self.selected, direction.cmp(&0)) {
            (None, Ordering::Less) => self.last_displayed_entry(),
            (None, _) => self.first_displayed_entry(),
            (Some(current), Ordering::Less) => Some(self.step_back(current)),
            (Some(current), Ordering::Greater) => Some(self.step_forward(current)),
            (Some(current), Ordering::Equal) => match self.shows(&self.entries[current]) {
                true => Some(current),
                false => self.first_visible(),
            },
        }
    }

    fn step_back(&self, current: usize) -> usize {
        self.entries[..current]
            .iter()
            .rposition(|entry| self.shows(entry))
            .unwrap_or_else(|| self.stay_visible_or(current, || self.first_visible()))
    }

    fn step_forward(&self, current: usize) -> usize {
        self.entries
            .get(current + 1..)
            .and_then(|after| after.iter().position(|entry| self.shows(entry)))
            .map(|offset| current + 1 + offset)
            .unwrap_or_else(|| self.stay_visible_or(current, || self.last_visible()))
    }

    /// Where a range starts: the cursor, or the end shown that `direction` moves from.
    fn range_origin(&self, direction: i32) -> Option<usize> {
        self.first_visible()?;
        match self.selected {
            Some(current) => Some(current),
            None if direction < 0 => self.last_displayed_entry(),
            None => self.first_displayed_entry(),
        }
    }

    /// A visible cursor stays put; a hidden one lands on `fallback`.
    fn stay_visible_or(&self, current: usize, fallback: impl FnOnce() -> Option<usize>) -> usize {
        match self.shows(&self.entries[current]) {
            true => current,
            false => fallback().unwrap_or(current),
        }
    }
}

impl NavigationState {
    pub fn extend_selection(&mut self, direction: i32) -> Option<(usize, usize, Vec<usize>)> {
        self.visual = None;
        let depth = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))?;
        let column = self.columns.get_mut(depth)?;
        if column.entries.is_empty() {
            return None;
        }

        let show_hidden = column.preferences.show_hidden;
        let is_visible = |entry: &FileEntry| show_hidden || !entry.is_hidden;

        let current = column.range_origin(direction)?;

        // Escape clears filled selection without dropping the cursor or leftover
        // range anchor. Start a new range from that cursor instead of stepping.
        let starting_from_empty = column.selected_locations.is_empty();
        let focused = if starting_from_empty {
            current
        } else if direction < 0 {
            column.entries[..current]
                .iter()
                .rposition(is_visible)
                .unwrap_or(current)
        } else if current + 1 < column.entries.len() {
            column.entries[current + 1..]
                .iter()
                .position(is_visible)
                .map(|offset| current + 1 + offset)
                .unwrap_or(current)
        } else {
            current
        };

        let anchor = if starting_from_empty {
            current
        } else {
            column
                .selection_anchor
                .as_ref()
                .and_then(|location| {
                    column
                        .entries
                        .iter()
                        .position(|entry| &entry.location == location)
                })
                .unwrap_or(current)
        };
        column.selection_anchor = Some(column.entries[anchor].location.clone());
        let start = anchor.min(focused);
        let end = anchor.max(focused);
        let selected_positions: Vec<usize> = (start..=end)
            .filter(|&index| is_visible(&column.entries[index]))
            .collect();
        adopt_selected_positions(column, selected_positions.clone(), true);
        column.selected = Some(focused);
        self.active_column = Some(depth);
        Some((depth, focused, selected_positions))
    }

    pub fn extend_visual_selection(
        &mut self,
        depth: usize,
        focused: usize,
        order: &[usize],
    ) -> Option<Vec<usize>> {
        self.visual = None;
        let column = self.columns.get_mut(depth)?;
        let end = order.iter().position(|position| *position == focused)?;
        let start = column
            .selection_anchor
            .as_ref()
            .and_then(|anchor| {
                order.iter().position(|position| {
                    column
                        .entries
                        .get(*position)
                        .is_some_and(|entry| &entry.location == anchor)
                })
            })
            .unwrap_or(end);
        let positions = order[start.min(end)..=start.max(end)].to_vec();
        if positions
            .iter()
            .any(|position| *position >= column.entries.len())
        {
            return None;
        }
        column.selection_anchor = Some(column.entries[order[start]].location.clone());
        adopt_selected_positions(column, positions.clone(), true);
        column.selected = Some(focused);
        self.active_column = Some(depth);
        Some(positions)
    }

    pub fn move_selection(&mut self, direction: i32) -> Option<(usize, usize)> {
        let depth = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))?;
        let column = self.columns.get_mut(depth)?;
        if column.entries.is_empty() {
            return None;
        }

        if column.first_visible().is_none() {
            column.clear_cursor();
            return None;
        }
        let position = column.stepped_cursor(direction)?;
        place_committed_cursor(column, position);
        self.active_column = Some(depth);
        Some((depth, position))
    }

    /// Moves the focus `page` visible entries at a time, clamped to the first and
    /// last visible entry, for page-sized keyboard navigation. `order` is the
    /// displayed source indices when the view is not in source order.
    pub fn page_along(
        &mut self,
        direction: i32,
        page: usize,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize)> {
        let (depth, _, position) = self.page_target(direction, page, order)?;
        let column = self.columns.get_mut(depth)?;
        self.visual = None;
        place_committed_cursor(column, position);
        self.active_column = Some(depth);
        Some((depth, position))
    }

    pub fn page_cursor(
        &mut self,
        direction: i32,
        page: usize,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, bool)> {
        let (depth, _, position) = self.page_target(direction, page, order)?;
        let column = self.columns.get_mut(depth)?;
        let cleared = place_cursor(column, position);
        self.active_column = Some(depth);
        Some((depth, position, cleared))
    }

    pub fn extend_page_selection(
        &mut self,
        direction: i32,
        page: usize,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, Vec<usize>)> {
        let (depth, visible, position) = self.page_target(direction, page, order)?;
        let column = self.columns.get_mut(depth)?;
        let anchored = column
            .selection_anchor
            .as_ref()
            .is_some_and(|anchor| column.entries.iter().any(|entry| &entry.location == anchor));
        if column.selected_locations.is_empty() || !anchored {
            column.selection_anchor = column
                .selected
                .and_then(|cursor| column.entries.get(cursor))
                .map(|entry| entry.location.clone());
        }
        let positions = self.extend_visual_selection(depth, position, &visible)?;
        Some((depth, position, positions))
    }

    pub(super) fn page_target(
        &self,
        direction: i32,
        page: usize,
        order: Option<&[usize]>,
    ) -> Option<(usize, Vec<usize>, usize)> {
        if direction == 0 {
            return None;
        }
        let depth = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))?;
        let column = self.columns.get(depth)?;
        let visible: Vec<usize> = match order {
            Some(order) if !order.is_empty() => order.to_vec(),
            _ => visible_positions(column),
        };
        let last = visible.len().checked_sub(1)?;
        let steps = page.max(1);
        let current = column.selected.and_then(|selected| {
            visible
                .iter()
                .position(|position| *position == selected)
                .or_else(|| visible.iter().position(|position| *position >= selected))
        });
        let target = match (current, direction < 0) {
            // A full jump (Home/End, Ctrl+Up/Down) is absolute even without a cursor.
            _ if page == usize::MAX => {
                if direction < 0 {
                    0
                } else {
                    last
                }
            }
            (None, true) => last,
            (None, false) => 0,
            (Some(current), true) => current.saturating_sub(steps),
            (Some(current), false) => current.saturating_add(steps).min(last),
        };
        let position = visible[target];
        Some((depth, visible, position))
    }

    pub fn focus_column(&mut self, depth: usize) -> bool {
        if depth >= self.columns.len() {
            return false;
        }
        self.activate_pane(depth);
        true
    }

    pub(super) fn activate_pane(&mut self, depth: usize) {
        if self
            .visual
            .as_ref()
            .is_some_and(|range| range.depth != depth)
        {
            self.visual = None;
        }
        self.active_column = Some(depth);
    }

    /// A parent restored by Back has no cursor yet; it takes the folder it has
    /// open, as descending into that folder would have left it. A parent still
    /// loading takes it once listed, unless a cursor was placed meanwhile.
    pub fn focus_parent(&mut self) -> Option<(usize, Option<usize>)> {
        let depth = self.active_column?;
        let parent_depth = depth.checked_sub(1)?;
        let open_child = self.active_child_position(parent_depth);
        let open_folder = self.columns[depth].location.clone();
        let column = &mut self.columns[parent_depth];
        if column.selected.is_none() {
            match open_child {
                Some(position) if column.shows(&column.entries[position]) => {
                    place_committed_cursor(column, position);
                }
                None if column.load_state == LoadState::Loading
                    && column.preferred_on_load.is_none() =>
                {
                    column.open_child_on_load = Some(open_folder);
                }
                _ => {}
            }
        }
        self.activate_pane(parent_depth);
        Some((parent_depth, self.columns[parent_depth].selected))
    }

    pub fn focus_child(&mut self) -> Option<(usize, Option<usize>)> {
        let child_depth = self.active_column?.checked_add(1)?;
        let column = self.columns.get_mut(child_depth)?;
        let position = column.selected.or_else(|| {
            column
                .entries
                .iter()
                .position(|entry| column.preferences.show_hidden || !entry.is_hidden)
        });
        if column.selected.is_none() {
            if let Some(position) = position {
                place_committed_cursor(column, position);
            } else if column.load_state == LoadState::Loading {
                column.select_first_on_load = true;
            }
        }
        self.activate_pane(child_depth);
        Some((child_depth, position))
    }

    pub fn close_deepest(&mut self) -> Option<(usize, Option<usize>)> {
        let depth = self.columns.len().checked_sub(1)?;
        self.close_from(depth)
    }

    pub fn close_from(&mut self, depth: usize) -> Option<(usize, Option<usize>)> {
        if depth == 0 || depth >= self.columns.len() {
            return None;
        }
        self.record_navigation();
        self.drop_columns_from(depth)
    }

    /// Like `close_from`, without a history entry.
    pub fn drop_columns_from(&mut self, depth: usize) -> Option<(usize, Option<usize>)> {
        if depth == 0 || depth >= self.columns.len() {
            return None;
        }
        self.peek = None;
        self.visual = None;
        self.columns.truncate(depth);
        let parent_depth = depth - 1;
        self.active_column = Some(parent_depth);
        Some((parent_depth, self.columns[parent_depth].selected))
    }
}

/// Cursor, single selection and anchor on `position`, as a user's own choice.
pub(super) fn place_committed_cursor(column: &mut ColumnState, position: usize) {
    let location = column.entries[position].location.clone();
    adopt_selected_locations(column, HashSet::from([location.clone()]), true);
    column.selected = Some(position);
    column.selection_anchor = Some(location);
}

pub(super) fn place_cursor(column: &mut ColumnState, position: usize) -> bool {
    let moved = column.selected != Some(position);
    let mut cleared = false;
    if moved {
        // A resolved reveal still waiting for more batches must not pull the cursor back.
        column.drop_load_intents();
    }
    if moved && column.load_cursor.is_some() {
        column.selected_locations.clear();
        column.load_cursor = None;
        cleared = true;
    }
    column.selected = Some(position);
    cleared
}

pub(super) fn visible_positions(column: &ColumnState) -> Vec<usize> {
    let show_hidden = column.preferences.show_hidden;
    column
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| show_hidden || !entry.is_hidden)
        .map(|(position, _)| position)
        .collect()
}
