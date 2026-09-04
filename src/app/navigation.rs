// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

use crate::{
    app::peek::PeekState,
    model::{FileEntry, Location, MetadataValue, SortDirection, SortKey, ViewPreferences},
    services::{DirectoryChange, MetadataUpdate, RequestId},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadState {
    Loading,
    Ready,
    Empty,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct EntryInsertion {
    pub position: usize,
    pub entries: Vec<FileEntry>,
}

#[derive(Clone, Debug)]
pub struct EntrySplice {
    pub position: usize,
    pub removed: usize,
    pub entries: Vec<FileEntry>,
}

#[derive(Clone, Debug)]
pub struct ColumnState {
    pub location: Location,
    pub entries: Vec<FileEntry>,
    pub selected: Option<usize>,
    selected_locations: HashSet<Location>,
    selection_anchor: Option<Location>,
    selection_target: Option<Location>,
    pub load_state: LoadState,
    pub truncated: bool,
    preferences: ViewPreferences,
    request_id: RequestId,
    select_first_on_load: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NavigationPath {
    locations: Vec<Location>,
}

impl NavigationPath {
    pub fn from_locations(locations: Vec<Location>) -> Self {
        Self { locations }
    }

    pub fn locations(&self) -> &[Location] {
        &self.locations
    }

    fn parent(&self) -> Option<Self> {
        if self.locations.len() > 1 {
            let mut locations = self.locations.clone();
            locations.pop();
            return Some(Self { locations });
        }

        let current = self.locations.first()?;
        Some(Self::from_locations(vec![current.parent()?]))
    }
}

#[derive(Default)]
pub struct NavigationState {
    pub columns: Vec<ColumnState>,
    active_column: Option<usize>,
    peek: Option<PeekState>,
    back_history: Vec<NavigationPath>,
    forward_history: Vec<NavigationPath>,
    preferences: ViewPreferences,
}

impl NavigationState {
    pub fn with_preferences(preferences: ViewPreferences) -> Self {
        Self {
            preferences,
            ..Self::default()
        }
    }

    pub fn navigate(&mut self, location: Location, request_id: RequestId) {
        self.record_navigation();
        self.restore(NavigationPath::from_locations(vec![location]), [request_id]);
    }

    pub fn descend(
        &mut self,
        parent_depth: usize,
        location: Location,
        request_id: RequestId,
    ) -> bool {
        if parent_depth >= self.columns.len() {
            return false;
        }

        self.record_navigation();
        self.peek = None;
        self.columns.truncate(parent_depth + 1);
        self.push_column(location, request_id);
        self.active_column = self.columns.len().checked_sub(1);
        true
    }

    pub fn can_go_back(&self) -> bool {
        !self.back_history.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward_history.is_empty()
    }

    pub fn can_go_parent(&self) -> bool {
        self.current_path().and_then(|path| path.parent()).is_some()
    }

    pub fn go_back(&mut self) -> Option<NavigationPath> {
        let target = self.back_history.pop()?;
        if let Some(current) = self.current_path() {
            self.forward_history.push(current);
        }
        Some(target)
    }

    pub fn go_forward(&mut self) -> Option<NavigationPath> {
        let target = self.forward_history.pop()?;
        if let Some(current) = self.current_path() {
            self.back_history.push(current);
        }
        Some(target)
    }

    pub fn go_parent(&mut self) -> Option<NavigationPath> {
        let target = self.current_path()?.parent()?;
        self.record_navigation();
        Some(target)
    }

    pub fn restore(
        &mut self,
        path: NavigationPath,
        request_ids: impl IntoIterator<Item = RequestId>,
    ) {
        self.peek = None;
        let preferences = self.preferences;
        self.columns = path
            .locations
            .into_iter()
            .zip(request_ids)
            .map(|(location, request_id)| ColumnState {
                location,
                entries: Vec::new(),
                selected: None,
                selected_locations: HashSet::new(),
                selection_anchor: None,
                selection_target: None,
                load_state: LoadState::Loading,
                truncated: false,
                preferences,
                request_id,
                select_first_on_load: false,
            })
            .collect();
        self.active_column = self.columns.len().checked_sub(1);
    }

    pub fn current_path(&self) -> Option<NavigationPath> {
        (!self.columns.is_empty()).then(|| {
            NavigationPath::from_locations(
                self.columns
                    .iter()
                    .map(|column| column.location.clone())
                    .collect(),
            )
        })
    }

    pub fn path_after_external_change(
        &self,
        origin_depth: usize,
        change: &DirectoryChange,
    ) -> Option<NavigationPath> {
        let mut locations = self.current_path()?.locations;
        match change {
            DirectoryChange::Move { from, entry } => {
                let mut changed = false;
                for location in locations.iter_mut().skip(origin_depth + 1) {
                    let Some(rebased) = location.rebase(from, &entry.location) else {
                        continue;
                    };
                    *location = rebased;
                    changed = true;
                }
                changed.then(|| NavigationPath::from_locations(locations))
            }
            DirectoryChange::Remove(removed) => {
                let affected = locations
                    .iter()
                    .enumerate()
                    .skip(origin_depth + 1)
                    .find(|(_, location)| location.is_within(removed))
                    .map(|(depth, _)| depth)?;
                locations.truncate(affected);
                Some(NavigationPath::from_locations(locations))
            }
            DirectoryChange::Upsert(_) | DirectoryChange::Rescan => None,
        }
    }

    fn record_navigation(&mut self) {
        if let Some(current) = self.current_path() {
            self.back_history.push(current);
            self.forward_history.clear();
        }
    }

    fn push_column(&mut self, location: Location, request_id: RequestId) {
        self.columns.push(ColumnState {
            location,
            entries: Vec::new(),
            selected: None,
            selected_locations: HashSet::new(),
            selection_anchor: None,
            selection_target: None,
            load_state: LoadState::Loading,
            truncated: false,
            preferences: self.preferences,
            request_id,
            select_first_on_load: false,
        });
    }

    pub fn select_first_on_load(&mut self, depth: usize) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.select_first_on_load = true;
        }
    }

    pub fn apply_batch(
        &mut self,
        request_id: RequestId,
        entries: Vec<FileEntry>,
    ) -> Option<(usize, Vec<EntryInsertion>)> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        let preferences = column.preferences;
        let selected_location = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        let (merged, insertions) =
            merge_entries(std::mem::take(&mut column.entries), entries, preferences);
        column.entries = merged;
        if let Some(selected_location) =
            selected_location.or_else(|| column.selection_target.clone())
        {
            column.selected = column
                .entries
                .iter()
                .position(|entry| entry.location == selected_location);
            if column.selected.is_some() {
                column.selection_target = None;
            }
        }
        if column.select_first_on_load && !column.entries.is_empty() {
            let first_visible = column
                .entries
                .iter()
                .position(|entry| preferences.show_hidden || !entry.is_hidden);
            if let Some(position) = first_visible {
                let location = column.entries[position].location.clone();
                column.selected = Some(position);
                column.selected_locations.clear();
                column.selected_locations.insert(location.clone());
                column.selection_anchor = Some(location);
                column.select_first_on_load = false;
            }
        }
        Some((depth, insertions))
    }

    /// Entries must arrive pre-sorted; the caller marks the load finished and publishes.
    pub fn install_snapshot(
        &mut self,
        request_id: RequestId,
        entries: Vec<FileEntry>,
    ) -> Option<usize> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        column.entries = entries;
        if let Some(selected_location) = column.selection_target.clone() {
            column.selected = column
                .entries
                .iter()
                .position(|entry| entry.location == selected_location);
            if column.selected.is_some() {
                column.selection_target = None;
            }
        }
        if column.select_first_on_load && !column.entries.is_empty() {
            let first_visible = column
                .entries
                .iter()
                .position(|entry| column.preferences.show_hidden || !entry.is_hidden);
            if let Some(position) = first_visible {
                let location = column.entries[position].location.clone();
                column.selected = Some(position);
                column.selected_locations.clear();
                column.selected_locations.insert(location.clone());
                column.selection_anchor = Some(location);
                column.select_first_on_load = false;
            }
        }
        Some(depth)
    }

    pub fn open_load_depth(&self, request_id: RequestId) -> Option<usize> {
        self.columns.iter().enumerate().find_map(|(depth, column)| {
            (column.request_id == request_id && column.load_state == LoadState::Loading)
                .then_some(depth)
        })
    }
    /// Order never changes, so views can refresh rows in place.
    pub fn apply_metadata(
        &mut self,
        request_id: RequestId,
        updates: Vec<MetadataUpdate>,
    ) -> Option<(usize, Vec<usize>)> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        let updates: HashMap<&Location, &MetadataUpdate> = updates
            .iter()
            .map(|update| (&update.location, update))
            .collect();
        let mut positions = Vec::new();
        for (position, entry) in column.entries.iter_mut().enumerate() {
            if let Some(update) = updates.get(&entry.location)
                && apply_metadata_update(entry, update)
            {
                positions.push(position);
            }
        }
        if positions.is_empty() {
            return None;
        }
        Some((depth, positions))
    }

    /// Stale rows keep their placeholders and retry on the next bind.
    pub fn apply_positioned_metadata(
        &mut self,
        request_id: RequestId,
        updates: Vec<(usize, MetadataUpdate)>,
    ) -> Option<(usize, Vec<usize>, Vec<Location>)> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        let mut positions = Vec::new();
        let mut stale = Vec::new();
        for (position, update) in &updates {
            let current = column.entries.get(*position);
            if current.is_some_and(|entry| entry.location == update.location) {
                let entry = column.entries.get_mut(*position).expect("position checked");
                if apply_metadata_update(entry, update) {
                    positions.push(*position);
                }
            } else {
                stale.push(update.location.clone());
            }
        }
        if positions.is_empty() && stale.is_empty() {
            return None;
        }
        Some((depth, positions, stale))
    }

    pub fn apply_directory_change(
        &mut self,
        depth: usize,
        watched: &Location,
        change: DirectoryChange,
    ) -> Option<(Vec<EntrySplice>, Option<usize>)> {
        let column = self
            .columns
            .get_mut(depth)
            .filter(|column| &column.location == watched)?;
        let preferences = column.preferences;
        let mut selected_location = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        let mut splices = Vec::new();

        match change {
            DirectoryChange::Upsert(entry) => {
                if column
                    .entries
                    .iter()
                    .find(|current| current.location == entry.location)
                    == Some(&entry)
                {
                    return None;
                }
                remove_monitored_entry(&mut column.entries, &entry.location, &mut splices);
                insert_monitored_entry(&mut column.entries, entry, preferences, &mut splices);
            }
            DirectoryChange::Remove(location) => {
                let removed_position = column
                    .entries
                    .iter()
                    .position(|entry| entry.location == location);
                let selected_was_removed = selected_location.as_ref() == Some(&location);
                column.selected_locations.remove(&location);
                remove_monitored_entry(&mut column.entries, &location, &mut splices);
                if selected_was_removed {
                    selected_location = removed_position.and_then(|position| {
                        column
                            .entries
                            .get(position.min(column.entries.len().saturating_sub(1)))
                            .map(|entry| entry.location.clone())
                    });
                }
            }
            DirectoryChange::Move { from, entry } => {
                if selected_location.as_ref() == Some(&from) {
                    selected_location = Some(entry.location.clone());
                }
                if column.selected_locations.remove(&from) {
                    column.selected_locations.insert(entry.location.clone());
                }
                remove_monitored_entry(&mut column.entries, &from, &mut splices);
                if entry.location != from {
                    remove_monitored_entry(&mut column.entries, &entry.location, &mut splices);
                }
                insert_monitored_entry(&mut column.entries, entry, preferences, &mut splices);
            }
            DirectoryChange::Rescan => return None,
        }

        if splices.is_empty() {
            return None;
        }
        column.selected = selected_location.and_then(|location| {
            column
                .entries
                .iter()
                .position(|entry| entry.location == location)
        });
        column.selected_locations.retain(|location| {
            column
                .entries
                .iter()
                .any(|entry| &entry.location == location)
        });
        column.load_state = if column.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
        Some((splices, column.selected))
    }

    pub fn reload_column(&mut self, depth: usize, request_id: RequestId) -> Option<Location> {
        let column = self.columns.get_mut(depth)?;
        column.selection_target = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        column.entries.clear();
        column.selected = None;
        column.load_state = LoadState::Loading;
        column.truncated = false;
        column.request_id = request_id;
        Some(column.location.clone())
    }

    pub fn set_show_hidden(&mut self, show_hidden: bool) {
        self.preferences.show_hidden = show_hidden;
        for column in &mut self.columns {
            column.preferences.show_hidden = show_hidden;
            if !show_hidden {
                if let Some(selected) = column.selected
                    && column.entries.get(selected).is_some_and(|e| e.is_hidden)
                {
                    let nearest_visible = (selected + 1..column.entries.len())
                        .find(|&i| !column.entries[i].is_hidden)
                        .or_else(|| (0..selected).rev().find(|&i| !column.entries[i].is_hidden));
                    column.selected = nearest_visible;
                    column.selected_locations.clear();
                    if let Some(pos) = nearest_visible {
                        let loc = column.entries[pos].location.clone();
                        column.selected_locations.insert(loc.clone());
                        column.selection_anchor = Some(loc);
                    } else {
                        column.selection_anchor = None;
                    }
                }
                column.selected_locations.retain(|loc| {
                    column
                        .entries
                        .iter()
                        .any(|entry| &entry.location == loc && !entry.is_hidden)
                });
            }
        }
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
        self.preferences.sort_key = preferences.sort_key;
        self.preferences.sort_direction = preferences.sort_direction;
        self.preferences.folders_first = preferences.folders_first;
        let column = &mut self.columns[depth];
        let selected_location = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        column.preferences = preferences;
        // Unstable: tie-breakers in `compare_entries` keep distinct entries ordered.
        column
            .entries
            .sort_unstable_by(|left, right| compare_entries(left, right, preferences));
        column.selected = selected_location.and_then(|location| {
            column
                .entries
                .iter()
                .position(|entry| entry.location == location)
        });
        let selected_positions = column
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| {
                column
                    .selected_locations
                    .contains(&entry.location)
                    .then_some(position)
            })
            .collect();
        Some((column.selected, selected_positions))
    }

    pub fn active_focus(&self) -> Option<(usize, Option<usize>)> {
        let depth = self.active_column?;
        Some((depth, self.columns.get(depth)?.selected))
    }

    pub fn active_location(&self) -> Option<Location> {
        let depth = self.active_column?;
        Some(self.columns.get(depth)?.location.clone())
    }

    pub fn active_depth(&self) -> Option<usize> {
        self.active_column
    }

    pub fn location_at(&self, depth: usize) -> Option<Location> {
        Some(self.columns.get(depth)?.location.clone())
    }

    pub fn finish(&mut self, request_id: RequestId, truncated: bool) -> Option<usize> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        column.select_first_on_load = false;
        column.truncated = truncated;
        column.load_state = if column.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
        Some(depth)
    }

    pub fn fail(&mut self, request_id: RequestId, message: String) -> Option<usize> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        column.load_state = LoadState::Error(message);
        Some(depth)
    }

    pub fn begin_peek(
        &mut self,
        origin_depth: usize,
        location: Location,
        request_id: RequestId,
    ) -> bool {
        if origin_depth >= self.columns.len() {
            return false;
        }
        self.peek = Some(PeekState::new(origin_depth, location, request_id));
        true
    }

    pub fn peek_target(&self) -> Option<(usize, Location)> {
        self.peek
            .as_ref()
            .map(|peek| (peek.origin_depth, peek.location.clone()))
    }

    pub fn clear_peek(&mut self) -> bool {
        self.peek.take().is_some()
    }

    pub fn apply_peek_batch(&mut self, request_id: RequestId, entries: &[FileEntry]) -> bool {
        let Some(peek) = self.peek.as_mut().filter(|peek| peek.accepts(request_id)) else {
            return false;
        };
        peek.append(entries);
        true
    }

    pub fn finish_peek(&mut self, request_id: RequestId) -> bool {
        let Some(peek) = self.peek.as_mut().filter(|peek| peek.accepts(request_id)) else {
            return false;
        };
        peek.finish();
        true
    }

    pub fn fail_peek(&mut self, request_id: RequestId, message: String) -> bool {
        let Some(peek) = self.peek.as_mut().filter(|peek| peek.accepts(request_id)) else {
            return false;
        };
        peek.fail(message);
        true
    }

    pub fn select(&mut self, depth: usize, position: usize) -> bool {
        let Some(column) = self.columns.get_mut(depth) else {
            return false;
        };
        let Some(entry) = column.entries.get(position) else {
            return false;
        };
        column.selected = Some(position);
        column.selected_locations.clear();
        column.selected_locations.insert(entry.location.clone());
        column.selection_anchor = Some(entry.location.clone());
        self.active_column = Some(depth);
        true
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
        if positions
            .iter()
            .any(|position| *position >= column.entries.len())
            || focused.is_some_and(|position| position >= column.entries.len())
        {
            return false;
        }
        column.selected_locations = positions
            .iter()
            .map(|position| column.entries[*position].location.clone())
            .collect();
        column.selected = focused.filter(|position| positions.contains(position));
        if column.selection_anchor.is_none() {
            column.selection_anchor = column
                .selected
                .and_then(|position| column.entries.get(position))
                .map(|entry| entry.location.clone());
        }
        self.active_column = Some(depth);
        true
    }

    pub fn extend_selection(&mut self, direction: i32) -> Option<(usize, usize, Vec<usize>)> {
        let depth = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))?;
        let column = self.columns.get_mut(depth)?;
        if column.entries.is_empty() {
            return None;
        }

        let show_hidden = column.preferences.show_hidden;
        let is_visible = |entry: &FileEntry| show_hidden || !entry.is_hidden;

        let first_visible = column.entries.iter().position(is_visible)?;
        let last_visible = column.entries.iter().rposition(is_visible)?;

        let current = column.selected.unwrap_or(if direction < 0 {
            last_visible
        } else {
            first_visible
        });

        let focused = if direction < 0 {
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

        let anchor = column
            .selection_anchor
            .as_ref()
            .and_then(|location| {
                column
                    .entries
                    .iter()
                    .position(|entry| &entry.location == location)
            })
            .unwrap_or(current);
        if column.selection_anchor.is_none() {
            column.selection_anchor = Some(column.entries[anchor].location.clone());
        }
        let start = anchor.min(focused);
        let end = anchor.max(focused);
        let selected_positions: Vec<usize> = (start..=end)
            .filter(|&index| is_visible(&column.entries[index]))
            .collect();
        column.selected_locations = selected_positions
            .iter()
            .map(|&index| column.entries[index].location.clone())
            .collect();
        column.selected = Some(focused);
        self.active_column = Some(depth);
        Some((depth, focused, selected_positions))
    }

    pub fn selected_positions(&self, depth: usize) -> Vec<usize> {
        let Some(column) = self.columns.get(depth) else {
            return Vec::new();
        };
        column
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| {
                column
                    .selected_locations
                    .contains(&entry.location)
                    .then_some(position)
            })
            .collect()
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
            .entries
            .iter()
            .filter(|entry| column.selected_locations.contains(&entry.location))
            .cloned()
            .collect()
    }

    pub fn move_selection(&mut self, direction: i32) -> Option<(usize, usize)> {
        let depth = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))?;
        let column = self.columns.get_mut(depth)?;
        if column.entries.is_empty() {
            return None;
        }

        let show_hidden = column.preferences.show_hidden;
        let is_visible = |entry: &FileEntry| show_hidden || !entry.is_hidden;

        if !column.entries.iter().any(is_visible) {
            column.selected = None;
            column.selected_locations.clear();
            column.selection_anchor = None;
            return None;
        }

        let position = match (column.selected, direction.cmp(&0)) {
            (None, std::cmp::Ordering::Less) => column.entries.iter().rposition(is_visible)?,
            (None, _) => column.entries.iter().position(is_visible)?,
            (Some(current), std::cmp::Ordering::Less) => column.entries[..current]
                .iter()
                .rposition(is_visible)
                .unwrap_or_else(|| {
                    if is_visible(&column.entries[current]) {
                        current
                    } else {
                        column
                            .entries
                            .iter()
                            .position(is_visible)
                            .unwrap_or(current)
                    }
                }),
            (Some(current), std::cmp::Ordering::Greater) => {
                if current + 1 < column.entries.len() {
                    column.entries[current + 1..]
                        .iter()
                        .position(is_visible)
                        .map(|offset| current + 1 + offset)
                        .unwrap_or_else(|| {
                            if is_visible(&column.entries[current]) {
                                current
                            } else {
                                column
                                    .entries
                                    .iter()
                                    .rposition(is_visible)
                                    .unwrap_or(current)
                            }
                        })
                } else if is_visible(&column.entries[current]) {
                    current
                } else {
                    column
                        .entries
                        .iter()
                        .rposition(is_visible)
                        .unwrap_or(current)
                }
            }
            (Some(current), std::cmp::Ordering::Equal) => {
                if is_visible(&column.entries[current]) {
                    current
                } else {
                    column.entries.iter().position(is_visible)?
                }
            }
        };
        focus_only(column, position);
        self.active_column = Some(depth);
        Some((depth, position))
    }

    /// Moves the focus `page` visible entries at a time, clamped to the first and
    /// last visible entry, for page-sized keyboard navigation.
    pub fn page_selection(&mut self, direction: i32, page: usize) -> Option<(usize, usize)> {
        if direction == 0 {
            return None;
        }
        let depth = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))?;
        let column = self.columns.get_mut(depth)?;
        let show_hidden = column.preferences.show_hidden;
        let visible: Vec<usize> = column
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| show_hidden || !entry.is_hidden)
            .map(|(position, _)| position)
            .collect();
        let last = visible.len().checked_sub(1)?;
        let steps = page.max(1);
        let current = column
            .selected
            .and_then(|selected| visible.iter().position(|position| *position >= selected));
        let target = match (current, direction < 0) {
            (None, true) => last,
            (None, false) => 0,
            (Some(current), true) => current.saturating_sub(steps),
            (Some(current), false) => current.saturating_add(steps).min(last),
        };
        let position = visible[target];
        focus_only(column, position);
        self.active_column = Some(depth);
        Some((depth, position))
    }

    pub fn focus_parent(&mut self) -> Option<(usize, Option<usize>)> {
        let depth = self.active_column?;
        let parent_depth = depth.checked_sub(1)?;
        self.active_column = Some(parent_depth);
        Some((parent_depth, self.columns[parent_depth].selected))
    }

    pub fn focus_child(&mut self) -> Option<(usize, Option<usize>)> {
        let child_depth = self.active_column?.checked_add(1)?;
        let position = self.columns.get(child_depth)?.selected;
        self.active_column = Some(child_depth);
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
        self.peek = None;
        self.columns.truncate(depth);
        let parent_depth = depth - 1;
        self.active_column = Some(parent_depth);
        Some((parent_depth, self.columns[parent_depth].selected))
    }

    pub fn entry_at(&self, depth: usize, position: usize) -> Option<FileEntry> {
        self.columns.get(depth)?.entries.get(position).cloned()
    }

    pub fn active_child_position(&self, depth: usize) -> Option<usize> {
        let child = &self.columns.get(depth + 1)?.location;
        self.columns
            .get(depth)?
            .entries
            .iter()
            .position(|entry| &entry.location == child)
    }

    pub fn focused_entry(&self) -> Option<(usize, usize, FileEntry)> {
        let depth = self.active_column?;
        let column = self.columns.get(depth)?;
        let position = column.selected?;
        let entry = column.entries.get(position)?.clone();
        Some((depth, position, entry))
    }
    pub fn loading_column(&self, request_id: RequestId) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(depth, column)| {
            (column.request_id == request_id).then_some((depth, column.entries.len()))
        })
    }

    /// Directories qualify for mtime only; directory size stays unknown by design.
    pub fn column_unknown_metadata(&self, depth: usize) -> Option<Vec<(usize, Location)>> {
        let column = self.columns.get(depth)?;
        let gap: Vec<(usize, Location)> = column
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.modified_unix_seconds == MetadataValue::Unknown
                    || (!entry.is_directory() && entry.size == MetadataValue::Unknown)
            })
            .map(|(position, entry)| (position, entry.location.clone()))
            .collect();
        if gap.is_empty() {
            return None;
        }
        Some(gap)
    }

    /// Follow-up fills using this request die with a reload.
    pub fn request_id_for_depth(&self, depth: usize) -> Option<RequestId> {
        self.columns.get(depth).map(|column| column.request_id)
    }

    pub fn depth_for_request(&self, request_id: RequestId) -> Option<usize> {
        self.columns
            .iter()
            .position(|column| column.request_id == request_id)
    }
    fn column_for_request_mut(
        &mut self,
        request_id: RequestId,
    ) -> Option<(usize, &mut ColumnState)> {
        self.columns
            .iter_mut()
            .enumerate()
            .find(|(_, column)| column.request_id == request_id)
    }
}

