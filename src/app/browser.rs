// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::PathBuf,
    rc::{Rc, Weak},
    time::Duration,
};

use crate::{
    app::navigation::{EntryInsertion, EntrySplice, NavigationPath, NavigationState},
    model::{FileEntry, Location, SortDirection, SortKey, ViewPreferences},
    services::{
        ArchiveFormat, CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest,
        DirectoryChange, DirectoryEvent, DirectoryRequest, ExtractRequest, FileSource, LoadHandle,
        LocationValidationError, OperationEvent, OperationProvider, OperationRequestId, PasteItem,
        PasteRequest, RenameRequest, RequestId, RestoreRequest, TransferConflict,
        validate_basename, validate_uri_credentials,
    },
};

/// Caps a normal directory load at this project's own documented performance baseline for
/// 100,000 entries (docs/performance-baseline.md: 3,755 ms, 286 MiB) -- past this, per-batch
/// merge cost grows enough that browsing stops feeling responsive.
const MAX_DIRECTORY_ENTRIES: usize = 100_000;
const DIRECTORY_LOAD_TIME_BUDGET: Duration = Duration::from_secs(10);

/// A hover peek only ever displays a handful of entries (`PeekBehavior::item_limit`), so it
/// needs far less headroom than a full directory load -- just enough to survive hidden-file
/// filtering, not enough to enumerate an entire large directory for a preview that discards
/// nearly all of it.
const PEEK_MAX_ENTRIES: usize = 64;
const PEEK_TIME_BUDGET: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
pub struct BrowserColumnSnapshot {
    pub location: Location,
    pub entries: Vec<FileEntry>,
    pub selected_positions: Vec<usize>,
    pub loading: bool,
}

#[derive(Clone, Debug)]
pub enum BrowserEvent {
    Reset,
    ColumnsTruncated {
        len: usize,
    },
    ColumnAdded {
        depth: usize,
        location: Location,
    },
    EntriesInserted {
        depth: usize,
        insertions: Vec<EntryInsertion>,
    },
    EntriesReplaced {
        depth: usize,
        entries: Vec<FileEntry>,
    },
    SortingStarted {
        depth: usize,
    },
    SortingFinished {
        depth: usize,
    },
    EntriesSpliced {
        depth: usize,
        splices: Vec<EntrySplice>,
        selected: Option<usize>,
    },
    ColumnReloaded {
        depth: usize,
    },
    LoadFinished {
        depth: usize,
        truncated: bool,
    },
    LoadFailed {
        depth: usize,
        message: String,
    },
    PeekStarted {
        location: Location,
    },
    PeekEntriesAdded {
        entries: Vec<FileEntry>,
    },
    PeekFinished,
    PeekFailed {
        message: String,
    },
    PeekClosed,
    FocusChanged {
        depth: usize,
        position: Option<usize>,
    },
    SelectionSetChanged {
        depth: usize,
        positions: Vec<usize>,
        focused: usize,
        take_focus: bool,
    },
    PreviewRequested {
        entry: FileEntry,
    },
    OpenRequested {
        location: Location,
    },
    RenameCompleted,
    RenameFailed {
        message: String,
    },
    TransferStarted {
        total: usize,
        moving: bool,
    },
    TransferProgress {
        completed: usize,
        total: usize,
    },
    TransferFinished {
        moved_locations: Vec<Location>,
    },
    DeletionStarted {
        total: usize,
    },
    DeletionProgress {
        completed: usize,
        total: usize,
    },
    DeletionFinished,
    RestorationStarted {
        total: usize,
    },
    RestorationProgress {
        completed: usize,
        total: usize,
    },
    RestorationFinished,
    OperationFailed {
        message: String,
    },
    OperationCompletedWithErrors {
        message: String,
    },
    OperationCancelled {
        completed: usize,
        failed: usize,
        not_attempted: usize,
        affected_locations: HashSet<Location>,
    },
    NavigationRejected {
        parent_depth: usize,
        error: LocationValidationError,
    },
    LocationNavigationRejected {
        error: LocationValidationError,
    },
    EmptyTrashRequested,
    ArchiveStarted {
        total: usize,
    },
    ArchiveProgress {
        completed: usize,
        total: usize,
    },
    ArchiveCompleted {
        select_name: String,
    },
    TransferCompleted,
}

type Observer = Rc<dyn Fn(BrowserEvent)>;
type PreferencesObserver = Rc<dyn Fn(ViewPreferences)>;

const MAX_INCREMENTAL_OPERATION_UPDATES: usize = 64;

pub struct Browser {
    source: Rc<dyn FileSource>,
    state: RefCell<NavigationState>,
    loads: RefCell<Vec<LoadHandle>>,
    monitors: RefCell<Vec<Option<LoadHandle>>>,
    peek_load: RefCell<Option<LoadHandle>>,
    validation_load: RefCell<Option<LoadHandle>>,
    validation_generation: Cell<u64>,
    operation_provider: RefCell<Option<Rc<dyn OperationProvider>>>,
    operation_load: RefCell<Option<LoadHandle>>,
    current_operation: Cell<Option<OperationRequestId>>,
    transfer_operation: Cell<Option<bool>>,
    deletion_operation: Cell<bool>,
    restoration_operation: Cell<bool>,
    next_request: Cell<u64>,
    pending_sort: Cell<Option<(u64, usize)>>,
    preferences: Cell<ViewPreferences>,
    observers: RefCell<Vec<Observer>>,
    preferences_observers: RefCell<Vec<PreferencesObserver>>,
}

impl Browser {
    #[cfg(test)]
    pub fn new(source: Rc<dyn FileSource>) -> Rc<Self> {
        Self::with_preferences(source, ViewPreferences::default())
    }

    pub fn with_preferences(source: Rc<dyn FileSource>, preferences: ViewPreferences) -> Rc<Self> {
        Rc::new(Self {
            source,
            state: RefCell::new(NavigationState::with_preferences(preferences)),
            loads: RefCell::new(Vec::new()),
            monitors: RefCell::new(Vec::new()),
            peek_load: RefCell::new(None),
            validation_load: RefCell::new(None),
            validation_generation: Cell::new(0),
            operation_provider: RefCell::new(None),
            operation_load: RefCell::new(None),
            current_operation: Cell::new(None),
            transfer_operation: Cell::new(None),
            deletion_operation: Cell::new(false),
            restoration_operation: Cell::new(false),
            next_request: Cell::new(1),
            pending_sort: Cell::new(None),
            preferences: Cell::new(preferences),
            observers: RefCell::new(Vec::new()),
            preferences_observers: RefCell::new(Vec::new()),
        })
    }

