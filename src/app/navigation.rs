// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    cmp::Ordering,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    app::peek::PeekState,
    model::{
        FileEntry, FolderSort, Location, MetadataValue, SortDirection, SortKey, ViewPreferences,
    },
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

pub(crate) type EntrySpliceApplication = Option<(Vec<EntrySplice>, Option<usize>)>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ColumnEntryCounts {
    pub total: usize,
    pub files: usize,
    pub folders: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SelectedLocations {
    Explicit(HashSet<Location>),
    All {
        excluded: HashSet<Location>,
        count: usize,
    },
}

impl Default for SelectedLocations {
    fn default() -> Self {
        Self::Explicit(HashSet::new())
    }
}

impl SelectedLocations {
    fn all_visible(entries: &[FileEntry], show_hidden: bool) -> Self {
        let excluded = if show_hidden {
            HashSet::new()
        } else {
            entries
                .iter()
                .filter(|entry| entry.is_hidden)
                .map(|entry| entry.location.clone())
                .collect()
        };
        Self::All {
            count: entries.len().saturating_sub(excluded.len()),
            excluded,
        }
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        match self {
            Self::Explicit(locations) => locations.len(),
            Self::All { count, .. } => *count,
        }
    }

    fn contains(&self, location: &Location) -> bool {
        match self {
            Self::Explicit(locations) => locations.contains(location),
            Self::All { excluded, .. } => !excluded.contains(location),
        }
    }

    fn insert(&mut self, location: Location) -> bool {
        match self {
            Self::Explicit(locations) => locations.insert(location),
            Self::All { excluded, count } => {
                let inserted = excluded.remove(&location);
                if inserted {
                    *count = count.saturating_add(1);
                }
                inserted
            }
        }
    }

    fn remove(&mut self, location: &Location) -> bool {
        match self {
            Self::Explicit(locations) => locations.remove(location),
            Self::All { excluded, count } => {
                let removed = excluded.insert(location.clone());
                if removed {
                    *count = count.saturating_sub(1);
                }
                removed
            }
        }
    }

    fn excludes_new(&mut self, location: Location) {
        if let Self::All { excluded, .. } = self {
            excluded.insert(location);
        }
    }

    fn relocate(&mut self, from: &Location, to: Location) {
        match self {
            Self::Explicit(locations) => {
                if locations.remove(from) {
                    locations.insert(to);
                }
            }
            Self::All { excluded, .. } => {
                if excluded.remove(from) {
                    excluded.insert(to);
                }
            }
        }
    }

    fn materialize(&self, entries: &[FileEntry]) -> HashSet<Location> {
        match self {
            Self::Explicit(locations) => locations.clone(),
            Self::All { excluded, .. } => entries
                .iter()
                .filter(|entry| !excluded.contains(&entry.location))
                .map(|entry| entry.location.clone())
                .collect(),
        }
    }

    fn equals_explicit(&self, locations: &HashSet<Location>, entries: &[FileEntry]) -> bool {
        self.len() == locations.len()
            && match self {
                Self::Explicit(selected) => selected == locations,
                Self::All { excluded, .. } => entries.iter().all(|entry| {
                    excluded.contains(&entry.location) != locations.contains(&entry.location)
                }),
            }
    }

    fn retain(&mut self, entries: &[FileEntry], hide_hidden: bool) {
        match self {
            Self::Explicit(locations) => {
                let mut previous = std::mem::take(locations);
                *locations = entries
                    .iter()
                    .filter(|entry| !hide_hidden || !entry.is_hidden)
                    .filter_map(|entry| previous.take(&entry.location))
                    .collect();
            }
            Self::All { excluded, count } => {
                if hide_hidden {
                    excluded.extend(
                        entries
                            .iter()
                            .filter(|entry| entry.is_hidden)
                            .map(|entry| entry.location.clone()),
                    );
                }
                *count = entries
                    .iter()
                    .filter(|entry| !excluded.contains(&entry.location))
                    .count();
            }
        }
    }
}

impl From<HashSet<Location>> for SelectedLocations {
    fn from(locations: HashSet<Location>) -> Self {
        Self::Explicit(locations)
    }
}

#[derive(Clone, Debug)]
pub struct ColumnState {
    pub location: Location,
    pub entries: Vec<FileEntry>,
    entry_counts: Cell<Option<(bool, ColumnEntryCounts)>>,
    metadata_positions: Option<HashMap<Location, usize>>,
    pub selected: Option<usize>,
    selected_locations: SelectedLocations,
    selection_anchor: Option<Location>,
    selection_target: Option<Location>,
    pending_selection: HashSet<Location>,
    pending_reveal: Vec<Location>,
    selection_from_reveal: bool,
    pub load_state: LoadState,
    pub truncated: bool,
    /// Whether entries here can be moved to Trash, resolved from a listed entry
    /// when the directory loads (see `DirectoryEvent::Finished`). `None` before
    /// the first load finishes, for an empty directory, or when the capability
    /// couldn't be answered; treated as "assume trashable" by consumers.
    pub can_trash: Option<bool>,
    /// Whether entries here can be permanently deleted, resolved the same way
    /// as `can_trash`. `None` carries the same "assume deletable" meaning.
    pub can_delete: Option<bool>,
    preferences: ViewPreferences,
    request_id: RequestId,
    select_first_on_load: bool,
    /// Soft load target: selected if it arrives, otherwise the first visible entry
    /// once the listing is complete. Never reveals hidden files or reports failure.
    preferred_on_load: Option<Location>,
    /// Cursor position before a reload; the cursor moves to its neighbour when the
    /// entry is gone.
    reload_cursor: Option<usize>,
    // Auto-selection must not redirect paste into the first folder.
    load_cursor: Option<Location>,
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

pub type FolderSortResolver = Rc<dyn Fn(&Location) -> FolderSort>;

#[derive(Default)]
pub struct NavigationState {
    pub columns: Vec<ColumnState>,
    active_column: Option<usize>,
    peek: Option<PeekState>,
    back_history: Vec<NavigationPath>,
    forward_history: Vec<NavigationPath>,
    preferences: ViewPreferences,
    folder_sorts: Option<FolderSortResolver>,
    // GTK focus/rebuild selection echoes must not arm paste-into.
    selection_commit: bool,
    selectionless_removals: HashSet<Location>,
    preserve_fill_on_removal: bool,
    visual: Option<VisualRange>,
}

/// A walked range over one pane in displayed order. The fill is recomputed from
/// `base` on every cursor move, so contracting the range restores what it covered.
pub struct VisualRange {
    depth: usize,
    directory: Location,
    anchor: Location,
    kind: VisualKind,
    base: HashSet<Location>,
    // Space inside the range flips an item on top of the walked result.
    toggled: HashSet<Location>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualKind {
    Select,
    Unset,
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
        self.visual = None;
        self.selection_commit = false;
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
        self.visual = None;
        self.selection_commit = false;
        self.columns = path
            .locations
            .into_iter()
            .zip(request_ids)
            .map(|(location, request_id)| ColumnState {
                preferences: self.initial_preferences(&location),
                location,
                entries: Vec::new(),
                entry_counts: Cell::new(None),
                metadata_positions: None,
                selected: None,
                selected_locations: SelectedLocations::default(),
                selection_anchor: None,
                selection_target: None,
                pending_selection: HashSet::new(),
                pending_reveal: Vec::new(),
                selection_from_reveal: false,
                load_state: LoadState::Loading,
                truncated: false,
                can_trash: None,
                can_delete: None,
                request_id,
                select_first_on_load: false,
                preferred_on_load: None,
                reload_cursor: None,
                load_cursor: None,
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
            preferences: self.initial_preferences(&location),
            location,
            entries: Vec::new(),
            entry_counts: Cell::new(None),
            metadata_positions: None,
            selected: None,
            selected_locations: SelectedLocations::default(),
            selection_anchor: None,
            selection_target: None,
            pending_selection: HashSet::new(),
            pending_reveal: Vec::new(),
            selection_from_reveal: false,
            load_state: LoadState::Loading,
            truncated: false,
            can_trash: None,
            can_delete: None,
            request_id,
            select_first_on_load: false,
            preferred_on_load: None,
            reload_cursor: None,
            load_cursor: None,
        });
    }

    pub fn select_first_on_load(&mut self, depth: usize) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.select_first_on_load = true;
        }
    }

    /// Sets the soft load target (`preferred_on_load`) without clearing what the load restores.
    pub fn prefer_location_on_load(&mut self, depth: usize, location: Location) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.preferred_on_load = Some(location);
            column.select_first_on_load = false;
        }
    }

    pub fn select_locations_on_load(&mut self, depth: usize, targets: Vec<Location>) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.preferred_on_load = None;
            column.selected = None;
            column.selected_locations.clear();
            column.selection_anchor = None;
            column.selection_target = None;
            column.pending_selection.clear();
            column.pending_reveal = targets;
            column.selection_from_reveal = false;
            column.select_first_on_load = false;
            column.load_cursor = None;
        }
    }

    pub fn reveal_pending(&self, depth: usize) -> bool {
        self.columns
            .get(depth)
            .is_some_and(|column| !column.pending_reveal.is_empty())
    }

    /// Consume the focus request so later publications cannot steal focus again.
    pub fn take_selection_from_reveal(&mut self, depth: usize) -> bool {
        self.columns
            .get_mut(depth)
            .is_some_and(|column| std::mem::take(&mut column.selection_from_reveal))
    }

    pub fn take_resolved_location_reveal(
        &mut self,
        depth: usize,
        request_id: RequestId,
    ) -> Option<bool> {
        let column = self.columns.get_mut(depth)?;
        if column.request_id != request_id {
            return None;
        }
        let requested: HashSet<&Location> = column.pending_reveal.iter().collect();
        let (mut listed, mut any_hidden) = (0, false);
        for entry in column
            .entries
            .iter()
            .filter(|entry| requested.contains(&entry.location))
        {
            listed += 1;
            any_hidden |= entry.is_hidden;
        }
        if listed == 0 {
            return None;
        }
        if listed == requested.len() {
            column.pending_reveal.clear();
        }
        Some(any_hidden)
    }

    pub fn lists_every_target(&self, depth: usize, targets: &[Location]) -> bool {
        let requested: HashSet<&Location> = targets.iter().collect();
        self.columns.get(depth).is_some_and(|column| {
            column
                .entries
                .iter()
                .filter(|entry| requested.contains(&entry.location))
                .count()
                == requested.len()
        })
    }

    pub fn take_unresolved_location_reveal(
        &mut self,
        depth: usize,
        request_id: RequestId,
    ) -> Option<Location> {
        let column = self.columns.get_mut(depth)?;
        if column.request_id != request_id {
            return None;
        }
        let targets = std::mem::take(&mut column.pending_reveal);
        let requested: HashSet<&Location> = targets.iter().collect();
        if column
            .entries
            .iter()
            .any(|entry| requested.contains(&entry.location))
        {
            return None;
        }
        targets.into_iter().next()
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
        if matches!(column.selected_locations, SelectedLocations::All { .. }) {
            let existing: HashSet<_> = column.entries.iter().map(|entry| &entry.location).collect();
            for entry in &entries {
                if !existing.contains(&entry.location) {
                    column
                        .selected_locations
                        .excludes_new(entry.location.clone());
                }
            }
        }
        let (merged, insertions) =
            merge_entries(std::mem::take(&mut column.entries), entries, preferences);
        column.entries = merged;
        column.invalidate_entry_indexes();
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
        column.resolve_pending_reveal();
        column.restore_pending_selection();
        column.resolve_preferred_on_load();
        if column.select_first_on_load && !column.entries.is_empty() {
            column.select_first_visible_entry();
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
        column.invalidate_entry_indexes();
        if let Some(selected_location) = column.selection_target.clone() {
            column.selected = column
                .entries
                .iter()
                .position(|entry| entry.location == selected_location);
            if column.selected.is_some() {
                column.selection_target = None;
            }
        }
        column.resolve_pending_reveal();
        column.restore_pending_selection();
        column.resolve_preferred_on_load();
        if column.select_first_on_load && !column.entries.is_empty() {
            column.select_first_visible_entry();
        }
        // The listing is complete, and its selection is published before `finish`.
        column.complete_load_selection();
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
        if updates.is_empty() {
            return None;
        }
        // Full-sort fills arrive in small chunks. Build routing once, not a directory scan
        // per chunk; structural changes invalidate it and the sort terminal releases it.
        let index = column.metadata_positions.get_or_insert_with(|| {
            column
                .entries
                .iter()
                .enumerate()
                .map(|(position, entry)| (entry.location.clone(), position))
                .collect()
        });
        let mut positions = Vec::with_capacity(updates.len());
        if index.len() == column.entries.len() {
            for (location, update) in updates {
                if let Some(&position) = index.get(location)
                    && apply_metadata_update(&mut column.entries[position], update)
                {
                    positions.push(position);
                }
            }
        } else {
            // Providers may repeat a location; keep updating every matching entry.
            for (position, entry) in column.entries.iter_mut().enumerate() {
                if let Some(update) = updates.get(&entry.location)
                    && apply_metadata_update(entry, update)
                {
                    positions.push(position);
                }
            }
        }
        if positions.is_empty() {
            return None;
        }
        positions.sort_unstable();
        Some((depth, positions))
    }

    pub(super) fn clear_metadata_positions(&mut self, depth: usize) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.metadata_positions = None;
        }
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

    pub fn set_preserve_fill_on_removal(&mut self, preserve: bool) {
        self.preserve_fill_on_removal = preserve;
    }

    pub fn set_selectionless_removals(&mut self, locations: impl IntoIterator<Item = Location>) {
        self.selectionless_removals = locations.into_iter().collect();
    }

    pub fn retain_selectionless_removals(&mut self, locations: impl IntoIterator<Item = Location>) {
        let locations: HashSet<_> = locations.into_iter().collect();
        self.selectionless_removals
            .retain(|location| locations.contains(location));
    }

    pub fn apply_new_entries_batch(
        &mut self,
        depth: usize,
        watched: &Location,
        entries: Vec<FileEntry>,
    ) -> Result<EntrySpliceApplication, ()> {
        let Some(column) = self
            .columns
            .get_mut(depth)
            .filter(|column| &column.location == watched)
        else {
            return Err(());
        };
        if entries.is_empty() {
            return Ok(None);
        }
        let incoming_locations: HashSet<_> =
            entries.iter().map(|entry| entry.location.clone()).collect();
        if incoming_locations.len() != entries.len()
            || column
                .entries
                .iter()
                .any(|entry| incoming_locations.contains(&entry.location))
        {
            return Err(());
        }

        let selected_location = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        for entry in &entries {
            column
                .selected_locations
                .excludes_new(entry.location.clone());
        }
        let (merged, insertions) = merge_entries(
            std::mem::take(&mut column.entries),
            entries,
            column.preferences,
        );
        column.entries = merged;
        column.selected = selected_location.and_then(|location| {
            column
                .entries
                .iter()
                .position(|entry| entry.location == location)
        });
        column.invalidate_entry_indexes();
        column.load_state = LoadState::Ready;
        Ok(Some((
            insertions
                .into_iter()
                .map(|insertion| EntrySplice {
                    position: insertion.position,
                    removed: 0,
                    entries: insertion.entries,
                })
                .collect(),
            column.selected,
        )))
    }

    pub fn apply_removals_batch(
        &mut self,
        depth: usize,
        watched: &Location,
        locations: impl IntoIterator<Item = Location>,
    ) -> EntrySpliceApplication {
        let column = self
            .columns
            .get_mut(depth)
            .filter(|column| &column.location == watched)?;
        let removals: HashSet<_> = locations.into_iter().collect();
        if removals.is_empty() {
            return None;
        }

        let selected_position = column.selected;
        let selected_location = selected_position
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        let mut retained = Vec::with_capacity(column.entries.len());
        let mut removed_locations = Vec::new();
        let mut splices = Vec::<EntrySplice>::new();
        let mut retained_before_selected = 0;
        for (position, entry) in std::mem::take(&mut column.entries).into_iter().enumerate() {
            if removals.contains(&entry.location) {
                removed_locations.push(entry.location);
                let splice_position = retained.len();
                if let Some(splice) = splices
                    .last_mut()
                    .filter(|splice| splice.position == splice_position)
                {
                    splice.removed += 1;
                } else {
                    splices.push(EntrySplice {
                        position: splice_position,
                        removed: 1,
                        entries: Vec::new(),
                    });
                }
            } else {
                if selected_position.is_some_and(|selected| position < selected) {
                    retained_before_selected += 1;
                }
                retained.push(entry);
            }
        }
        column.entries = retained;
        if removed_locations.is_empty() {
            return None;
        }

        let selected_was_removed = selected_location
            .as_ref()
            .is_some_and(|location| removals.contains(location));
        let mut replace_selection = false;
        for location in &removed_locations {
            let replace = !self.selectionless_removals.remove(location);
            if selected_location.as_ref() == Some(location) {
                replace_selection = replace;
            }
            column.selected_locations.remove(location);
        }
        column.selected = if selected_was_removed {
            if replace_selection {
                let replacement = column.visible_neighbor(retained_before_selected);
                if !self.preserve_fill_on_removal
                    && let Some(position) = replacement
                {
                    column
                        .selected_locations
                        .insert(column.entries[position].location.clone());
                }
                replacement
            } else {
                None
            }
        } else {
            selected_location.and_then(|location| {
                column
                    .entries
                    .iter()
                    .position(|entry| entry.location == location)
            })
        };
        column.invalidate_entry_indexes();
        column.load_state = if column.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
        Some((splices, column.selected))
    }

    pub fn apply_directory_change(
        &mut self,
        depth: usize,
        watched: &Location,
        change: DirectoryChange,
    ) -> EntrySpliceApplication {
        if !self
            .columns
            .get(depth)
            .is_some_and(|column| &column.location == watched)
        {
            return None;
        }
        let replace_selection = match &change {
            DirectoryChange::Remove(location) => !self.selectionless_removals.remove(location),
            _ => true,
        };
        let column = self.columns.get_mut(depth).expect("column checked");
        let preferences = column.preferences;
        let mut selected_location = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        let mut splices = Vec::new();

        match change {
            DirectoryChange::Upsert(entry) => {
                let existing = column
                    .entries
                    .iter()
                    .find(|current| current.location == entry.location);
                if existing == Some(&entry) {
                    return None;
                }
                if existing.is_none() {
                    column
                        .selected_locations
                        .excludes_new(entry.location.clone());
                }
                upsert_monitored_entry(&mut column.entries, entry, preferences, &mut splices);
            }
            DirectoryChange::Remove(location) => {
                let removed_position = column
                    .entries
                    .iter()
                    .position(|entry| entry.location == location);
                let selected_was_removed = selected_location.as_ref() == Some(&location);
                column.selected_locations.remove(&location);
                remove_monitored_entry(&mut column.entries, &location, &mut splices);
                if selected_was_removed && replace_selection {
                    selected_location = removed_position.and_then(|position| {
                        column
                            .visible_neighbor(position)
                            .and_then(|index| column.entries.get(index))
                            .map(|entry| entry.location.clone())
                    });
                    if !self.preserve_fill_on_removal
                        && let Some(ref replacement) = selected_location
                    {
                        column.selected_locations.insert(replacement.clone());
                    }
                }
            }
            DirectoryChange::Move { from, mut entry } => {
                if let Some(previous) = column
                    .entries
                    .iter()
                    .find(|previous| previous.location == from)
                    && previous.kind == entry.kind
                    && matches!(entry.size, MetadataValue::Known(_))
                    && previous.size == entry.size
                    && matches!(entry.modified_unix_seconds, MetadataValue::Known(_))
                    && previous.modified_unix_seconds == entry.modified_unix_seconds
                    && std::path::Path::new(&previous.native_name).extension()
                        == std::path::Path::new(&entry.native_name).extension()
                {
                    // Monitor entries contain stat data, not the details already loaded for this file.
                    if entry.image_dimensions == MetadataValue::Unknown {
                        entry.image_dimensions = previous.image_dimensions.clone();
                    }
                    if entry.child_count == MetadataValue::Unknown {
                        entry.child_count = previous.child_count.clone();
                    }
                    if entry.duration_seconds == MetadataValue::Unknown {
                        entry.duration_seconds = previous.duration_seconds.clone();
                    }
                }
                if selected_location.as_ref() == Some(&from) {
                    selected_location = Some(entry.location.clone());
                }
                column
                    .selected_locations
                    .relocate(&from, entry.location.clone());
                for target in [
                    &mut column.selection_target,
                    &mut column.selection_anchor,
                    &mut column.load_cursor,
                ] {
                    if target.as_ref() == Some(&from) {
                        *target = Some(entry.location.clone());
                    }
                }
                remove_monitored_entry(&mut column.entries, &from, &mut splices);
                upsert_monitored_entry(&mut column.entries, entry, preferences, &mut splices);
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
        column.invalidate_entry_indexes();
        column.retain_selected_locations(false);
        column.load_state = if column.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
        Some((splices, column.selected))
    }

    pub fn refresh_column(&mut self, depth: usize, request_id: RequestId) -> Option<Location> {
        let column = self.columns.get_mut(depth)?;
        column.selection_target = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone())
            .or_else(|| column.selection_target.clone());
        column.pending_selection = column.selected_locations.materialize(&column.entries);
        column.selected_locations = column.pending_selection.clone().into();
        column.load_state = LoadState::Loading;
        column.truncated = false;
        column.request_id = request_id;
        Some(column.location.clone())
    }

    pub fn reload_column(&mut self, depth: usize, request_id: RequestId) -> Option<Location> {
        // An open child column belongs to the cursor's folder; another entry must not
        // take over the cursor beside it.
        let child_open = depth + 1 < self.columns.len();
        let column = self.columns.get_mut(depth)?;
        column.selection_target = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone())
            .or_else(|| column.selection_target.clone());
        column.pending_selection = column.selected_locations.materialize(&column.entries);
        column.selected_locations = column.pending_selection.clone().into();
        column.entries = Vec::new();
        column.invalidate_entry_indexes();
        // A reload that restarts before the previous one listed anything keeps its cursor.
        column.reload_cursor = (!child_open)
            .then(|| column.selected.or(column.reload_cursor))
            .flatten();
        column.selected = None;
        column.load_state = LoadState::Loading;
        column.truncated = false;
        column.can_trash = None;
        column.can_delete = None;
        column.request_id = request_id;
        Some(column.location.clone())
    }

    pub fn relocate_column(&mut self, depth: usize, location: Location, request_id: RequestId) {
        let Some(previous) = self.reload_column(depth, request_id) else {
            return;
        };
        let column = &mut self.columns[depth];
        column.selected_locations = column
            .pending_selection
            .iter()
            .filter_map(|selected| selected.rebase(&previous, &location))
            .collect::<HashSet<_>>()
            .into();
        column.pending_selection = column
            .pending_selection
            .iter()
            .filter_map(|selected| selected.rebase(&previous, &location))
            .collect();
        for target in [
            &mut column.selection_target,
            &mut column.selection_anchor,
            &mut column.load_cursor,
        ] {
            *target = target
                .as_ref()
                .and_then(|target| target.rebase(&previous, &location));
        }
        column.pending_reveal = column
            .pending_reveal
            .iter()
            .filter_map(|target| target.rebase(&previous, &location))
            .collect();
        column.location = location;
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
        self.folder_sorts
            .as_ref()
            .map_or(FolderSort::Unremembered, |resolve| resolve(location))
    }

    fn initial_preferences(&self, location: &Location) -> ViewPreferences {
        let mut preferences = self.preferences;
        if let FolderSort::Saved(sort_key, sort_direction) = self.folder_sort(location) {
            preferences.sort_key = sort_key;
            preferences.sort_direction = sort_direction;
        }
        preferences_for_location(preferences, location)
    }

    /// What a column's preferences become after its folder's sort or the
    /// application-wide folders-first choice changed. Unremembered folders keep
    /// their own sort, and Recent keeps its fixed order.
    pub fn synchronized_preferences(&self, depth: usize) -> Option<ViewPreferences> {
        let column = self.columns.get(depth)?;
        let mut preferences = column.preferences;
        if column.location.is_recent_root() {
            return Some(preferences);
        }
        preferences.folders_first = self.preferences.folders_first;
        match self.folder_sort(&column.location) {
            FolderSort::Saved(sort_key, sort_direction) => {
                preferences.sort_key = sort_key;
                preferences.sort_direction = sort_direction;
            }
            FolderSort::Default => {
                preferences.sort_key = self.preferences.sort_key;
                preferences.sort_direction = self.preferences.sort_direction;
            }
            FolderSort::Unremembered => {}
        }
        Some(preferences)
    }

    /// For a column still loading: its load sorts with these once it finishes.
    pub fn set_loading_column_preferences(&mut self, depth: usize, preferences: ViewPreferences) {
        if let Some(column) = self
            .columns
            .get_mut(depth)
            .filter(|column| column.load_state == LoadState::Loading)
        {
            column.preferences = preferences;
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
        }
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

    pub fn can_trash_at(&self, depth: usize) -> Option<bool> {
        self.columns.get(depth)?.can_trash
    }

    pub fn can_delete_at(&self, depth: usize) -> Option<bool> {
        self.columns.get(depth)?.can_delete
    }

    /// Returns the finished depth and whether completing the listing moved the cursor,
    /// which batch loads have not published yet.
    pub fn finish(
        &mut self,
        request_id: RequestId,
        truncated: bool,
        can_trash: Option<bool>,
        can_delete: Option<bool>,
    ) -> Option<(usize, bool)> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        let cursor_moved = column.complete_load_selection();
        column.select_first_on_load = false;
        column.retain_selected_locations(false);
        column.pending_selection.clear();
        column.truncated = truncated;
        column.can_trash = can_trash;
        column.can_delete = can_delete;
        column.load_state = if column.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
        Some((depth, cursor_moved))
    }

    pub fn fail(&mut self, request_id: RequestId, message: String) -> Option<usize> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        column.drop_load_intents();
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
        let location = entry.location.clone();
        adopt_selected_locations(column, HashSet::from([location.clone()]), true);
        column.selected = Some(position);
        column.selection_anchor = Some(location);
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

    fn replace_visible(&mut self, depth: usize, select_all: bool) -> Option<(usize, Vec<usize>)> {
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
        let locations = visible
            .iter()
            .filter_map(|position| {
                let location = &column.entries[*position].location;
                let selected = committed && column.selected_locations.contains(location);
                (select_all || !selected).then(|| location.clone())
            })
            .collect();
        adopt_selected_locations(column, locations, true);
        column.selected = Some(focused);
        self.active_column = Some(depth);
        let positions = selected_position_list(column);
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
        let locations = positions
            .iter()
            .map(|position| column.entries[*position].location.clone())
            .collect();
        adopt_selected_locations(column, locations, true);
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

    /// A load cursor is not included in the range's base fill.
    pub fn start_visual(
        &mut self,
        kind: VisualKind,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, Vec<usize>)> {
        self.visual = None;
        let depth = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))?;
        let column = self.columns.get_mut(depth)?;
        let order = displayed_order(column, order);
        let cursor = column
            .selected
            .filter(|position| order.contains(position))?;
        column.drop_load_intents();
        if column.load_cursor.is_some() {
            column.selected_locations.clear();
            column.load_cursor = None;
        }
        self.visual = Some(VisualRange {
            depth,
            directory: column.location.clone(),
            anchor: column.entries[cursor].location.clone(),
            kind,
            base: column.selected_locations.materialize(&column.entries),
            toggled: HashSet::new(),
        });
        self.refresh_visual(Some(&order))
    }

    /// A range whose pane, anchor, or cursor disappears keeps its last fill.
    pub fn refresh_visual(
        &mut self,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, Vec<usize>)> {
        let range = self.visual.as_ref()?;
        let depth = range.depth;
        let Some(column) = self
            .columns
            .get_mut(depth)
            .filter(|column| column.location == range.directory)
            .filter(|_| self.active_column == Some(depth))
        else {
            self.visual = None;
            return None;
        };
        let order = displayed_order(column, order);
        let index_of = |location: &Location| {
            order
                .iter()
                .position(|position| &column.entries[*position].location == location)
        };
        let anchor = index_of(&range.anchor);
        let cursor = column
            .selected
            .and_then(|cursor| order.iter().position(|position| *position == cursor));
        let (Some(anchor), Some(cursor)) = (anchor, cursor) else {
            self.visual = None;
            return None;
        };
        let span = order[anchor.min(cursor)..=anchor.max(cursor)]
            .iter()
            .map(|position| &column.entries[*position].location);
        let mut fill = range.base.clone();
        match range.kind {
            VisualKind::Select => fill.extend(span.cloned()),
            VisualKind::Unset => span.for_each(|location| {
                fill.remove(location);
            }),
        }
        for location in &range.toggled {
            if !fill.remove(location) {
                fill.insert(location.clone());
            }
        }
        adopt_selected_locations(column, fill, true);
        let focused = order[cursor];
        Some((depth, focused, selected_position_list(column)))
    }

    pub fn toggle_visual_cursor(
        &mut self,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, Vec<usize>)> {
        let range = self.visual.as_mut()?;
        let location = self
            .columns
            .get(range.depth)
            .and_then(|column| {
                column
                    .selected
                    .and_then(|cursor| column.entries.get(cursor))
            })
            .map(|entry| entry.location.clone());
        if let Some(location) = location
            && !range.toggled.remove(&location)
        {
            range.toggled.insert(location);
        }
        self.refresh_visual(order)
    }

    pub fn take_visual(&mut self) -> Option<VisualRange> {
        self.visual.take()
    }

    pub fn restore_visual(&mut self, range: Option<VisualRange>) {
        self.visual = range;
    }

    pub fn leave_visual(&mut self) -> bool {
        self.visual.take().is_some()
    }

    pub fn visual_kind(&self) -> Option<VisualKind> {
        self.live_range().map(|range| range.kind)
    }

    fn live_range(&self) -> Option<&VisualRange> {
        let range = self.visual.as_ref()?;
        let column = self.columns.get(range.depth)?;
        (column.location == range.directory
            && self.active_column == Some(range.depth)
            && column
                .entries
                .iter()
                .any(|entry| entry.location == range.anchor))
        .then_some(range)
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
        if positions
            .iter()
            .any(|position| *position >= column.entries.len())
            || focused.is_some_and(|position| position >= column.entries.len())
        {
            return false;
        }
        let locations: HashSet<_> = positions
            .iter()
            .map(|position| column.entries[*position].location.clone())
            .collect();
        // Repeating the fill is a widget echo, as is a report of only the cursor
        // while a different fill remains. After a clear, that cursor report is the
        // click on the still-focused file, so it becomes the selection.
        let cursor_only_outside_fill = !column.selected_locations.is_empty()
            && column.selected.is_some_and(|cursor| {
                column.entries.get(cursor).is_some_and(|entry| {
                    !column.selected_locations.contains(&entry.location)
                        && locations.len() == 1
                        && locations.contains(&entry.location)
                })
            });
        if !self.selection_commit
            && (column
                .selected_locations
                .equals_explicit(&locations, &column.entries)
                || cursor_only_outside_fill)
            && column
                .selected
                .is_some_and(|cursor| cursor < column.entries.len())
        {
            self.active_column = Some(depth);
            return true;
        }
        let commit = std::mem::take(&mut self.selection_commit);
        adopt_selected_locations(column, locations, commit);
        column.selected = focused.or(column.selected);
        self.visual = None;
        if column.selection_anchor.is_none() {
            column.selection_anchor = column
                .selected
                .and_then(|position| column.entries.get(position))
                .map(|entry| entry.location.clone());
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

        let first_visible = column.entries.iter().position(is_visible)?;
        let last_visible = column.entries.iter().rposition(is_visible)?;

        let current = column.selected.unwrap_or(if direction < 0 {
            last_visible
        } else {
            first_visible
        });

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
        let locations = selected_positions
            .iter()
            .map(|&index| column.entries[index].location.clone())
            .collect();
        adopt_selected_locations(column, locations, true);
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
        let locations = positions
            .iter()
            .map(|position| column.entries[*position].location.clone())
            .collect();
        adopt_selected_locations(column, locations, true);
        column.selected = Some(focused);
        self.active_column = Some(depth);
        Some(positions)
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
        if column.selected_locations.is_empty() {
            return Vec::new();
        }
        if let Some(position) = column.single_selected_position() {
            return vec![position];
        }
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
        if column.selected_locations.is_empty() {
            return Vec::new();
        }
        if let Some(position) = column.single_selected_position() {
            return vec![column.entries[position].clone()];
        }
        column
            .entries
            .iter()
            .filter(|entry| column.selected_locations.contains(&entry.location))
            .cloned()
            .collect()
    }

    /// The pane's committed fill in listing order, or its cursor item when
    /// nothing is filled. A load cursor is not a fill, and another pane's
    /// open-path marker is never a target.
    pub fn command_entries(&self, depth: usize) -> Vec<FileEntry> {
        let Some(column) = self.columns.get(depth) else {
            return Vec::new();
        };
        let visible = |entry: &FileEntry| column.preferences.show_hidden || !entry.is_hidden;
        if column.load_cursor.is_none() && !column.selected_locations.is_empty() {
            let filled: Vec<FileEntry> = column
                .entries
                .iter()
                .filter(|entry| {
                    visible(entry) && column.selected_locations.contains(&entry.location)
                })
                .cloned()
                .collect();
            if !filled.is_empty() {
                return filled;
            }
        }
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
            column.load_cursor = None;
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
    /// last visible entry, for page-sized keyboard navigation. `order` is the
    /// displayed source indices when the view is not in source order.
    pub fn page_along(
        &mut self,
        direction: i32,
        page: usize,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize)> {
        self.shift_cursor(direction, page, order, true)
            .map(|(depth, position, _)| (depth, position))
    }

    pub fn page_cursor(
        &mut self,
        direction: i32,
        page: usize,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, bool)> {
        self.shift_cursor(direction, page, order, false)
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

    fn page_target(
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

    fn shift_cursor(
        &mut self,
        direction: i32,
        page: usize,
        order: Option<&[usize]>,
        replace_fill: bool,
    ) -> Option<(usize, usize, bool)> {
        let (depth, _, position) = self.page_target(direction, page, order)?;
        let column = self.columns.get_mut(depth)?;
        let cleared = if replace_fill {
            self.visual = None;
            focus_only(column, position);
            false
        } else {
            place_cursor(column, position)
        };
        self.active_column = Some(depth);
        Some((depth, position, cleared))
    }

    pub fn focus_column(&mut self, depth: usize) -> bool {
        if depth >= self.columns.len() {
            return false;
        }
        self.activate_pane(depth);
        true
    }

    fn activate_pane(&mut self, depth: usize) {
        if self
            .visual
            .as_ref()
            .is_some_and(|range| range.depth != depth)
        {
            self.visual = None;
        }
        self.active_column = Some(depth);
    }

    pub fn focus_parent(&mut self) -> Option<(usize, Option<usize>)> {
        let depth = self.active_column?;
        let parent_depth = depth.checked_sub(1)?;
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
                let location = column.entries[position].location.clone();
                adopt_selected_locations(column, HashSet::from([location.clone()]), true);
                column.selected = Some(position);
                column.selection_anchor = Some(location);
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
        self.peek = None;
        self.visual = None;
        self.columns.truncate(depth);
        let parent_depth = depth - 1;
        self.active_column = Some(parent_depth);
        Some((parent_depth, self.columns[parent_depth].selected))
    }

    pub fn entry_at(&self, depth: usize, position: usize) -> Option<FileEntry> {
        self.columns.get(depth)?.entries.get(position).cloned()
    }

    pub fn column_entry_counts(&self, depth: usize) -> Option<ColumnEntryCounts> {
        let column = self.columns.get(depth)?;
        let show_hidden = column.preferences.show_hidden;
        if let Some((cached_visibility, counts)) = column.entry_counts.get()
            && cached_visibility == show_hidden
        {
            return Some(counts);
        }
        let mut folders = 0;
        let mut files = 0;
        let mut total = 0;
        for entry in &column.entries {
            if show_hidden || !entry.is_hidden {
                total += 1;
                if entry.is_directory() {
                    folders += 1;
                } else {
                    files += 1;
                }
            }
        }
        let counts = ColumnEntryCounts {
            total,
            files,
            folders,
        };
        column.entry_counts.set(Some((show_hidden, counts)));
        Some(counts)
    }

    pub fn active_child_position(&self, depth: usize) -> Option<usize> {
        let child = &self.columns.get(depth + 1)?.location;
        self.columns
            .get(depth)?
            .entries
            .iter()
            .position(|entry| &entry.location == child)
    }

    pub fn cursor_entry(&self, depth: usize) -> Option<FileEntry> {
        let column = self.columns.get(depth)?;
        column
            .selected
            .and_then(|position| column.entries.get(position))
            .filter(|entry| column.preferences.show_hidden || !entry.is_hidden)
            .cloned()
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

fn focus_only(column: &mut ColumnState, position: usize) {
    let location = column.entries[position].location.clone();
    adopt_selected_locations(column, HashSet::from([location.clone()]), true);
    column.selected = Some(position);
    column.selection_anchor = Some(location);
}

fn place_cursor(column: &mut ColumnState, position: usize) -> bool {
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

fn displayed_order(column: &ColumnState, order: Option<&[usize]>) -> Vec<usize> {
    match order {
        Some(order)
            if !order.is_empty()
                && order
                    .iter()
                    .all(|position| *position < column.entries.len()) =>
        {
            order.to_vec()
        }
        _ => visible_positions(column),
    }
}

fn visible_positions(column: &ColumnState) -> Vec<usize> {
    let show_hidden = column.preferences.show_hidden;
    column
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| show_hidden || !entry.is_hidden)
        .map(|(position, _)| position)
        .collect()
}

fn selected_position_list(column: &ColumnState) -> Vec<usize> {
    if column.selected_locations.is_empty() {
        return Vec::new();
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorToggle {
    Empty,
    Added,
    Removed,
}

impl ColumnState {
    fn visible_neighbor(&self, position: usize) -> Option<usize> {
        let visible =
            |&index: &usize| self.preferences.show_hidden || !self.entries[index].is_hidden;
        (position..self.entries.len())
            .find(visible)
            .or_else(|| (0..position.min(self.entries.len())).rev().find(visible))
    }

    fn invalidate_entry_indexes(&mut self) {
        self.entry_counts.set(None);
        self.metadata_positions = None;
    }

    fn retain_selected_locations(&mut self, hide_hidden: bool) {
        if self.selected_locations.is_empty() {
            return;
        }
        self.selected_locations.retain(&self.entries, hide_hidden);
    }

    fn single_selected_position(&self) -> Option<usize> {
        let position = self.selected?;
        (self.selected_locations.len() == 1
            && self
                .selected_locations
                .contains(&self.entries.get(position)?.location))
        .then_some(position)
    }

    /// A user selection or a failed load supersedes whatever the load meant to select.
    fn drop_load_intents(&mut self) {
        self.pending_reveal.clear();
        self.selection_from_reveal = false;
        self.preferred_on_load = None;
        self.reload_cursor = None;
    }

    /// Selects `position` as a load's automatic choice: cursor, single selection and
    /// anchor, marked so a later cursor move does not carry it along.
    fn select_loaded_entry(&mut self, position: usize) {
        let location = self.entries[position].location.clone();
        self.selected = Some(position);
        self.selected_locations.clear();
        self.selected_locations.insert(location.clone());
        self.selection_anchor = Some(location.clone());
        self.select_first_on_load = false;
        self.load_cursor = Some(location);
    }

    fn select_first_visible_entry(&mut self) {
        self.pending_selection.clear();
        let show_hidden = self.preferences.show_hidden;
        if let Some(position) = self
            .entries
            .iter()
            .position(|entry| show_hidden || !entry.is_hidden)
        {
            self.select_loaded_entry(position);
        }
    }

    /// An explicit target, or a selection the load already restored, wins over the
    /// preferred entry.
    fn resolve_preferred_on_load(&mut self) {
        if !self.pending_reveal.is_empty() || self.selected.is_some() {
            return;
        }
        let Some(preferred) = self.preferred_on_load.as_ref() else {
            return;
        };
        let show_hidden = self.preferences.show_hidden;
        let Some(position) = self
            .entries
            .iter()
            .position(|entry| entry.location == *preferred && (show_hidden || !entry.is_hidden))
        else {
            return;
        };
        self.preferred_on_load = None;
        self.pending_selection.clear();
        self.select_loaded_entry(position);
    }

    /// Fallbacks that need the complete listing: the first visible entry for a preferred
    /// entry that never arrived, and a cursor-only neighbour for a reloaded cursor whose
    /// entry is gone. Returns whether the cursor moved.
    fn complete_load_selection(&mut self) -> bool {
        let before = self.selected;
        if self.preferred_on_load.take().is_some()
            && self.selected.is_none()
            && self.pending_reveal.is_empty()
            && !self.entries.is_empty()
        {
            self.select_first_visible_entry();
        }
        if let Some(previous) = self.reload_cursor.take()
            && self.selected.is_none()
            && self.pending_reveal.is_empty()
            && !self.entries.is_empty()
        {
            self.selected = self.visible_neighbor(previous.min(self.entries.len() - 1));
            if self.selected.is_some() {
                self.selection_target = None;
            }
        }
        self.selected != before
    }

    fn resolve_pending_reveal(&mut self) {
        if self.pending_reveal.is_empty() {
            return;
        }
        let requested: HashSet<&Location> = self.pending_reveal.iter().collect();
        let listed: HashMap<&Location, usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| requested.contains(&entry.location))
            .map(|(position, entry)| (&entry.location, position))
            .collect();
        let Some((position, first)) = self
            .pending_reveal
            .iter()
            .find_map(|target| Some((*listed.get(target)?, target.clone())))
        else {
            return;
        };
        let selected: HashSet<Location> = listed.into_keys().cloned().collect();
        self.selected = Some(position);
        self.selected_locations = selected.into();
        self.selection_anchor = Some(first);
        self.selection_target = None;
        self.load_cursor = None;
        self.selection_from_reveal = true;
        self.preferred_on_load = None;
        self.reload_cursor = None;
    }

    fn restore_pending_selection(&mut self) {
        if self.pending_selection.is_empty() {
            return;
        }
        let restored: HashSet<_> = self
            .entries
            .iter()
            .filter(|entry| self.pending_selection.contains(&entry.location))
            .map(|entry| entry.location.clone())
            .collect();
        if restored.is_empty() {
            return;
        }
        self.selected_locations = restored.into();
        if self.selected.is_none() {
            self.selected = self
                .entries
                .iter()
                .position(|entry| self.selected_locations.contains(&entry.location));
        }
    }
}

fn adopt_selected_locations(column: &mut ColumnState, locations: HashSet<Location>, commit: bool) {
    if commit {
        column.load_cursor = None;
        column.drop_load_intents();
    }
    column.selected_locations = locations.into();
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
    if update.mode != MetadataValue::Unknown && entry.mode != update.mode {
        entry.mode = update.mode.clone();
        changed = true;
    }
    if update.image_dimensions != MetadataValue::Unknown
        && entry.image_dimensions != update.image_dimensions
    {
        entry.image_dimensions = update.image_dimensions.clone();
        changed = true;
    }
    if update.child_count != MetadataValue::Unknown && entry.child_count != update.child_count {
        entry.child_count = update.child_count.clone();
        changed = true;
    }
    if update.duration_seconds != MetadataValue::Unknown
        && entry.duration_seconds != update.duration_seconds
    {
        entry.duration_seconds = update.duration_seconds.clone();
        changed = true;
    }
    changed
}

fn preferences_for_location(
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

fn merge_entries(
    mut existing: Vec<FileEntry>,
    incoming: Vec<FileEntry>,
    preferences: ViewPreferences,
) -> (Vec<FileEntry>, Vec<EntryInsertion>) {
    if preferences.sort_key == SortKey::DeviceOrder {
        let insertion = EntryInsertion {
            position: existing.len(),
            entries: incoming.clone(),
        };
        existing.extend(incoming);
        return (existing, vec![insertion]);
    }
    let precompute_type = preferences.sort_key == SortKey::Type;
    let mut incoming_items: Vec<SortItem> = incoming
        .into_iter()
        .map(|entry| SortItem::new(entry, precompute_type))
        .collect();
    incoming_items.sort_unstable_by(|left, right| compare_sort_items(left, right, preferences));
    if existing.is_empty() {
        let incoming_entries: Vec<FileEntry> =
            incoming_items.into_iter().map(|item| item.entry).collect();
        let insertion = EntryInsertion {
            position: 0,
            entries: incoming_entries.clone(),
        };
        return (incoming_entries, vec![insertion]);
    }

    let existing_count = existing.len();
    let incoming_count = incoming_items.len();
    let mut existing_items = existing
        .into_iter()
        .map(|entry| SortItem::new(entry, precompute_type))
        .peekable();
    let mut incoming_items = incoming_items.into_iter().peekable();
    let mut merged = Vec::with_capacity(existing_count + incoming_count);
    let mut insertions = Vec::<EntryInsertion>::new();

    while existing_items.peek().is_some() || incoming_items.peek().is_some() {
        let take_incoming = match (existing_items.peek(), incoming_items.peek()) {
            (Some(left), Some(right)) => {
                compare_sort_items(right, left, preferences) != Ordering::Greater
            }
            (None, Some(_)) => true,
            _ => false,
        };

        if take_incoming {
            let Some(item) = incoming_items.next() else {
                break;
            };
            let entry = item.entry;
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
        } else if let Some(item) = existing_items.next() {
            merged.push(item.entry);
        }
    }

    (merged, insertions)
}

pub(crate) fn sort_entries(
    entries: Vec<FileEntry>,
    preferences: ViewPreferences,
) -> Vec<FileEntry> {
    if preferences.sort_key == SortKey::DeviceOrder || entries.len() <= 1 {
        return entries;
    }
    let precompute_type = preferences.sort_key == SortKey::Type;
    let mut items: Vec<SortItem> = entries
        .into_iter()
        .map(|entry| SortItem::new(entry, precompute_type))
        .collect();

    items.sort_unstable_by(|left, right| compare_sort_items(left, right, preferences));

    items.into_iter().map(|item| item.entry).collect()
}

enum FoldedName {
    Ascii,
    Folded(glib::GString),
}

impl FoldedName {
    fn new(name: &str) -> Self {
        if name.is_ascii() {
            Self::Ascii
        } else {
            Self::Folded(glib::casefold(name))
        }
    }

    fn as_bytes<'a>(&'a self, raw: &'a str) -> &'a [u8] {
        match self {
            Self::Ascii => raw.as_bytes(),
            Self::Folded(folded) => folded.as_bytes(),
        }
    }
}

fn compare_with_folded(
    left_folded: &FoldedName,
    left_raw: &str,
    right_folded: &FoldedName,
    right_raw: &str,
) -> Ordering {
    natural_compare(
        left_folded.as_bytes(left_raw),
        right_folded.as_bytes(right_raw),
    )
    .then_with(|| left_raw.cmp(right_raw))
}

struct SortItem {
    entry: FileEntry,
    folded_name: FoldedName,
    entry_type: Option<crate::services::EntryType>,
}

impl SortItem {
    fn new(entry: FileEntry, precompute_type: bool) -> Self {
        let folded_name = FoldedName::new(&entry.display_name);
        let entry_type = precompute_type.then(|| crate::services::entry_type(&entry));
        Self {
            entry,
            folded_name,
            entry_type,
        }
    }
}

fn compare_sort_items(left: &SortItem, right: &SortItem, preferences: ViewPreferences) -> Ordering {
    if preferences.sort_key == SortKey::DeviceOrder {
        return Ordering::Equal;
    }
    if preferences.folders_first {
        let directory_order = right.entry.is_directory().cmp(&left.entry.is_directory());
        if directory_order != Ordering::Equal {
            return directory_order;
        }
    }

    let ordering = match preferences.sort_key {
        SortKey::DeviceOrder => Ordering::Equal,
        SortKey::Recency => compare_metadata(
            &left.entry.recent_unix_seconds,
            &right.entry.recent_unix_seconds,
        ),
        SortKey::Name => compare_with_folded(
            &left.folded_name,
            &left.entry.display_name,
            &right.folded_name,
            &right.entry.display_name,
        ),
        SortKey::Type => {
            let left_type = left
                .entry_type
                .as_ref()
                .expect("Type sorting precomputes entry types");
            let right_type = right
                .entry_type
                .as_ref()
                .expect("Type sorting precomputes entry types");
            compare_entry_type_values(left_type, right_type)
        }
        SortKey::Size => compare_metadata(&left.entry.size, &right.entry.size),
        SortKey::Modified => compare_metadata(
            &left.entry.modified_unix_seconds,
            &right.entry.modified_unix_seconds,
        ),
    };
    let ordering = match preferences.sort_direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    };
    ordering
        .then_with(|| {
            compare_with_folded(
                &left.folded_name,
                &left.entry.display_name,
                &right.folded_name,
                &right.entry.display_name,
            )
        })
        .then_with(|| left.entry.location.compare(&right.entry.location))
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

fn upsert_monitored_entry(
    entries: &mut Vec<FileEntry>,
    entry: FileEntry,
    preferences: ViewPreferences,
    splices: &mut Vec<EntrySplice>,
) {
    if let Some(existing_position) = entries.iter().position(|e| e.location == entry.location) {
        let is_same_position = {
            let left_ok = existing_position == 0
                || compare_entries(&entries[existing_position - 1], &entry, preferences)
                    != Ordering::Greater;
            let right_ok = existing_position + 1 >= entries.len()
                || compare_entries(&entry, &entries[existing_position + 1], preferences)
                    != Ordering::Greater;
            left_ok && right_ok
        };

        if is_same_position {
            entries[existing_position] = entry.clone();
            splices.push(EntrySplice {
                position: existing_position,
                removed: 1,
                entries: vec![entry],
            });
            return;
        }

        remove_monitored_entry(entries, &entry.location, splices);
    }

    insert_monitored_entry(entries, entry, preferences, splices);
}

fn insert_monitored_entry(
    entries: &mut Vec<FileEntry>,
    entry: FileEntry,
    preferences: ViewPreferences,
    splices: &mut Vec<EntrySplice>,
) {
    let position = if preferences.sort_key == SortKey::DeviceOrder {
        entries.len()
    } else {
        entries
            .binary_search_by(|current| compare_entries(current, &entry, preferences))
            .unwrap_or_else(|position| position)
    };
    entries.insert(position, entry.clone());
    splices.push(EntrySplice {
        position,
        removed: 0,
        entries: vec![entry],
    });
}

fn compare_entries(left: &FileEntry, right: &FileEntry, preferences: ViewPreferences) -> Ordering {
    if preferences.sort_key == SortKey::DeviceOrder {
        return Ordering::Equal;
    }
    if preferences.folders_first {
        let directory_order = right.is_directory().cmp(&left.is_directory());
        if directory_order != Ordering::Equal {
            return directory_order;
        }
    }

    let left_folded = FoldedName::new(&left.display_name);
    let right_folded = FoldedName::new(&right.display_name);

    let ordering = match preferences.sort_key {
        SortKey::DeviceOrder => Ordering::Equal,
        SortKey::Recency => compare_metadata(&left.recent_unix_seconds, &right.recent_unix_seconds),
        SortKey::Name => compare_with_folded(
            &left_folded,
            &left.display_name,
            &right_folded,
            &right.display_name,
        ),
        SortKey::Type => compare_entry_types(left, right),
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
        .then_with(|| {
            compare_with_folded(
                &left_folded,
                &left.display_name,
                &right_folded,
                &right.display_name,
            )
        })
        .then_with(|| left.location.compare(&right.location))
}

fn compare_entry_types(left: &FileEntry, right: &FileEntry) -> Ordering {
    let left_type = crate::services::entry_type(left);
    let right_type = crate::services::entry_type(right);
    compare_entry_type_values(&left_type, &right_type)
}

fn compare_entry_type_values(
    left_type: &crate::services::EntryType,
    right_type: &crate::services::EntryType,
) -> Ordering {
    use crate::services::EntryType;
    match (left_type, right_type) {
        (EntryType::Other, EntryType::Other) => Ordering::Equal,
        (EntryType::Other, _) => Ordering::Greater,
        (_, EntryType::Other) => Ordering::Less,
        _ => compare_display_names(left_type.description(), right_type.description()),
    }
}

pub(crate) fn compare_display_names(left: &str, right: &str) -> Ordering {
    let left_folded = FoldedName::new(left);
    let right_folded = FoldedName::new(right);
    compare_with_folded(&left_folded, left, &right_folded, right)
}

fn natural_compare(left: &[u8], right: &[u8]) -> Ordering {
    let (mut li, mut ri) = (0, 0);
    while li < left.len() && ri < right.len() {
        if left[li].is_ascii_digit() && right[ri].is_ascii_digit() {
            let (lv, lo) = take_number(left, li);
            let (rv, ro) = take_number(right, ri);
            let cmp = lv
                .len()
                .cmp(&rv.len())
                .then_with(|| lv.cmp(rv))
                .then_with(|| left[li..lo].cmp(&right[ri..ro]));
            if cmp != Ordering::Equal {
                return cmp;
            }
            li = lo;
            ri = ro;
        } else {
            let lb = left[li].to_ascii_lowercase();
            let rb = right[ri].to_ascii_lowercase();
            if lb != rb {
                return lb.cmp(&rb);
            }
            li += 1;
            ri += 1;
        }
    }
    left.len().cmp(&right.len())
}

fn take_number(bytes: &[u8], start: usize) -> (&[u8], usize) {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    let mut significant = start;
    while significant < end && bytes[significant] == b'0' {
        significant += 1;
    }
    (&bytes[significant..end], end)
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
