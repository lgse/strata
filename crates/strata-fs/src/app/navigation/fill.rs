// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use crate::model::{FileEntry, Location};

use super::{
    ColumnState, CursorToggle, NavigationState, SelectedLocations, lists_each_location_once,
    place_committed_cursor, place_cursor, visible_positions,
};

impl NavigationState {
    pub fn select(&mut self, depth: usize, position: usize) -> bool {
        let Some(column) = self.columns.get_mut(depth) else {
            return false;
        };
        if position >= column.entries.len() {
            return false;
        }
        place_committed_cursor(column, position);
        self.active_column = Some(depth);
        self.visual = None;
        true
    }

    /// A load cursor is not a fill: the first Space adds that item.
    pub fn toggle_cursor_fill(&mut self) -> CursorToggle {
        let Some(depth) = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))
        else {
            return CursorToggle::Empty;
        };
        let Some(column) = self.columns.get_mut(depth) else {
            return CursorToggle::Empty;
        };
        let visible = visible_positions(column);
        let Some(position) = column
            .selected
            .filter(|position| visible.contains(position))
        else {
            return CursorToggle::Empty;
        };
        let location = column.entries[position].location.clone();
        let filled = column.load_cursor.is_none() && column.selected_locations.contains(&location);
        column.drop_load_intents();
        if column.load_cursor.is_some() {
            column.selected_locations.clear();
            column.load_cursor = None;
        }
        if filled {
            column.selected_locations.remove(&location);
            CursorToggle::Removed
        } else {
            column.selected_locations.insert(location);
            CursorToggle::Added
        }
    }

    pub fn select_visible(&mut self, depth: usize) -> Option<(usize, Vec<usize>)> {
        self.replace_visible(depth, true)
    }

    pub fn select_all(&mut self, depth: usize) -> Option<usize> {
        self.visual = None;
        self.selection_commit = false;
        let column = self.columns.get_mut(depth)?;
        let show_hidden = column.preferences.show_hidden;
        let focused = column
            .entries
            .iter()
            .rposition(|entry| show_hidden || !entry.is_hidden)?;
        column.selected_locations = SelectedLocations::all_visible(&column.entries, show_hidden);
        column.selected = Some(focused);
        column.selection_anchor = Some(column.entries[focused].location.clone());
        column.load_cursor = None;
        column.drop_load_intents();
        self.active_column = Some(depth);
        Some(focused)
    }

    pub fn invert_visible(&mut self, depth: usize) -> Option<(usize, Vec<usize>)> {
        self.replace_visible(depth, false)
    }

    pub(super) fn replace_visible(
        &mut self,
        depth: usize,
        select_all: bool,
    ) -> Option<(usize, Vec<usize>)> {
        self.visual = None;
        let column = self.columns.get_mut(depth)?;
        let visible = visible_positions(column);
        if visible.is_empty() {
            return None;
        }
        let focused = column
            .selected
            .filter(|position| visible.contains(position))
            .unwrap_or(visible[0]);
        let committed = column.load_cursor.is_none();
        let positions = visible
            .into_iter()
            .filter(|position| {
                select_all
                    || !(committed
                        && column
                            .selected_locations
                            .contains(&column.entries[*position].location))
            })
            .collect();
        adopt_selected_positions(column, positions, true);
        column.selected = Some(focused);
        self.active_column = Some(depth);
        let positions = column.selected_positions().to_vec();
        Some((focused, positions))
    }

    pub fn install_pane_fill(&mut self, depth: usize, positions: &[usize], cursor: usize) -> bool {
        let Some(column) = self.columns.get_mut(depth) else {
            return false;
        };
        if cursor >= column.entries.len()
            || positions
                .iter()
                .any(|position| *position >= column.entries.len())
        {
            return false;
        }
        adopt_selected_positions(column, positions.to_vec(), true);
        column.selected = Some(cursor);
        self.active_column = Some(depth);
        true
    }

    /// Leaving a load cursor drops its uncommitted highlight.
    pub fn place_cursor(&mut self, depth: usize, position: usize) -> Option<bool> {
        let column = self.columns.get_mut(depth)?;
        if position >= column.entries.len() {
            return None;
        }
        let cleared = place_cursor(column, position);
        self.active_column = Some(depth);
        Some(cleared)
    }

    pub fn commit_selection(&mut self) {
        self.selection_commit = true;
    }

    pub fn set_selection(
        &mut self,
        depth: usize,
        positions: &[usize],
        focused: Option<usize>,
    ) -> bool {
        let Some(column) = self.columns.get_mut(depth) else {
            return false;
        };
        let out_of_range = positions
            .iter()
            .any(|position| *position >= column.entries.len())
            || focused.is_some_and(|position| position >= column.entries.len());
        if out_of_range {
            return false;
        }
        let mut positions = positions.to_vec();
        positions.sort_unstable();
        positions.dedup();
        let report = match self.selection_commit {
            true => ReportedFill::Changed(None),
            false => column.classify_reported_fill(&positions),
        };
        let ReportedFill::Changed(located) = report else {
            self.active_column = Some(depth);
            return true;
        };
        let commit = std::mem::take(&mut self.selection_commit);
        column.adopt_reported_fill(positions, located, commit);
        column.selected = focused.or(column.selected);
        self.visual = None;
        if column.selection_anchor.is_none() {
            column.selection_anchor = column.cursor_location();
        }
        self.active_column = Some(depth);
        true
    }

    pub fn clear_active_selection(&mut self) -> Option<(usize, usize)> {
        let depth = self.active_depth()?;
        let column = self.columns.get_mut(depth)?;
        if column.selected_locations.is_empty() {
            return None;
        }
        let focused = column.selected.unwrap_or(0);
        adopt_selected_locations(column, HashSet::new(), true);
        self.visual = None;
        Some((depth, focused))
    }

    pub fn selection_anchor_position(&self, depth: usize) -> Option<usize> {
        let column = self.columns.get(depth)?;
        let anchor = column.selection_anchor.as_ref()?;
        column
            .entries
            .iter()
            .position(|entry| &entry.location == anchor)
    }

    pub fn set_selection_anchor(&mut self, depth: usize, position: usize) -> bool {
        let Some(column) = self.columns.get_mut(depth) else {
            return false;
        };
        let Some(entry) = column.entries.get(position) else {
            return false;
        };
        column.selection_anchor = Some(entry.location.clone());
        true
    }

    pub fn selected_positions(&self, depth: usize) -> Vec<usize> {
        let Some(column) = self.columns.get(depth) else {
            return Vec::new();
        };
        column.selected_positions().to_vec()
    }

    pub fn active_selection_contains(&self, depth: usize, position: usize) -> bool {
        if self.active_column != Some(depth) {
            return false;
        }
        self.columns
            .get(depth)
            .is_some_and(|column| column.selected_positions().contains(&position))
    }

    /// Clone-free length for hot selection paths.
    pub fn selected_count(&self) -> usize {
        let Some(depth) = self.active_column else {
            return 0;
        };
        let Some(column) = self.columns.get(depth) else {
            return 0;
        };
        column.selected_locations.len()
    }

    pub fn selected_entries(&self) -> Vec<FileEntry> {
        let Some(depth) = self.active_column else {
            return Vec::new();
        };
        let Some(column) = self.columns.get(depth) else {
            return Vec::new();
        };
        column
            .selected_positions()
            .iter()
            .map(|position| column.entries[*position].clone())
            .collect()
    }

    /// The pane's committed fill in listing order. A load cursor is not a fill,
    /// and another pane's open-path marker is never a target.
    pub fn filled_entries(&self, depth: usize) -> Vec<FileEntry> {
        let Some(column) = self.columns.get(depth) else {
            return Vec::new();
        };
        if column.load_cursor.is_some() || column.selected_locations.is_empty() {
            return Vec::new();
        }
        let visible = |entry: &FileEntry| column.preferences.show_hidden || !entry.is_hidden;
        column
            .selected_positions()
            .iter()
            .map(|position| &column.entries[*position])
            .filter(|entry| visible(entry))
            .cloned()
            .collect()
    }

    /// `filled_entries`, or the pane's cursor item when nothing is filled.
    pub fn command_entries(&self, depth: usize) -> Vec<FileEntry> {
        let filled = self.filled_entries(depth);
        if !filled.is_empty() {
            return filled;
        }
        let Some(column) = self.columns.get(depth) else {
            return Vec::new();
        };
        let visible = |entry: &FileEntry| column.preferences.show_hidden || !entry.is_hidden;
        column
            .selected
            .and_then(|position| column.entries.get(position))
            .filter(|entry| visible(entry))
            .cloned()
            .into_iter()
            .collect()
    }

    pub fn selection_is_load_cursor(&self) -> bool {
        self.active_column
            .and_then(|depth| self.columns.get(depth))
            .is_some_and(|column| column.load_cursor.is_some())
    }
}

