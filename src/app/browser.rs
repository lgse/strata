// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::Duration,
};

#[cfg(test)]
use crate::services::{
    ArchiveFormat, CreateDirectoryRequest, CreateFileRequest, DeleteRequest, DirectoryRequest,
    DirectoryStamp, MoveRecord, NameMode, OperationEvent, PasteItem, PasteRequest,
    PlaceEntryRequest, RenameRecord, RenameRequest, RestoreRequest, RestoreSource,
    RestoreTrashItem, TransferConflict, UndoMergeRequest, UndoMoveItem, UndoRenameRequest,
    validate_basename,
};
use crate::{
    app::navigation::{EntryInsertion, EntrySplice, NavigationState},
    model::{FileEntry, Location, SortDirection, SortKey, ViewPreferences},
    services::{
        DirectoryChange, FileSource, ItemIdentity, LoadHandle, LocationValidationError,
        OperationProvider, OperationRequestId, PasswordFailure, RequestId, TransferReport,
    },
};

pub use crate::app::navigation::{ColumnEntryCounts, CursorToggle, VisualKind, VisualRange};

mod deferred;
mod directory_changes;
mod folder_sorts;
mod freshness;
mod leftovers;
mod loading;
mod navigation_lifecycle;
mod operation_events;
mod operation_updates;
mod publication;
mod remote;
mod sorting;

mod activation;
mod columns;
mod entries;
mod file_operations;
mod listing;
mod location_input;
mod metadata_fills;
mod navigate;
mod operation_start;
mod refresh;
mod replay;
mod selection;
mod sort_settings;
mod undo;

use folder_sorts::SortOrigin;
pub use freshness::{FreshnessInput, Presence, VisibleDepths};
#[cfg(test)]
use location_input::location_from_input_with_home;
pub(crate) use operation_start::{folder_destination_texts, trash_destination_texts};
use publication::{PublicationPlan, PublishTerminal, StagedPublish};
#[cfg(test)]
use refresh::deletion_parent_location;
use remote::RemoteState;
#[cfg(test)]
use undo::{
    MAX_UNDO_HISTORY, PENDING_UNDO, UndoState, claim_pending_undo, claim_replay, finish_undo,
    peek_replay, pending_redo_entry, pending_undo_entry, retain_replay_merge_items,
    retain_replay_move_items, undo_move_parents,
};
use undo::{
    MergeUndoState, UndoEntry, completed_replay_items, finish_replay, mark_replay_item_completed,
    move_records, moved_target, push_background_undo, push_pending_redo, push_pending_undo,
    push_regenerated_undo, record_replaced_originals, retain_merge_records,
};

/// Caps a normal directory load. Memory sets the ceiling, not time: a listed entry costs
/// about 0.8 KiB of resident memory (its `FileEntry` plus one GTK string object), so a
/// 250,000-entry List view settles in under a second at roughly 490 MiB RSS in release.
const MAX_DIRECTORY_ENTRIES: usize = 250_000;
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

#[derive(Clone)]
enum LoadSelection {
    Nothing,
    FirstEntry,
    Target(Vec<Location>),
    /// Selected if it is listed, otherwise the first visible entry; never revealed.
    Prefer(Location),
}

fn load_selection(select_first: bool) -> LoadSelection {
    if select_first {
        LoadSelection::FirstEntry
    } else {
        LoadSelection::Nothing
    }
}

fn select_on_load(state: &mut NavigationState, depth: usize, selection: LoadSelection) {
    match selection {
        LoadSelection::FirstEntry => state.select_first_on_load(depth),
        LoadSelection::Target(targets) => state.select_locations_on_load(depth, targets),
        LoadSelection::Prefer(location) => state.prefer_location_on_load(depth, location),
        LoadSelection::Nothing => {}
    }
}

