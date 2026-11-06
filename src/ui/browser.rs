// SPDX-License-Identifier: MIT

//! Browser composition and public commands. Feature modules share this view's state and the
//! application controller; they must not create independent navigation or operation state.

use crate::app::{Browser, FreshnessInput, Presence, VisibleDepths};
use crate::model::{FileEntry, Location};
use crate::services::{FileSource, LoadHandle, LocationValidationError, OperationProvider};
use crate::ui::browser::clipboard::{copy_locations, register_cut_view};
use crate::ui::browser::collection::cancel_source;
pub(super) use crate::ui::browser::columns::COLUMN_WIDTH;
use crate::ui::browser::columns::ColumnView;
use crate::ui::browser::desktop::selected_terminal_location;
use crate::ui::browser::inline_edit::{PendingEntryRename, PendingRename};
use crate::ui::browser::location::{
    MountCredentials, PendingConnect, UnlockProgressSlot, is_breadcrumb_button_target,
};
use crate::ui::browser::paths::{can_pin_entry, is_trash_location};
use crate::ui::browser::peek::{PeekAnchor, PeekView};
use crate::ui::browser::progress::{BackgroundProgress, FileProgressState};
use crate::ui::browser::transfer::FinishedSendToCompletion;
use crate::ui::browser::transfer::PendingSendToCompletion;
use crate::ui::browser::transfer::duplicate_transfer;
use crate::ui::browser::trash::TrashLoadingView;
use crate::ui::browser_modes::{BrowserDensity, BrowserMode, ClickActivation, ModeViews};
use crate::ui::keys;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

const SIZE_NOTICE_DURATION: Duration = Duration::from_millis(1200);

pub(in crate::ui) type PlaybackHandoff = Rc<dyn Fn(&Location) -> Option<Duration>>;

mod archive;
pub(super) mod camera_scroll;
mod clipboard;
mod collection;
mod columns;
mod content_drop;
pub(super) mod context_menu;
mod customization;
pub(super) use customization::show_customize_modal;
mod desktop;
mod destination;
mod dissolve_delete;
mod entry;
mod entry_animation;
mod events;
mod file_commands;
pub(in crate::ui) mod find;
pub(super) mod fly_to_trash;
mod inline_edit;
mod leftovers;
mod listing_filter;
mod listing_search;
mod location;
mod pane_header;
pub(in crate::ui) mod paths;
mod peek;
mod preferences;
mod presentation;
mod preview;
mod progress;
mod properties;
mod result_selection;
mod tab_location;
mod tab_stops;
mod transfer;
mod transfer_report;
mod trash;
mod view_edit;
mod view_filter;
mod view_modes;
mod view_navigation;
mod view_selection;
mod view_setup;
mod view_state;

#[cfg(test)]
pub(super) use crate::ui::browser::clipboard::clipboard_mark;
pub(in crate::ui) use crate::ui::browser::clipboard::drag_preview_icon;
#[cfg(feature = "qa")]
pub(crate) use crate::ui::browser::clipboard::formats::read_mime_bytes;
pub(super) use crate::ui::browser::clipboard::{
    ClipboardMark, ClipboardMarks, file_drag_content, mark_in, set_mark_result_style,
};
pub(crate) use crate::ui::browser::clipboard::{
    PreparedFileDrop, active_drag_snapshot, active_file_drag_source, arm_spring_load_navigation,
    clear_active_drag, drag_actions_for_modifiers, file_drag_hover_target, file_drop_action,
    locations_from_file_list_value, prepare_file_drop_target,
};
pub(crate) use crate::ui::browser::collection::{
    ActivePaneFilter, bind_listing_filter, detach_collection_view, filter_placeholder,
    focus_collection_item_when_allocated, focus_filter_entry, notify_filter_query,
    prepare_collection_inline_edit, refocus_filter_entry, restore_filter_controls,
    reveal_collection_after_layout, scroll_collection_when_allocated, search_result_entry,
};
pub(in crate::ui) use crate::ui::browser::collection::{FilterQueryBinding, bind_filter_query};
pub(crate) use crate::ui::browser::content_drop::{
    DropHandler, DropRequest, attach_drop_target, commit_dropped_value,
};
pub(super) use crate::ui::browser::context_menu::{
    ContextMenuTarget, ContextMenuTrigger, ContextResolver, install_folder_context_menu,
    install_item_context_menu, install_resolved_item_context_menu,
};
pub(super) use crate::ui::browser::desktop::{launch_terminal, open_location_at};
#[cfg(test)]
pub(in crate::ui) use crate::ui::browser::entry::catppuccin;
pub(super) use crate::ui::browser::entry::{
    entry_filter, entry_icon, entry_model_value, format_file_size, icon_for_name,
    metadata_needs_fill, model_type_group,
};
pub(crate) use crate::ui::browser::file_commands::{
    ConflictFocus, CreateRefusal, PinChange, TargetCommand, Yank, can_rename,
};
pub(super) use crate::ui::browser::inline_edit::{queue_rename, reveal_rename_row};
pub(in crate::ui) use crate::ui::browser::listing_filter::{
    FilterFocus, FilterStatus, results_step_target, scroll_results_to, selected_cursor,
};
pub(in crate::ui) use crate::ui::browser::location::ConnectOutcome;
pub(super) use crate::ui::browser::pane_header::{
    attach_back_history, column_sort_direction_toggle, column_sort_menu, empty_trash_button,
    leftovers_button, list_sort_menu, lists_back_history, pane_new_folder_button,
    pane_refresh_button, sync_column_sort_direction, sync_list_sort_menu,
};
pub(super) use crate::ui::browser::paths::is_trash_root;
pub use crate::ui::browser::peek::PeekBehavior;
pub(super) use crate::ui::browser::presentation::{administrator_notice, not_responding_notice};
pub use crate::ui::browser::properties::format_permissions;
pub(super) use crate::ui::modal::{
    animate_in, animate_out, dismiss_modal_layer, modal_layer, show_error_dialog, slide_out,
};