pub(super) fn adopt_selected_locations(
    column: &mut ColumnState,
    locations: HashSet<Location>,
    commit: bool,
) {
    if commit {
        column.load_cursor = None;
        column.drop_load_intents();
    }
    column.selected_locations = locations.into();
}

pub(super) fn adopt_selected_positions(
    column: &mut ColumnState,
    mut positions: Vec<usize>,
    commit: bool,
) {
    if commit {
        column.load_cursor = None;
        column.drop_load_intents();
    }
    positions.sort_unstable();
    positions.dedup();
    column
        .selected_locations
        .select_positions(&column.entries, positions, || {
            lists_each_location_once(&column.entries, &column.each_location_once)
        });
}

/// A selection the view reported, compared with the pane's current fill.
enum ReportedFill {
    /// The report only echoes what the pane already shows.
    Echo,
    /// A new fill, with its locations when the comparison had to resolve them.
    Changed(Option<HashSet<Location>>),
}

impl ColumnState {
    /// Repeating the fill is a widget echo, as is a report of only the cursor
    /// while a different fill remains. After a clear, that cursor report is the
    /// click on the still-focused file, so it becomes the selection.
    fn classify_reported_fill(&mut self, positions: &[usize]) -> ReportedFill {
        let Some(cursor) = self.selected.and_then(|cursor| self.entries.get(cursor)) else {
            return ReportedFill::Changed(None);
        };
        let (unchanged, located) = self.compare_reported_fill(positions);
        if !unchanged && !self.cursor_only_outside_fill(&cursor.location, positions) {
            return ReportedFill::Changed(located);
        }
        if unchanged && located.is_some() {
            self.remember_fill_positions(positions.to_vec());
        }
        ReportedFill::Echo
    }