#[derive(Clone, Debug)]
pub struct BrowserColumnSnapshot {
    pub location: Location,
    pub count: usize,
    pub selected_positions: Vec<usize>,
    pub loading: bool,
    pub error: Option<String>,
    /// Offered beside `error` when only permissions kept the user out.
    pub administrator: Option<Location>,
    pub truncated: bool,
    pub reveal_pending: bool,
    pub not_responding: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionUpdate {
    All,
    Positions(Vec<usize>),
}

#[derive(Clone, Debug)]
pub enum BrowserEvent {
    /// The outgoing directory is still available for presentation-state capture.
    /// `history` marks a return to a recorded path: Back, Forward, Up, or a path an
    /// external change or undo restores. Other routes navigate explicitly.
    NavigationStarting {
        history: bool,
    },
    /// The column at `depth` is about to be cleared for a reload; its entries,
    /// selection and cursor are still readable.
    ColumnReloading {
        depth: usize,
    },
    Reset,
    ColumnsTruncated {
        len: usize,
        /// A sibling's `ColumnAdded` at `len` follows immediately.
        replacing: bool,
    },
    ColumnAdded {
        depth: usize,
        location: Location,
    },
    /// Existing directories moved; retain the unaffected views and current focus.
    ColumnsRelocated {
        from_depth: usize,
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
    },
    /// Refreshed entries for already-rendered rows; the order never changes here.
    MetadataFilled {
        depth: usize,
        updates: Vec<(usize, FileEntry)>,
    },
    /// Re-admit viewport metadata without replacing rows or clearing their last displayed count.
    FolderCountsInvalidated,
    ColumnReloaded {
        depth: usize,
    },
    ColumnRefreshing {
        depth: usize,
    },
    ColumnNotResponding {
        depth: usize,
        not_responding: bool,
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
        /// The folder's `admin://` form when only permissions kept the user out.
        administrator: Option<Location>,
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
        /// A deletion, not the user, moved focus here.
        triggered_by_removal: bool,
    },
    SelectionSetChanged {
        depth: usize,
        selection: SelectionUpdate,
        focused: usize,
        take_focus: bool,
    },
    /// Selection already applied by a view. Observers must not reapply it or move focus.
    SelectionSynced {
        depth: usize,
        focused: Option<usize>,
    },
    PreviewRequested {
        entry: FileEntry,
        automatic: bool,
    },
    ExtractRequested {
        entry: FileEntry,
    },
    OpenRequested {
        location: Location,
    },
    RenameCompleted {
        request_id: OperationRequestId,
    },
    RenameAbandoned {
        request_id: OperationRequestId,
    },
    EntryCreated {
        location: Location,
    },
    /// `None` means the request was rejected before an operation was allocated.
    RenameFailed {
        request_id: Option<OperationRequestId>,
        message: String,
    },
    BackgroundOperation {
        request_id: OperationRequestId,
        event: Box<BrowserEvent>,
    },
    TransferStarted {
        total: usize,
        moving: bool,
    },
    TransferProgress {
        completed_items: usize,
        completed_files: usize,
        total_files: Option<usize>,
        current_file: Option<String>,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
    },
    FlushingToDevice,
    TransferCancellationPending,
    TransferFinished {
        moved_locations: Vec<Location>,
    },
    /// Only after a clean completion; a stop or errors carry the report in
    /// their outcome event.
    ItemsReported {
        report: TransferReport,
    },
    /// Only for remembered folders; the default sort is left unchanged.
    FolderSortChosen {
        location: Location,
        sort_key: SortKey,
        sort_direction: SortDirection,
        name_comparison: crate::model::NameComparison,
    },
    /// An item an operation of this browser renamed or moved, published as it
    /// lands, even when a later operation superseded that one. `merged` means
    /// `to` is an existing folder the item was merged into.
    ItemRelocated {
        from: Location,
        to: Location,
        merged: bool,
    },
    /// An item an operation of this browser removed from `location`.
    /// `trash_identity` identifies an item moved to Trash; others were deleted.
    ItemRemoved {
        location: Location,
        trash_identity: Option<ItemIdentity>,
    },
    /// An item an operation of this browser brought back from Trash to `location`.
    ItemRestored {
        location: Location,
        identity: Option<ItemIdentity>,
    },
    DeletionStarted {
        total: usize,
    },
    DeletionProgress {
        completed: usize,
        total: usize,
    },
    DeletionEntriesRemoved {
        removed: usize,
    },
    DeletionFinished {
        succeeded: bool,
    },
    RestorationStarted {
        total: usize,
    },
    RestorationProgress {
        completed: usize,
        total: usize,
    },
    RestorationFinished {
        succeeded: bool,
    },
    OperationFailed {
        message: String,
        password_failure: Option<PasswordFailure>,
        report: TransferReport,
    },
    OperationCompletedWithErrors {
        message: String,
        /// Entries a retry with `permanent: true` would likely delete
        /// successfully, e.g. ones that failed only because this location
        /// doesn't support Trash. Always empty for a restore failure.
        retryable_locations: Vec<Location>,
        has_non_retryable_failures: bool,
        report: TransferReport,
    },
    OperationCancelled {
        completed: usize,
        failed: usize,
        not_attempted: usize,
        affected_locations: HashSet<Location>,
        report: TransferReport,
    },
    OperationRefreshRequired {
        depths: Vec<usize>,
    },
    NavigationRejected {
        parent_depth: usize,
        location: Location,
        error: LocationValidationError,
    },
    LocationNavigationRejected {
        location: Location,
        error: LocationValidationError,
    },
    LocationRevealFailed {
        location: Location,
    },
    EmptyTrashRequested,
    LeftoverReviewRequested {
        depth: usize,
    },
    LeftoversChanged {
        depth: usize,
        announce: bool,
    },
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
    TransferReveal {
        destination: Location,
        locations: Vec<Location>,
    },
}

/// Events dispatch by reference: payloads move once into authoritative state,
/// observers borrow, and the observer list is cloned up front so reentrant
/// emission stays safe.
type Observer = Rc<dyn Fn(&BrowserEvent)>;
type PreferencesObserver = Rc<dyn Fn(ViewPreferences)>;
type DeferredDirectoryChanges = HashMap<usize, Vec<(Location, DirectoryChange)>>;

const MAX_INCREMENTAL_OPERATION_UPDATES: usize = 64;
const OPERATION_PUBLICATION_BATCH: usize = 512;

/// Bounds one metadata fill; partial results still apply, the rest retries on
/// its next bind.
const METADATA_FILL_TIME_BUDGET: Duration = Duration::from_secs(5);
/// Defensive cap per depth: the UI only ever asks for its visible window.
const MAX_PENDING_FILL_LOCATIONS: usize = 1024;
const MAX_VIEWPORT_FILL_BATCH: usize = 16;

/// Remote loads only (native loads stage instead): entries accumulate this far
/// before an early flush bounds first-result latency.
const COALESCE_ENTRIES: usize = 2048;
/// Bounds one remote progressive flush: a slow link must not turn one timer
/// fire into a multi-frame GTK mutation.
const REMOTE_FLUSH_CAP: usize = 512;
// Camera folders repeat filenames, creating many interleaved insertions rather
// than one cheap append. Keep each publication small enough for input/frame work.
const CAMERA_FLUSH_CAP: usize = 32;
/// Latency bound so later remote batches flush on the next idle/frame.
const REMOTE_FLUSH_DELAY: Duration = Duration::from_millis(50);

/// Last selection emitted per depth on the batch path, keyed by request so a
/// new load re-emits; lets background batches skip redundant refreshes.
type BatchSelectionState = HashMap<usize, (RequestId, Vec<usize>, usize)>;

/// One bound row's fill request: stable location plus its source position, so
/// fills apply after validating the row has not moved.
struct ViewportTarget {
    position: usize,
    location: Location,
    include_icon_details: bool,
}

/// Routing for one metadata request: own ids, validated against the owning
/// directory request.
struct ViewportFill {
    depth: usize,
    directory_request: RequestId,
    tokens: Vec<(usize, Location)>,
    include_icon_details: bool,
}

#[derive(Clone, Copy)]
struct SortFill {
    generation: u64,
    depth: usize,
    origin: SortOrigin,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MonitorState {
    #[default]
    Pending,
    Active,
    Unavailable,
}

#[derive(Clone, Copy)]
struct KeptListing {
    request_id: RequestId,
    truncated: bool,
    background: bool,
}

#[derive(Default)]
struct ColumnMonitor {
    requested: bool,
    state: Rc<Cell<MonitorState>>,
    watch: Option<LoadHandle>,
    gap_check: Rc<RefCell<Option<LoadHandle>>>,
    freshness: Rc<RefCell<freshness::ColumnFreshness>>,
}

pub struct Browser {
    source: Rc<dyn FileSource>,
    state: RefCell<NavigationState>,
    loads: RefCell<Vec<LoadHandle>>,
    monitors: RefCell<Vec<ColumnMonitor>>,
    /// The depth-0 folder's identity when it loaded, which finds it after a rename.
    root_identity: RefCell<Option<(Location, ItemIdentity)>>,
    /// The current step of the one departure check that runs at a time.
    departure_probe: RefCell<Option<directory_changes::DepartureProbe>>,
    freshness: freshness::Freshness,
    metadata_pending: RefCell<HashMap<usize, Vec<ViewportTarget>>>,
    folder_counts_after_load: RefCell<HashMap<usize, RequestId>>,
    metadata_idle: RefCell<Option<gio::glib::SourceId>>,
    staging: RefCell<HashMap<usize, StagingLoad>>,
    sorting: RefCell<HashMap<usize, SortingLoad>>,
    staged_publishes: RefCell<HashMap<usize, StagedPublish>>,
    preserving_refreshes: RefCell<HashSet<usize>>,
    kept_listings: RefCell<HashMap<usize, KeptListing>>,
    publish_timer: RefCell<Option<gio::glib::SourceId>>,
    remote: RefCell<RemoteState>,
    metadata_loads: RefCell<HashMap<usize, LoadHandle>>,
    fill_tokens: RefCell<HashMap<RequestId, ViewportFill>>,
    /// Full-column sort fills, kept apart from viewport fills so a viewport
    /// metadata dispatch can never overwrite or cancel an active full sort.
    sort_loads: RefCell<HashMap<usize, LoadHandle>>,
    sort_awaiting_fill: RefCell<Option<SortFill>>,
    last_batch_selection: RefCell<BatchSelectionState>,
    peek_load: RefCell<Option<LoadHandle>>,
    validation_load: RefCell<Option<LoadHandle>>,
    deferred_reveal: RefCell<Option<(Location, Vec<Location>)>>,
    preview_revealed_location: RefCell<Option<Location>>,
    validation_generation: Cell<u64>,
    navigation_lifecycle: navigation_lifecycle::NavigationLifecycle,
    navigation_cleanup: RefCell<Option<Box<dyn FnOnce()>>>,
    operation_provider: RefCell<Option<Rc<dyn OperationProvider>>>,
    operation_load: RefCell<Option<LoadHandle>>,
    background_operations:
        RefCell<HashMap<OperationRequestId, Rc<operation_events::BackgroundOperation>>>,
    operation_backgroundable: Cell<bool>,
    operation_cancel_requested: Cell<bool>,
    operation_description: RefCell<String>,
    /// Accessible description of the operation's destination.
    operation_destination_description: RefCell<String>,
    /// Compact visible form of the destination, read only while a description is set.
    operation_destination_label: RefCell<String>,
    transfer_cancel_pending: Cell<bool>,
    current_operation: Cell<Option<OperationRequestId>>,
    last_started_operation: Cell<Option<OperationRequestId>>,
    rename_operation: Cell<Option<OperationRequestId>>,
    transfer_operation: Cell<Option<bool>>,
    deletion_operation: Cell<bool>,
    deletion_permanent: Cell<bool>,
    deferred_file_operation_changes: RefCell<DeferredDirectoryChanges>,
    operation_rescan_depths: RefCell<HashSet<usize>>,
    restoration_operation: Cell<bool>,
    recovery_operation: Cell<bool>,
    archive_operation: Cell<bool>,
    transfer_destination: RefCell<Option<Location>>,
    /// Whether a completed transfer should navigate to/reveal its destination.
    /// A drop onto a folder row moves or copies the file without leaving the
    /// source listing; a paste or explicit "move to" still shows where it landed.
    transfer_reveal: Cell<bool>,
    created_locations: RefCell<Vec<Location>>,
    merged_undo: RefCell<MergeUndoState>,
    /// Where each moved item landed; Keep Both and fixed names can differ
    /// from the source name.
    moved_items: RefCell<HashMap<Location, Location>>,
    transfer_report: RefCell<TransferReport>,
    undo_claim: RefCell<Option<(u64, UndoEntry)>>,
    redo_claim: RefCell<Option<(u64, UndoEntry)>>,
    next_request: Cell<u64>,
    pending_sort: Cell<Option<(u64, usize)>>,
    requested_sort: RefCell<Option<(metadata_fills::SortPass, Location)>>,
    /// A debounced sort stays pending after it reorders its column, until a newer
    /// sort or a cancel replaces it; this tells that apart from one not yet applied.
    pending_sort_applied: Cell<bool>,
    sort_generation: Cell<u64>,
    preferences: Cell<ViewPreferences>,
    chooser_mode: Cell<bool>,
    suppress_child_mirror: Cell<bool>,
    external_change_focus: Cell<bool>,
    background_load_focus: Cell<bool>,
    observers: RefCell<Vec<Observer>>,
    preferences_observers: RefCell<Vec<PreferencesObserver>>,
    sort_resync: folder_sorts::SortResync,
    _operations_ended: crate::services::file_activity::Subscription,
    self_weak: Weak<Browser>,
}

impl Browser {
    #[cfg(test)]
    pub fn new(source: Rc<dyn FileSource>) -> Rc<Self> {
        Self::with_preferences(source, ViewPreferences::default())
    }