    pub fn observe(&self, observer: impl Fn(BrowserEvent) + 'static) {
        self.observers.borrow_mut().push(Rc::new(observer));
    }

    pub fn clear_observer(&self) {
        self.observers.borrow_mut().clear();
    }

    pub fn observe_preferences(&self, observer: impl Fn(ViewPreferences) + 'static) {
        self.preferences_observers
            .borrow_mut()
            .push(Rc::new(observer));
    }

    fn notify_preferences_observers(&self) {
        let preferences = self.preferences.get();
        for observer in self.preferences_observers.borrow().iter() {
            observer(preferences);
        }
    }

    pub fn set_operation_provider(&self, provider: Rc<dyn OperationProvider>) {
        self.operation_provider.replace(Some(provider));
    }

    pub fn navigate_input(self: &Rc<Self>, input: &str) -> Result<(), LocationValidationError> {
        if input.is_empty() {
            return Err(LocationValidationError::Empty);
        }

        if let Some(message) = unsupported_shorthand_message(input) {
            return Err(LocationValidationError::UnsupportedShorthand(
                message.to_owned(),
            ));
        }
        if let Some(current) = self
            .active_location()
            .filter(|current| current.display_path() == input)
        {
            self.navigate_validated(current);
            return Ok(());
        }
        let location = location_from_input(input)?;
        if location.native_path().is_some() && !location.is_absolute_native() {
            return Err(LocationValidationError::NotAbsolute);
        }
        if location.native_path().is_some() {
            self.source.validate_location(&location)?;
            self.navigate(location);
        } else {
            self.navigate_validated(location);
        }
        Ok(())
    }