    /// Whether `positions` match the fill, with the locations they name when
    /// the fill keeps no positions to compare against.
    fn compare_reported_fill(&self, positions: &[usize]) -> (bool, Option<HashSet<Location>>) {
        if let Some(previous) = self
            .selected_locations
            .exact_positions(|| lists_each_location_once(&self.entries, &self.each_location_once))
        {
            return (previous == positions, None);
        }
        let locations: HashSet<_> = positions
            .iter()
            .map(|position| self.entries[*position].location.clone())
            .collect();
        let unchanged = self
            .selected_locations
            .equals_explicit(&locations, &self.entries);
        (unchanged, Some(locations))
    }

    fn cursor_only_outside_fill(&self, cursor: &Location, positions: &[usize]) -> bool {
        !self.selected_locations.is_empty()
            && !self.selected_locations.contains(cursor)
            && !positions.is_empty()
            && positions
                .iter()
                .all(|position| self.entries[*position].location == *cursor)
    }

    fn adopt_reported_fill(
        &mut self,
        positions: Vec<usize>,
        located: Option<HashSet<Location>>,
        commit: bool,
    ) {
        match located {
            Some(locations) => {
                adopt_selected_locations(self, locations, commit);
                self.remember_fill_positions(positions);
            }
            None => adopt_selected_positions(self, positions, commit),
        }
    }

    fn remember_fill_positions(&mut self, positions: Vec<usize>) {
        self.selected_locations.remember(positions, || {
            lists_each_location_once(&self.entries, &self.each_location_once)
        });
    }
}