    pub fn with_preferences(source: Rc<dyn FileSource>, preferences: ViewPreferences) -> Rc<Self> {
        let browser = Rc::new_cyclic(|self_weak| Self {
            source,
            state: RefCell::new(NavigationState::with_preferences(preferences)),
            loads: RefCell::new(Vec::new()),
            monitors: RefCell::new(Vec::new()),
            root_identity: RefCell::new(None),
            departure_probe: RefCell::new(None),
            freshness: freshness::Freshness::default(),
            metadata_pending: RefCell::new(HashMap::new()),
            folder_counts_after_load: RefCell::new(HashMap::new()),
            metadata_idle: RefCell::new(None),
            staging: RefCell::new(HashMap::new()),
            sorting: RefCell::new(HashMap::new()),
            staged_publishes: RefCell::new(HashMap::new()),
            preserving_refreshes: RefCell::new(HashSet::new()),
            kept_listings: RefCell::new(HashMap::new()),
            publish_timer: RefCell::new(None),
            remote: RefCell::new(RemoteState::new()),
            metadata_loads: RefCell::new(HashMap::new()),
            fill_tokens: RefCell::new(HashMap::new()),
            sort_loads: RefCell::new(HashMap::new()),
            sort_awaiting_fill: RefCell::new(None),
            last_batch_selection: RefCell::new(HashMap::new()),
            peek_load: RefCell::new(None),
            validation_load: RefCell::new(None),
            deferred_reveal: RefCell::new(None),
            preview_revealed_location: RefCell::new(None),
            validation_generation: Cell::new(0),
            navigation_lifecycle: navigation_lifecycle::NavigationLifecycle::default(),
            navigation_cleanup: RefCell::new(None),
            operation_provider: RefCell::new(None),
            operation_load: RefCell::new(None),
            background_operations: RefCell::new(HashMap::new()),
            operation_backgroundable: Cell::new(false),
            operation_cancel_requested: Cell::new(false),
            operation_description: RefCell::new(String::new()),
            operation_destination_description: RefCell::new(String::new()),
            operation_destination_label: RefCell::new(String::new()),
            transfer_cancel_pending: Cell::new(false),
            current_operation: Cell::new(None),
            last_started_operation: Cell::new(None),
            rename_operation: Cell::new(None),
            transfer_operation: Cell::new(None),
            deletion_operation: Cell::new(false),
            deletion_permanent: Cell::new(false),
            deferred_file_operation_changes: RefCell::new(HashMap::new()),
            operation_rescan_depths: RefCell::new(HashSet::new()),
            restoration_operation: Cell::new(false),
            recovery_operation: Cell::new(false),
            archive_operation: Cell::new(false),
            transfer_destination: RefCell::new(None),
            transfer_reveal: Cell::new(true),
            created_locations: RefCell::new(Vec::new()),
            merged_undo: RefCell::new(MergeUndoState::default()),
            moved_items: RefCell::new(HashMap::new()),
            transfer_report: RefCell::new(TransferReport::default()),
            undo_claim: RefCell::new(None),
            redo_claim: RefCell::new(None),
            next_request: Cell::new(1),
            pending_sort: Cell::new(None),
            requested_sort: RefCell::new(None),
            pending_sort_applied: Cell::new(false),
            sort_generation: Cell::new(0),
            preferences: Cell::new(preferences),
            chooser_mode: Cell::new(false),
            suppress_child_mirror: Cell::new(false),
            external_change_focus: Cell::new(false),
            background_load_focus: Cell::new(false),
            observers: RefCell::new(Vec::new()),
            preferences_observers: RefCell::new(Vec::new()),
            sort_resync: folder_sorts::SortResync::default(),
            _operations_ended: Self::follow_ended_operations(self_weak.clone()),
            self_weak: self_weak.clone(),
        });
        browser.follow_lanes();
        browser
    }