/// Collapses a column's selection onto a single entry and anchors further
/// range selections there.
fn focus_only(column: &mut ColumnState, position: usize) {
    column.selected = Some(position);
    column.selected_locations.clear();
    column
        .selected_locations
        .insert(column.entries[position].location.clone());
    column.selection_anchor = Some(column.entries[position].location.clone());
}

fn apply_metadata_update(entry: &mut FileEntry, update: &MetadataUpdate) -> bool {
    let mut changed = false;
    if update.size != MetadataValue::Unknown && entry.size != update.size {
        entry.size = update.size.clone();
        changed = true;
    }
    if update.modified_unix_seconds != MetadataValue::Unknown
        && entry.modified_unix_seconds != update.modified_unix_seconds
    {
        entry.modified_unix_seconds = update.modified_unix_seconds.clone();
        changed = true;
    }
    changed
}

fn merge_entries(
    mut existing: Vec<FileEntry>,
    mut incoming: Vec<FileEntry>,
    preferences: ViewPreferences,
) -> (Vec<FileEntry>, Vec<EntryInsertion>) {
    incoming.sort_unstable_by(|left, right| compare_entries(left, right, preferences));
    if existing.is_empty() {
        let insertion = EntryInsertion {
            position: 0,
            entries: incoming.clone(),
        };
        return (incoming, vec![insertion]);
    }

    let mut merged = Vec::with_capacity(existing.len() + incoming.len());
    let mut existing = existing.drain(..).peekable();
    let mut incoming = incoming.into_iter().peekable();
    let mut insertions = Vec::<EntryInsertion>::new();

    while existing.peek().is_some() || incoming.peek().is_some() {
        let take_incoming = match (existing.peek(), incoming.peek()) {
            (Some(left), Some(right)) => {
                compare_entries(right, left, preferences) != Ordering::Greater
            }
            (None, Some(_)) => true,
            _ => false,
        };

        if take_incoming {
            let Some(entry) = incoming.next() else {
                break;
            };
            let position = merged.len();
            if let Some(insertion) = insertions
                .last_mut()
                .filter(|insertion| insertion.position + insertion.entries.len() == position)
            {
                insertion.entries.push(entry.clone());
            } else {
                insertions.push(EntryInsertion {
                    position,
                    entries: vec![entry.clone()],
                });
            }
            merged.push(entry);
        } else if let Some(entry) = existing.next() {
            merged.push(entry);
        }
    }

    (merged, insertions)
}

