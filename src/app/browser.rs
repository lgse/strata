// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::Duration,
};

use crate::{
    app::navigation::{EntryInsertion, EntrySplice, NavigationPath, NavigationState, sort_entries},
    model::{FileEntry, Location, SortDirection, SortKey, ViewPreferences},
    services::{
        ArchiveFormat, CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest,
        DirectoryChange, DirectoryEvent, DirectoryRequest, ExtractRequest, FileSource, LoadHandle,
        LocationValidationError, MetadataOutcome, MetadataRequest, MoveRecord, OperationEvent,
        OperationProvider, OperationRequestId, PasteItem, PasteRequest, RenameRequest, RequestId,
        RestoreRequest, RestoreSource, TransferConflict, UndoMoveItem, UndoMoveRequest,
        validate_basename, validate_uri_credentials,
    },
};

/// Caps a normal directory load at this project's own documented performance baseline for
/// 100,000 entries (docs/performance-baseline.md: 3,755 ms, 286 MiB) -- past this, per-batch
/// merge cost grows enough that browsing stops feeling responsive.
const MAX_DIRECTORY_ENTRIES: usize = 100_000;
const DIRECTORY_LOAD_TIME_BUDGET: Duration = Duration::from_secs(10);

/// Larger GIO batches cut per-batch merge, selection scan, and GTK splice
/// cost on large listings; remote locations keep small batches for fast first paint.
const NATIVE_DIRECTORY_BATCH_SIZE: usize = 512;
const REMOTE_DIRECTORY_BATCH_SIZE: usize = 128;

/// A hover peek only ever displays a handful of entries (`PeekBehavior::item_limit`), so it
/// needs far less headroom than a full directory load -- just enough to survive hidden-file
/// filtering, not enough to enumerate an entire large directory for a preview that discards
/// nearly all of it.
const PEEK_MAX_ENTRIES: usize = 64;
const PEEK_TIME_BUDGET: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
pub struct BrowserColumnSnapshot {
    pub location: Location,
    pub count: usize,
    pub selected_positions: Vec<usize>,
    pub loading: bool,
    pub error: Option<String>,
    pub truncated: bool,
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
        count: usize,
    },
    /// A range already installed in authoritative state; views borrow it during
    /// synchronous dispatch instead of receiving a deep clone.
    EntriesPublished {
        depth: usize,
        position: usize,
        count: usize,
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
    /// Refreshed entries for already-rendered rows; the order never changes here.
    MetadataFilled {
        depth: usize,
        updates: Vec<(usize, FileEntry)>,
    },
    ColumnReloaded {
        depth: usize,
    },
    HiddenToggled {
        show_hidden: bool,
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
        completed_items: usize,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
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
        /// Entries a retry with `permanent: true` would likely delete
        /// successfully, e.g. ones that failed only because this location
        /// doesn't support Trash. Always empty for a restore failure.
        retryable_locations: Vec<Location>,
        has_non_retryable_failures: bool,
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

/// Events dispatch by reference: payloads move once into authoritative state,
/// observers borrow, and the observer list is cloned up front so reentrant
/// emission stays safe.
type Observer = Rc<dyn Fn(&BrowserEvent)>;
type PreferencesObserver = Rc<dyn Fn(ViewPreferences)>;

const MAX_INCREMENTAL_OPERATION_UPDATES: usize = 64;

/// The latest reversible operation. Trash entries restore from Trash; moved
/// entries transfer back to the location they started from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UndoEntry {
    Trash(Vec<Location>),
    Move(Vec<MoveRecord>),
}

impl UndoEntry {
    fn is_empty(&self) -> bool {
        match self {
            Self::Trash(locations) => locations.is_empty(),
            Self::Move(records) => records.is_empty(),
        }
    }
}

#[derive(Default)]
struct UndoState {
    generation: u64,
    entry: Option<UndoEntry>,
    claimed: bool,
}

// Undo follows the latest operation across every Strata window on the GTK main thread.
thread_local! {
    static PENDING_UNDO: RefCell<UndoState> = RefCell::new(UndoState::default());
}

fn replace_pending_undo(entry: UndoEntry) {
    if entry.is_empty() {
        return;
    }
    PENDING_UNDO.with(|pending| {
        let generation = pending.borrow().generation.saturating_add(1);
        pending.replace(UndoState {
            generation,
            entry: Some(entry),
            claimed: false,
        });
    });
}

fn peek_pending_undo() -> Option<(u64, UndoEntry)> {
    PENDING_UNDO.with(|pending| {
        let pending = pending.borrow();
        if pending.claimed {
            return None;
        }
        Some((pending.generation, pending.entry.clone()?))
    })
}

/// Claims the pending entry so a second undo cannot run the same work twice.
/// `expected` pins the claim to the entry a caller already inspected.
fn claim_pending_undo(expected: Option<u64>) -> Option<(u64, UndoEntry)> {
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.claimed || expected.is_some_and(|generation| generation != pending.generation) {
            return None;
        }
        let entry = pending.entry.clone()?;
        pending.claimed = true;
        Some((pending.generation, entry))
    })
}

/// Drops one still-pending item so a partial undo leaves only the remainder
/// for the next attempt.
fn mark_undo_item_completed(generation: u64, location: &Location) {
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.generation != generation {
            return;
        }
        match pending.entry.as_mut() {
            Some(UndoEntry::Trash(locations)) => {
                locations.retain(|candidate| candidate != location);
            }
            Some(UndoEntry::Move(records)) => {
                records.retain(|record| &record.current != location);
            }
            None => {}
        }
    });
}

fn finish_undo(generation: u64, completed: bool) {
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.generation == generation {
            if completed {
                pending.entry = None;
            }
            pending.claimed = false;
        }
    });
}

fn retain_pending_move_items(generation: u64, items: &[UndoMoveItem]) {
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.generation == generation
            && let Some(UndoEntry::Move(records)) = pending.entry.as_mut()
        {
            records.retain(|record| items.iter().any(|item| &item.record == record));
        }
    });
}

/// Pairs each moved source with where the transfer left it. Items that never
/// left their own directory carry no undo.
fn move_records(sources: &[Location], destination: &Location) -> Vec<MoveRecord> {
    sources
        .iter()
        .filter_map(|source| {
            let current = source.transfer_target(destination)?;
            (&current != source).then(|| MoveRecord {
                original: source.clone(),
                current,
            })
        })
        .collect()
}

#[cfg(test)]
fn pending_undo_entry() -> Option<UndoEntry> {
    PENDING_UNDO.with(|pending| pending.borrow().entry.clone())
}

/// Settles scrolling before asking for viewport metadata, so a fling never
/// stats hundreds of rows it never shows.
const METADATA_FILL_DEBOUNCE: Duration = Duration::from_millis(100);
/// Bounds one metadata fill; partial results still apply, the rest retries on
/// its next bind.
const METADATA_FILL_TIME_BUDGET: Duration = Duration::from_secs(5);
/// Defensive cap per depth: the UI only ever asks for its visible window.
const MAX_PENDING_FILL_LOCATIONS: usize = 1024;

/// Remote loads only (native loads stage instead): entries accumulate this far
/// before an early flush bounds first-result latency.
const COALESCE_ENTRIES: usize = 2048;
/// Bounds one remote progressive flush: a slow link must not turn one timer
/// fire into a multi-frame GTK mutation.
const REMOTE_FLUSH_CAP: usize = 512;
/// Latency bound so later remote batches flush on the next idle/frame.
const REMOTE_FLUSH_DELAY: Duration = Duration::from_millis(50);
/// Rows published synchronously with a staged load or sort; the rest stream
/// from idle callbacks inside an 8 ms work budget.
const FIRST_PUBLISH_COUNT: usize = 128;
const STAGE_INLINE_LIMIT: usize = 512;
/// Snapshots at or below this size sort synchronously; larger ones sort in a
/// blocking worker.
const SORT_INLINE_LIMIT: usize = 2048;
const PUBLISH_TAIL_CHUNK: usize = 2048;
const PUBLISH_SLICE_BUDGET: Duration = Duration::from_millis(8);

/// Last selection emitted per depth on the batch path, keyed by request so a
/// new load re-emits; lets background batches skip redundant refreshes.
type BatchSelectionState = HashMap<usize, (RequestId, Vec<usize>, usize)>;