    pub fn observe(&self, observer: impl Fn(&BrowserEvent) + 'static) {
        self.observers.borrow_mut().push(Rc::new(observer));
    }

    #[cfg(test)]
    pub fn observer_count(&self) -> usize {
        self.observers.borrow().len()
    }

    pub fn set_chooser_mode(&self, chooser: bool) {
        self.chooser_mode.set(chooser);
    }

    pub fn is_chooser_mode(&self) -> bool {
        self.chooser_mode.get()
    }

    pub fn clear_observer(&self) {
        self.observers.borrow_mut().clear();
        self.clear_navigation_observers();
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

    pub fn active_location(&self) -> Option<Location> {
        self.state.borrow().active_location()
    }

    pub(crate) fn navigation_generation(&self) -> u64 {
        self.validation_generation.get()
    }

    /// Invalidates work whose result is guarded by the navigation generation.
    pub(crate) fn bump_navigation_generation(&self) -> u64 {
        let _update = self.navigation_update();
        let generation = self.validation_generation.get().saturating_add(1);
        self.validation_generation.set(generation);
        self.navigation_lifecycle.pending.set(None);
        let load = self.validation_load.take();
        let cleanup = self.navigation_cleanup.take();
        drop(load);
        if let Some(cleanup) = cleanup {
            cleanup();
        }
        generation
    }

    pub(crate) fn set_navigation_cleanup(&self, cleanup: impl FnOnce() + 'static) {
        if let Some(previous) = self.navigation_cleanup.replace(Some(Box::new(cleanup))) {
            previous();
        }
    }

    pub(crate) fn finish_navigation_cleanup(&self) {
        self.navigation_cleanup.take();
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

    pub fn allows_entry(&self, entry: &FileEntry) -> bool {
        self.source.allows_entry(entry)
    }

    pub(crate) fn confined_to(&self) -> Option<PathBuf> {
        self.source.confined_to().map(Path::to_path_buf)
    }

    /// Synchronizes widget focus without changing selection or reopening a directory.
    pub fn set_active_column(&self, depth: usize) {
        self.state.borrow_mut().focus_column(depth);
    }

    #[cfg(test)]
    pub fn select_first_on_load(&self, depth: usize) {
        self.state.borrow_mut().select_first_on_load(depth);
    }

    pub fn selection_anchor_position(&self, depth: usize) -> Option<usize> {
        self.state.borrow().selection_anchor_position(depth)
    }

    pub fn set_selection_anchor(&self, depth: usize, position: usize) {
        self.state
            .borrow_mut()
            .set_selection_anchor(depth, position);
    }

    pub fn focus_active(&self) {
        let focus = self.state.borrow().active_focus();
        if let Some((depth, position)) = focus {
            self.emit_suppressed_focus(depth, position);
        }
    }

    fn emit(&self, event: BrowserEvent) {
        self.invalidate_loaded_folder_counts(&event);
        let observers = self.observers.borrow().clone();
        for observer in &observers {
            observer(&event);
        }
        self.follow_leftovers(&event);
        self.follow_preview_reveals(&event);
    }

    fn follow_preview_reveals(&self, event: &BrowserEvent) {
        match event {
            BrowserEvent::SelectionSetChanged {
                depth,
                focused,
                take_focus: true,
                ..
            } => {
                let target = self.preview_revealed_location.borrow().clone();
                if let Some(target) = target
                    && let Some(entry) = self
                        .entry_at(*depth, *focused)
                        .filter(|e| e.location == target)
                {
                    self.preview_revealed_location.take();
                    self.request_preview(entry);
                }
            }
            BrowserEvent::LocationRevealFailed { location } => {
                let matches_target = self
                    .preview_revealed_location
                    .borrow()
                    .as_ref()
                    .is_some_and(|target| target == location);
                if matches_target {
                    self.preview_revealed_location.take();
                }
            }
            _ => {}
        }
    }

    fn new_request_id(&self) -> RequestId {
        let id = self.next_request.get();
        self.next_request.set(id.saturating_add(1));
        RequestId(id)
    }
}

#[cfg(test)]
mod tests;