pub(crate) fn sort_entries(
    mut entries: Vec<FileEntry>,
    preferences: ViewPreferences,
) -> Vec<FileEntry> {
    entries.sort_unstable_by(|left, right| compare_entries(left, right, preferences));
    entries
}

fn remove_monitored_entry(
    entries: &mut Vec<FileEntry>,
    location: &Location,
    splices: &mut Vec<EntrySplice>,
) {
    if let Some(position) = entries.iter().position(|entry| &entry.location == location) {
        entries.remove(position);
        splices.push(EntrySplice {
            position,
            removed: 1,
            entries: Vec::new(),
        });
    }
}

fn insert_monitored_entry(
    entries: &mut Vec<FileEntry>,
    entry: FileEntry,
    preferences: ViewPreferences,
    splices: &mut Vec<EntrySplice>,
) {
    let position = entries
        .binary_search_by(|current| compare_entries(current, &entry, preferences))
        .unwrap_or_else(|position| position);
    entries.insert(position, entry.clone());
    splices.push(EntrySplice {
        position,
        removed: 0,
        entries: vec![entry],
    });
}

fn compare_entries(left: &FileEntry, right: &FileEntry, preferences: ViewPreferences) -> Ordering {
    if preferences.folders_first {
        let directory_order = right.is_directory().cmp(&left.is_directory());
        if directory_order != Ordering::Equal {
            return directory_order;
        }
    }

    let ordering = match preferences.sort_key {
        SortKey::Name => compare_display_names(&left.display_name, &right.display_name),
        SortKey::Type => left.kind.cmp(&right.kind),
        SortKey::Size => compare_metadata(&left.size, &right.size),
        SortKey::Modified => {
            compare_metadata(&left.modified_unix_seconds, &right.modified_unix_seconds)
        }
    };
    let ordering = match preferences.sort_direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    };
    ordering
        .then_with(|| compare_display_names(&left.display_name, &right.display_name))
        .then_with(|| left.location.compare(&right.location))
}

fn compare_display_names(left: &str, right: &str) -> Ordering {
    let folded = if left.is_ascii() && right.is_ascii() {
        left.bytes()
            .map(|byte| byte.to_ascii_lowercase())
            .cmp(right.bytes().map(|byte| byte.to_ascii_lowercase()))
    } else {
        glib::casefold(left).cmp(&glib::casefold(right))
    };
    folded.then_with(|| left.cmp(right))
}

fn compare_metadata<T: Ord>(left: &MetadataValue<T>, right: &MetadataValue<T>) -> Ordering {
    match (left, right) {
        (MetadataValue::Known(left), MetadataValue::Known(right)) => left.cmp(right),
        (MetadataValue::Known(_), _) => Ordering::Less,
        (_, MetadataValue::Known(_)) => Ordering::Greater,
        (MetadataValue::Unknown, MetadataValue::Unavailable) => Ordering::Less,
        (MetadataValue::Unavailable, MetadataValue::Unknown) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests;