// Receives whether the pin was saved, once the bookmark snapshot is published.
type PinCompletion = Box<dyn FnOnce(bool)>;
type PinHandler = Rc<dyn Fn(Location, String, PinCompletion)>;
type UnpinHandler = Rc<dyn Fn(&Location)>;
type PinStatusHandler = Rc<dyn Fn(&Location) -> PinStatus>;
type PreparingTransfer = (
    transfer::Preparing,
    gio::Cancellable,
    Weak<FileProgressState>,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PinStatus {
    Available,
    Pinned,
    Unavailable,
}

#[derive(Default)]
struct GlobalActivityState {
    next_id: u64,
    active: Vec<(u64, String)>,
}

impl GlobalActivityState {
    fn begin(&mut self, label: impl Into<String>) -> u64 {
        self.next_id = self.next_id.saturating_add(1);
        self.active.push((self.next_id, label.into()));
        self.next_id
    }

    fn finish(&mut self, id: u64) {
        self.active.retain(|(active_id, _)| *active_id != id);
    }

    fn current_label(&self) -> Option<&str> {
        self.active.last().map(|(_, label)| label.as_str())
    }
}

pub struct GlobalActivity {
    state: std::rc::Weak<ViewState>,
    id: u64,
}

impl Drop for GlobalActivity {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            state.finish_global_activity(self.id);
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct SendToMenuTestOverride {
    pub(super) destinations: Vec<crate::ui::RemovableDestination>,
    pub(super) recent_destinations: std::collections::HashMap<String, Vec<std::path::PathBuf>>,
    pub(super) handlers: SendToMenuHandlers,
}

#[derive(Clone)]
pub(super) struct SendToMenuHandlers {
    pub(super) activate_root: Rc<dyn Fn(String, Vec<Location>)>,
    pub(super) activate_recent: Rc<dyn Fn(String, std::path::PathBuf, Vec<Location>)>,
    pub(super) choose_folder: Rc<dyn Fn(String, Vec<Location>)>,
}

pub(super) struct ViewState {
    overlay: gtk::Overlay,
    location_control: gtk::Box,
    location_stack: gtk::Stack,
    global_activity_spinner: gtk::Spinner,
    global_activity: RefCell<GlobalActivityState>,
    breadcrumbs: gtk::Box,
    breadcrumb_reveal_pending: Rc<Cell<bool>>,
    location_entry: gtk::Entry,
    path_completion: Rc<location::completion::PathCompletion>,
    columns_widget: gtk::Box,
    scroller: gtk::ScrolledWindow,
    mode_views: RefCell<ModeViews>,
    mode: Cell<BrowserMode>,
    columns: RefCell<Vec<ColumnView>>,
    hovered_column: Cell<Option<usize>>,
    // The preview drawer holds the keys, so no column is the keyboard destination.
    preview_owns_keys: Cell<bool>,
    playback_handoff: RefCell<Option<PlaybackHandoff>>,
    context_menu_column: Cell<Option<usize>>,
    context_menu_generation: Cell<u64>,
    context_menu_focus: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    input_ownership: RefCell<super::input_ownership::InputOwnership>,
    /// True only while a column row gesture is writing the selection model.
    /// Focus echoes of the cursor are not pointer-owned.
    pointer_owns_selection: Cell<bool>,
    folder_press: RefCell<Option<columns::FolderPress>>,
    /// The depth whose column a sibling swap just removed, so its replacement fades in.
    swap_slot: Cell<Option<usize>>,
    tab_location: RefCell<tab_location::TabLocation>,
    column_resizing: Cell<bool>,
    /// Visible browser area published by the preview layout; bounds Miller column widths.
    column_width_limit: Cell<Option<i32>>,
    /// Set until the first limit, and after a text-size change, so the next
    /// published limit narrows wider columns once.
    clamp_to_next_column_limit: Cell<bool>,
    horizontal_scroll_generation: Rc<Cell<u64>>,
    suppress_focus_scroll: Cell<bool>,
    /// Set while a footer prompt moves the cursor; the prompt keeps the keys.
    cursor_keeps_focus: Cell<bool>,
    tab_crossing: Cell<tab_stops::TabCrossing>,
    pending_mirror: RefCell<Option<glib::SourceId>>,
    source_generation: Rc<Cell<u64>>,
    peek: RefCell<Option<PeekView>>,
    pending_peek: RefCell<Option<glib::SourceId>>,
    pending_close: RefCell<Option<glib::SourceId>>,
    peek_anchor: RefCell<Option<PeekAnchor>>,
    peek_behavior: PeekBehavior,
    peek_enabled: Cell<bool>,
    single_click_previews: Cell<bool>,
    columns_mirror_selection: Cell<bool>,
    multiple_selection: Rc<Cell<bool>>,
    interactive: bool,
    chooser_allows_create: Cell<bool>,
    columns_click_activation: Cell<ClickActivation>,
    active_rename: crate::ui::collection_edit::ActiveEdits,
    pending_rename: RefCell<Option<PendingRename>>,
    /// A footer rename to a path: its operation and what replaces the item's row.
    typed_placement: RefCell<Option<(crate::services::OperationRequestId, Location)>>,
    rename_generation: Cell<u64>,
    rename_reveal_generation: Cell<u64>,
    keyboard_refocus: Cell<Option<file_commands::KeyboardRefocus>>,
    typed_rename: RefCell<Option<file_commands::TypedRenameFollow>>,
    pending_click_rename: RefCell<Option<glib::SourceId>>,
    click_rename_generation: Cell<u64>,
    pending_new_entry: RefCell<Option<Rc<PendingEntryRename>>>,
    progress_state: RefCell<Rc<FileProgressState>>,
    background_file_progress:
        RefCell<HashMap<crate::services::OperationRequestId, Rc<BackgroundProgress>>>,
    pending_send_to_completion: RefCell<Option<PendingSendToCompletion>>,
    finished_send_to_completion: RefCell<Option<FinishedSendToCompletion>>,

    pin_handler: RefCell<Option<PinHandler>>,
    unpin_handler: RefCell<Option<UnpinHandler>>,
    pin_status_handler: RefCell<Option<PinStatusHandler>>,
    search_selection_handlers: RefCell<Vec<Rc<dyn Fn()>>>,
    pending_select: RefCell<Vec<String>>,
    pending_properties: RefCell<Option<Location>>,
    pending_location_selection: RefCell<Option<(Location, Vec<Location>)>>,
    pending_extract_retry: RefCell<Option<(FileEntry, Location)>>,
    extract_destination: RefCell<Option<Location>>,
    pending_archive_destination: RefCell<Option<Location>>,
    /// The entries a just-dispatched, non-permanent delete requested,
    /// snapshotted so a `CompletedWithErrors` response naming entries that
    /// failed only because the location doesn't support Trash can offer a
    /// permanent-delete retry for exactly those entries.
    pending_delete_entries: RefCell<Vec<FileEntry>>,
    pending_file_operation_animation: RefCell<Option<fly_to_trash::PreparedFlight>>,
    /// Visible permanent-delete rows captured before the operation mutates the model.
    pending_delete_dissolve: RefCell<Option<(usize, dissolve_delete::PreparedDissolve)>>,
    delete_dissolve_request: Cell<Option<crate::services::OperationRequestId>>,
    deferred_delete_empty_depth: Cell<Option<usize>>,
    pending_navigate: RefCell<Option<Location>>,
    pending_location_credentials: RefCell<Option<MountCredentials>>,
    pending_connect: RefCell<Option<Rc<PendingConnect>>>,
    pending_trash_lookup: RefCell<Option<LoadHandle>>,
    leftovers: leftovers::LeftoverState,
    pending_empty_trash: RefCell<Option<LoadHandle>>,
    trash_loading: RefCell<Option<TrashLoadingView>>,
    unlock_slots: RefCell<Vec<UnlockProgressSlot>>,
    trash_button: glib::WeakRef<gtk::Button>,
    recent_removal: crate::adapters::RecentRemovalState,
    drag_autoscroll: RefCell<Option<Rc<columns::drag_scroll::DragAutoscroll>>>,
    drag_source_depth: Cell<Option<usize>>,
    suppress_scroll_after_drop: Cell<bool>,
    transfer_replaces_cursor: Cell<bool>,
    transfer_preparation: RefCell<Option<Rc<FileProgressState>>>,
    preparing_transfers: RefCell<Vec<PreparingTransfer>>,
    drop_active_depths: Cell<Option<(usize, usize)>>,
    find: RefCell<find::FindState>,
    listing_filter: listing_filter::FilterState,
    listing_search: listing_search::SearchState,
    result_selection: result_selection::ResultSelection,
    #[cfg(test)]
    send_to_menu_test_override: RefCell<Option<SendToMenuTestOverride>>,
    browser: Rc<Browser>,
}

fn focus_header_action(actions: &gtk::Box, direction: gtk::DirectionType) -> bool {
    let mut action = if direction == gtk::DirectionType::Left {
        actions.last_child()
    } else {
        actions.first_child()
    };
    while let Some(candidate) = action {
        action = if direction == gtk::DirectionType::Left {
            candidate.prev_sibling()
        } else {
            candidate.next_sibling()
        };
        if candidate.is_visible() && candidate.grab_focus() {
            return true;
        }
    }
    false
}

#[derive(Clone)]
pub struct BrowserView {
    state: Rc<ViewState>,
}

#[derive(Clone)]
pub(crate) struct WeakBrowserView(Weak<ViewState>);

pub(in crate::ui) fn claim_keyboard_navigation(state: &Rc<ViewState>) {
    BrowserView {
        state: state.clone(),
    }
    .keyboard_navigation();
}

impl WeakBrowserView {
    pub(crate) fn upgrade(&self) -> Option<BrowserView> {
        self.0.upgrade().map(|state| BrowserView { state })
    }
}

impl BrowserView {
    pub fn new(source: Rc<dyn FileSource>, peek_behavior: PeekBehavior) -> Self {
        Self::with_options(source, peek_behavior, true, true)
    }

    pub(super) fn new_chooser(source: Rc<dyn FileSource>, multiple: bool) -> Self {
        Self::with_options(source, PeekBehavior::default(), multiple, false)
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.overlay.clone().upcast()
    }

    pub fn navigate_location(&self, location: Location) {
        self.state.browser.navigate_location(location, true);
    }

    pub(crate) fn overlay(&self) -> gtk::Overlay {
        self.state.overlay.clone()
    }

    pub(crate) fn set_navigation_cleanup(&self, cleanup: impl FnOnce() + 'static) {
        self.state.browser.set_navigation_cleanup(cleanup);
    }

    pub(crate) fn finish_navigation_cleanup(&self) {
        self.state.browser.finish_navigation_cleanup();
    }

    #[cfg(test)]
    pub(crate) fn connect_navigation_cleanup(&self, window: &gtk::Window) {
        let weak = self.downgrade();
        window.connect_close_request(move |_| {
            if let Some(browser) = weak.upgrade() {
                browser.browser().bump_navigation_generation();
            }
            glib::Propagation::Proceed
        });
    }

    pub(crate) fn downgrade(&self) -> WeakBrowserView {
        WeakBrowserView(Rc::downgrade(&self.state))
    }

    pub(super) fn can_trash_file_drop(sources: &[Location]) -> bool {
        !sources.is_empty()
            && sources.iter().all(|source| {
                source.parent().is_some()
                    && !paths::is_trash_location(source)
                    && !source.is_administrator()
                    // A dropped web address is a link, never something to trash or reach.
                    && !source
                        .uri_value()
                        .is_some_and(crate::services::has_web_scheme)
            })
    }

    pub(super) fn trash_file_drop(&self, sources: Vec<Location>) -> bool {
        if !Self::can_trash_file_drop(&sources) {
            return false;
        }
        self.state.trash_dropped_locations(sources);
        true
    }

    pub(crate) fn commit_drop(&self, destination: Location, request: DropRequest) {
        self.state.commit_drop(destination, request);
    }

    pub(crate) fn reveal_location(&self, location: Location) {
        if let Some(parent) = location.parent() {
            self.state.reveal_locations(parent, vec![location], false);
        }
    }

    pub(crate) fn reveal_locations(
        &self,
        directory: Location,
        targets: Vec<Location>,
        properties: bool,
    ) {
        self.state.reveal_locations(directory, targets, properties);
    }

    pub(super) fn refresh_source_filter(&self) {
        if let Some(last) = self.state.browser.active_depth() {
            for depth in 0..=last {
                self.state.browser.retry_column(depth);
            }
        }
        let columns = self.state.columns.borrow().clone();
        let mut changed = false;
        for column in &columns {
            changed |= columns::refresh_source_filter(column, &self.state.browser);
        }
        if changed {
            self.state.notify_search_selection_changed();
        }
        self.state.mode_views.borrow().refresh_source_filter();
    }

    pub fn browser(&self) -> Rc<Browser> {
        self.state.browser.clone()
    }

    pub(super) fn set_pin_handlers(
        &self,
        handler: PinHandler,
        unpin_handler: UnpinHandler,
        status_handler: PinStatusHandler,
    ) {
        self.state.pin_handler.replace(Some(handler));
        self.state.unpin_handler.replace(Some(unpin_handler));
        self.state.pin_status_handler.replace(Some(status_handler));
    }

    pub fn set_operation_provider(&self, provider: Rc<dyn OperationProvider>) {
        self.state.browser.set_operation_provider(provider);
    }

    pub fn set_trash_button(&self, button: gtk::Button) {
        self.state.trash_button.set(Some(&button));
    }

    /// Lets a marquee drag begin on blank chrome beside the file panes — the sidebar or
    /// the preview pane — and run into whichever view the current mode shows. The pane
    /// nearest the `edge` the surface sits on is the target, since that is the one such
    /// a drag runs into.
    pub(super) fn add_marquee_origin(
        &self,
        surface: &impl IsA<gtk::Widget>,
        edge: gtk::PackType,
        click: super::marquee::OriginClick,
    ) {
        let weak_state = Rc::downgrade(&self.state);
        super::marquee::install_shared_origin_surface(surface, click, move |_, _, _, _| {
            let state = weak_state.upgrade()?;
            // Sidebar-origin events do not reach the browser's pointer controllers.
            state.hovered_column.set(None);
            state.pointer_navigation();
            let mode = state.mode.get();
            if mode == BrowserMode::Columns {
                let columns = state.columns.borrow();
                let column = match edge {
                    gtk::PackType::End => columns.last(),
                    _ => columns.first(),
                };
                return column.map(|column| column.marquee.clone());
            }
            state.mode_views.borrow().leading_marquee()
        });
    }
}

fn paste_destination(
    selected: &[FileEntry],
    column: Option<Location>,
    load_cursor: bool,
) -> Option<Location> {
    match selected {
        [folder] if folder.is_directory() && !load_cursor => Some(folder.location.clone()),
        _ => column,
    }
    .filter(|location| !is_trash_location(location) && !location.is_recent_location())
}

/// Keyboard-triggered folder creation must ignore the pointer so a resting mouse
/// cannot redirect the new folder into a pane the keyboard never visited.
fn new_folder_destination_depth(
    focused: Option<usize>,
    active: Option<usize>,
    pane_count: usize,
) -> Option<usize> {
    focused
        .filter(|depth| *depth < pane_count)
        .or_else(|| active.filter(|depth| *depth < pane_count))
        .or_else(|| pane_count.checked_sub(1))
}

fn single_pane_preview_reservation(width: i32) -> i32 {
    width.max(0) / 2
}

mod chooser_context;