/// One bound row's fill request: stable location plus its source position, so
/// fills apply after validating the row has not moved.
struct ViewportTarget {
    position: usize,
    location: Location,
}

/// Routing for one metadata request: own ids, validated against the owning
/// directory request.
struct ViewportFill {
    depth: usize,
    directory_request: RequestId,
    tokens: Vec<(usize, Location)>,
}

#[derive(Clone, Copy)]
struct SortFill {
    generation: u64,
    depth: usize,
    fill_request: RequestId,
    directory_request: RequestId,
    preferences: ViewPreferences,
}

/// A native initial load in flight: batches accumulate with no merge walk and
/// no UI events; monitor deltas queue for one reconcile, and removed locations
/// filter later batches so stale batches never resurrect deleted entries.
struct StagingLoad {
    request_id: RequestId,
    entries: Vec<FileEntry>,
    removed: HashSet<Location>,
    deltas: Vec<(Location, DirectoryChange)>,
    metadata_incomplete: bool,
}

/// A native load sorting off-thread after enumeration finished. Deltas
/// arriving here queue for the completion's silent reconcile.
struct SortingLoad {
    request_id: RequestId,
    deltas: Vec<(Location, DirectoryChange)>,
}

#[derive(Clone, Copy)]
struct SortPlan {
    ordering_preferences: ViewPreferences,
    staged_preferences: ViewPreferences,
    retry_metadata: bool,
    truncated: bool,
    can_trash: Option<bool>,
    can_delete: Option<bool>,
}

enum PublishTerminal {
    LoadFinished {
        truncated: bool,
        retry_metadata: bool,
    },
    SortingFinished,
}

enum RemoteTerminal {
    Finished {
        request_id: RequestId,
        truncated: bool,
        can_trash: Option<bool>,
        can_delete: Option<bool>,
    },
    Failed {
        request_id: RequestId,
        message: String,
    },
}

/// A staged publication streaming to the UI: the prefix is already in the model,
/// tails append from idle callbacks in a work budget, and selection plus the
/// terminal event wait for the final tail.
struct StagedPublish {
    request_id: RequestId,
    published: usize,
    total: usize,
    focused: Option<usize>,
    positions: Vec<usize>,
    terminal: PublishTerminal,
}