    fn navigate_validated(self: &Rc<Self>, location: Location) {
        let generation = self.validation_generation.get().saturating_add(1);
        self.validation_generation.set(generation);
        self.validation_load.borrow_mut().take();
        let weak = Rc::downgrade(self);
        let pending_location = location.clone();
        let emit = Rc::new(move |result| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if browser.validation_generation.get() != generation {
                return;
            }
            match result {
                Ok(()) => browser.navigate(pending_location.clone()),
                Err(error) => browser.emit(BrowserEvent::LocationNavigationRejected { error }),
            }
        });
        let load = self.source.validate_location_async(location, emit);
        self.validation_load.replace(Some(load));
    }

    pub fn active_location(&self) -> Option<Location> {
        self.state.borrow().active_location()
    }

    pub fn active_depth(&self) -> Option<usize> {
        self.state.borrow().active_depth()
    }

    pub fn location_at(&self, depth: usize) -> Option<Location> {
        self.state.borrow().location_at(depth)
    }

    pub fn focus_active(&self) {
        let focus = self.state.borrow().active_focus();
        if let Some((depth, position)) = focus {
            self.emit(BrowserEvent::FocusChanged { depth, position });
        }
    }

    /// Navigates directly for native paths and validates URI locations first so mountable
    /// locations can be mounted by the UI before loading them.
    pub(crate) fn navigate_location(self: &Rc<Self>, location: Location) {
        if location.native_path().is_some() {
            self.navigate(location);
        } else {
            self.navigate_validated(location);
        }
    }

    pub fn navigate(self: &Rc<Self>, location: Location) {
        self.validation_generation
            .set(self.validation_generation.get().saturating_add(1));
        self.validation_load.borrow_mut().take();
        if self.active_location().as_ref() == Some(&location) {
            return;
        }
        self.close_peek();
        self.loads.borrow_mut().clear();
        self.monitors.borrow_mut().clear();
        let request_id = self.new_request_id();
        self.state
            .borrow_mut()
            .navigate(location.clone(), request_id);
        self.emit(BrowserEvent::Reset);
        self.emit(BrowserEvent::ColumnAdded {
            depth: 0,
            location: location.clone(),
        });
        self.emit(BrowserEvent::FocusChanged {
            depth: 0,
            position: None,
        });
        self.start_load(0, location, request_id);
    }

    pub fn descend(self: &Rc<Self>, parent_depth: usize, location: Location) {
        self.descend_with_selection(parent_depth, location, false);
    }

    fn descend_with_selection(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        select_first_on_load: bool,
    ) {
        self.validation_generation
            .set(self.validation_generation.get().saturating_add(1));
        self.validation_load.borrow_mut().take();
        if self.is_open_child(parent_depth, &location) {
            return;
        }
        self.close_peek();
        if location.native_path().is_some() {
            if let Err(error) = self.source.validate_location(&location) {
                self.emit(BrowserEvent::NavigationRejected {
                    parent_depth,
                    error,
                });
                self.focus_active();
                return;
            }
            self.descend_validated(parent_depth, location, select_first_on_load);
            return;
        }

        let generation = self.validation_generation.get().saturating_add(1);
        self.validation_generation.set(generation);
        self.validation_load.borrow_mut().take();
        let weak = Rc::downgrade(self);
        let pending_location = location.clone();
        let parent_location = self.location_at(parent_depth);
        let emit = Rc::new(move |result| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if browser.validation_generation.get() != generation
                || browser.location_at(parent_depth) != parent_location
            {
                return;
            }
            match result {
                Ok(()) => browser.descend_validated(
                    parent_depth,
                    pending_location.clone(),
                    select_first_on_load,
                ),
                Err(error) => {
                    browser.emit(BrowserEvent::NavigationRejected {
                        parent_depth,
                        error,
                    });
                    browser.focus_active();
                }
            }
        });
        let load = self.source.validate_location_async(location, emit);
        self.validation_load.replace(Some(load));
    }

    fn descend_validated(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        select_first_on_load: bool,
    ) {
        let request_id = self.new_request_id();
        let mut state = self.state.borrow_mut();
        if !state.descend(parent_depth, location.clone(), request_id) {
            return;
        }
        if select_first_on_load {
            state.select_first_on_load(parent_depth + 1);
        }
        drop(state);

        let retained = parent_depth + 1;
        self.loads.borrow_mut().truncate(retained);
        self.monitors.borrow_mut().truncate(retained);
        self.emit(BrowserEvent::ColumnsTruncated { len: retained });
        self.emit(BrowserEvent::ColumnAdded {
            depth: retained,
            location: location.clone(),
        });
        self.emit(BrowserEvent::FocusChanged {
            depth: retained,
            position: None,
        });
        self.start_load(retained, location, request_id);
    }

    pub fn begin_peek(self: &Rc<Self>, origin_depth: usize, location: Location) {
        self.close_peek();
        let request_id = self.new_request_id();
        if !self
            .state
            .borrow_mut()
            .begin_peek(origin_depth, location.clone(), request_id)
        {
            return;
        }

        self.emit(BrowserEvent::PeekStarted {
            location: location.clone(),
        });
        let weak: Weak<Self> = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_event(event);
            }
        });
        let handle = self.source.enumerate(
            DirectoryRequest {
                id: request_id,
                location,
                batch_size: 128,
                include_hidden: self.preferences.get().show_hidden,
                max_entries: PEEK_MAX_ENTRIES,
                time_budget: PEEK_TIME_BUDGET,
            },
            emit,
        );
        self.peek_load.replace(Some(handle));
    }

    pub fn close_peek(&self) -> bool {
        self.peek_load.take();
        let closed = self.state.borrow_mut().clear_peek();
        if closed {
            self.emit(BrowserEvent::PeekClosed);
        }
        closed
    }

    pub fn escape(&self) {
        if self.close_peek() {
            return;
        }

        let closed = self.state.borrow_mut().close_deepest();
        if let Some((depth, position)) = closed {
            let len = depth + 1;
            self.loads.borrow_mut().truncate(len);
            self.monitors.borrow_mut().truncate(len);
            self.emit(BrowserEvent::ColumnsTruncated { len });
            self.emit(BrowserEvent::FocusChanged { depth, position });
        }
    }

    pub fn close_column(&self, depth: usize) {
        self.close_peek();
        let closed = self.state.borrow_mut().close_from(depth);
        if let Some((parent_depth, position)) = closed {
            self.loads.borrow_mut().truncate(depth);
            self.monitors.borrow_mut().truncate(depth);
            self.emit(BrowserEvent::ColumnsTruncated { len: depth });
            self.emit(BrowserEvent::FocusChanged {
                depth: parent_depth,
                position,
            });
        }
    }

    pub fn commit_peek(self: &Rc<Self>) {
        let target = self.state.borrow().peek_target();
        if let Some((origin_depth, location)) = target {
            self.close_peek();
            self.descend(origin_depth, location);
        }
    }

    pub fn set_sort_key(self: &Rc<Self>, depth: usize, sort_key: SortKey) {
        self.apply_column_preferences(depth, move |preferences| preferences.sort_key = sort_key);
    }

    pub fn set_sort(
        self: &Rc<Self>,
        depth: usize,
        sort_key: SortKey,
        sort_direction: SortDirection,
    ) {
        self.apply_column_preferences(depth, move |preferences| {
            preferences.sort_key = sort_key;
            preferences.sort_direction = sort_direction;
        });
    }

    pub fn set_sort_direction(self: &Rc<Self>, depth: usize, sort_direction: SortDirection) {
        self.apply_column_preferences(depth, move |preferences| {
            preferences.sort_direction = sort_direction;
        });
    }

    pub fn set_folders_first(self: &Rc<Self>, depth: usize, folders_first: bool) {
        self.apply_column_preferences(depth, move |preferences| {
            preferences.folders_first = folders_first;
        });
    }

    pub fn toggle_hidden(self: &Rc<Self>) {
        let mut preferences = self.preferences.get();
        preferences.show_hidden = !preferences.show_hidden;
        self.preferences.set(preferences);
        self.notify_preferences_observers();

        let locations = {
            let mut state = self.state.borrow_mut();
            state.set_show_hidden(preferences.show_hidden);
            state
                .columns
                .iter()
                .map(|column| column.location.clone())
                .collect::<Vec<_>>()
        };
        for (depth, location) in locations.into_iter().enumerate() {
            self.refresh_column(depth);
            let monitor = self.install_monitor(depth, location);
            if let Some(slot) = self.monitors.borrow_mut().get_mut(depth) {
                *slot = monitor;
            }
        }
    }

    fn apply_column_preferences(
        self: &Rc<Self>,
        depth: usize,
        update: impl FnOnce(&mut ViewPreferences) + 'static,
    ) {
        if self.state.borrow().column_preferences(depth).is_none() {
            return;
        }
        let generation = self
            .pending_sort
            .get()
            .map_or(1, |(generation, _)| generation.saturating_add(1));
        if let Some((_, previous_depth)) = self.pending_sort.replace(Some((generation, depth))) {
            self.emit(BrowserEvent::SortingFinished {
                depth: previous_depth,
            });
        }
        self.emit(BrowserEvent::SortingStarted { depth });
        let weak = Rc::downgrade(self);
        gio::glib::timeout_add_local_once(Duration::from_millis(16), move || {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if browser.pending_sort.get() != Some((generation, depth)) {
                return;
            }
            let result = {
                let mut state = browser.state.borrow_mut();
                let Some(mut preferences) = state.column_preferences(depth) else {
                    drop(state);
                    browser.pending_sort.set(None);
                    browser.emit(BrowserEvent::SortingFinished { depth });
                    return;
                };
                update(&mut preferences);
                let result = state.set_column_preferences(depth, preferences);
                browser.preferences.set(preferences);
                result
            };
            browser.notify_preferences_observers();
            if let Some((entries, focused, positions)) = result {
                browser.emit(BrowserEvent::EntriesReplaced { depth, entries });
                if let Some(focused) = focused {
                    browser.emit(BrowserEvent::SelectionSetChanged {
                        depth,
                        positions,
                        focused,
                        take_focus: false,
                    });
                }
            }
            browser.pending_sort.set(None);
            browser.emit(BrowserEvent::SortingFinished { depth });
        });
    }

    pub fn can_go_back(&self) -> bool {
        self.state.borrow().can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.state.borrow().can_go_forward()
    }

    pub fn can_go_parent(&self) -> bool {
        self.state.borrow().can_go_parent()
    }

    pub fn back(self: &Rc<Self>) {
        let target = self.state.borrow_mut().go_back();
        if let Some(target) = target {
            self.restore_path(target);
        }
    }

    pub fn forward(self: &Rc<Self>) {
        let target = self.state.borrow_mut().go_forward();
        if let Some(target) = target {
            self.restore_path(target);
        }
    }

    pub fn parent(self: &Rc<Self>) {
        let target = self.state.borrow_mut().go_parent();
        if let Some(target) = target {
            self.restore_path(target);
        }
    }

    pub fn select(&self, depth: usize, position: usize) {
        let selected = self.state.borrow_mut().select(depth, position);
        if selected {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: Some(position),
            });
        }
    }

    pub fn entry_at(&self, depth: usize, position: usize) -> Option<FileEntry> {
        self.state.borrow().entry_at(depth, position)
    }

    pub fn column_preferences(&self, depth: usize) -> Option<ViewPreferences> {
        self.state.borrow().column_preferences(depth)
    }

    pub fn column_snapshot(&self, depth: usize) -> Option<BrowserColumnSnapshot> {
        let state = self.state.borrow();
        let column = state.columns.get(depth)?;
        Some(BrowserColumnSnapshot {
            location: column.location.clone(),
            entries: column.entries.clone(),
            selected_positions: state.selected_positions(depth),
            loading: column.load_state == crate::app::navigation::LoadState::Loading,
        })
    }

    pub fn focused_item(&self) -> Option<(usize, usize, FileEntry)> {
        self.state.borrow().focused_entry()
    }

    pub fn rename_item(&self) -> Option<(usize, usize, FileEntry)> {
        let state = self.state.borrow();
        if let Some(focused) = state.focused_entry() {
            return Some(focused);
        }
        let depth = state.active_depth()?.checked_sub(1)?;
        let position = state.active_child_position(depth)?;
        let entry = state.entry_at(depth, position)?;
        Some((depth, position, entry))
    }

    pub fn focused_entry(&self) -> Option<FileEntry> {
        self.focused_item().map(|(_, _, entry)| entry)
    }

    pub fn selected_entries(&self) -> Vec<FileEntry> {
        self.state.borrow().selected_entries()
    }

    pub fn deletion_entries(&self) -> Vec<FileEntry> {
        let state = self.state.borrow();
        let selected = state.selected_entries();
        if !selected.is_empty() {
            return selected;
        }

        let Some(parent_depth) = state.active_depth().and_then(|depth| depth.checked_sub(1)) else {
            return Vec::new();
        };
        let Some(position) = state.active_child_position(parent_depth) else {
            return Vec::new();
        };
        state.entry_at(parent_depth, position).into_iter().collect()
    }

    pub fn set_selection(&self, depth: usize, positions: &[usize], focused: Option<usize>) {
        let mut state = self.state.borrow_mut();
        if state.set_selection(depth, positions, focused) {
            tracing::debug!(
                depth,
                selected = state.selected_entries().len(),
                "selection changed"
            );
        }
    }

    pub fn select_all(&self, depth: usize) {
        let count = self
            .state
            .borrow()
            .columns
            .get(depth)
            .map_or(0, |column| column.entries.len());
        if count == 0 {
            return;
        }
        let positions: Vec<_> = (0..count).collect();
        let focused = count - 1;
        if self
            .state
            .borrow_mut()
            .set_selection(depth, &positions, Some(focused))
        {
            self.emit(BrowserEvent::SelectionSetChanged {
                depth,
                positions,
                focused,
                take_focus: true,
            });
        }
    }

    pub fn active_child_position(&self, depth: usize) -> Option<usize> {
        self.state.borrow().active_child_position(depth)
    }

    pub fn rename(self: &Rc<Self>, entry: FileEntry, new_name: String) {
        if let Err(message) = validate_basename(&new_name) {
            self.emit(BrowserEvent::RenameFailed {
                message: message.to_owned(),
            });
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::RenameFailed {
                message: "File operations are unavailable".to_owned(),
            });
            return;
        };
        let request_id = self.begin_operation();
        let refresh_locations = entry.location.parent().into_iter().collect();
        let emit = self.operation_callback(request_id, true, refresh_locations);
        let load = provider.rename(
            RenameRequest {
                id: request_id,
                entry,
                new_name,
            },
            emit,
        );
        self.operation_load.replace(Some(load));
    }

    pub fn create_directory(self: &Rc<Self>, parent: Location, name: String) {
        if let Err(message) = validate_basename(&name) {
            self.emit(BrowserEvent::OperationFailed {
                message: message.to_owned(),
            });
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: "File operations are unavailable".to_owned(),
            });
            return;
        };
        let request_id = self.begin_operation();
        let refresh_parent = parent.clone();
        let load = provider.create_directory(
            CreateDirectoryRequest {
                id: request_id,
                parent,
                name,
            },
            self.operation_callback(request_id, false, HashSet::from([refresh_parent])),
        );
        self.operation_load.replace(Some(load));
    }

    pub fn create_file(self: &Rc<Self>, parent: Location, name: String) {
        if let Err(message) = validate_basename(&name) {
            self.emit(BrowserEvent::OperationFailed {
                message: message.to_owned(),
            });
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: "File operations are unavailable".to_owned(),
            });
            return;
        };
        let request_id = self.begin_operation();
        let refresh_parent = parent.clone();
        let load = provider.create_file(
            CreateFileRequest {
                id: request_id,
                parent,
                name,
            },
            self.operation_callback(request_id, false, HashSet::from([refresh_parent])),
        );
        self.operation_load.replace(Some(load));
    }

    pub fn transfer(
        self: &Rc<Self>,
        destination: Location,
        items: Vec<PasteItem>,
        move_sources: bool,
    ) {
        if items.is_empty() {
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: "File operations are unavailable".to_owned(),
            });
            return;
        };
        let request_id = self.begin_operation();
        self.transfer_operation.set(Some(move_sources));
        self.emit(BrowserEvent::TransferStarted {
            total: items.len(),
            moving: move_sources,
        });
        let mut refresh_locations = HashSet::from([destination.clone()]);
        if move_sources {
            for parent in items.iter().filter_map(|item| item.source.parent()) {
                refresh_locations.insert(parent);
            }
        }
        let load = provider.paste(
            PasteRequest {
                id: request_id,
                destination,
                items,
                move_sources,
            },
            self.operation_callback(request_id, false, refresh_locations),
        );
        self.operation_load.replace(Some(load));
    }

    pub fn delete(self: &Rc<Self>, entries: Vec<FileEntry>, permanent: bool) {
        if entries.is_empty() {
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: "File operations are unavailable".to_owned(),
            });
            return;
        };
        let total = entries.len();
        let request_id = self.begin_operation();
        self.deletion_operation.set(true);
        self.emit(BrowserEvent::DeletionStarted { total });
        let load = provider.delete(
            DeleteRequest {
                id: request_id,
                entries,
                permanent,
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.operation_load.replace(Some(load));
    }

    pub fn restore(self: &Rc<Self>, entries: Vec<FileEntry>) {
        if entries.is_empty() {
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: "File operations are unavailable".to_owned(),
            });
            return;
        };
        let total = entries.len();
        let request_id = self.begin_operation();
        self.restoration_operation.set(true);
        self.emit(BrowserEvent::RestorationStarted { total });
        let load = provider.restore(
            RestoreRequest {
                id: request_id,
                entries,
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.operation_load.replace(Some(load));
    }

    pub fn compress(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        destination: Location,
        archive_name: String,
        conflict: TransferConflict,
        format: ArchiveFormat,
        password: Option<String>,
    ) {
        if entries.is_empty() {
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: "File operations are unavailable".to_owned(),
            });
            return;
        };
        let request_id = self.begin_operation();
        let load = provider.compress(
            CompressRequest {
                id: request_id,
                entries,
                destination,
                archive_name,
                conflict,
                format,
                password,
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.operation_load.replace(Some(load));
    }

    pub fn extract(
        self: &Rc<Self>,
        entry: FileEntry,
        destination: Location,
        password: Option<String>,
    ) {
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: "File operations are unavailable".to_owned(),
            });
            return;
        };
        let request_id = self.begin_operation();
        let load = provider.extract(
            ExtractRequest {
                id: request_id,
                entry,
                destination,
                password,
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.operation_load.replace(Some(load));
    }

    pub fn cancel_file_operation(&self) {
        if self.transfer_operation.get().is_none()
            && !self.deletion_operation.get()
            && !self.restoration_operation.get()
        {
            let had_operation = self.current_operation.replace(None).is_some();
            self.operation_load.borrow_mut().take();
            if had_operation {
                self.emit(BrowserEvent::ArchiveCompleted {
                    select_name: String::new(),
                });
            }
            return;
        }
        self.operation_load.borrow_mut().take();
    }

    fn begin_operation(&self) -> OperationRequestId {
        self.operation_load.borrow_mut().take();
        self.transfer_operation.set(None);
        self.deletion_operation.set(false);
        self.restoration_operation.set(false);
        let request_id = OperationRequestId(self.next_request.get());
        self.next_request
            .set(self.next_request.get().saturating_add(1));
        self.current_operation.set(Some(request_id));
        request_id
    }

    fn operation_callback(
        self: &Rc<Self>,
        request_id: OperationRequestId,
        rename: bool,
        refresh_locations: HashSet<Location>,
    ) -> Rc<dyn Fn(OperationEvent)> {
        let weak = Rc::downgrade(self);
        Rc::new(move |event| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            let event_id = match &event {
                OperationEvent::Renamed { request_id }
                | OperationEvent::Created { request_id }
                | OperationEvent::Pasted { request_id, .. }
                | OperationEvent::TransferFailed { request_id, .. }
                | OperationEvent::TransferProgress { request_id, .. }
                | OperationEvent::DeleteProgress { request_id, .. }
                | OperationEvent::RestoreProgress { request_id, .. }
                | OperationEvent::Deleted { request_id, .. }
                | OperationEvent::CompletedWithErrors { request_id, .. }
                | OperationEvent::Restored { request_id, .. }
                | OperationEvent::RestoreCompletedWithErrors { request_id, .. }
                | OperationEvent::Failed { request_id, .. }
                | OperationEvent::Compressed { request_id, .. }
                | OperationEvent::Extracted { request_id, .. }
                | OperationEvent::ArchiveStarted { request_id, .. }
                | OperationEvent::Cancelled { request_id, .. }
                | OperationEvent::ArchiveProgress { request_id, .. } => *request_id,
            };
            if event_id != request_id || browser.current_operation.get() != Some(event_id) {
                return;
            }
            if let OperationEvent::DeleteProgress {
                completed,
                total,
                deleted_location,
                ..
            } = &event
            {
                if *total <= MAX_INCREMENTAL_OPERATION_UPDATES
                    && let Some(location) = deleted_location
                {
                    browser.remove_deleted_locations(std::slice::from_ref(location));
                }
                browser.emit(BrowserEvent::DeletionProgress {
                    completed: *completed,
                    total: *total,
                });
                return;
            }
            if let OperationEvent::TransferProgress {
                completed, total, ..
            } = &event
            {
                browser.emit(BrowserEvent::TransferProgress {
                    completed: *completed,
                    total: *total,
                });
                return;
            }
            if let OperationEvent::RestoreProgress {
                completed,
                total,
                restored_location,
                ..
            } = &event
            {
                if *total <= MAX_INCREMENTAL_OPERATION_UPDATES
                    && let Some(location) = restored_location
                {
                    browser.remove_deleted_locations(std::slice::from_ref(location));
                }
                browser.emit(BrowserEvent::RestorationProgress {
                    completed: *completed,
                    total: *total,
                });
                return;
            }
            if let OperationEvent::ArchiveStarted { total, .. } = &event {
                browser.emit(BrowserEvent::ArchiveStarted { total: *total });
                return;
            }
            if let OperationEvent::ArchiveProgress {
                completed, total, ..
            } = &event
            {
                browser.emit(BrowserEvent::ArchiveProgress {
                    completed: *completed,
                    total: *total,
                });
                return;
            }
            browser.current_operation.set(None);
            let moving = browser.transfer_operation.replace(None);
            let deleting = browser.deletion_operation.replace(false);
            let restoring = browser.restoration_operation.replace(false);
            if moving.is_some() {
                let moved_locations = match &event {
                    OperationEvent::Pasted { locations, .. } if moving == Some(true) => {
                        locations.clone()
                    }
                    OperationEvent::Cancelled { result, .. } if moving == Some(true) => {
                        result.completed.clone()
                    }
                    OperationEvent::TransferFailed {
                        completed_locations,
                        ..
                    } if moving == Some(true) => completed_locations.clone(),
                    _ => Vec::new(),
                };
                browser.emit(BrowserEvent::TransferFinished { moved_locations });
            }
            if deleting {
                browser.emit(BrowserEvent::DeletionFinished);
            }
            if restoring {
                browser.emit(BrowserEvent::RestorationFinished);
            }
            browser.operation_load.borrow_mut().take();
            match event {
                OperationEvent::Failed { message, .. } if rename => {
                    browser.emit(BrowserEvent::RenameFailed { message });
                }
                OperationEvent::Failed { message, .. } => {
                    browser.emit(BrowserEvent::OperationFailed { message });
                }
                OperationEvent::TransferFailed { message, .. } => {
                    for location in &refresh_locations {
                        browser.refresh_columns_at(location);
                    }
                    browser.emit(BrowserEvent::OperationFailed { message });
                }
                OperationEvent::CompletedWithErrors {
                    deleted_locations,
                    message,
                    ..
                } => {
                    browser.remove_deleted_locations(&deleted_locations);
                    browser.emit(BrowserEvent::OperationCompletedWithErrors { message });
                }
                OperationEvent::Deleted { locations, .. }
                | OperationEvent::Restored { locations, .. } => {
                    browser.remove_deleted_locations(&locations);
                }
                OperationEvent::RestoreCompletedWithErrors {
                    restored_locations,
                    message,
                    ..
                } => {
                    browser.remove_deleted_locations(&restored_locations);
                    browser.emit(BrowserEvent::OperationCompletedWithErrors { message });
                }
                OperationEvent::Cancelled { result, .. } => {
                    let mut affected_locations = refresh_locations.clone();
                    affected_locations.extend(result.affected_locations);
                    browser.emit(BrowserEvent::OperationCancelled {
                        completed: result.completed.len(),
                        failed: result.failed.len(),
                        not_attempted: result.not_attempted.len(),
                        affected_locations,
                    });
                }
                OperationEvent::Renamed { .. } => {
                    browser.emit(BrowserEvent::RenameCompleted);
                    for location in &refresh_locations {
                        if location.native_path().is_none() {
                            browser.refresh_columns_at(location);
                        }
                    }
                }
                OperationEvent::Compressed { archive_name, .. } => {
                    browser.emit(BrowserEvent::ArchiveCompleted {
                        select_name: archive_name.clone(),
                    });
                }
                OperationEvent::Extracted { first_name, .. } => {
                    browser.emit(BrowserEvent::ArchiveCompleted {
                        select_name: first_name.unwrap_or_default(),
                    });
                }
                OperationEvent::Pasted { .. } => {
                    browser.emit(BrowserEvent::TransferCompleted);
                    for location in &refresh_locations {
                        if location.native_path().is_none() {
                            browser.refresh_columns_at(location);
                        }
                    }
                }
                OperationEvent::Created { .. } => {
                    for location in &refresh_locations {
                        if location.native_path().is_none() {
                            browser.refresh_columns_at(location);
                        }
                    }
                }
                OperationEvent::TransferProgress { .. }
                | OperationEvent::DeleteProgress { .. }
                | OperationEvent::RestoreProgress { .. }
                | OperationEvent::ArchiveStarted { .. }
                | OperationEvent::ArchiveProgress { .. } => {}
            }
        })
    }

    pub fn preview(self: &Rc<Self>, depth: usize, position: usize) {
        let Some(entry) = self.entry_at(depth, position) else {
            return;
        };
        if entry.is_directory() && self.is_open_child(depth, &entry.location) {
            self.close_column(depth + 1);
            return;
        }
        self.select(depth, position);
        if entry.is_directory() {
            self.descend(depth, entry.location);
        } else {
            self.emit(BrowserEvent::PreviewRequested { entry });
        }
    }

    pub fn open_location(&self, location: Location) {
        self.emit(BrowserEvent::OpenRequested { location });
    }

    pub fn request_empty_trash(&self) {
        self.emit(BrowserEvent::EmptyTrashRequested);
    }

    pub fn activate(self: &Rc<Self>, depth: usize, position: usize) {
        if self
            .entry_at(depth, position)
            .is_some_and(|entry| entry.is_directory() && self.is_open_child(depth, &entry.location))
        {
            self.close_column(depth + 1);
            return;
        }
        self.select(depth, position);
        self.activate_focused();
    }

    fn is_open_child(&self, parent_depth: usize, location: &Location) -> bool {
        parent_depth
            .checked_add(1)
            .and_then(|depth| self.location_at(depth))
            .as_ref()
            == Some(location)
    }

    /// Activates an item using conventional single-pane explorer navigation.
    pub fn activate_in_place(self: &Rc<Self>, depth: usize, position: usize) {
        self.select(depth, position);
        let Some(entry) = self.entry_at(depth, position) else {
            return;
        };
        if entry.is_directory() {
            self.navigate(entry.location);
        } else {
            self.emit(BrowserEvent::OpenRequested {
                location: entry.location,
            });
        }
    }

    pub fn activate_focused_in_place(self: &Rc<Self>) {
        let Some((depth, position, _)) = self.focused_item() else {
            self.move_selection(1);
            return;
        };
        self.activate_in_place(depth, position);
    }

    pub fn move_selection(&self, direction: i32) {
        let moved = self.state.borrow_mut().move_selection(direction);
        if let Some((depth, position)) = moved {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: Some(position),
            });
        }
    }

    pub fn extend_selection(&self, direction: i32) {
        let extended = self.state.borrow_mut().extend_selection(direction);
        if let Some((depth, focused, positions)) = extended {
            self.emit(BrowserEvent::SelectionSetChanged {
                depth,
                positions,
                focused,
                take_focus: true,
            });
        }
    }

    pub fn focus_parent(&self) {
        let focus = self.state.borrow_mut().focus_parent();
        if let Some((depth, position)) = focus {
            self.emit(BrowserEvent::FocusChanged { depth, position });
        }
    }

    fn focus_child(&self) {
        let focus = self.state.borrow_mut().focus_child();
        if let Some((depth, position)) = focus {
            self.emit(BrowserEvent::FocusChanged { depth, position });
        }
    }

    pub fn activate_focused(self: &Rc<Self>) {
        let focused = self.state.borrow().focused_entry();
        let Some((depth, _, entry)) = focused else {
            self.move_selection(1);
            return;
        };

        if entry.is_directory() {
            if self.is_open_child(depth, &entry.location) {
                self.focus_child();
            } else {
                self.descend_with_selection(depth, entry.location, true);
            }
        } else {
            self.emit(BrowserEvent::OpenRequested {
                location: entry.location,
            });
        }
    }

    fn restore_path(self: &Rc<Self>, path: NavigationPath) {
        self.close_peek();
        self.loads.borrow_mut().clear();
        self.monitors.borrow_mut().clear();
        let loads: Vec<_> = path
            .locations()
            .iter()
            .cloned()
            .map(|location| {
                let request_id = self.new_request_id();
                (location, request_id)
            })
            .collect();
        self.state
            .borrow_mut()
            .restore(path, loads.iter().map(|(_, request_id)| *request_id));

        self.emit(BrowserEvent::Reset);
        let active_depth = loads.len().checked_sub(1);
        for (depth, (location, request_id)) in loads.into_iter().enumerate() {
            self.emit(BrowserEvent::ColumnAdded {
                depth,
                location: location.clone(),
            });
            self.start_load(depth, location, request_id);
        }
        if let Some(depth) = active_depth {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: None,
            });
        }
    }

    fn start_load(self: &Rc<Self>, depth: usize, location: Location, request_id: RequestId) {
        let handle = self.request_directory(location.clone(), request_id);
        self.loads.borrow_mut().push(handle);

        let monitor = self.install_monitor(depth, location);
        self.monitors.borrow_mut().push(monitor);
    }

    fn install_monitor(self: &Rc<Self>, depth: usize, location: Location) -> Option<LoadHandle> {
        let weak: Weak<Self> = Rc::downgrade(self);
        let watched = location.clone();
        let notify = Rc::new(move |change| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_change(depth, &watched, change);
            }
        });
        self.source
            .watch(location, self.preferences.get().show_hidden, notify)
    }

    fn request_directory(self: &Rc<Self>, location: Location, request_id: RequestId) -> LoadHandle {
        let weak: Weak<Self> = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_event(event);
            }
        });
        self.source.enumerate(
            DirectoryRequest {
                id: request_id,
                location,
                batch_size: 128,
                include_hidden: self.preferences.get().show_hidden,
                max_entries: MAX_DIRECTORY_ENTRIES,
                time_budget: DIRECTORY_LOAD_TIME_BUDGET,
            },
            emit,
        )
    }

    pub(crate) fn refresh_columns_at(self: &Rc<Self>, location: &Location) {
        let depths = {
            let state = self.state.borrow();
            let mut depths = Vec::new();
            let mut depth = 0;
            while let Some(open_location) = state.location_at(depth) {
                if &open_location == location {
                    depths.push(depth);
                }
                depth += 1;
            }
            depths
        };
        for depth in depths {
            self.refresh_column(depth);
        }
    }

    pub(crate) fn refresh_after_cancellation(self: &Rc<Self>, roots: &HashSet<Location>) {
        self.refresh_columns_at_or_below(roots);
    }

    fn refresh_columns_at_or_below(self: &Rc<Self>, roots: &HashSet<Location>) {
        let open_locations = {
            let state = self.state.borrow();
            let mut locations = Vec::new();
            let mut depth = 0;
            while let Some(location) = state.location_at(depth) {
                locations.push((depth, location));
                depth += 1;
            }
            locations
        };
        for (depth, location) in open_locations {
            if location_or_ancestor_is_affected(&location, roots) {
                self.refresh_column(depth);
            }
        }
    }

    fn remove_deleted_locations(self: &Rc<Self>, locations: &[Location]) {
        if locations.len() > MAX_INCREMENTAL_OPERATION_UPDATES {
            let parents: HashSet<_> = locations
                .iter()
                .filter_map(deletion_parent_location)
                .collect();
            for parent in parents {
                self.refresh_columns_at(&parent);
            }
            return;
        }
        for location in locations {
            let Some(parent) = deletion_parent_location(location) else {
                continue;
            };
            let depths = {
                let state = self.state.borrow();
                let mut depths = Vec::new();
                let mut depth = 0;
                while let Some(open_location) = state.location_at(depth) {
                    if open_location == parent {
                        depths.push(depth);
                    }
                    depth += 1;
                }
                depths
            };
            for depth in depths {
                self.handle_directory_change(
                    depth,
                    &parent,
                    DirectoryChange::Remove(location.clone()),
                );
            }
        }
    }

    pub fn retry_column(self: &Rc<Self>, depth: usize) {
        self.refresh_column(depth);
    }

    fn refresh_column(self: &Rc<Self>, depth: usize) {
        let request_id = self.new_request_id();
        let location = self.state.borrow_mut().reload_column(depth, request_id);
        let Some(location) = location else {
            return;
        };
        self.emit(BrowserEvent::ColumnReloaded { depth });
        let handle = self.request_directory(location, request_id);
        if let Some(load) = self.loads.borrow_mut().get_mut(depth) {
            *load = handle;
        }
    }

    pub fn reload_active(self: &Rc<Self>) {
        if let Some(depth) = self.active_depth() {
            self.refresh_column(depth);
        }
    }

    pub fn select_entries_by_name(self: &Rc<Self>, names: &[String]) {
        let Some(depth) = self.active_depth() else {
            return;
        };
        let requested: HashSet<&str> = names.iter().map(String::as_str).collect();
        let state = self.state.borrow();
        let Some(column) = state.columns.get(depth) else {
            return;
        };
        let positions: Vec<usize> = column
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| {
                requested
                    .contains(entry.display_name.as_str())
                    .then_some(position)
            })
            .collect();
        drop(state);
        let Some(&focused) = positions.last() else {
            return;
        };
        self.set_selection(depth, &positions, Some(focused));
        self.emit(BrowserEvent::SelectionSetChanged {
            depth,
            positions,
            focused,
            take_focus: true,
        });
    }

    fn handle_directory_change(
        self: &Rc<Self>,
        depth: usize,
        watched: &Location,
        change: DirectoryChange,
    ) {
        if matches!(&change, DirectoryChange::Rescan) {
            self.refresh_column(depth);
            return;
        }
        let path_update = self
            .state
            .borrow()
            .path_after_external_change(depth, &change);
        if let Some(path) = path_update {
            self.restore_path(path);
            return;
        }

        let application = self
            .state
            .borrow_mut()
            .apply_directory_change(depth, watched, change);
        if let Some((splices, selected)) = application {
            let positions = self.state.borrow().selected_positions(depth);
            self.emit(BrowserEvent::EntriesSpliced {
                depth,
                splices,
                selected,
            });
            if let Some(focused) = selected {
                self.emit(BrowserEvent::SelectionSetChanged {
                    depth,
                    positions,
                    focused,
                    take_focus: false,
                });
            }
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: selected,
            });
        }
    }

    fn handle_directory_event(&self, event: DirectoryEvent) {
        match event {
            DirectoryEvent::Batch {
                request_id,
                entries,
            } => {
                let mut state = self.state.borrow_mut();
                let application = state.apply_batch(request_id, entries.clone());
                if let Some((depth, insertions)) = application {
                    tracing::debug!(
                        request_id = request_id.0,
                        location = %state.columns[depth].location.diagnostic_path(),
                        entries = entries.len(),
                        "directory batch accepted"
                    );
                    let selected = state.columns[depth].selected;
                    let positions = state.selected_positions(depth);
                    drop(state);
                    self.emit(BrowserEvent::EntriesInserted { depth, insertions });
                    if let Some(focused) = selected {
                        self.emit(BrowserEvent::SelectionSetChanged {
                            depth,
                            positions,
                            focused,
                            take_focus: false,
                        });
                    }
                } else if state.apply_peek_batch(request_id, &entries) {
                    drop(state);
                    self.emit(BrowserEvent::PeekEntriesAdded { entries });
                }
            }
            DirectoryEvent::Finished {
                request_id,
                truncated,
            } => {
                let mut state = self.state.borrow_mut();
                if let Some(depth) = state.finish(request_id) {
                    drop(state);
                    self.emit(BrowserEvent::LoadFinished { depth, truncated });
                } else if state.finish_peek(request_id) {
                    drop(state);
                    self.emit(BrowserEvent::PeekFinished);
                }
            }
            DirectoryEvent::Failed {
                request_id,
                message,
            } => {
                let mut state = self.state.borrow_mut();
                if let Some(depth) = state.fail(request_id, message.clone()) {
                    drop(state);
                    self.emit(BrowserEvent::LoadFailed { depth, message });
                } else if state.fail_peek(request_id, message.clone()) {
                    drop(state);
                    self.emit(BrowserEvent::PeekFailed { message });
                }
            }
        }
    }

    fn emit(&self, event: BrowserEvent) {
        let observers = self.observers.borrow().clone();
        for observer in observers {
            observer(event.clone());
        }
    }

    fn new_request_id(&self) -> RequestId {
        let id = self.next_request.get();
        self.next_request.set(id.saturating_add(1));
        RequestId(id)
    }
}

