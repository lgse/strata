// SPDX-License-Identifier: MIT

use crate::model::{FolderSort, Location, SortDirection, SortKey, ViewPreferences};

use super::{ColumnState, FolderSortResolver, LoadState, NavigationState, sort_entries};

impl ColumnState {
    /// A hidden cursor entry hands the cursor and a single fill to its nearest
    /// visible neighbour, preferring the entries after it.
    fn move_cursor_off_hidden(&mut self) {
        let Some(selected) = self
            .selected
            .filter(|&selected| self.entries.get(selected).is_some_and(|e| e.is_hidden))
        else {
            return;
        };
        let nearest_visible = (selected + 1..self.entries.len())
            .find(|&i| !self.entries[i].is_hidden)
            .or_else(|| (0..selected).rev().find(|&i| !self.entries[i].is_hidden));
        self.selected = nearest_visible;
        self.selected_locations.clear();
        self.selection_anchor = None;
        if let Some(pos) = nearest_visible {
            let loc = self.entries[pos].location.clone();
            self.selected_locations.insert(loc.clone());
            self.selection_anchor = Some(loc);
        }
    }
}

impl NavigationState {
    pub fn set_show_hidden(&mut self, show_hidden: bool) {
        self.preferences.show_hidden = show_hidden;
        for column in &mut self.columns {
            column.preferences.show_hidden = show_hidden;
            if !show_hidden {
                column.move_cursor_off_hidden();
                column.retain_selected_locations(true);
            }
        }
    }

    /// Existing columns retain their local sort; new columns inherit the shared defaults.
    pub fn set_default_preferences(&mut self, preferences: ViewPreferences) {
        self.preferences = preferences;
    }

    pub fn set_folder_sorts(&mut self, resolver: Option<FolderSortResolver>) {
        self.folder_sorts = resolver;
    }

    pub fn remembers_folder_sorts(&self) -> bool {
        self.folder_sorts.is_some()
    }

    pub fn folder_sort(&self, location: &Location) -> FolderSort {
        self.resolve_folder_sort(location, false)
    }

    pub(super) fn resolve_folder_sort(&self, location: &Location, opened: bool) -> FolderSort {
        self.folder_sorts
            .as_ref()
            .map_or(FolderSort::Unremembered, |resolve| {
                resolve(location, opened)
            })
    }

    /// `kept` is the column's preferences before a rebuild of the same path,
    /// whose sort survives when its folder's sort is not remembered.
    pub(super) fn initial_preferences(
        &self,
        location: &Location,
        kept: Option<ViewPreferences>,
    ) -> ViewPreferences {
        let mut preferences = preferences_for_location(self.preferences, location);
        match (self.resolve_folder_sort(location, true), kept) {
            (FolderSort::Saved(sort_key, sort_direction, comparison), _) => {
                preferences.set_sort_options((sort_key, sort_direction, comparison));
            }
            (FolderSort::Unremembered, Some(kept)) => {
                preferences.set_sort_options(kept.sort_options());
            }
            _ => {}
        }
        preferences
    }

    /// What a column's preferences become after its folder's sort or the
    /// application-wide folders-first choice changed. Recent keeps its fixed order.
    pub fn synchronized_preferences(&self, depth: usize) -> Option<ViewPreferences> {
        let column = self.columns.get(depth)?;
        let mut preferences = column.preferences;
        if column.location.is_recent_root() {
            return Some(preferences);
        }
        preferences.folders_first = self.preferences.folders_first;
        match self.folder_sort(&column.location) {
            FolderSort::Saved(sort_key, sort_direction, comparison) => {
                preferences.set_sort_options((sort_key, sort_direction, comparison));
            }
            FolderSort::Default => preferences.set_sort_options(self.preferences.sort_options()),
            FolderSort::Unremembered => {}
        }
        Some(preferences)
    }

    /// For a column still loading with nothing listed yet: its load sorts with
    /// these once it finishes. Listed entries need a sort, because later
    /// batches merge into them in order.
    pub fn set_loading_column_preferences(
        &mut self,
        depth: usize,
        preferences: ViewPreferences,
    ) -> bool {
        let Some(column) = self
            .columns
            .get_mut(depth)
            .filter(|column| column.load_state == LoadState::Loading && column.entries.is_empty())
        else {
            return false;
        };
        column.preferences = preferences;
        true
    }

    pub fn column_preferences(&self, depth: usize) -> Option<ViewPreferences> {
        self.columns.get(depth).map(|column| column.preferences)
    }

    pub fn apply_sort_preferences(
        &mut self,
        depth: usize,
        preferences: ViewPreferences,
    ) -> Option<(Option<usize>, Vec<usize>)> {
        if depth >= self.columns.len() {
            return None;
        }
        let recent = self.columns[depth].location.is_recent_root();
        if !recent && preferences.sort_key == SortKey::Recency {
            return None;
        }
        let preferences = if recent {
            ViewPreferences {
                folders_first: false,
                ..preferences
            }
        } else {
            preferences
        };
        let column = &mut self.columns[depth];
        let selected_location = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        column.preferences = preferences;
        column.metadata_positions = None;
        if preferences.sort_key != SortKey::DeviceOrder {
            column.entries = sort_entries(std::mem::take(&mut column.entries), preferences);
            column.leftovers.take();
            column.selected_locations.forget_positions();
        }
        column.selected = selected_location.and_then(|location| {
            column
                .entries
                .iter()
                .position(|entry| entry.location == location)
        });
        Some((column.selected, column.selected_positions().to_vec()))
    }
}

pub(super) fn preferences_for_location(
    mut preferences: ViewPreferences,
    location: &Location,
) -> ViewPreferences {
    if location.is_recent_root() {
        preferences.folders_first = false;
        preferences.sort_key = SortKey::Recency;
        preferences.sort_direction = SortDirection::Descending;
    } else if location.is_camera_photo_root() {
        preferences.sort_key = SortKey::DeviceOrder;
    }
    preferences
}