pub struct Browser {
    source: Rc<dyn FileSource>,
    state: RefCell<NavigationState>,
    loads: RefCell<Vec<LoadHandle>>,
    monitors: RefCell<Vec<Option<LoadHandle>>>,
    metadata_pending: RefCell<HashMap<usize, Vec<ViewportTarget>>>,
    metadata_timer: RefCell<Option<gio::glib::SourceId>>,
    staging: RefCell<HashMap<usize, StagingLoad>>,
    sorting: RefCell<HashMap<usize, SortingLoad>>,
    staged_publishes: RefCell<HashMap<usize, StagedPublish>>,
    publish_timer: RefCell<Option<gio::glib::SourceId>>,
    remote_flush_timer: RefCell<Option<gio::glib::SourceId>>,
    remote_terminals: RefCell<HashMap<usize, RemoteTerminal>>,
    metadata_loads: RefCell<HashMap<usize, LoadHandle>>,
    fill_tokens: RefCell<HashMap<RequestId, ViewportFill>>,
    /// Full-column sort fills, kept apart from viewport fills so a viewport
    /// settle timer can never overwrite or cancel an active full sort.
    sort_loads: RefCell<HashMap<usize, LoadHandle>>,
    coalesce_pending: RefCell<HashMap<usize, (RequestId, Vec<FileEntry>)>>,
    sort_awaiting_fill: RefCell<Option<SortFill>>,
    last_batch_selection: RefCell<BatchSelectionState>,
    peek_load: RefCell<Option<LoadHandle>>,
    validation_load: RefCell<Option<LoadHandle>>,
    validation_generation: Cell<u64>,
    operation_provider: RefCell<Option<Rc<dyn OperationProvider>>>,
    operation_load: RefCell<Option<LoadHandle>>,
    current_operation: Cell<Option<OperationRequestId>>,
    transfer_operation: Cell<Option<bool>>,
    deletion_operation: Cell<bool>,
    deletion_permanent: Cell<bool>,
    restoration_operation: Cell<bool>,
    transfer_destination: RefCell<Option<Location>>,
    undo_claim: RefCell<Option<(u64, UndoEntry)>>,
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
            metadata_pending: RefCell::new(HashMap::new()),
            metadata_timer: RefCell::new(None),
            staging: RefCell::new(HashMap::new()),
            sorting: RefCell::new(HashMap::new()),
            staged_publishes: RefCell::new(HashMap::new()),
            publish_timer: RefCell::new(None),
            remote_flush_timer: RefCell::new(None),
            remote_terminals: RefCell::new(HashMap::new()),
            metadata_loads: RefCell::new(HashMap::new()),
            fill_tokens: RefCell::new(HashMap::new()),
            sort_loads: RefCell::new(HashMap::new()),
            coalesce_pending: RefCell::new(HashMap::new()),
            sort_awaiting_fill: RefCell::new(None),
            last_batch_selection: RefCell::new(HashMap::new()),
            peek_load: RefCell::new(None),
            validation_load: RefCell::new(None),
            validation_generation: Cell::new(0),
            operation_provider: RefCell::new(None),
            operation_load: RefCell::new(None),
            current_operation: Cell::new(None),
            transfer_operation: Cell::new(None),
            deletion_operation: Cell::new(false),
            deletion_permanent: Cell::new(false),
            restoration_operation: Cell::new(false),
            transfer_destination: RefCell::new(None),
            undo_claim: RefCell::new(None),
            next_request: Cell::new(1),
            pending_sort: Cell::new(None),
            preferences: Cell::new(preferences),
            observers: RefCell::new(Vec::new()),
            preferences_observers: RefCell::new(Vec::new()),
        })
    }

    pub fn observe(&self, observer: impl Fn(&BrowserEvent) + 'static) {
        self.observers.borrow_mut().push(Rc::new(observer));
    }

    pub fn clear_observer(&self) {
        self.observers.borrow_mut().clear();
    }

    pub fn preferences(&self) -> ViewPreferences {
        self.preferences.get()
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
        let input = input.trim();
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

    pub fn can_trash_at(&self, depth: usize) -> Option<bool> {
        self.state.borrow().can_trash_at(depth)
    }

    pub fn can_delete_at(&self, depth: usize) -> Option<bool> {
        self.state.borrow().can_delete_at(depth)
    }

    /// Synchronizes widget focus without changing selection or reopening a directory.
    pub fn set_active_column(&self, depth: usize) {
        self.state.borrow_mut().focus_column(depth);
    }

    pub fn select_first_on_load(&self, depth: usize) {
        self.state.borrow_mut().select_first_on_load(depth);
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
        self.cancel_deferred_work();
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
        self.truncate_deferred_from(retained);
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
        if self.is_open_child(origin_depth, &location) {
            return;
        }
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
        // Peeks stay small and show metadata immediately, so they skip the streaming split.
        let handle = self.source.enumerate(
            DirectoryRequest {
                id: request_id,
                location,
                batch_size: 128,
                include_metadata: true,
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

    pub fn escape(self: &Rc<Self>) {
        if self.close_peek() {
            return;
        }

        let closed = self.state.borrow_mut().close_deepest();
        if let Some((depth, position)) = closed {
            let len = depth + 1;
            self.loads.borrow_mut().truncate(len);
            self.monitors.borrow_mut().truncate(len);
            self.truncate_deferred_from(len);
            self.emit(BrowserEvent::ColumnsTruncated { len });
            self.emit(BrowserEvent::FocusChanged { depth, position });
        }
    }

    pub fn close_column(self: &Rc<Self>, depth: usize) {
        self.close_peek();
        let closed = self.state.borrow_mut().close_from(depth);
        if let Some((parent_depth, position)) = closed {
            self.loads.borrow_mut().truncate(depth);
            self.monitors.borrow_mut().truncate(depth);
            self.truncate_deferred_from(depth);
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

        self.close_peek();
        self.state
            .borrow_mut()
            .set_show_hidden(preferences.show_hidden);
        self.emit(BrowserEvent::HiddenToggled {
            show_hidden: preferences.show_hidden,
        });
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
            if let Some(browser) = weak.upgrade() {
                browser.apply_debounced_sort(depth, generation, update);
            }
        });
    }

    fn apply_debounced_sort(
        self: &Rc<Self>,
        depth: usize,
        generation: u64,
        update: impl FnOnce(&mut ViewPreferences),
    ) {
        if self.pending_sort.get() != Some((generation, depth)) {
            return;
        }
        let result = {
            let mut state = self.state.borrow_mut();
            let Some(mut preferences) = state.column_preferences(depth) else {
                drop(state);
                self.pending_sort.set(None);
                self.emit(BrowserEvent::SortingFinished { depth });
                return;
            };
            update(&mut preferences);
            // Size/date sorts need the metadata streaming enumeration skipped:
            // fill the whole column first instead of sorting placeholders.
            let targets = state.column_unknown_metadata(depth).unwrap_or_default();
            if matches!(preferences.sort_key, SortKey::Size | SortKey::Modified)
                && !targets.is_empty()
            {
                drop(state);
                self.request_sort_fill(depth, generation, preferences, targets);
                return;
            }
            let result = state.apply_sort_preferences(depth, preferences);
            self.preferences.set(preferences);
            let request_id = state.request_id_for_depth(depth);
            let total = state.columns.get(depth).map(|column| column.entries.len());
            result.map(|(focused, positions)| (request_id, total, focused, positions))
        };
        self.notify_preferences_observers();
        if let Some((request_id, total, focused, positions)) = result {
            if let (Some(request_id), Some(total)) = (request_id, total) {
                self.publish_staged(
                    depth,
                    request_id,
                    total,
                    focused,
                    positions,
                    PublishTerminal::SortingFinished,
                );
            } else {
                self.pending_sort.set(None);
                self.emit(BrowserEvent::SortingFinished { depth });
            }
        } else {
            self.pending_sort.set(None);
            self.emit(BrowserEvent::SortingFinished { depth });
        }
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

    pub fn with_entries<R>(
        &self,
        depth: usize,
        range: std::ops::Range<usize>,
        read: impl FnOnce(&[FileEntry]) -> R,
    ) -> Option<R> {
        let state = self.state.borrow();
        let entries = &state.columns.get(depth)?.entries;
        Some(read(entries.get(range)?))
    }

    pub fn column_preferences(&self, depth: usize) -> Option<ViewPreferences> {
        self.state.borrow().column_preferences(depth)
    }

    pub fn column_snapshot(&self, depth: usize) -> Option<BrowserColumnSnapshot> {
        let state = self.state.borrow();
        let column = state.columns.get(depth)?;
        Some(BrowserColumnSnapshot {
            location: column.location.clone(),
            count: column.entries.len(),
            selected_positions: state.selected_positions(depth),
            loading: column.load_state == crate::app::navigation::LoadState::Loading,
            error: match &column.load_state {
                crate::app::navigation::LoadState::Error(message) => Some(message.clone()),
                _ => None,
            },
            truncated: column.truncated,
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

    pub fn selected_positions(&self, depth: usize) -> Vec<usize> {
        self.state.borrow().selected_positions(depth)
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
                selected = state.selected_count(),
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
        self.transfer_destination.replace(Some(destination.clone()));
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
        self.deletion_permanent.set(permanent);
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
                source: RestoreSource::TrashEntries(entries),
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.operation_load.replace(Some(load));
    }

    /// The pending move undo, if the latest reversible operation was a move.
    pub fn pending_undo_move(&self) -> Option<(u64, Vec<MoveRecord>)> {
        if self.current_operation.get().is_some() {
            return None;
        }
        match peek_pending_undo()? {
            (generation, UndoEntry::Move(records)) => Some((generation, records)),
            (_, UndoEntry::Trash(_)) => None,
        }
    }

    /// Drops a pending undo whose items are no longer where it recorded them.
    pub fn discard_pending_undo(&self, generation: u64) {
        if claim_pending_undo(Some(generation)).is_some() {
            finish_undo(generation, true);
        }
    }

    pub fn undo_last_trash(self: &Rc<Self>) -> bool {
        if self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_pending_undo(None) else {
            return false;
        };
        let UndoEntry::Trash(locations) = entry else {
            finish_undo(generation, false);
            return false;
        };
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_undo(generation, false);
            return false;
        };
        let total = locations.len();
        let request_id = self.begin_operation();
        self.restoration_operation.set(true);
        self.undo_claim
            .replace(Some((generation, UndoEntry::Trash(locations.clone()))));
        self.emit(BrowserEvent::RestorationStarted { total });
        let load = provider.restore(
            RestoreRequest {
                id: request_id,
                source: RestoreSource::OriginalLocations(locations),
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.operation_load.replace(Some(load));
        true
    }

    /// Moves completed transfers back. `generation` pins the undo the caller
    /// inspected, so an operation started while conflicts were being confirmed
    /// wins instead of being reverted.
    pub fn undo_move(self: &Rc<Self>, generation: u64, items: Vec<UndoMoveItem>) -> bool {
        if items.is_empty() || self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_pending_undo(Some(generation)) else {
            return false;
        };
        let UndoEntry::Move(_) = entry else {
            finish_undo(generation, false);
            return false;
        };
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_undo(generation, false);
            return false;
        };
        retain_pending_move_items(generation, &items);
        let total = items.len();
        let mut refresh_locations = HashSet::new();
        for item in &items {
            for location in [&item.record.current, &item.record.original] {
                if let Some(parent) = location.parent() {
                    refresh_locations.insert(parent);
                }
            }
        }
        let request_id = self.begin_operation();
        self.transfer_operation.set(Some(true));
        self.undo_claim.replace(Some((
            generation,
            UndoEntry::Move(items.iter().map(|item| item.record.clone()).collect()),
        )));
        self.emit(BrowserEvent::TransferStarted {
            total,
            moving: true,
        });
        let load = provider.undo_move(
            UndoMoveRequest {
                id: request_id,
                items,
            },
            self.operation_callback(request_id, false, refresh_locations),
        );
        self.operation_load.replace(Some(load));
        true
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
        if let Some((generation, _)) = self.undo_claim.take() {
            finish_undo(generation, false);
        }
        self.transfer_operation.set(None);
        self.transfer_destination.replace(None);
        self.deletion_operation.set(false);
        self.deletion_permanent.set(false);
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
                completed_items,
                transferred_bytes,
                total_bytes,
                ..
            } = &event
            {
                browser.emit(BrowserEvent::TransferProgress {
                    completed_items: *completed_items,
                    transferred_bytes: *transferred_bytes,
                    total_bytes: *total_bytes,
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
                if restored_location.is_some()
                    && let Some((generation, UndoEntry::Trash(locations))) =
                        browser.undo_claim.borrow().as_ref()
                    && let Some(location) = completed
                        .checked_sub(1)
                        .and_then(|index| locations.get(index))
                {
                    mark_undo_item_completed(*generation, location);
                }
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
            let deletion_permanent = browser.deletion_permanent.replace(false);
            let restoring = browser.restoration_operation.replace(false);
            let destination = browser.transfer_destination.replace(None);
            let undoing = browser.undo_claim.take();
            if let Some((generation, entry)) = &undoing {
                let completed = match (entry, &event) {
                    (UndoEntry::Trash(_), _) => Vec::new(),
                    (UndoEntry::Move(_), OperationEvent::Pasted { locations, .. }) => {
                        locations.clone()
                    }
                    (
                        UndoEntry::Move(_),
                        OperationEvent::TransferFailed {
                            completed_locations,
                            ..
                        },
                    ) => completed_locations.clone(),
                    (UndoEntry::Move(_), OperationEvent::Cancelled { result, .. }) => {
                        result.completed.clone()
                    }
                    (UndoEntry::Move(_), _) => Vec::new(),
                };
                for location in &completed {
                    mark_undo_item_completed(*generation, location);
                }
                finish_undo(
                    *generation,
                    match entry {
                        UndoEntry::Trash(_) => {
                            matches!(&event, OperationEvent::Restored { .. })
                        }
                        UndoEntry::Move(_) => matches!(&event, OperationEvent::Pasted { .. }),
                    },
                );
            }
            if deleting && !deletion_permanent {
                let locations = match &event {
                    OperationEvent::Deleted { locations, .. } => locations.clone(),
                    OperationEvent::CompletedWithErrors {
                        deleted_locations, ..
                    } => deleted_locations.clone(),
                    OperationEvent::Cancelled { result, .. } => result.completed.clone(),
                    _ => Vec::new(),
                };
                replace_pending_undo(UndoEntry::Trash(locations));
            }
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
                if undoing.is_none()
                    && let Some(destination) = destination.as_ref()
                {
                    replace_pending_undo(UndoEntry::Move(move_records(
                        &moved_locations,
                        destination,
                    )));
                }
                browser.emit(BrowserEvent::TransferFinished {
                    moved_locations: if undoing.is_some() {
                        Vec::new()
                    } else {
                        moved_locations
                    },
                });
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
                    retryable_locations,
                    has_non_retryable_failures,
                    message,
                    ..
                } => {
                    browser.remove_deleted_locations(&deleted_locations);
                    browser.emit(BrowserEvent::OperationCompletedWithErrors {
                        message,
                        retryable_locations,
                        has_non_retryable_failures,
                    });
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
                    browser.emit(BrowserEvent::OperationCompletedWithErrors {
                        message,
                        retryable_locations: Vec::new(),
                        has_non_retryable_failures: true,
                    });
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

    pub(crate) fn is_open_child(&self, parent_depth: usize, location: &Location) -> bool {
        parent_depth
            .checked_add(1)
            .and_then(|depth| self.location_at(depth))
            .as_ref()
            == Some(location)
    }

    /// Activates an item using conventional single-pane list navigation.
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

    /// Moves the focus by `page` visible entries, for `Page Up` and `Page Down`.
    pub fn page_selection(&self, direction: i32, page: usize) {
        let moved = self.state.borrow_mut().page_selection(direction, page);
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

    pub fn extend_visual_selection(&self, depth: usize, focused: usize, order: &[usize]) {
        let positions = self
            .state
            .borrow_mut()
            .extend_visual_selection(depth, focused, order);
        if let Some(positions) = positions {
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

    pub fn enter_focused_directory(self: &Rc<Self>) {
        match self.focused_entry() {
            Some(entry) if !entry.is_directory() => self.focus_child(),
            _ => self.activate_focused(),
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
        self.cancel_deferred_work();
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
        let handle = self.request_directory(depth, location.clone(), request_id);
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

    fn apply_owned_batch(self: &Rc<Self>, request_id: RequestId, entries: Vec<FileEntry>) {
        let install_started = std::time::Instant::now();
        let mut state = self.state.borrow_mut();
        let batch_len = entries.len();
        let Some((depth, insertions)) = state.apply_batch(request_id, entries) else {
            return;
        };
        tracing::debug!(
            request_id = request_id.0,
            location = %state.columns[depth].location.diagnostic_path(),
            entries = batch_len,
            "directory batch accepted"
        );
        let selected = state.columns[depth].selected;
        drop(state);
        crate::metrics::record_stage(
            "state-install",
            install_started.elapsed().as_millis() as u64,
        );
        self.emit(BrowserEvent::EntriesInserted { depth, insertions });
        // The full-column scan is the most expensive per-batch work after the merge.
        if let Some(focused) = selected {
            let positions = self.state.borrow().selected_positions(depth);
            let current = (request_id, positions.clone(), focused);
            let mut last = self.last_batch_selection.borrow_mut();
            if last.get(&depth) != Some(&current) {
                last.insert(depth, current);
                drop(last);
                self.emit(BrowserEvent::SelectionSetChanged {
                    depth,
                    positions,
                    focused,
                    take_focus: false,
                });
            }
        }
    }

    fn stage_batch(self: &Rc<Self>, request_id: RequestId, depth: usize, entries: Vec<FileEntry>) {
        let mut staging = self.staging.borrow_mut();
        let slot = staging.entry(depth).or_insert_with(|| StagingLoad {
            request_id,
            entries: Vec::new(),
            removed: HashSet::new(),
            deltas: Vec::new(),
            metadata_incomplete: false,
        });
        if slot.request_id != request_id {
            *slot = StagingLoad {
                request_id,
                entries,
                removed: HashSet::new(),
                deltas: Vec::new(),
                metadata_incomplete: false,
            };
            return;
        }
        if slot.entries.is_empty() {
            slot.entries = entries;
        } else {
            slot.entries.extend(entries);
        }
    }

    fn accumulate_batch(
        self: &Rc<Self>,
        request_id: RequestId,
        depth: usize,
        entries: Vec<FileEntry>,
    ) {
        let mut pending = self.coalesce_pending.borrow_mut();
        let slot = pending
            .entry(depth)
            .or_insert_with(|| (request_id, Vec::new()));
        if slot.0 != request_id {
            *slot = (request_id, Vec::new());
        }
        slot.1.extend(entries);
        let full = slot.1.len() >= COALESCE_ENTRIES;
        drop(pending);
        if full {
            self.flush_coalesced_capped(Some(depth));
        } else {
            self.arm_remote_flush_timer();
        }
    }

    fn flush_coalesced_capped(self: &Rc<Self>, depth: Option<usize>) {
        let depths: Vec<usize> = match depth {
            Some(depth) => vec![depth],
            None => self.coalesce_pending.borrow().keys().copied().collect(),
        };
        for &depth in &depths {
            self.drain_publish(depth);
            let chunk: Option<(RequestId, Vec<FileEntry>)> = self
                .coalesce_pending
                .borrow_mut()
                .get_mut(&depth)
                .and_then(|slot| {
                    if slot.1.is_empty() {
                        return None;
                    }
                    let take = slot.1.len().min(REMOTE_FLUSH_CAP);
                    let entries: Vec<FileEntry> = slot.1.drain(..take).collect();
                    Some((slot.0, entries))
                });
            if let Some((request_id, entries)) = chunk {
                self.apply_owned_batch(request_id, entries);
            }
        }
        self.coalesce_pending
            .borrow_mut()
            .retain(|_, (_, entries)| !entries.is_empty());
        if self.coalesce_pending.borrow().is_empty() {
            if let Some(source) = self.remote_flush_timer.borrow_mut().take() {
                source.remove();
            }
        } else {
            self.arm_remote_flush_timer();
        }
        for depth in depths {
            self.finish_remote_if_drained(depth);
        }
    }

    fn finish_remote_if_drained(self: &Rc<Self>, depth: usize) {
        if self.coalesce_pending.borrow().contains_key(&depth) {
            return;
        }
        let Some(terminal) = self.remote_terminals.borrow_mut().remove(&depth) else {
            return;
        };
        match terminal {
            RemoteTerminal::Finished {
                request_id,
                truncated,
                can_trash,
                can_delete,
            } => {
                let finished = self
                    .state
                    .borrow_mut()
                    .finish(request_id, truncated, can_trash, can_delete);
                if let Some(depth) = finished {
                    self.emit(BrowserEvent::LoadFinished { depth, truncated });
                    self.ensure_sorted_after_load(depth);
                }
            }
            RemoteTerminal::Failed {
                request_id,
                message,
            } => {
                let failed = self.state.borrow_mut().fail(request_id, message.clone());
                if let Some(depth) = failed {
                    self.emit(BrowserEvent::LoadFailed { depth, message });
                }
            }
        }
    }

    fn arm_remote_flush_timer(self: &Rc<Self>) {
        if self.remote_flush_timer.borrow().is_some() {
            return;
        }
        let weak: Weak<Self> = Rc::downgrade(self);
        let source = gio::glib::timeout_add_local_once(REMOTE_FLUSH_DELAY, move || {
            if let Some(browser) = weak.upgrade() {
                // Spent: disarm before flushing; a fired id refuses removal.
                browser.remote_flush_timer.borrow_mut().take();
                browser.flush_coalesced_capped(None);
            }
        });
        *self.remote_flush_timer.borrow_mut() = Some(source);
    }

    /// Sorts a staged snapshot off-thread, then installs, reconciles, and publishes
    /// it with the loading state up throughout: no provisional list is exposed.
    fn finish_staged_load(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
        truncated: bool,
        can_trash: Option<bool>,
        can_delete: Option<bool>,
    ) {
        let staging = self.staging.borrow_mut().remove(&depth);
        let Some(staging) = staging.filter(|staged| staged.request_id == request_id) else {
            return;
        };
        let preferences = self
            .state
            .borrow()
            .column_preferences(depth)
            .unwrap_or_else(|| self.preferences.get());
        let removed = staging.removed;
        let mut entries = staging.entries;
        entries.retain(|entry| !removed.contains(&entry.location));
        let retry_metadata = staging.metadata_incomplete
            && matches!(preferences.sort_key, SortKey::Size | SortKey::Modified);
        let ordering_preferences = if retry_metadata {
            ViewPreferences {
                sort_key: SortKey::Name,
                ..preferences
            }
        } else {
            preferences
        };
        let deltas = staging.deltas;
        self.sorting
            .borrow_mut()
            .insert(depth, SortingLoad { request_id, deltas });
        self.run_sort_task(
            depth,
            request_id,
            entries,
            SortPlan {
                ordering_preferences,
                staged_preferences: preferences,
                retry_metadata,
                truncated,
                can_trash,
                can_delete,
            },
        );
    }

    /// Small snapshots sort synchronously; large ones sort in a blocking worker
    /// with completion back on the main thread.
    fn run_sort_task(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
        entries: Vec<FileEntry>,
        plan: SortPlan,
    ) {
        if entries.len() <= SORT_INLINE_LIMIT {
            let sorted = sort_entries(entries, plan.ordering_preferences);
            self.finish_staged_sort(depth, request_id, sorted, plan);
            return;
        }
        let weak: Weak<Self> = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let sorted =
                gio::spawn_blocking(move || sort_entries(entries, plan.ordering_preferences)).await;
            let Some(browser) = weak.upgrade() else {
                return;
            };
            match sorted {
                Ok(sorted) => browser.finish_staged_sort(depth, request_id, sorted, plan),
                Err(_) => browser.fail_staged_sort(depth, request_id),
            }
        });
    }

    fn finish_staged_sort(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
        sorted: Vec<FileEntry>,
        plan: SortPlan,
    ) {
        let staged_preferences = plan.staged_preferences;
        let truncated = plan.truncated;
        let can_trash = plan.can_trash;
        let can_delete = plan.can_delete;
        let retry_metadata = plan.retry_metadata;
        let sorting = self.sorting.borrow_mut().remove(&depth);
        let Some(sorting) = sorting.filter(|sorting| sorting.request_id == request_id) else {
            return;
        };
        if self.state.borrow().request_id_for_depth(depth) != Some(request_id) {
            return;
        }
        if self
            .state
            .borrow_mut()
            .install_snapshot(request_id, sorted)
            .is_none()
        {
            return;
        }
        // Reconcile silently: the UI model is still empty, so delta events would
        // splice invalid positions; one staged publication carries the result.
        for (watched, change) in sorting.deltas {
            if matches!(change, DirectoryChange::Rescan) {
                continue;
            }
            let _applied = self
                .state
                .borrow_mut()
                .apply_directory_change(depth, &watched, change);
        }
        let current = self
            .state
            .borrow()
            .column_preferences(depth)
            .unwrap_or_else(|| self.preferences.get());
        if current != staged_preferences {
            // Resorted mid-load: re-sort with the current preferences; the loading
            // terminal fires exactly once, on whichever path finishes the load.
            if matches!(current.sort_key, SortKey::Size | SortKey::Modified)
                && self.state.borrow().column_unknown_metadata(depth).is_some()
            {
                self.state
                    .borrow_mut()
                    .finish(request_id, truncated, can_trash, can_delete);
                self.emit(BrowserEvent::LoadFinished { depth, truncated });
                self.ensure_sorted_after_load(depth);
            } else {
                self.resort_installed_column(
                    depth, request_id, current, truncated, can_trash, can_delete,
                );
            }
            return;
        }
        let focused = self
            .state
            .borrow()
            .columns
            .get(depth)
            .and_then(|column| column.selected);
        let positions = self.state.borrow().selected_positions(depth);
        let total = self
            .state
            .borrow()
            .columns
            .get(depth)
            .map(|column| column.entries.len())
            .unwrap_or(0);
        self.state
            .borrow_mut()
            .finish(request_id, truncated, can_trash, can_delete);
        self.publish_staged(
            depth,
            request_id,
            total,
            focused,
            positions,
            PublishTerminal::LoadFinished {
                truncated,
                retry_metadata,
            },
        );
    }

    fn resort_installed_column(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
        preferences: ViewPreferences,
        truncated: bool,
        can_trash: Option<bool>,
        can_delete: Option<bool>,
    ) {
        let Some(entries) = self
            .state
            .borrow()
            .columns
            .get(depth)
            .map(|column| column.entries.clone())
        else {
            return;
        };
        self.sorting.borrow_mut().insert(
            depth,
            SortingLoad {
                request_id,
                deltas: Vec::new(),
            },
        );
        self.run_sort_task(
            depth,
            request_id,
            entries,
            SortPlan {
                ordering_preferences: preferences,
                staged_preferences: preferences,
                retry_metadata: false,
                truncated,
                can_trash,
                can_delete,
            },
        );
    }

    /// Fails a staged load whose sort task died, so no spinner hangs.
    fn fail_staged_sort(self: &Rc<Self>, depth: usize, request_id: RequestId) {
        self.sorting.borrow_mut().remove(&depth);
        let mut state = self.state.borrow_mut();
        if state
            .fail(request_id, "Sorting the directory failed.".to_owned())
            .is_some()
        {
            drop(state);
            self.emit(BrowserEvent::LoadFailed {
                depth,
                message: "Sorting the directory failed.".to_owned(),
            });
        }
    }

    /// Publishes an installed column in stages: a synchronous prefix for fast first
    /// rows, tails from idle callbacks in a work budget, and deferred selection
    /// plus terminal on the final tail.
    fn publish_staged(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
        total: usize,
        focused: Option<usize>,
        positions: Vec<usize>,
        terminal: PublishTerminal,
    ) {
        self.drain_publish(depth);
        if total <= STAGE_INLINE_LIMIT {
            if self.state.borrow().columns.get(depth).is_none() {
                return;
            }
            self.emit(BrowserEvent::EntriesReplaced {
                depth,
                count: total,
            });
            if let Some(focused) = focused {
                self.emit(BrowserEvent::SelectionSetChanged {
                    depth,
                    positions,
                    focused,
                    take_focus: false,
                });
            }
            self.emit_publish_terminal(depth, terminal);
            return;
        }
        let published = self
            .state
            .borrow()
            .columns
            .get(depth)
            .map_or(0, |column| column.entries.len().min(FIRST_PUBLISH_COUNT));
        self.emit(BrowserEvent::EntriesReplaced {
            depth,
            count: published,
        });
        self.staged_publishes.borrow_mut().insert(
            depth,
            StagedPublish {
                request_id,
                published,
                total,
                focused,
                positions,
                terminal,
            },
        );
        self.arm_publish_timer();
    }

    fn emit_publish_terminal(self: &Rc<Self>, depth: usize, terminal: PublishTerminal) {
        match terminal {
            PublishTerminal::LoadFinished {
                truncated,
                retry_metadata,
            } => {
                self.emit(BrowserEvent::LoadFinished { depth, truncated });
                if retry_metadata {
                    self.ensure_sorted_after_load(depth);
                }
            }
            PublishTerminal::SortingFinished => self.emit(BrowserEvent::SortingFinished { depth }),
        }
    }

    /// Completes a staged publication synchronously before any mutation assuming a
    /// converged model.
    fn drain_publish(self: &Rc<Self>, depth: usize) {
        let staged = self.staged_publishes.borrow_mut().remove(&depth);
        let Some(staged) = staged else {
            return;
        };
        let remainder = self.state.borrow().columns.get(depth).map_or(0, |column| {
            column.entries.len().saturating_sub(staged.published)
        });
        if remainder > 0 {
            self.emit(BrowserEvent::EntriesPublished {
                depth,
                position: staged.published,
                count: remainder,
            });
        }
        if let Some(focused) = staged.focused {
            self.emit(BrowserEvent::SelectionSetChanged {
                depth,
                positions: staged.positions,
                focused,
                take_focus: false,
            });
        }
        self.emit_publish_terminal(depth, staged.terminal);
    }

    fn cancel_publish(&self, depth: usize) {
        self.staged_publishes.borrow_mut().remove(&depth);
        if self.staged_publishes.borrow().is_empty()
            && let Some(source) = self.publish_timer.borrow_mut().take()
        {
            source.remove();
        }
    }

    fn arm_publish_timer(self: &Rc<Self>) {
        if self.publish_timer.borrow().is_some() {
            return;
        }
        let weak: Weak<Self> = Rc::downgrade(self);
        // Run after GDK redraw (priority 120), but before default idle (200):
        // frames stay smooth without letting continuous frame work starve tails.
        let source = gio::glib::idle_add_local_full(glib::Priority::from(130), move || {
            if let Some(browser) = weak.upgrade() {
                browser.fire_publish_tails();
            }
            glib::ControlFlow::Break
        });
        *self.publish_timer.borrow_mut() = Some(source);
    }

    fn fire_publish_tails(self: &Rc<Self>) {
        self.publish_timer.borrow_mut().take();
        let started = std::time::Instant::now();
        loop {
            let depth = self.staged_publishes.borrow().keys().copied().next();
            let Some(depth) = depth else {
                return;
            };
            let current = self
                .staged_publishes
                .borrow()
                .get(&depth)
                .map(|staged| staged.request_id);
            if current.is_some_and(|id| self.state.borrow().request_id_for_depth(depth) != Some(id))
            {
                self.staged_publishes.borrow_mut().remove(&depth);
                continue;
            }
            if started.elapsed() >= PUBLISH_SLICE_BUDGET {
                self.arm_publish_timer();
                return;
            }
            let chunk: Option<(usize, usize)> = self
                .staged_publishes
                .borrow()
                .get(&depth)
                .and_then(|staged| {
                    self.state.borrow().columns.get(depth).map(|column| {
                        let end = (staged.published + PUBLISH_TAIL_CHUNK)
                            .min(column.entries.len())
                            .min(staged.total);
                        (staged.published, end.saturating_sub(staged.published))
                    })
                });
            let Some((position, chunk)) = chunk else {
                self.staged_publishes.borrow_mut().remove(&depth);
                continue;
            };
            if chunk == 0 {
                let staged = self.staged_publishes.borrow_mut().remove(&depth);
                let Some(staged) = staged else {
                    continue;
                };
                if let Some(focused) = staged.focused {
                    self.emit(BrowserEvent::SelectionSetChanged {
                        depth,
                        positions: staged.positions,
                        focused,
                        take_focus: false,
                    });
                }
                self.emit_publish_terminal(depth, staged.terminal);
                continue;
            }
            self.emit(BrowserEvent::EntriesPublished {
                depth,
                position,
                count: chunk,
            });
            if let Some(staged) = self.staged_publishes.borrow_mut().get_mut(&depth) {
                staged.published += chunk;
            }
        }
    }
    pub fn request_metadata_fill(
        self: &Rc<Self>,
        depth: usize,
        position: usize,
        location: Location,
    ) {
        // Defer to the provider instead of rejecting remote locations owner-side:
        // unsupported sources answer `Unsupported`.
        if !self.source.supports_metadata_fill(&location) {
            return;
        }
        {
            let mut pending = self.metadata_pending.borrow_mut();
            let queued = pending.entry(depth).or_default();
            if queued.len() < MAX_PENDING_FILL_LOCATIONS
                && !queued.iter().any(|target| target.location == location)
            {
                queued.push(ViewportTarget { position, location });
            }
        }
        if let Some(source) = self.metadata_timer.borrow_mut().take() {
            source.remove();
        }
        let weak: Weak<Self> = Rc::downgrade(self);
        let source = gio::glib::timeout_add_local_once(METADATA_FILL_DEBOUNCE, move || {
            if let Some(browser) = weak.upgrade() {
                browser.flush_metadata_fills();
            }
        });
        *self.metadata_timer.borrow_mut() = Some(source);
    }

    fn request_sort_fill(
        self: &Rc<Self>,
        depth: usize,
        generation: u64,
        preferences: ViewPreferences,
        targets: Vec<(usize, Location)>,
    ) {
        let Some(directory_request) = self.state.borrow().request_id_for_depth(depth) else {
            self.pending_sort.set(None);
            self.emit(BrowserEvent::SortingFinished { depth });
            return;
        };
        let fill_request = self.new_request_id();
        self.sort_awaiting_fill.borrow_mut().replace(SortFill {
            generation,
            depth,
            fill_request,
            directory_request,
            preferences,
        });
        let weak: Weak<Self> = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_event(event);
            }
        });
        let handle = self.source.fill_metadata(
            MetadataRequest {
                id: fill_request,
                entries: targets.into_iter().map(|(_, location)| location).collect(),
                full: true,
                time_budget: DIRECTORY_LOAD_TIME_BUDGET,
            },
            emit,
        );
        if self
            .sort_awaiting_fill
            .borrow()
            .as_ref()
            .is_some_and(|fill| fill.fill_request == fill_request)
        {
            self.sort_loads.borrow_mut().insert(depth, handle);
        }
    }

    fn finish_awaited_sort(
        self: &Rc<Self>,
        depth: usize,
        generation: u64,
        preferences: ViewPreferences,
    ) {
        self.sort_awaiting_fill.borrow_mut().take();
        self.sort_loads.borrow_mut().remove(&depth);
        let outcome = {
            let mut state = self.state.borrow_mut();
            if self.pending_sort.get() != Some((generation, depth)) {
                return;
            }
            let outcome = state.apply_sort_preferences(depth, preferences);
            self.preferences.set(preferences);
            self.pending_sort.set(None);
            outcome.map(|(focused, positions)| {
                let request_id = state.request_id_for_depth(depth);
                let total = state.columns.get(depth).map(|column| column.entries.len());
                (request_id, total, focused, positions)
            })
        };
        self.notify_preferences_observers();
        match outcome {
            Some((Some(request_id), Some(total), focused, positions)) => {
                self.publish_staged(
                    depth,
                    request_id,
                    total,
                    focused,
                    positions,
                    PublishTerminal::SortingFinished,
                );
            }
            _ => {
                self.emit(BrowserEvent::SortingFinished { depth });
            }
        }
    }
    /// Only `Complete` sorts: a partial pass is never published as correct, and
    /// unfilled rows keep placeholders for their next bind.
    fn handle_metadata_finished(self: &Rc<Self>, request_id: RequestId, outcome: MetadataOutcome) {
        let awaiting = *self.sort_awaiting_fill.borrow();
        if let Some(awaiting) = awaiting
            && awaiting.fill_request == request_id
        {
            self.sort_loads.borrow_mut().remove(&awaiting.depth);
            if outcome == MetadataOutcome::Complete
                && self.pending_sort.get() == Some((awaiting.generation, awaiting.depth))
            {
                self.finish_awaited_sort(awaiting.depth, awaiting.generation, awaiting.preferences);
            } else {
                self.abandon_awaited_sort(awaiting.depth, awaiting.generation, outcome);
            }
            return;
        }
        // Only a fill's own id releases its handle; terminals from superseded fills
        // cannot affect a sort or a newer request.
        if let Some(fill) = self.fill_tokens.borrow_mut().remove(&request_id) {
            self.metadata_loads.borrow_mut().remove(&fill.depth);
        }
    }

    /// Every `SortingStarted` still pairs with exactly one `SortingFinished`.
    fn abandon_awaited_sort(&self, depth: usize, generation: u64, outcome: MetadataOutcome) {
        self.sort_awaiting_fill.borrow_mut().take();
        self.sort_loads.borrow_mut().remove(&depth);
        if self.pending_sort.get() != Some((generation, depth)) {
            return;
        }
        self.pending_sort.set(None);
        tracing::warn!(
            depth,
            generation,
            ?outcome,
            "metadata sort abandoned; prior order preserved"
        );
        self.emit(BrowserEvent::SortingFinished { depth });
    }
    fn cancel_pending_sort_for(&self, depth: usize) {
        let awaiting = *self.sort_awaiting_fill.borrow();
        if let Some(awaiting) = awaiting
            && awaiting.depth == depth
        {
            self.abandon_awaited_sort(depth, awaiting.generation, MetadataOutcome::Cancelled);
            return;
        }
        self.sort_loads.borrow_mut().remove(&depth);
        if self
            .pending_sort
            .get()
            .is_some_and(|(_, pending_depth)| pending_depth == depth)
        {
            self.pending_sort.set(None);
            self.emit(BrowserEvent::SortingFinished { depth });
        }
    }

    fn truncate_deferred_from(self: &Rc<Self>, len: usize) {
        if let Some(source) = self.metadata_timer.borrow_mut().take() {
            source.remove();
        }
        self.metadata_pending
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        if !self.metadata_pending.borrow().is_empty() {
            let weak: Weak<Self> = Rc::downgrade(self);
            let source = gio::glib::timeout_add_local_once(METADATA_FILL_DEBOUNCE, move || {
                if let Some(browser) = weak.upgrade() {
                    browser.flush_metadata_fills();
                }
            });
            *self.metadata_timer.borrow_mut() = Some(source);
        }
        self.metadata_loads
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        let state = self.state.borrow();
        self.fill_tokens.borrow_mut().retain(|_, fill| {
            fill.depth < len
                && state.request_id_for_depth(fill.depth) == Some(fill.directory_request)
        });
        let awaiting = *self.sort_awaiting_fill.borrow();
        if let Some(awaiting) = awaiting
            && awaiting.depth >= len
        {
            self.abandon_awaited_sort(
                awaiting.depth,
                awaiting.generation,
                MetadataOutcome::Cancelled,
            );
        } else {
            self.sort_loads.borrow_mut().retain(|depth, _| *depth < len);
        }
        self.coalesce_pending
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        self.remote_terminals
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        self.last_batch_selection
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        self.staging.borrow_mut().retain(|depth, _| *depth < len);
        self.sorting.borrow_mut().retain(|depth, _| *depth < len);
        self.staged_publishes
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        if self.staged_publishes.borrow().is_empty()
            && let Some(source) = self.publish_timer.borrow_mut().take()
        {
            source.remove();
        }
    }

    fn ensure_sorted_after_load(self: &Rc<Self>, depth: usize) {
        let (needs, preferences) = {
            let state = self.state.borrow();
            let Some(preferences) = state.column_preferences(depth) else {
                return;
            };
            let needs = matches!(preferences.sort_key, SortKey::Size | SortKey::Modified)
                && state.column_unknown_metadata(depth).is_some();
            (needs, preferences)
        };
        if !needs {
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
        let targets = self
            .state
            .borrow()
            .column_unknown_metadata(depth)
            .unwrap_or_default();
        self.request_sort_fill(depth, generation, preferences, targets);
    }

    fn flush_metadata_fills(self: &Rc<Self>) {
        self.metadata_timer.borrow_mut().take();
        let pending: Vec<(usize, Vec<ViewportTarget>)> =
            self.metadata_pending.borrow_mut().drain().collect();
        for (depth, targets) in pending {
            let Some(directory_request) = self.state.borrow().request_id_for_depth(depth) else {
                continue;
            };
            let fill_request = self.new_request_id();
            let weak: Weak<Self> = Rc::downgrade(self);
            let emit = Rc::new(move |event| {
                if let Some(browser) = weak.upgrade() {
                    browser.handle_directory_event(event);
                }
            });
            let tokens: Vec<(usize, Location)> = targets
                .iter()
                .map(|target| (target.position, target.location.clone()))
                .collect();
            // Stored before the provider runs: synchronous fills answer inside the call.
            self.fill_tokens
                .borrow_mut()
                .retain(|_, fill| fill.depth != depth);
            self.metadata_loads.borrow_mut().remove(&depth);
            self.fill_tokens.borrow_mut().insert(
                fill_request,
                ViewportFill {
                    depth,
                    directory_request,
                    tokens,
                },
            );
            let handle = self.source.fill_metadata(
                MetadataRequest {
                    id: fill_request,
                    entries: targets.into_iter().map(|target| target.location).collect(),
                    full: false,
                    time_budget: METADATA_FILL_TIME_BUDGET,
                },
                emit,
            );
            if self.fill_tokens.borrow().contains_key(&fill_request) {
                self.metadata_loads.borrow_mut().insert(depth, handle);
            }
        }
    }

    /// Drops everything a discarded load queued. Coalesced rows are safe to drop
    /// because every site that clears loads replaces the data source wholesale;
    /// dropping a sort's fill handle aborts provider work without a terminal event.
    fn cancel_deferred_work(&self) {
        if let Some(source) = self.metadata_timer.borrow_mut().take() {
            source.remove();
        }
        self.metadata_pending.borrow_mut().clear();
        self.metadata_loads.borrow_mut().clear();
        self.fill_tokens.borrow_mut().clear();
        let awaiting = self.sort_awaiting_fill.borrow_mut().take();
        if let Some(awaiting) = awaiting {
            self.abandon_awaited_sort(
                awaiting.depth,
                awaiting.generation,
                MetadataOutcome::Cancelled,
            );
        } else {
            self.sort_loads.borrow_mut().clear();
            if let Some((_, depth)) = self.pending_sort.take() {
                self.emit(BrowserEvent::SortingFinished { depth });
            }
        }
        self.coalesce_pending.borrow_mut().clear();
        self.remote_terminals.borrow_mut().clear();
        self.last_batch_selection.borrow_mut().clear();
        self.staging.borrow_mut().clear();
        self.sorting.borrow_mut().clear();
        self.staged_publishes.borrow_mut().clear();
        if let Some(source) = self.publish_timer.borrow_mut().take() {
            source.remove();
        }
        if let Some(source) = self.remote_flush_timer.borrow_mut().take() {
            source.remove();
        }
    }

    fn request_directory(
        self: &Rc<Self>,
        depth: usize,
        location: Location,
        request_id: RequestId,
    ) -> LoadHandle {
        let weak: Weak<Self> = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_event(event);
            }
        });
        let batch_size = if location.native_path().is_some() {
            NATIVE_DIRECTORY_BATCH_SIZE
        } else {
            REMOTE_DIRECTORY_BATCH_SIZE
        };
        // Size/date loads stat inline: sorting placeholders and re-sorting afterwards
        // costs more than one stat per file up front.
        let sort_key = self
            .state
            .borrow()
            .column_preferences(depth)
            .map(|preferences| preferences.sort_key)
            .unwrap_or_else(|| self.preferences.get().sort_key);
        let include_metadata = matches!(sort_key, SortKey::Size | SortKey::Modified);
        self.source.enumerate(
            DirectoryRequest {
                id: request_id,
                location,
                batch_size,
                include_metadata,
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
        let handle = self.request_directory(depth, location, request_id);
        if let Some(load) = self.loads.borrow_mut().get_mut(depth) {
            *load = handle;
        }
        self.metadata_loads.borrow_mut().remove(&depth);
        self.metadata_pending.borrow_mut().remove(&depth);
        self.coalesce_pending.borrow_mut().remove(&depth);
        self.remote_terminals.borrow_mut().remove(&depth);
        self.last_batch_selection.borrow_mut().remove(&depth);
        self.cancel_pending_sort_for(depth);
        self.staging.borrow_mut().remove(&depth);
        self.sorting.borrow_mut().remove(&depth);
        self.cancel_publish(depth);
        self.fill_tokens.borrow_mut().retain(|_, fill| {
            self.state.borrow().request_id_for_depth(fill.depth) == Some(fill.directory_request)
        });
    }

    pub fn reload_active(self: &Rc<Self>) {
        if let Some(depth) = self.active_depth() {
            self.refresh_column(depth);
        }
    }

    pub fn refresh_all(self: &Rc<Self>) {
        let depths: Vec<usize> = {
            let state = self.state.borrow();
            (0..state.columns.len()).collect()
        };
        if depths.is_empty() {
            self.reload_active();
            return;
        }
        for depth in depths {
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

    fn load_target(&self, request_id: RequestId) -> Option<(usize, bool)> {
        let state = self.state.borrow();
        let depth = state.depth_for_request(request_id)?;
        let native = state.location_at(depth)?.native_path().is_some();
        Some((depth, native))
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
        if let Some(staging) = self.staging.borrow_mut().get_mut(&depth) {
            match &change {
                DirectoryChange::Remove(location) => {
                    staging.removed.insert(location.clone());
                }
                DirectoryChange::Upsert(entry) => {
                    staging.removed.remove(&entry.location);
                }
                DirectoryChange::Move { from, entry } => {
                    staging.removed.insert(from.clone());
                    staging.removed.remove(&entry.location);
                }
                DirectoryChange::Rescan => {}
            }
            staging.deltas.push((watched.clone(), change));
            return;
        }
        if let Some(sorting) = self.sorting.borrow_mut().get_mut(&depth) {
            sorting.deltas.push((watched.clone(), change));
            return;
        }
        // A staged publication converges the model first: deltas splice positions
        // that only exist past the tails.
        self.drain_publish(depth);
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

    fn handle_directory_event(self: &Rc<Self>, event: DirectoryEvent) {
        match event {
            DirectoryEvent::Batch {
                request_id,
                entries,
            } => {
                let target = self.load_target(request_id);
                let open = self.state.borrow().open_load_depth(request_id);
                match (target, open) {
                    (Some((depth, true)), Some(_)) => {
                        self.stage_batch(request_id, depth, entries);
                    }
                    (Some((depth, false)), Some(_)) => {
                        let entry_count = self
                            .state
                            .borrow()
                            .loading_column(request_id)
                            .map(|(_, count)| count)
                            .unwrap_or(0);
                        if entry_count == 0 {
                            self.apply_owned_batch(request_id, entries);
                        } else {
                            self.accumulate_batch(request_id, depth, entries);
                        }
                    }
                    _ => {
                        let peek_entries: Vec<_> = if self.preferences.get().show_hidden {
                            entries
                        } else {
                            entries
                                .into_iter()
                                .filter(|entry| !entry.is_hidden)
                                .collect()
                        };
                        let mut state = self.state.borrow_mut();
                        if state.apply_peek_batch(request_id, &peek_entries) {
                            drop(state);
                            self.emit(BrowserEvent::PeekEntriesAdded {
                                entries: peek_entries,
                            });
                        }
                    }
                }
            }

            DirectoryEvent::Finished {
                request_id,
                truncated,
                can_trash,
                can_delete,
            } => {
                // Bound to a variable first: an if-let scrutinee borrow would stay live
                // across the flush and panic inside it.
                let target = self.load_target(request_id);
                let open = self.state.borrow().open_load_depth(request_id);
                match (target, open) {
                    (Some((depth, true)), Some(_)) => {
                        self.stage_batch(request_id, depth, Vec::new());
                        self.finish_staged_load(
                            depth, request_id, truncated, can_trash, can_delete,
                        );
                    }
                    (Some((depth, _)), Some(_)) => {
                        self.remote_terminals.borrow_mut().insert(
                            depth,
                            RemoteTerminal::Finished {
                                request_id,
                                truncated,
                                can_trash,
                                can_delete,
                            },
                        );
                        self.flush_coalesced_capped(Some(depth));
                    }
                    _ => {
                        let mut state = self.state.borrow_mut();
                        if state.finish_peek(request_id) {
                            drop(state);
                            self.emit(BrowserEvent::PeekFinished);
                        }
                    }
                }
            }
            DirectoryEvent::MetadataIncomplete { request_id } => {
                let target = self.load_target(request_id);
                let open = self.state.borrow().open_load_depth(request_id);
                if let Some((depth, true)) = target.filter(|_| open.is_some()) {
                    self.stage_batch(request_id, depth, Vec::new());
                    if let Some(staging) = self.staging.borrow_mut().get_mut(&depth) {
                        staging.metadata_incomplete = true;
                    }
                }
            }
            DirectoryEvent::Failed {
                request_id,
                message,
            } => {
                let target = self.load_target(request_id);
                let open = self.state.borrow().open_load_depth(request_id);
                if let Some((depth, true)) = target.filter(|_| open.is_some()) {
                    self.staging.borrow_mut().remove(&depth);
                    self.sorting.borrow_mut().remove(&depth);
                    self.cancel_publish(depth);
                } else if let Some((depth, false)) = target.filter(|_| open.is_some()) {
                    self.remote_terminals.borrow_mut().insert(
                        depth,
                        RemoteTerminal::Failed {
                            request_id,
                            message,
                        },
                    );
                    self.flush_coalesced_capped(Some(depth));
                    return;
                }
                let mut state = self.state.borrow_mut();
                if let Some(depth) = state.fail(request_id, message.clone()) {
                    drop(state);
                    self.emit(BrowserEvent::LoadFailed { depth, message });
                } else if state.fail_peek(request_id, message.clone()) {
                    drop(state);
                    self.emit(BrowserEvent::PeekFailed { message });
                }
            }
            DirectoryEvent::MetadataFilled {
                request_id,
                updates,
            } => {
                // Full sort fills apply by location: positional tokens would only add
                // validation churn to an already O(n log n) path.
                let awaiting_sort = self
                    .sort_awaiting_fill
                    .borrow()
                    .as_ref()
                    .copied()
                    .filter(|fill| fill.fill_request == request_id);
                if let Some(awaiting) = awaiting_sort {
                    let mut state = self.state.borrow_mut();
                    if let Some((depth, positions)) =
                        state.apply_metadata(awaiting.directory_request, updates)
                    {
                        let filled = filled_entries(&state, depth, &positions);
                        tracing::debug!(
                            request_id = request_id.0,
                            depth,
                            filled = positions.len(),
                            "metadata fill applied"
                        );
                        drop(state);
                        self.emit(BrowserEvent::MetadataFilled {
                            depth,
                            updates: filled,
                        });
                    }
                    return;
                }
                let fill = self
                    .fill_tokens
                    .borrow()
                    .get(&request_id)
                    .map(|fill| (fill.directory_request, fill.tokens.clone()));
                let Some((directory_request, tokens)) = fill else {
                    return;
                };
                let token_positions: HashMap<&Location, usize> = tokens
                    .iter()
                    .map(|(position, location)| (location, *position))
                    .collect();
                let mut positioned = Vec::with_capacity(updates.len());
                for update in &updates {
                    if let Some(position) = token_positions.get(&update.location) {
                        positioned.push((*position, update.clone()));
                    }
                }
                let mut state = self.state.borrow_mut();
                if let Some((depth, positions, stale)) =
                    state.apply_positioned_metadata(directory_request, positioned)
                {
                    let filled = filled_entries(&state, depth, &positions);
                    tracing::debug!(
                        request_id = request_id.0,
                        depth,
                        filled = positions.len(),
                        stale = stale.len(),
                        "metadata fill applied"
                    );
                    drop(state);
                    if !filled.is_empty() {
                        self.emit(BrowserEvent::MetadataFilled {
                            depth,
                            updates: filled,
                        });
                    }
                }
            }
            DirectoryEvent::MetadataFinished {
                request_id,
                outcome,
            } => {
                self.handle_metadata_finished(request_id, outcome);
            }
        }
    }

    fn emit(&self, event: BrowserEvent) {
        let observers = self.observers.borrow().clone();
        for observer in &observers {
            observer(&event);
        }
    }

    fn new_request_id(&self) -> RequestId {
        let id = self.next_request.get();
        self.next_request.set(id.saturating_add(1));
        RequestId(id)
    }
}

fn filled_entries(
    state: &NavigationState,
    depth: usize,
    positions: &[usize],
) -> Vec<(usize, FileEntry)> {
    positions
        .iter()
        .filter_map(|position| {
            let entry = state.columns.get(depth)?.entries.get(*position)?.clone();
            Some((*position, entry))
        })
        .collect()
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
    location_from_input_with_home(input, &glib::home_dir())
}

fn location_from_input_with_home(
    input: &str,
    home: &Path,
) -> Result<Location, LocationValidationError> {
    if input == "~" {
        return Ok(Location::local(home));
    }
    if let Some(relative) = input.strip_prefix("~/") {
        return Ok(Location::local(home.join(relative.trim_start_matches('/'))));
    }
    if input.starts_with('~') {
        return Err(LocationValidationError::UnsupportedShorthand(
            "Only ~ and ~/ paths are supported for the current user's home directory.".to_owned(),
        ));
    }
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
