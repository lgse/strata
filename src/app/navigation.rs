// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, OnceCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[cfg(test)]
use std::cmp::Ordering;

use crate::{
    app::peek::PeekState,
    model::{
        FileEntry, FolderSort, Location, ViewPreferences,
        hidden_names::{self, Leftover},
    },
    services::RequestId,
};
#[cfg(test)]
use crate::{
    model::{SortDirection, SortKey},
    services::MetadataUpdate,
};

mod cursor;
mod fill;
mod history;
mod listing;
mod location_set;
mod metadata;
mod monitored;
mod ordering;
mod preferences;
mod selection;
mod type_groups;
mod visual;

use cursor::{place_committed_cursor, place_cursor, visible_positions};
use fill::{adopt_selected_locations, adopt_selected_positions};
use location_set::LocationSet;
#[cfg(test)]
use monitored::insert_monitored_entry;
pub(crate) use ordering::{compare_display_names, sort_entries};
use ordering::{compare_entries, merge_entries};
use selection::SelectedLocations;
pub(crate) use type_groups::compare_type_groups;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadState {
    Loading,
    Ready,
    Empty,
    Error(String),
    /// Permissions kept the user out of a native folder that `administrator` may open.
    Denied {
        message: String,
        administrator: Location,
    },
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

#[derive(Clone, Debug)]
pub struct ColumnState {
    pub location: Location,
    pub entries: Vec<FileEntry>,
    entry_counts: Cell<Option<(bool, ColumnEntryCounts)>>,
    leftovers: OnceCell<Rc<[(usize, Leftover)]>>,
    pub(crate) surfaced_leftovers: Cell<usize>,
    // Providers may list a location twice.
    each_location_once: OnceCell<bool>,
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
    /// The folder this column has open, for a cursor Left asked for before the
    /// listing arrived: committed if it lists visibly, otherwise nothing is selected.
    open_child_on_load: Option<Location>,
    /// Cursor position before a reload; the cursor moves to its neighbour when the
    /// entry is gone.
    reload_cursor: Option<usize>,
    // Auto-selection must not redirect paste into the first folder.
    load_cursor: Option<Location>,
    type_grouping: bool,
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

/// The flag is true when a column opens the folder, which counts as using it;
/// re-syncs pass false.
pub type FolderSortResolver = Rc<dyn Fn(&Location, bool) -> FolderSort>;

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
    type_grouping: bool,
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

    pub(crate) fn column_leftover_names(&self, depth: usize) -> Rc<[(usize, Leftover)]> {
        let Some(column) = self.columns.get(depth) else {
            return Rc::new([]);
        };
        column
            .leftovers
            .get_or_init(|| {
                column
                    .entries
                    .iter()
                    .enumerate()
                    .filter_map(|(position, entry)| {
                        Some((position, hidden_names::classify(&entry.native_name)?))
                    })
                    .collect()
            })
            .clone()
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

    /// The folder the active column lists, as its parent column's entry, when the
    /// active column is the deepest one. An ancestor column with its own child open
    /// never stands for the folder its parent has open.
    pub fn entered_folder(&self) -> Option<(usize, usize, FileEntry)> {
        let depth = self.active_column?;
        if depth + 1 != self.columns.len() {
            return None;
        }
        let parent_depth = depth.checked_sub(1)?;
        let position = self.active_child_position(parent_depth)?;
        let entry = self.entry_at(parent_depth, position)?;
        Some((parent_depth, position, entry))
    }

    pub fn loading_column(&self, request_id: RequestId) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(depth, column)| {
            (column.request_id == request_id).then_some((depth, column.entries.len()))
        })
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorToggle {
    Empty,
    Added,
    Removed,
}

impl ColumnState {
    fn cursor_location(&self) -> Option<Location> {
        self.selected
            .and_then(|position| self.entries.get(position))
            .map(|entry| entry.location.clone())
    }

    fn position_of(&self, location: &Location) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.location == *location)
    }

    /// A finished or monitored listing is empty or ready.
    fn settle_listed_state(&mut self) {
        self.load_state = if self.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
    }

    fn visible_neighbor(&self, position: usize) -> Option<usize> {
        let visible =
            |&index: &usize| self.preferences.show_hidden || !self.entries[index].is_hidden;
        (position..self.entries.len())
            .find(visible)
            .or_else(|| (0..position.min(self.entries.len())).rev().find(visible))
    }

    fn invalidate_entry_indexes(&mut self) {
        self.each_location_once.take();
        self.invalidate_spliced_entry_indexes();
    }

    /// Splices add, replace and remove rows by location, so they never add a repeat.
    fn invalidate_spliced_entry_indexes(&mut self) {
        self.entry_counts.set(None);
        self.leftovers.take();
        self.metadata_positions = None;
        self.selected_locations.forget_positions();
    }

    fn retain_selected_locations(&mut self, hide_hidden: bool) {
        if self.selected_locations.is_empty() {
            return;
        }
        self.selected_locations.retain(&self.entries, hide_hidden);
    }

    fn selected_positions(&self) -> &[usize] {
        self.selected_locations
            .positions(&self.entries, self.selected, || {
                lists_each_location_once(&self.entries, &self.each_location_once)
            })
    }
}

fn lists_each_location_once(entries: &[FileEntry], each_location_once: &OnceCell<bool>) -> bool {
    *each_location_once.get_or_init(|| {
        let mut listed = HashSet::with_capacity(entries.len());
        entries.iter().all(|entry| listed.insert(&entry.location))
    })
}

#[cfg(test)]
mod tests;