fn location_or_ancestor_is_affected(location: &Location, roots: &HashSet<Location>) -> bool {
    let mut current = Some(location.clone());
    while let Some(location) = current {
        if roots.contains(&location) {
            return true;
        }
        current = location.parent();
    }
    false
}

fn deletion_parent_location(location: &Location) -> Option<Location> {
    if location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
    {
        Some(Location::uri("trash:///"))
    } else {
        location.parent()
    }
}

fn location_from_input(input: &str) -> Result<Location, LocationValidationError> {
    if !is_uri_like(input) {
        return Ok(Location::local(PathBuf::from(input)));
    }
    let scheme_end = input.find("://").unwrap_or_default();
    let scheme = &input[..scheme_end];
    let normalized = scheme.to_ascii_lowercase();
    if !matches!(
        normalized.as_str(),
        "smb" | "sftp" | "ftp" | "ftps" | "dav" | "davs" | "trash" | "network"
    ) {
        return Err(LocationValidationError::UnsupportedScheme(format!(
            "The {scheme}:// scheme isn't supported. Use an absolute local path or one of: \
             smb://, sftp://, ftp://, ftps://, dav://, or davs://."
        )));
    }
    validate_uri_credentials(input)?;
    let uri = format!("{normalized}{}", &input[scheme_end..]);
    Ok(Location::uri(uri))
}

/// UNC paths (`\\host\share`, bare `//host/share`) and SCP-style addresses
/// (`user@host:path`) are deliberately not accepted as location-bar shorthand
/// (see lgse/strata#20) so a proper URI (`smb://`, `sftp://`, ...) is always
/// preserved verbatim rather than being guessed at. Report a clear message
/// instead of silently treating either as a relative local path.
fn unsupported_shorthand_message(input: &str) -> Option<&'static str> {
    let looks_like_unc = input.starts_with("\\\\")
        || ["smb:", "SMB:"].iter().any(|prefix| {
            input
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with("\\\\"))
        });
    // A bare `//host/share` has no scheme, so it is not a valid URI (unlike
    // `smb://host/share`, which `is_uri_like` already accepts untouched).
    let looks_like_bare_network_shorthand = input.starts_with("//") && !is_uri_like(input);
    if looks_like_unc || looks_like_bare_network_shorthand || looks_like_scp_shorthand(input) {
        Some(
            "UNC paths (\\\\host\\share) and SCP-style addresses (user@host:path) aren't \
             supported. Use a URI instead, such as smb://host/share, sftp://host/path, \
             ftp://host/path, or dav://host/path.",
        )
    } else {
        None
    }
}

fn looks_like_scp_shorthand(input: &str) -> bool {
    if is_uri_like(input) {
        return false;
    }
    let Some((_user, after_at)) = input.split_once('@') else {
        return false;
    };
    let Some(host) = after_at.split(':').next() else {
        return false;
    };
    !host.is_empty() && after_at.contains(':') && !host.contains('/') && !host.contains('\\')
}

fn is_uri_like(input: &str) -> bool {
    let Some(scheme_end) = input.find("://") else {
        return false;
    };
    let scheme = &input[..scheme_end];
    scheme.starts_with(|character: char| character.is_ascii_alphabetic())
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '-')
        })
}

#[cfg(test)]
mod tests;
