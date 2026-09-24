// SPDX-License-Identifier: MIT

//! Browser composition and public commands. Feature modules share this view's state and the
//! application controller; they must not create independent navigation or operation state.

use crate::app::Browser;
use crate::model::{FileEntry, Location};
use crate::services::{FileSource, LoadHandle, OperationProvider};
use crate::ui::browser::clipboard::{copy_locations, copy_names, register_cut_view};
use crate::ui::browser::collection::cancel_source;
pub(super) use crate::ui::browser::columns::COLUMN_WIDTH;
use crate::ui::browser::columns::ColumnView;
use crate::ui::browser::desktop::selected_terminal_location;
use crate::ui::browser::inline_edit::{PendingEntryRename, PendingRename};
use crate::ui::browser::location::{
    MountCredentials, UnlockProgressSlot, is_breadcrumb_button_target,
};
use crate::ui::browser::paths::{can_pin_entry, is_trash_location};
use crate::ui::browser::peek::{PeekAnchor, PeekView};
use crate::ui::browser::progress::FileProgressView;
use crate::ui::browser::transfer::duplicate_transfer;
use crate::ui::browser::trash::TrashLoadingView;
use crate::ui::browser_modes::{BrowserDensity, BrowserMode, ClickActivation, ModeViews};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

mod archive;
pub(super) mod camera_scroll;
mod clipboard;
mod collection;
mod columns;
pub(super) mod context_menu;
mod customization;
mod desktop;
mod destination;
mod dissolve_delete;
mod entry;
mod entry_animation;
mod events;
pub(in crate::ui) mod find_highlight;
pub(super) mod fly_to_trash;
mod inline_edit;
mod location;
mod pane_header;
pub(in crate::ui) mod paths;
mod peek;
mod preferences;
mod presentation;
mod preview;
mod progress;
mod properties;
mod transfer;
mod trash;

pub(in crate::ui) use crate::ui::browser::clipboard::drag_icon_with_count;
pub(super) use crate::ui::browser::clipboard::{
    ClipboardMarks, file_drag_content, set_cut_result_style,
};
pub(crate) use crate::ui::browser::clipboard::{
    PreparedFileDrop, drag_actions_for_modifiers, file_drop_action, file_drop_commit,
    locations_from_file_list_value, prepare_file_drop_target,
};
pub(crate) use crate::ui::browser::collection::{
    ActivePaneFilter, FILTER_DEBOUNCE_DELAY, debounce_filter_entry, detach_collection_view,
    filter_placeholder, focus_collection_item_when_allocated, focus_filter_entry,
    notify_filter_query, prepare_collection_inline_edit, restore_filter_controls,
    reveal_collection_after_layout, scroll_collection_when_allocated, search_result_entry,
    search_result_navigation_position, set_filter_entry_query,
};
pub(in crate::ui) use crate::ui::browser::collection::{FilterQueryBinding, bind_filter_query};
pub(super) use crate::ui::browser::context_menu::{
    ContextMenuTarget, ContextMenuTrigger, ContextResolver, install_folder_context_menu,
    install_item_context_menu, install_resolved_item_context_menu,
};
pub(super) use crate::ui::browser::desktop::{launch_terminal, open_location};
pub(super) use crate::ui::browser::entry::{
    FOLDER_TYPE_GROUP, OTHER_TYPE_GROUP, entry_filter, entry_icon, entry_model_value,
    format_file_size, icon_for_name, metadata_needs_fill, model_type_group, rounded_size_and_unit,
};
pub(super) use crate::ui::browser::inline_edit::{queue_rename, reveal_rename_row};
pub(super) use crate::ui::browser::pane_header::{
    bind_minimal_chrome, column_sort_direction_toggle, column_sort_menu, empty_trash_button,
    pane_new_folder_button, pane_refresh_button, sync_column_sort_direction,
};
pub(super) use crate::ui::browser::paths::is_trash_root;
pub use crate::ui::browser::peek::PeekBehavior;
pub use crate::ui::browser::properties::format_permissions;
pub(super) use crate::ui::modal::{
    animate_in, animate_out, dismiss_modal_layer, modal_layer, show_error_dialog, slide_out,
};

type PinHandler = Rc<dyn Fn(Location, String)>;
type UnpinHandler = Rc<dyn Fn(&Location)>;
type PinStatusHandler = Rc<dyn Fn(&Location) -> PinStatus>;
type PrintHandler = Rc<dyn Fn(FileEntry)>;

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

pub(super) type SearchPointerCursor = Rc<dyn Fn(u32)>;

pub(super) struct ViewState {
    overlay: gtk::Overlay,
    location_control: gtk::Box,
    location_stack: gtk::Stack,
    global_activity_spinner: gtk::Spinner,
    global_activity: RefCell<GlobalActivityState>,
    breadcrumbs: gtk::Box,
    breadcrumb_scroller: gtk::ScrolledWindow,
    location_entry: gtk::Entry,
    path_completion: Rc<location::completion::PathCompletion>,
    columns_widget: gtk::Box,
    scroller: gtk::ScrolledWindow,
    mode_views: RefCell<ModeViews>,
    mode: Cell<BrowserMode>,
    columns: RefCell<Vec<ColumnView>>,
    hovered_column: Cell<Option<usize>>,
    context_menu_column: Cell<Option<usize>>,
    context_menu_generation: Cell<u64>,
    context_menu_focus: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    input_ownership: RefCell<super::input_ownership::InputOwnership>,
    horizontal_scroll_generation: Rc<Cell<u64>>,
    suppress_focus_scroll: Cell<bool>,
    pending_mirror: RefCell<Option<glib::SourceId>>,
    source_generation: Rc<Cell<u64>>,
    refreshing_source_filter: Cell<bool>,
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
    columns_click_activation: Cell<ClickActivation>,
    active_rename: crate::ui::collection_edit::ActiveEdits,
    pending_rename: RefCell<Option<PendingRename>>,
    rename_generation: Cell<u64>,
    rename_reveal_generation: Cell<u64>,
    pending_click_rename: RefCell<Option<glib::SourceId>>,
    click_rename_generation: Cell<u64>,
    pending_new_entry: RefCell<Option<Rc<PendingEntryRename>>>,
    file_progress_view: RefCell<Option<FileProgressView>>,
    pending_file_progress: RefCell<Option<glib::SourceId>>,
    file_operation_progress: Cell<(usize, usize)>,
    transfer_progress: Cell<Option<(usize, u64, Option<u64>)>>,
    flushing_to_device: Cell<bool>,
    pin_handler: RefCell<Option<PinHandler>>,
    unpin_handler: RefCell<Option<UnpinHandler>>,
    pin_status_handler: RefCell<Option<PinStatusHandler>>,
    print_handler: RefCell<Option<PrintHandler>>,
    search_selection_handlers: RefCell<Vec<Rc<dyn Fn()>>>,
    pending_select: RefCell<Vec<String>>,
    pending_location_selection: RefCell<Option<(Location, Vec<Location>)>>,
    /// Set when the pending selection came from a properties request, so the
    /// dialog opens once the entry it describes is actually loaded.
    pending_select_properties: Cell<bool>,
    pending_extract_retry: RefCell<Option<(FileEntry, Location)>>,
    pending_archive_destination: RefCell<Option<Location>>,
    /// The entries a just-dispatched, non-permanent delete requested,
    /// snapshotted so a `CompletedWithErrors` response naming entries that
    /// failed only because the location doesn't support Trash can offer a
    /// permanent-delete retry for exactly those entries.
    pending_delete_entries: RefCell<Vec<FileEntry>>,
    /// Visible permanent-delete rows captured before the operation mutates the model.
    pending_delete_dissolve: RefCell<Option<(usize, dissolve_delete::PreparedDissolve)>>,
    deferred_delete_empty_depth: Cell<Option<usize>>,
    pending_navigate: RefCell<Option<Location>>,
    pending_location_credentials: RefCell<Option<MountCredentials>>,
    pending_trash_lookup: RefCell<Option<LoadHandle>>,
    pending_empty_trash: RefCell<Option<LoadHandle>>,
    trash_loading: RefCell<Option<TrashLoadingView>>,
    unlock_slots: RefCell<Vec<UnlockProgressSlot>>,
    auto_refresh: RefCell<Option<glib::SourceId>>,
    trash_button: RefCell<Option<gtk::Button>>,
    visible_listing_observers: RefCell<Vec<Rc<dyn Fn()>>>,
    find_highlight_query: Rc<RefCell<String>>,
    find_prompt_active: Cell<bool>,
    open_with_request: RefCell<desktop::OpenWithRequest>,
    find_highlights_on: Cell<bool>,
    notifying_visible_listing: Cell<bool>,
    drag_autoscroll: RefCell<Option<Rc<columns::drag_scroll::DragAutoscroll>>>,
    suppress_scroll_after_drop: Cell<bool>,
    /// When set, miller `.active-column` header chrome is withheld so an
    /// owned preview pane is the only focused header.
    suppress_column_header_focus: Cell<bool>,
    /// Minimal mode reads this when a pointer press focuses a search hit.
    search_pointer_cursor: RefCell<Option<SearchPointerCursor>>,
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

impl ViewState {
    pub(super) fn note_search_pointer_cursor(&self, index: u32) {
        let handler = self.search_pointer_cursor.borrow().clone();
        if let Some(handler) = handler {
            handler(index);
        }
    }
}

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

    fn with_options(
        source: Rc<dyn FileSource>,
        peek_behavior: PeekBehavior,
        multiple: bool,
        interactive: bool,
    ) -> Self {
        let columns_widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        columns_widget.add_css_class("columns");
        columns_widget.set_halign(gtk::Align::Start);
        columns_widget.set_vexpand(true);

        let scroller = gtk::ScrolledWindow::builder()
            .child(&columns_widget)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();
        scroller.add_css_class("mode-scroll");
        scroller.add_css_class("columns-scroll");
        if let Some(viewport) = scroller.child().and_downcast::<gtk::Viewport>() {
            viewport.set_scroll_to_focus(false);
        }
        let overlay = gtk::Overlay::new();

        let location_entry = gtk::Entry::builder()
            .hexpand(true)
            .width_chars(36)
            .placeholder_text("Enter a path or URI…")
            .tooltip_text(super::accessibility::LOCATION_LABEL)
            .build();
        location_entry.add_css_class("location-entry");
        let confirm_location = gtk::Button::builder()
            .tooltip_text(super::accessibility::LOCATION_CONFIRM_LABEL)
            .build();
        confirm_location.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::CHECK,
            16,
        )));
        confirm_location.add_css_class("location-action");
        let cancel_location = gtk::Button::builder()
            .tooltip_text(super::accessibility::LOCATION_CANCEL_LABEL)
            .build();
        cancel_location.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            16,
        )));
        cancel_location.add_css_class("location-action");
        super::accessibility::describe_location_controls(
            &location_entry,
            &confirm_location,
            &cancel_location,
        );
        let entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        entry_row.append(&location_entry);
        entry_row.append(&confirm_location);
        entry_row.append(&cancel_location);
        let entry_control = gtk::Box::new(gtk::Orientation::Vertical, 0);
        entry_control.set_hexpand(true);
        entry_control.append(&entry_row);

        let breadcrumbs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        breadcrumbs.add_css_class("breadcrumbs");
        let breadcrumb_scroller = gtk::ScrolledWindow::builder()
            .child(&breadcrumbs)
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .build();
        breadcrumb_scroller.add_css_class("breadcrumb-scroller");

        let fade_left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fade_left.add_css_class("breadcrumb-fade");
        fade_left.add_css_class("breadcrumb-fade-left");
        fade_left.set_halign(gtk::Align::Start);
        fade_left.set_valign(gtk::Align::Fill);
        fade_left.set_can_target(false);
        fade_left.set_opacity(0.0);

        let fade_right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fade_right.add_css_class("breadcrumb-fade");
        fade_right.add_css_class("breadcrumb-fade-right");
        fade_right.set_halign(gtk::Align::End);
        fade_right.set_valign(gtk::Align::Fill);
        fade_right.set_can_target(false);
        fade_right.set_opacity(0.0);

        let breadcrumb_overlay = gtk::Overlay::new();
        breadcrumb_overlay.set_child(Some(&breadcrumb_scroller));
        breadcrumb_overlay.add_overlay(&fade_left);
        breadcrumb_overlay.add_overlay(&fade_right);
        breadcrumb_overlay.set_hexpand(true);

        let hadjustment = breadcrumb_scroller.hadjustment();
        let breadcrumb_scrollbar =
            gtk::Scrollbar::new(gtk::Orientation::Horizontal, Some(&hadjustment));
        breadcrumb_scrollbar.add_css_class("breadcrumb-scrollbar");
        breadcrumb_scrollbar.set_visible(false);

        let breadcrumb_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        breadcrumb_container.add_css_class("breadcrumb-container");
        breadcrumb_container.append(&breadcrumb_overlay);
        breadcrumb_container.append(&breadcrumb_scrollbar);
        breadcrumb_container.set_hexpand(true);

        let motion_controller = gtk::EventControllerMotion::new();
        let weak_scrollbar = breadcrumb_scrollbar.downgrade();
        motion_controller.connect_enter(move |_, _, _| {
            if let Some(scrollbar) = weak_scrollbar.upgrade() {
                scrollbar.add_css_class("hovered");
            }
        });
        let weak_scrollbar = breadcrumb_scrollbar.downgrade();
        motion_controller.connect_leave(move |_| {
            if let Some(scrollbar) = weak_scrollbar.upgrade() {
                scrollbar.remove_css_class("hovered");
            }
        });
        breadcrumb_container.add_controller(motion_controller);

        let scroll_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let flash_scrollbar = {
            let breadcrumb_scrollbar = breadcrumb_scrollbar.downgrade();
            let scroll_timer = scroll_timer.clone();
            move || {
                let Some(breadcrumb_scrollbar) = breadcrumb_scrollbar.upgrade() else {
                    return;
                };
                breadcrumb_scrollbar.add_css_class("scrolling");
                if let Some(source) = scroll_timer.borrow_mut().take() {
                    source.remove();
                }
                let weak_scrollbar = breadcrumb_scrollbar.downgrade();
                let timer_cell = scroll_timer.clone();
                let source = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(1200),
                    move || {
                        timer_cell.borrow_mut().take();
                        if let Some(sb) = weak_scrollbar.upgrade() {
                            sb.remove_css_class("scrolling");
                        }
                    },
                );
                *scroll_timer.borrow_mut() = Some(source);
            }
        };

        let update_overflow = {
            let fade_left = fade_left.downgrade();
            let fade_right = fade_right.downgrade();
            let breadcrumb_scrollbar = breadcrumb_scrollbar.downgrade();
            move |hadjustment: &gtk::Adjustment| {
                let (Some(fade_left), Some(fade_right), Some(breadcrumb_scrollbar)) = (
                    fade_left.upgrade(),
                    fade_right.upgrade(),
                    breadcrumb_scrollbar.upgrade(),
                ) else {
                    return;
                };
                let value = hadjustment.value();
                let upper = hadjustment.upper();
                let page_size = hadjustment.page_size();
                let max_scroll = (upper - page_size).max(0.0);
                let has_overflow = max_scroll > 1.0;
                breadcrumb_scrollbar.set_visible(has_overflow);
                let show_left = value > 1.0;
                let show_right = has_overflow && value < max_scroll - 1.0;
                fade_left.set_opacity(if show_left { 1.0 } else { 0.0 });
                fade_right.set_opacity(if show_right { 1.0 } else { 0.0 });
            }
        };

        hadjustment.connect_value_changed({
            let update_overflow = update_overflow.clone();
            let flash_scrollbar = flash_scrollbar.clone();
            move |adjustment| {
                update_overflow(adjustment);
                flash_scrollbar();
            }
        });
        hadjustment.connect_changed(update_overflow);

        let location_stack = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(100)
            .build();
        location_stack.add_named(&breadcrumb_container, Some("breadcrumbs"));
        location_stack.add_named(&entry_control, Some("entry"));
        location_stack.set_visible_child_name("breadcrumbs");
        location_stack.set_hexpand(true);
        location_stack.set_valign(gtk::Align::Center);

        let global_activity_spinner = gtk::Spinner::new();
        global_activity_spinner.add_css_class("global-activity-spinner");
        global_activity_spinner.set_tooltip_text(Some("Working…"));
        global_activity_spinner.set_visible(false);
        let location_control = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        location_control.add_css_class("location-control");
        location_control.set_hexpand(true);
        location_control.set_valign(gtk::Align::Center);
        location_control.append(&location_stack);
        location_control.append(&global_activity_spinner);

        let preferences = super::preferences::PreferenceManager::shared();
        let browser = Browser::with_preferences(source, preferences.sort_preferences());
        browser.set_chooser_mode(!interactive);
        let preferences_for_sorting = preferences.clone();
        browser.observe_preferences(move |sorting| {
            preferences_for_sorting.set_sort_preferences(sorting);
        });
        let source_generation = Rc::new(Cell::new(0u64));
        let multiple_selection = Rc::new(Cell::new(multiple));
        let mode_views = ModeViews::new(&scroller, browser.clone(), multiple_selection.clone());
        overlay.set_child(Some(&mode_views.widget()));
        let pending_submit = Rc::new(RefCell::new(None::<Rc<dyn Fn()>>));
        let pending_submit_cb = pending_submit.clone();
        let location_stack_for_completion = location_stack.clone();
        let path_completion = location::completion::PathCompletion::attach(
            &location_entry,
            browser.clone(),
            move || {
                location_stack_for_completion
                    .visible_child_name()
                    .as_deref()
                    == Some("entry")
            },
            move || {
                if let Some(submit) = pending_submit_cb.borrow().as_ref() {
                    submit();
                }
            },
        );
        let state = Rc::new(ViewState {
            overlay,
            location_control,
            location_stack,
            global_activity_spinner,
            global_activity: RefCell::new(GlobalActivityState::default()),
            breadcrumbs,
            breadcrumb_scroller: breadcrumb_scroller.clone(),
            location_entry,
            path_completion,
            columns_widget,
            scroller,
            mode: Cell::new(mode_views.mode()),
            mode_views: RefCell::new(mode_views),
            columns: RefCell::new(Vec::new()),
            hovered_column: Cell::new(None),
            context_menu_column: Cell::new(None),
            context_menu_generation: Cell::new(0),
            context_menu_focus: RefCell::new(None),
            input_ownership: RefCell::new(super::input_ownership::InputOwnership::default()),
            horizontal_scroll_generation: Rc::new(Cell::new(0)),
            suppress_focus_scroll: Cell::new(false),
            pending_mirror: RefCell::new(None),
            source_generation,
            refreshing_source_filter: Cell::new(false),
            peek: RefCell::new(None),
            pending_peek: RefCell::new(None),
            pending_close: RefCell::new(None),
            peek_anchor: RefCell::new(None),
            peek_behavior,
            peek_enabled: Cell::new(true),
            single_click_previews: Cell::new(true),
            columns_mirror_selection: Cell::new(true),
            multiple_selection,
            interactive,
            columns_click_activation: Cell::new(ClickActivation::default()),
            active_rename: Rc::new(RefCell::new(None)),
            pending_rename: RefCell::new(None),
            rename_generation: Cell::new(0),
            rename_reveal_generation: Cell::new(0),
            pending_click_rename: RefCell::new(None),
            click_rename_generation: Cell::new(0),
            pending_new_entry: RefCell::new(None),
            file_progress_view: RefCell::new(None),
            pending_file_progress: RefCell::new(None),
            file_operation_progress: Cell::new((0, 0)),
            transfer_progress: Cell::new(None),
            flushing_to_device: Cell::new(false),
            pin_handler: RefCell::new(None),
            unpin_handler: RefCell::new(None),
            pin_status_handler: RefCell::new(None),
            print_handler: RefCell::new(None),
            search_selection_handlers: RefCell::new(Vec::new()),
            pending_select: RefCell::new(Vec::new()),
            pending_location_selection: RefCell::new(None),
            pending_select_properties: Cell::new(false),
            pending_extract_retry: RefCell::new(None),
            pending_archive_destination: RefCell::new(None),
            pending_delete_entries: RefCell::new(Vec::new()),
            pending_delete_dissolve: RefCell::new(None),
            deferred_delete_empty_depth: Cell::new(None),
            pending_navigate: RefCell::new(None),
            pending_location_credentials: RefCell::new(None),
            pending_trash_lookup: RefCell::new(None),
            pending_empty_trash: RefCell::new(None),
            trash_loading: RefCell::new(None),
            unlock_slots: RefCell::new(Vec::new()),
            auto_refresh: RefCell::new(None),
            trash_button: RefCell::new(None),
            visible_listing_observers: RefCell::new(Vec::new()),
            find_highlight_query: Rc::new(RefCell::new(String::new())),
            find_prompt_active: Cell::new(false),
            open_with_request: RefCell::new(desktop::OpenWithRequest::default()),
            find_highlights_on: Cell::new(false),
            notifying_visible_listing: Cell::new(false),
            drag_autoscroll: RefCell::new(None),
            suppress_scroll_after_drop: Cell::new(false),
            suppress_column_header_focus: Cell::new(false),
            search_pointer_cursor: RefCell::new(None),
            browser,
        });

        let weak_state = Rc::downgrade(&state);
        *pending_submit.borrow_mut() = Some(Rc::new(move || {
            if let Some(state) = weak_state.upgrade() {
                state.submit_location();
            }
        }));

        // Columns are laid out from the start edge, so the blank strip beside the last
        // one is the natural place to begin a marquee that runs into it.
        register_cut_view(&state);
        state.install_input_ownership();
        state.install_column_peek_targets();
        state.install_drag_autoscroll();
        if interactive {
            columns::install_resize_edges(&state);
        }

        let weak_state = Rc::downgrade(&state);
        columns::install_horizontal_scroll(&state);
        super::marquee::install_shared_origin_surface(&state.scroller, move |surface, _, x, _| {
            let state = weak_state.upgrade()?;
            let laid_out = state.columns_widget.compute_bounds(surface)?;
            if x < f64::from(laid_out.x() + laid_out.width()) {
                return None;
            }
            let columns = state.columns.borrow();
            columns.last().map(|column| column.marquee.clone())
        });

        state
            .mode_views
            .borrow()
            .set_context_state(Rc::downgrade(&state));
        if !interactive {
            state
                .mode_views
                .borrow()
                .set_new_folder_state(Rc::downgrade(&state));
        }
        if interactive {
            let weak_state = Rc::downgrade(&state);
            state.mode_views.borrow().set_transfer_handler(Rc::new(
                move |destination, sources, commit| {
                    if let Some(state) = weak_state.upgrade() {
                        state.commit_file_drop(destination, sources, commit);
                    }
                },
            ));
        }

        // The observer owns the view state while its window is alive. The window clears
        // the observer on destruction to break this deliberate lifecycle cycle.
        let observer_state = state.clone();
        state
            .browser
            .observe(move |event| observer_state.handle(event));

        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_state = Rc::downgrade(&state);
        click.connect_pressed(move |_, _, _, _| {
            if let Some(state) = weak_state.upgrade() {
                state.cancel_click_rename();
            }
        });
        state.overlay.add_controller(click);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_state = Rc::downgrade(&state);
        keys.connect_key_pressed(move |_, _, _, _| {
            if let Some(state) = weak_state.upgrade() {
                state.cancel_click_rename();
            }
            glib::Propagation::Proceed
        });
        state.overlay.add_controller(keys);

        let weak_state = Rc::downgrade(&state);
        state.location_entry.connect_activate(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.submit_location();
            }
        });
        let weak_state = Rc::downgrade(&state);
        confirm_location.connect_clicked(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.submit_location();
            }
        });
        let weak_state = Rc::downgrade(&state);
        cancel_location.connect_clicked(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.cancel_location_edit();
            }
        });
        breadcrumb_scroller.set_cursor_from_name(Some("text"));
        let edit_location = gtk::GestureClick::new();
        let weak_state = Rc::downgrade(&state);
        edit_location.connect_released(move |gesture, _, x, y| {
            let clicked_button = gesture
                .widget()
                .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
                .is_some_and(is_breadcrumb_button_target);
            if !clicked_button && let Some(state) = weak_state.upgrade() {
                state.begin_location_edit();
            }
        });
        breadcrumb_scroller.add_controller(edit_location);

        let hierarchy_menu = gtk::GestureClick::new();
        hierarchy_menu.set_button(gtk::gdk::BUTTON_SECONDARY);
        hierarchy_menu.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_state = Rc::downgrade(&state);
        hierarchy_menu.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(state) = weak_state.upgrade()
                && let Some(widget) = gesture.widget()
            {
                state.show_breadcrumb_hierarchy_menu(&widget, x, y);
            }
        });
        breadcrumb_scroller.add_controller(hierarchy_menu);

        let scroll_controller =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let hadjustment_for_scroll = breadcrumb_scroller.hadjustment();
        scroll_controller.connect_scroll(move |controller, dx, dy| {
            let delta = if dx.abs() > 0.001 { dx } else { dy };
            if delta.abs() > 0.001 {
                let unit = controller.unit();
                let step = delta
                    * match unit {
                        gtk::gdk::ScrollUnit::Wheel => {
                            hadjustment_for_scroll.page_size().powf(2.0 / 3.0).max(30.0)
                        }
                        gtk::gdk::ScrollUnit::Surface => 2.5,
                        _ => 1.0,
                    };
                let current = hadjustment_for_scroll.value();
                let max =
                    (hadjustment_for_scroll.upper() - hadjustment_for_scroll.page_size()).max(0.0);
                hadjustment_for_scroll.set_value((current + step).clamp(0.0, max));
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        breadcrumb_scroller.add_controller(scroll_controller);

        let view = Self { state };
        view.bind_preferences(&preferences);
        view
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

    pub(crate) fn connect_navigation_cleanup(&self, window: &gtk::Window) {
        let weak = self.downgrade();
        window.connect_close_request(move |_| {
            if let Some(browser) = weak.upgrade() {
                browser.cancel_pending_open_with();
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
            && sources
                .iter()
                .all(|source| source.parent().is_some() && !paths::is_trash_location(source))
    }

    pub(super) fn trash_file_drop(&self, sources: Vec<Location>) -> bool {
        if !Self::can_trash_file_drop(&sources) {
            return false;
        }
        self.state.trash_dropped_locations(sources);
        true
    }

    pub fn commit_file_drop(
        &self,
        destination: Location,
        sources: Vec<Location>,
        commit: crate::services::DropCommit,
    ) {
        self.state.commit_file_drop(destination, sources, commit);
    }

    /// Selects `names` in the active column once it finishes loading,
    /// optionally opening the properties dialog for the focused one.
    pub fn select_after_load(&self, names: Vec<String>, properties: bool) {
        self.state.pending_select.borrow_mut().extend(names);
        self.state.pending_select_properties.set(properties);
    }

    pub(super) fn reveal_location(&self, location: Location) {
        let Some(parent) = location.parent() else {
            return;
        };
        self.state.pending_archive_destination.take();
        self.state.pending_navigate.take();
        self.state.pending_select.take();
        self.state.pending_select_properties.set(false);
        self.state
            .pending_location_selection
            .replace(Some((parent.clone(), vec![location])));
        if self.state.browser.active_location().as_ref() == Some(&parent) {
            if let Some(depth) = self.state.browser.active_depth() {
                if let Some(column) = self.state.columns.borrow().get(depth) {
                    column.filter_entry.set_text("");
                }
                self.state.mode_views.borrow().clear_filter(depth);
            }
            self.state.browser.reload_active();
        } else {
            self.state.browser.navigate_location(parent, false);
        }
    }

    pub(super) fn refresh_source_filter(&self) {
        self.state.refreshing_source_filter.set(true);
        if let Some(last) = self.state.browser.active_depth() {
            for depth in 0..=last {
                self.state.browser.retry_column(depth);
            }
        }
        self.state.refreshing_source_filter.set(false);
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

    pub fn observe_visible_listing(&self, observer: impl Fn() + 'static) {
        self.state
            .visible_listing_observers
            .borrow_mut()
            .push(Rc::new(observer));
    }

    /// Displayed filter/search results, independent of the hidden directory's fill.
    pub fn search_result_listing(&self) -> Option<Vec<FileEntry>> {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().search_result_listing();
        }
        let columns = self.state.columns.borrow();
        let column = self
            .state
            .destination_depth()
            .and_then(|depth| columns.get(depth))
            .filter(|column| column.search_session.is_active())
            .or_else(|| {
                columns
                    .iter()
                    .find(|column| column.search_session.is_active())
            })?;
        Some(
            column
                .search_results
                .borrow()
                .iter()
                .map(search_result_entry)
                .collect(),
        )
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

    pub(super) fn set_print_handler(&self, handler: PrintHandler) {
        self.state.print_handler.replace(Some(handler));
    }

    pub fn set_operation_provider(&self, provider: Rc<dyn OperationProvider>) {
        self.state.browser.set_operation_provider(provider);
    }

    /// Records the search hit a pointer press focused, without changing selection.
    pub(in crate::ui) fn set_search_pointer_cursor_handler(&self, handler: SearchPointerCursor) {
        self.state.search_pointer_cursor.replace(Some(handler));
    }

    pub fn set_trash_button(&self, button: gtk::Button) {
        self.state.trash_button.replace(Some(button));
    }

    pub fn begin_rename(&self) -> bool {
        if self.filter_has_focus() || self.selected_search_results().is_some() {
            let Some(entry) = self.selected_search_result() else {
                return false;
            };
            if self.state.rename_operation_pending() {
                return false;
            }
            self.state.cancel_new_entry();
            context_menu::rename_context_entry(
                &self.state,
                self.state.destination_depth().unwrap_or(0),
                None,
                entry,
            );
            return true;
        }
        self.state.begin_rename()
    }

    pub fn cancel_rename(&self) -> bool {
        self.state.cancel_rename()
    }

    pub fn cancel_new_entry(&self) -> bool {
        self.state.cancel_new_entry()
    }

    pub fn rename_is_active(&self) -> bool {
        self.state.active_rename.borrow().is_some()
            || self.state.mode_views.borrow().rename_is_active()
    }

    pub(in crate::ui) fn cancel_pending_click_rename(&self) {
        self.state.cancel_click_rename();
    }

    pub fn active_rename_field(&self) -> Option<gtk::Entry> {
        self.state
            .active_rename
            .borrow()
            .as_ref()
            .map(|rename| rename.field.clone())
            .or_else(|| self.state.mode_views.borrow().active_rename_field())
    }

    pub fn new_entry_is_active(&self) -> bool {
        self.state.pending_new_entry.borrow().is_some()
    }

    /// Lets a marquee drag begin on blank chrome beside the file panes — the sidebar or
    /// the preview pane — and run into whichever view the current mode shows. The pane
    /// nearest the `edge` the surface sits on is the target, since that is the one such
    /// a drag runs into.
    pub(super) fn add_marquee_origin(&self, surface: &impl IsA<gtk::Widget>, edge: gtk::PackType) {
        let weak_state = Rc::downgrade(&self.state);
        super::marquee::install_shared_origin_surface(surface, move |_, _, _, _| {
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

    pub fn view_mode(&self) -> BrowserMode {
        self.state.mode.get()
    }

    pub fn connect_view_mode_changed(&self, handler: impl Fn(BrowserMode) + 'static) {
        let state = Rc::downgrade(&self.state);
        self.state
            .mode_views
            .borrow()
            .widget()
            .connect_visible_child_name_notify(move |stack| {
                let mode = match stack.visible_child_name().as_deref() {
                    Some("icons") => BrowserMode::Icons,
                    Some("list") => BrowserMode::List,
                    _ => BrowserMode::Columns,
                };
                if let Some(state) = state.upgrade() {
                    state.mode.set(mode);
                }
                handler(mode);
            });
    }

    pub fn set_view_mode(&self, mode: BrowserMode) {
        let previous = self.state.mode.get();
        if mode == previous {
            return;
        }
        let filter = match previous {
            BrowserMode::Columns => self.state.capture_active_column_filter(),
            BrowserMode::Icons | BrowserMode::List => {
                self.state.mode_views.borrow().capture_active_filter()
            }
        };
        self.state.mode.set(mode);
        self.state.mode_views.borrow().show_mode(mode);
        self.state.mode_views.borrow_mut().prepare_mode(mode);
        if mode == BrowserMode::Columns {
            self.state.rebuild_columns_from(0);
            self.state.restore_active_column_filter(&filter);
        } else {
            let pane = self.state.mode_views.borrow().active_filter_pane();
            if let Some(pane) = pane {
                pane.restore_filter(&filter);
            }
        }
        match previous {
            BrowserMode::Columns => self.state.truncate(0),
            BrowserMode::Icons | BrowserMode::List => self
                .state
                .mode_views
                .borrow_mut()
                .clear_inactive_mode(previous),
        }
        if mode == BrowserMode::Columns {
            if !self.state.find_prompt_active.get() {
                self.state.focus_rebuilt_active_column();
            }
        } else if !self.state.find_prompt_active.get()
            && let Some(depth) = self.state.browser.active_depth()
        {
            self.state.mode_views.borrow().focus_visible_pane(depth);
        }
        self.state.notify_visible_listing();
        self.schedule_find_highlights();
    }

    // Columns grabs the collection view itself; item-level focus lands on
    // editable cells that would swallow navigation keys.
    pub(in crate::ui) fn focus_file_view(&self) {
        if self.view_mode() == BrowserMode::Columns {
            self.state.focus_rebuilt_active_column();
        } else {
            self.state.browser.focus_active();
        }
    }

    pub fn set_density(&self, density: BrowserDensity) {
        self.state.mode_views.borrow_mut().set_density(density);
        self.state.overlay.remove_css_class("density-compact");
        self.state.overlay.remove_css_class("density-airy");
        self.state.overlay.add_css_class(match density {
            BrowserDensity::Compact => "density-compact",
            BrowserDensity::Airy => "density-airy",
        });
    }

    /// Groups List entries under file-type headings. Icons and Columns keep the
    /// preference but do not apply it.
    pub fn set_group_by_type(&self, enabled: bool) {
        self.state
            .mode_views
            .borrow_mut()
            .set_group_by_type(enabled);
    }

    pub fn set_icons_thumbnail_size(&self, size: i32) {
        self.state
            .mode_views
            .borrow_mut()
            .set_icons_thumbnail_size(size);
    }

    pub fn activate_focused(&self) {
        if self.view_mode() != BrowserMode::Columns {
            self.state.browser.activate_focused_in_place();
        } else {
            self.state.browser.activate_focused();
        }
    }

    pub(in crate::ui) fn activate_directory_column(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return false;
        }
        if let Some(entry) = self.selected_search_result() {
            if entry.is_directory() {
                self.state.browser.navigate(entry.location);
                return true;
            }
        } else if self
            .state
            .browser
            .focused_entry()
            .is_some_and(|entry| entry.is_directory())
        {
            self.activate_focused();
            return true;
        }
        false
    }

    pub fn commit_selection(&self) {
        self.state.browser.commit_selection();
    }

    pub fn resume_native_selection(&self) -> bool {
        self.state.mode_views.borrow().resume_native_selection()
    }

    pub fn navigate_left(&self) {
        if self.view_mode() != BrowserMode::Columns {
            self.state.browser.parent();
        } else {
            self.state.browser.focus_parent();
        }
    }

    pub fn synchronize_native_selection(&self, extend: bool) {
        let position = self.state.mode_views.borrow().focused_position();
        if let Some((depth, focused)) = position {
            self.select_native_target(depth, focused, extend);
        }
    }

    pub fn cross_type_group(&self, _direction: gtk::DirectionType, _extend: bool) -> bool {
        false
    }

    fn select_native_target(&self, depth: usize, focused: usize, extend: bool) {
        if extend {
            let order = self.state.mode_views.borrow().visual_order(depth);
            self.state
                .browser
                .extend_visual_selection(depth, focused, &order);
        } else {
            self.state.browser.select(depth, focused);
        }
    }

    pub fn item_at_sidebar_edge(&self) -> bool {
        if self.view_mode() == BrowserMode::Columns {
            self.first_column_has_focus()
        } else {
            self.state.mode_views.borrow().item_at_left_edge()
        }
    }

    pub fn at_left_edge(&self) -> bool {
        self.state.mode_views.borrow().at_left_edge()
    }

    pub fn first_column_has_focus(&self) -> bool {
        self.view_mode() == BrowserMode::Columns && self.state.focused_column_depth() == Some(0)
    }

    pub fn focus_header_from_top_item(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self
                .state
                .mode_views
                .borrow()
                .focus_header_from_top_item(!self.state.interactive);
        }
        let Some((depth, position, _)) = self.state.browser.focused_item() else {
            return false;
        };
        let columns = self.state.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return false;
        };
        if column.map.view_position(position) != Some(0) {
            return false;
        }
        let mut control = column.header_actions.first_child();
        let focused = loop {
            let Some(candidate) = control else {
                break false;
            };
            control = candidate.next_sibling();
            if candidate.is_visible() && candidate.grab_focus() {
                break true;
            }
        };
        if focused && let Some(window) = self.state.overlay.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        focused
    }

    pub fn header_actions_have_focus(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().header_has_focus();
        }
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        self.state.columns.borrow().iter().any(|column| {
            focused.as_ref().is_some_and(|focused| {
                focused == column.header_actions.upcast_ref::<gtk::Widget>()
                    || focused.is_ancestor(&column.header_actions)
            })
        })
    }

    pub fn move_header_focus(&self, direction: gtk::DirectionType) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().move_header_focus(direction);
        }
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let columns = self.state.columns.borrow();
        let Some(index) = columns.iter().position(|column| {
            focused.as_ref().is_some_and(|focused| {
                focused == column.header_actions.upcast_ref::<gtk::Widget>()
                    || focused.is_ancestor(&column.header_actions)
            })
        }) else {
            return false;
        };
        if columns[index].header_actions.child_focus(direction) {
            return true;
        }
        let adjacent = match direction {
            gtk::DirectionType::Left => index.checked_sub(1),
            gtk::DirectionType::Right => (index + 1 < columns.len()).then_some(index + 1),
            _ => None,
        };
        let Some(column) = adjacent.and_then(|index| columns.get(index)) else {
            return false;
        };
        column
            .header_actions_stack
            .set_visible_child_name("actions");
        let moved = focus_header_action(&column.header_actions, direction);
        self.state.refresh_destination_style();
        if moved && let Some(window) = self.state.overlay.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        moved
    }

    pub fn focus_items_from_header(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().focus_items_from_header();
        }
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        self.state
            .columns
            .borrow()
            .iter()
            .find(|column| {
                focused.as_ref().is_some_and(|focused| {
                    focused == column.header_actions.upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(&column.header_actions)
                })
            })
            .is_some_and(|column| column.list.grab_focus())
    }

    pub fn navigate_up(&self) {
        if self.view_mode() != BrowserMode::Columns {
            self.state.browser.parent();
            return;
        }

        match self
            .state
            .focused_column_depth()
            .or_else(|| self.state.browser.active_depth())
        {
            Some(0) => {
                if let Some(parent) = self
                    .state
                    .browser
                    .location_at(0)
                    .and_then(|location| location.parent())
                {
                    self.state.browser.navigate(parent);
                }
            }
            Some(depth) => self.state.browser.close_column(depth),
            None => {}
        }
    }

    pub fn location_widget(&self) -> gtk::Widget {
        self.state.location_control.clone().upcast()
    }

    /// Shows the shared header spinner until the returned activity guard is dropped.
    pub fn begin_global_activity(&self, label: impl Into<String>) -> GlobalActivity {
        self.state.begin_global_activity(label)
    }

    pub fn begin_location_edit(&self) {
        self.state.begin_location_edit();
    }

    pub fn location_has_focus(&self) -> bool {
        let entry = self.state.location_entry.upcast_ref::<gtk::Widget>();
        self.state.location_entry.has_focus()
            || self
                .state
                .overlay
                .root()
                .and_then(|root| root.focus())
                .as_ref()
                .is_some_and(|focused| focused == entry || focused.is_ancestor(entry))
    }

    pub(super) fn location_edit_is_active(&self) -> bool {
        self.state.location_stack.visible_child_name().as_deref() == Some("entry")
    }

    pub(super) fn location_edit_contains(&self, target: &gtk::Widget) -> bool {
        let location_stack = self.state.location_stack.upcast_ref::<gtk::Widget>();
        target == location_stack || target.is_ancestor(location_stack)
    }

    pub fn cancel_location_edit(&self) {
        self.state.cancel_location_edit();
    }

    pub fn set_peek_enabled(&self, enabled: bool) {
        self.state.peek_enabled.set(enabled);
        if !enabled {
            cancel_source(&self.state.pending_peek);
            self.state.browser.close_peek();
        }
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.state.single_click_previews.set(enabled);
        self.state
            .mode_views
            .borrow()
            .set_single_click_previews(enabled);
    }

    #[cfg(test)]
    pub(in crate::ui) fn single_click_previews_enabled(&self) -> bool {
        self.state.single_click_previews.get()
            && self
                .state
                .mode_views
                .borrow()
                .single_click_previews_enabled()
    }

    pub fn set_columns_mirror_selection(&self, enabled: bool) {
        self.state.columns_mirror_selection.set(enabled);
    }

    #[cfg(test)]
    pub(in crate::ui) fn columns_mirror_selection_enabled(&self) -> bool {
        self.state.columns_mirror_selection.get()
    }

    pub fn set_click_activation(&self, mode: BrowserMode, activation: ClickActivation) {
        if mode == BrowserMode::Columns {
            self.state.columns_click_activation.set(activation);
        } else {
            self.state
                .mode_views
                .borrow()
                .set_click_activation(mode, activation);
        }
    }

    pub fn create_new_folder(&self) {
        let mode = self.view_mode();
        let depth = if mode == BrowserMode::Columns {
            new_folder_destination_depth(
                self.state.focused_column_depth(),
                self.state.browser.active_depth(),
                self.state.columns.borrow().len(),
            )
        } else {
            self.state.browser.active_depth()
        };
        if let Some((depth, location)) = depth.and_then(|depth| {
            self.state
                .browser
                .location_at(depth)
                .map(|location| (depth, location))
        }) {
            self.state.begin_new_entry(depth, location, true);
        }
    }

    pub(in crate::ui) fn set_archive_preview_active(&self, active: bool) {
        if active {
            self.state.overlay.add_css_class("archive-preview");
        } else {
            self.state.overlay.remove_css_class("archive-preview");
        }
    }

    pub fn keyboard_navigation(&self) {
        self.state
            .input_ownership
            .borrow_mut()
            .keyboard_navigation();
        self.state.overlay.add_css_class("keyboard-navigation");
        if let Some(window) = self.state.overlay.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        cancel_source(&self.state.pending_peek);
        self.state.browser.close_peek();
        self.state.sync_mode_selection();
        self.state.refresh_destination_style();
    }

    /// Miller `.active-column` header accent. Off while a minimal-mode preview
    /// owns the keyboard so only that pane looks focused.
    pub(in crate::ui) fn set_column_header_focus(&self, focused: bool) {
        self.state.suppress_column_header_focus.set(!focused);
        self.state.refresh_destination_style();
    }

    pub fn paste(&self) {
        self.paste_with(transfer::ConflictPrimary::Replace, true);
    }

    pub fn copy_selection(&self) -> bool {
        let entries = self.selected_search_results().unwrap_or_else(|| {
            self.state.sync_mode_selection();
            self.state.browser.transfer_entries()
        });
        if entries.is_empty() {
            return false;
        }
        self.state.copy_entries(&entries);
        true
    }

    pub fn duplicate_selection(&self) -> bool {
        self.state.sync_mode_selection();
        let entries = self.state.browser.transfer_entries();
        let Some((destination, sources)) = duplicate_transfer(&entries) else {
            return false;
        };
        self.state.start_transfer(destination, sources, false);
        true
    }

    pub fn cut_selection(&self) -> bool {
        let entries = self.selected_search_results().unwrap_or_else(|| {
            self.state.sync_mode_selection();
            self.state.browser.transfer_entries()
        });
        if entries.is_empty() {
            return false;
        }
        self.state.cut_entries(&entries)
    }

    pub fn copy_path(&self) -> bool {
        let entries = self.named_clipboard_entries();
        if entries.is_empty() {
            return false;
        }
        copy_locations(&entries);
        true
    }

    pub fn copy_name(&self) -> bool {
        let entries = self.named_clipboard_entries();
        if entries.is_empty() {
            return false;
        }
        copy_names(&entries);
        true
    }

    fn named_clipboard_entries(&self) -> Vec<FileEntry> {
        if let Some(entries) = self.selected_search_results() {
            if !entries.is_empty() {
                return entries;
            }
            return self
                .search_result_listing()
                .and_then(|listing| {
                    let index = self.search_hit_index().unwrap_or(0) as usize;
                    listing
                        .get(index)
                        .cloned()
                        .or_else(|| listing.into_iter().next())
                })
                .into_iter()
                .collect();
        }
        self.state.sync_mode_selection();
        let entries = self.state.browser.selected_entries();
        if entries.is_empty() {
            self.state.browser.focused_entry().into_iter().collect()
        } else {
            entries
        }
    }

    pub fn show_open_with(&self) {
        self.state.sync_mode_selection();
        desktop::show_open_with_for_entries(&self.state, self.open_with_entries());
    }

    pub(crate) fn cancel_pending_open_with(&self) {
        self.state.open_with_request.borrow_mut().cancel();
    }

    fn open_with_entries(&self) -> Vec<FileEntry> {
        let mut entries = self
            .selected_search_results()
            .unwrap_or_else(|| self.state.browser.selected_entries());
        if entries.is_empty()
            && let Some(entry) = self.state.browser.focused_entry()
        {
            entries.push(entry);
        }
        entries
    }

    pub fn pin_focused(&self) {
        self.state.sync_mode_selection();
        let Some(entry) = self.state.browser.focused_entry() else {
            return;
        };
        let status = self
            .state
            .pin_status_handler
            .borrow()
            .as_ref()
            .map_or(PinStatus::Unavailable, |handler| handler(&entry.location));
        if !can_pin_entry(&entry, status) {
            return;
        }
        if let Some(handler) = self.state.pin_handler.borrow().as_ref() {
            handler(entry.location, entry.display_name);
        }
    }

    pub fn select_all(&self) {
        self.keyboard_navigation();
        if self.view_mode() == BrowserMode::Columns {
            if let Some(depth) = self
                .state
                .focused_column_depth()
                .or_else(|| self.state.browser.active_depth())
            {
                self.state.select_all(depth);
            }
        } else if !self.state.mode_views.borrow().select_all_search_results()
            && let Some(depth) = self.state.browser.active_depth()
        {
            self.state.browser.select_all(depth);
        }
    }

    pub fn open_terminal(&self) {
        self.state.sync_mode_selection();
        let selected = self.state.browser.selected_entries();
        let location = selected_terminal_location(&selected).or_else(|| {
            let mode = self.view_mode();
            let depth = if mode == BrowserMode::Columns {
                self.state.destination_depth()
            } else {
                self.state.browser.active_depth()
            };
            depth.and_then(|depth| self.state.browser.location_at(depth))
        });
        let Some(location) = location else {
            return;
        };
        launch_terminal(&location, &self.state.overlay);
    }

    pub fn refresh(&self) {
        if self.view_mode() == BrowserMode::Columns {
            self.state.browser.refresh_all();
        } else {
            self.state.browser.reload_active();
        }
    }

    pub fn set_auto_refresh_interval(&self, secs: u32) {
        self.state.set_auto_refresh_interval(secs);
    }

    pub fn show_location_properties(&self, location: &Location) {
        self.state.show_folder_properties(location);
    }

    pub fn show_focused_properties(&self) -> bool {
        if let Some(selected) = self.selected_search_results() {
            if selected.is_empty() {
                return false;
            }
            self.state.show_selection_properties(selected);
            return true;
        }
        self.state.sync_mode_selection();
        let selected = self.state.browser.selected_entries();
        if selected.len() > 1 {
            self.state.show_selection_properties(selected);
            return true;
        }
        let Some(entry) = self.state.browser.focused_entry() else {
            return false;
        };
        self.state.show_entry_properties(entry);
        true
    }

    pub fn confirm_empty_trash(&self) {
        self.state.load_trash_summary();
    }

    pub fn confirm_delete(&self, permanent: bool) -> bool {
        self.delete_selection(permanent, false, false)
    }

    /// Permanent-delete dialog with Cancel focused, so unmodified `D` then
    /// Enter does not destroy. Default-map Shift+Delete still uses
    /// [`Self::confirm_delete`].
    pub fn confirm_delete_preferring_cancel(&self) -> bool {
        self.delete_selection(true, false, true)
    }

    pub fn confirm_trash(&self) -> bool {
        self.delete_selection(false, true, false)
    }

    fn delete_selection(&self, permanent: bool, confirm_trash: bool, prefer_cancel: bool) -> bool {
        self.state.sync_mode_selection();
        let entries = if let Some(entries) = self.selected_search_results() {
            entries
        } else if self.view_mode() == BrowserMode::Columns {
            self.state.browser.selected_entries()
        } else {
            self.state.browser.deletion_entries()
        };
        if entries.is_empty() {
            return false;
        }
        let in_trash = self
            .state
            .focused_column_depth()
            .and_then(|depth| self.state.browser.location_at(depth))
            .or_else(|| self.state.browser.active_location())
            .as_ref()
            .is_some_and(is_trash_location);
        let permanent = permanent || in_trash;
        if permanent {
            if prefer_cancel {
                self.state.request_delete_preferring_cancel(entries);
            } else {
                self.state.request_delete(entries, true);
            }
        } else if confirm_trash {
            self.state.request_confirmed_trash(entries);
        } else {
            self.state.request_delete(entries, false);
        }
        true
    }

    pub fn undo_last_operation(&self) -> bool {
        if let Some((generation, _, _)) = self.state.browser.pending_undo_rename() {
            return self.state.browser.undo_rename(generation);
        }
        if let Some((generation, records)) = self.state.browser.pending_undo_move() {
            return self.state.undo_move(generation, records);
        }
        if let Some((generation, locations)) = self.state.browser.pending_undo_copy() {
            return self.state.undo_copy(generation, locations);
        }
        if let Some((generation, created, overwritten)) = self.state.browser.pending_undo_merge() {
            return self.state.undo_merge(generation, created, overwritten);
        }
        let Some(locations) = self.state.browser.pending_undo_trash() else {
            return self.state.browser.undo_last_trash();
        };
        let is_trash = self
            .state
            .browser
            .active_location()
            .as_ref()
            .is_some_and(paths::is_trash_location);
        let entries: Vec<FileEntry> = if is_trash {
            let names: HashSet<String> = locations
                .iter()
                .filter_map(|location| location.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect();
            self.state.browser.entries_named(&names)
        } else {
            locations
                .iter()
                .map(|location| {
                    let display_name = location
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    FileEntry {
                        location: location.clone(),
                        native_name: location.file_name().unwrap_or_default().to_os_string(),
                        thumbnail_path: None,
                        display_name,
                        kind: crate::model::EntryKind::File,
                        size: crate::model::MetadataValue::Unknown,
                        modified_unix_seconds: crate::model::MetadataValue::Unknown,
                        recent_unix_seconds: crate::model::MetadataValue::Unknown,
                        is_hidden: false,
                        mode: crate::model::MetadataValue::Unknown,
                        image_dimensions: crate::model::MetadataValue::Unknown,
                        child_count: crate::model::MetadataValue::Unknown,
                        duration_seconds: crate::model::MetadataValue::Unknown,
                    }
                })
                .collect()
        };
        let trash_button = self.state.trash_button.borrow().clone();
        let undone = self.state.browser.undo_last_trash();
        if undone
            && let Some(trash_button) = trash_button
            && !entries.is_empty()
        {
            let source = self
                .state
                .delete_animation_source()
                .unwrap_or_else(|| self.state.overlay.clone().upcast());
            fly_to_trash::fly_from_trash(&source, &entries, &trash_button, || {});
        }
        undone
    }

    pub fn redo_last_operation(&self) -> bool {
        if let Some((generation, _, _)) = self.state.browser.pending_redo_rename() {
            return self.state.browser.redo_rename(generation);
        }
        if let Some((generation, records)) = self.state.browser.pending_redo_move() {
            return self.state.redo_move(generation, records);
        }
        if let Some((generation, locations)) = self.state.browser.pending_redo_copy() {
            return self.state.browser.redo_copy(generation, locations);
        }
        if let Some((generation, locations)) = self.state.browser.pending_redo_trash() {
            return self.state.redo_trash(generation, locations);
        }
        false
    }

    pub fn show_filter(&self) -> bool {
        self.show_filter_with_optional_query(None)
    }

    pub fn show_filter_with_query(&self, query: &str) -> bool {
        self.show_filter_with_optional_query(Some(query))
    }

    fn show_filter_with_optional_query(&self, query: Option<&str>) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().show_filter_with_query(query);
        }
        let depth = self.state.destination_depth();
        let column = depth.and_then(|depth| self.state.columns.borrow().get(depth).cloned());
        let Some(column) = column else {
            return false;
        };
        column.filter_button.set_active(true);
        focus_filter_entry(&column.filter_entry, query);
        true
    }

    pub fn filter_has_focus(&self) -> bool {
        match self.view_mode() {
            BrowserMode::Columns => {
                let focused = self.state.overlay.root().and_then(|root| root.focus());
                self.state.columns.borrow().iter().any(|column| {
                    column.filter_entry.has_focus()
                        || focused.as_ref().is_some_and(|focused| {
                            focused == column.filter_entry.upcast_ref::<gtk::Widget>()
                                || focused.is_ancestor(&column.filter_entry)
                        })
                })
            }
            BrowserMode::Icons | BrowserMode::List => {
                self.state.mode_views.borrow().filter_has_focus()
            }
        }
    }

    /// A selected displayed result, not an item from the hidden directory selection.
    pub fn selected_search_result(&self) -> Option<FileEntry> {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().selected_search_result();
        }
        let focused = self.state.overlay.root()?.focus()?;
        self.state
            .columns
            .borrow()
            .iter()
            .enumerate()
            .find_map(|(depth, column)| {
                if !(focused.is_ancestor(&column.filter_entry)
                    || focused == column.filter_entry.clone().upcast::<gtk::Widget>()
                    || focused.is_ancestor(&column.list)
                    || focused == column.list.clone().upcast::<gtk::Widget>())
                {
                    return None;
                }
                if column.recursive_search_active.get() {
                    let selected = column.selection.selection();
                    if selected.is_empty() {
                        return None;
                    }
                    return column
                        .search_results
                        .borrow()
                        .get(selected.maximum() as usize)
                        .map(search_result_entry);
                }
                if column.map.has_query() {
                    let selected = column.selection.selection();
                    if selected.is_empty() {
                        return None;
                    }
                    let source_position = column.map.source_position(selected.maximum())?;
                    return self.state.browser.entry_at(depth, source_position);
                }
                None
            })
    }

    pub(super) fn connect_search_selection_changed(&self, handler: Rc<dyn Fn()>) {
        self.state
            .search_selection_handlers
            .borrow_mut()
            .push(handler.clone());
        self.state
            .mode_views
            .borrow()
            .connect_search_selection_changed(Rc::new(move |_| handler()));
    }

    pub fn selected_search_results(&self) -> Option<Vec<FileEntry>> {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().selected_search_results();
        }
        let columns = self.state.columns.borrow();
        let depth = self.state.destination_depth();
        let (depth, column) = depth
            .and_then(|depth| columns.get(depth).map(|column| (depth, column)))
            .filter(|(_, column)| column.recursive_search_active.get() || column.map.has_query())
            .or_else(|| {
                columns.iter().enumerate().find(|(_, column)| {
                    column.recursive_search_active.get() || column.map.has_query()
                })
            })?;
        if column.recursive_search_active.get() {
            let results = column.search_results.borrow();
            return Some(
                collection::bitset_positions(&column.selection.selection())
                    .into_iter()
                    .filter_map(|position| results.get(position as usize).map(search_result_entry))
                    .collect(),
            );
        }
        if column.map.has_query() {
            let positions = collection::bitset_positions(&column.selection.selection());
            let mapped = column.map.source_positions(&positions);
            return Some(
                mapped
                    .into_iter()
                    .filter_map(|(_, source_pos)| self.state.browser.entry_at(depth, source_pos))
                    .collect(),
            );
        }
        None
    }

    pub fn item_view_has_focus(&self) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        self.state.mode_views.borrow().item_view_has_focus()
            || self.state.columns.borrow().iter().any(|column| {
                focused.as_ref().is_some_and(|focused| {
                    focused == column.presentation.stack.upcast_ref::<gtk::Widget>()
                        || focused == column.list.upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(&column.list)
                })
            })
    }

    pub fn page_selection(&self, direction: i32) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let Some((view, scroll)) = focused
            .as_ref()
            .and_then(super::scrolling::focused_collection)
        else {
            return false;
        };
        let page = super::scrolling::page(&view, &scroll);
        self.state.mode_views.borrow().suppress_focus_scroll();
        let target = self.state.browser.active_depth().and_then(|depth| {
            self.state
                .mode_views
                .borrow()
                .page_target(depth, direction, page.items)
                .map(|position| (depth, position))
        });
        if let Some((depth, position)) = target {
            self.state.browser.select(depth, position);
        } else {
            let order = self
                .state
                .browser
                .active_depth()
                .map(|depth| self.state.mode_views.borrow().visual_order(depth))
                .filter(|order| !order.is_empty());
            self.state
                .browser
                .page_along(direction, page.items, order.as_deref());
        }
        super::scrolling::reveal_selection(&view, &scroll, direction, &page);
        true
    }

    /// Moves the focus to the first or last visible entry of the active pane, for
    /// `Ctrl+Up` and `Ctrl+Down`.
    pub fn jump_selection(&self, direction: i32) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let Some((view, scroll)) = focused
            .as_ref()
            .and_then(super::scrolling::focused_collection)
        else {
            return false;
        };
        let order = self.active_visual_order();
        self.state
            .browser
            .page_along(direction, usize::MAX, order.as_deref());
        super::scrolling::reveal_jump(&view, &scroll, direction);
        true
    }

    /// The displayed source indices for Icons and List, or `None` in Columns
    /// where motions walk source order. Empty orders also collapse to `None`
    /// so callers fall back to the hidden-file-aware source walk.
    pub fn active_visual_order(&self) -> Option<Vec<usize>> {
        if self.view_mode() == BrowserMode::Columns {
            return None;
        }
        self.state
            .browser
            .active_depth()
            .map(|depth| self.state.mode_views.borrow().visual_order(depth))
            .filter(|order| !order.is_empty())
    }

    /// Moves the focus by an explicit number of visible entries in the focused
    /// pane only, for half-page motions. Browse only: visual motions must use
    /// [`crate::app::Browser::next_visible_index`] with extend or subtract so
    /// paging keeps a range instead of replacing the fill.
    pub fn page_by(&self, direction: i32, items: usize) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let Some((view, scroll)) = focused
            .as_ref()
            .and_then(super::scrolling::focused_collection)
        else {
            return false;
        };
        let page = super::scrolling::page(&view, &scroll);
        let page = super::scrolling::page_scaled(&page, items.max(1));
        self.state.mode_views.borrow().suppress_focus_scroll();
        let order = self.active_visual_order();
        self.state
            .browser
            .page_along(direction, page.items, order.as_deref());
        super::scrolling::reveal_selection(&view, &scroll, direction, &page);
        true
    }

    /// The viewport page of the focused collection, for callers that move a
    /// fraction of it (half-page motions).
    pub fn focused_page_items(&self) -> Option<usize> {
        let focused = self.state.overlay.root().and_then(|root| root.focus())?;
        let (view, scroll) = super::scrolling::focused_collection(&focused)?;
        Some(super::scrolling::page(&view, &scroll).items)
    }

    /// Moves the focus a single step in listing order in Icons and List, where
    /// [`crate::app::Browser::move_selection`] cannot honor grid wrap. Browse
    /// only; visual motions must extend or subtract instead.
    pub fn step_selection(&self, direction: i32) -> bool {
        if self.view_mode() == BrowserMode::Columns {
            return false;
        }
        let Some(depth) = self.state.browser.active_depth() else {
            return false;
        };
        let order = self.active_visual_order();
        self.state.mode_views.borrow().suppress_focus_scroll();
        self.state
            .browser
            .page_along(direction, 1, order.as_deref());
        let Some((_, position, _)) = self.state.browser.focused_item() else {
            return false;
        };
        self.state
            .mode_views
            .borrow()
            .reveal_selected_entry(depth, position);
        true
    }

    /// Moves the keyboard cursor in listing order without rewriting the filled
    /// selection, for minimal Browse once Space / Ctrl+A / Ctrl+R / visual has
    /// made an explicit fill.
    pub fn move_cursor(&self, direction: i32, steps: usize) -> bool {
        let order = self.active_visual_order();
        let Some(target) =
            self.state
                .browser
                .next_visible_index(direction, steps, order.as_deref())
        else {
            return false;
        };
        self.state.mode_views.borrow().suppress_focus_scroll();
        self.state.browser.focus_keeping_fill(target);
        let Some((depth, position, _)) = self.state.browser.focused_item() else {
            return false;
        };
        self.state
            .mode_views
            .borrow()
            .reveal_selected_entry(depth, position);
        true
    }

    /// Inverts the filled selection of the focused pane over visible entries.
    /// An `f` overlay or recursive `s` list inverts among displayed matches or
    /// hits; it does not rewrite the hidden directory listing.
    pub fn invert_selection(&self) {
        if self.invert_search_hits() {
            return;
        }
        self.keyboard_navigation();
        let depth = if self.view_mode() == BrowserMode::Columns {
            self.state
                .focused_column_depth()
                .or_else(|| self.state.browser.active_depth())
        } else {
            self.state.browser.active_depth()
        };
        if let Some(depth) = depth {
            self.state.browser.invert_selection(depth);
        }
    }

    /// Toggles the keyboard cursor item in the filled selection without moving
    /// the cursor and without toggling the preview.
    pub fn toggle_focused_selection(&self) -> bool {
        self.keyboard_navigation();
        self.state.browser.toggle_focused_selection()
    }

    /// Selects the keyboard cursor when the filled selection is empty, so
    /// yank, cut, and delete act on the cursor item instead of no-oping.
    /// Search-result selections keep their existing behavior.
    pub fn select_focused_if_empty(&self) {
        if self.selected_search_results().is_some() {
            return;
        }
        if !self.state.browser.selected_entries().is_empty() {
            return;
        }
        if let Some((depth, position, _)) = self.state.browser.focused_item() {
            self.state.browser.select(depth, position);
        }
    }

    /// Clears cut marks and, when this process owns the clipboard, its files
    /// payload. A foreign clipboard is never wiped.
    pub fn clear_yank_marks(&self, clipboard: &gtk::gdk::Clipboard) {
        self.state.clear_cut();
        if clipboard.is_local() {
            let _result = clipboard.set_content(None::<&gtk::gdk::ContentProvider>);
        }
    }

    /// Pastes into the keyboard destination directory, focusing Replace if a
    /// conflict dialog appears. Keep Both and Skip stay offered; the dialog is
    /// never skipped. Default-map Ctrl+V still uses [`Self::paste`].
    pub fn paste_preferring_replace(&self) {
        self.paste_with(transfer::ConflictPrimary::Replace, true);
    }

    /// Like [`Self::paste`], but focuses Keep Both (or Cancel when that button
    /// is hidden) if a conflict dialog appears.
    pub fn paste_preferring_keep_both(&self) {
        self.paste_with(transfer::ConflictPrimary::KeepBoth, true);
    }

    /// Minimal `p`/`P` for a cursor-only fill: the listing, not a hovered folder.
    pub fn paste_into_listing(&self, prefer_replace: bool) {
        let preferred = if prefer_replace {
            transfer::ConflictPrimary::Replace
        } else {
            transfer::ConflictPrimary::KeepBoth
        };
        self.paste_with(preferred, false);
    }

    fn paste_with(&self, preferred: transfer::ConflictPrimary, honor_selected_folder: bool) {
        self.state.sync_mode_selection();
        let selected = self.state.browser.selected_entries();
        let column = self
            .state
            .destination_depth()
            .and_then(|depth| self.state.browser.location_at(depth));
        if let Some(location) = paste_destination(
            &selected,
            column,
            !honor_selected_folder || self.state.browser.selection_is_load_cursor(),
        ) {
            self.state.paste_into(location, preferred);
        }
    }

    /// Clears every pane filter query without requiring the funnel to have
    /// focus, so leaving minimal mode or pressing Escape from the list
    /// restores all rows. Filter buttons are deactivated as well, and focus
    /// parked in a hidden funnel returns to its list.
    pub fn dismiss_hidden_filter(&self) -> bool {
        let mut dismissed = false;
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let columns = self.state.columns.borrow().clone();
        for column in &columns {
            column.force_recursive_search.set(false);
            if !column.filter_entry.text().trim().is_empty() {
                dismissed = true;
            }
            if column.filter_button.is_active() {
                dismissed = true;
                column.filter_button.set_active(false);
            }
            let entry_focused = column.filter_entry.has_focus()
                || focused.as_ref().is_some_and(|focused| {
                    focused == column.filter_entry.upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(&column.filter_entry)
                });
            column.filter_entry.set_text("");
            // Complete the model swap before callers restore listing focus.
            column.settle_filter_query();
            if entry_focused {
                dismissed = true;
                column.list.grab_focus();
            }
        }
        let panes = self.state.mode_views.borrow().filter_panes();
        for pane in panes {
            dismissed |= pane.clear_filter();
        }
        dismissed
    }

    /// Whether any pane carries a filter query, regardless of funnel focus.
    pub fn hidden_filter_active(&self) -> bool {
        self.state.columns.borrow().iter().any(|column| {
            !column.filter_entry.text().trim().is_empty() || column.filter_button.is_active()
        }) || self.state.mode_views.borrow().filter_active()
    }

    /// Whether the destination pane is running a forced-recursive `s` search.
    /// Listing `f` follows include-subfolders and returns false.
    pub fn force_recursive_search(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().force_recursive_search();
        }
        self.state.destination_depth().is_some_and(|depth| {
            self.state
                .columns
                .borrow()
                .get(depth)
                .is_some_and(|column| column.force_recursive_search.get())
        })
    }

    /// Current hidden filter query for the destination pane, for the footer
    /// `filter:` mark. Empty when no hidden filter is active.
    pub fn hidden_filter_query(&self) -> String {
        if self.view_mode() != BrowserMode::Columns {
            return self
                .state
                .mode_views
                .borrow()
                .active_filter_query()
                .unwrap_or_default();
        }
        let depth = self.state.destination_depth();
        depth
            .and_then(|depth| {
                self.state
                    .columns
                    .borrow()
                    .get(depth)
                    .map(|column| column.filter_entry.text().to_string())
            })
            .unwrap_or_default()
    }

    /// Writes the pane filter without revealing the funnel or focusing the
    /// header entry. The revealer stays collapsed; filtering still applies
    /// through the existing debounce path. Recursion follows
    /// `filter_include_subfolders`.
    pub fn set_filter_query_without_revealer(&self, query: &str) -> bool {
        self.write_hidden_query(query, false)
    }

    /// Current-location recursive name search (`index_filter` with recursion
    /// forced on). Independent of `filter_include_subfolders`.
    pub fn set_recursive_search_query(&self, query: &str) -> bool {
        self.write_hidden_query(query, true)
    }

    fn write_hidden_query(&self, query: &str, force_recursive: bool) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            let pane = self.state.mode_views.borrow().active_filter_pane();
            return pane.is_some_and(|pane| pane.write_hidden_query(query, force_recursive));
        }
        let depth = self.state.destination_depth();
        let Some(column) = depth.and_then(|depth| self.state.columns.borrow().get(depth).cloned())
        else {
            return false;
        };
        if column.filter_button.is_active() {
            column.filter_button.set_active(false);
        }
        let rescope = column.force_recursive_search.replace(force_recursive) != force_recursive;
        if rescope {
            column.search_session.cancel();
        }
        collection::set_filter_entry_query(&column.filter_entry, query, rescope);
        true
    }

    /// Live `/` `?` substring highlight. Empty query clears every bound name.
    /// Miller, list, and icons bind this query on rebuilds, including after a
    /// submitted find until browse Escape dismisses it.
    pub fn set_find_highlight(&self, query: &str) {
        *self.state.find_highlight_query.borrow_mut() = query.to_owned();
        self.state.apply_find_highlights();
        self.schedule_find_highlights();
    }

    pub fn set_find_prompt_active(&self, active: bool) {
        self.state.find_prompt_active.set(active);
    }

    pub fn find_highlights_active(&self) -> bool {
        !self.state.find_highlight_query().is_empty()
    }

    fn schedule_find_highlights(&self) {
        if self.state.find_highlight_query.borrow().is_empty()
            && !self.state.find_highlights_on.get()
        {
            return;
        }
        let view = self.clone();
        glib::idle_add_local_once(move || {
            view.state.apply_find_highlights();
        });
    }

    /// Incremental `/` `?` find: moves the cursor to the next visible entry
    /// whose display name contains `query` (case-insensitive), wrapping.
    /// Never hides rows. Empty queries are a no-op.
    pub fn find_in_listing(&self, query: &str, direction: i32) -> bool {
        use crate::services::fold_for_search;

        let folded = fold_for_search(query.trim());
        if folded.is_empty() {
            return false;
        }
        let Some((depth, cursor, _)) = self.state.browser.focused_item() else {
            return false;
        };
        let Some(snapshot) = self.state.browser.column_snapshot(depth) else {
            return false;
        };
        if snapshot.count == 0 {
            return false;
        }
        let order = self.active_visual_order();
        let ordered: Vec<usize> = match order {
            Some(order) if !order.is_empty() => order,
            _ => {
                let show_hidden = self
                    .state
                    .browser
                    .column_preferences(depth)
                    .is_some_and(|prefs| prefs.show_hidden);
                (0..snapshot.count)
                    .filter(|&source| {
                        self.state
                            .browser
                            .entry_at(depth, source)
                            .is_some_and(|entry| show_hidden || !entry.is_hidden)
                    })
                    .collect()
            }
        };
        if ordered.is_empty() {
            return false;
        }
        let cursor_rank = ordered
            .iter()
            .position(|index| *index == cursor)
            .unwrap_or(0);
        let step = if direction >= 0 { 1 } else { ordered.len() - 1 };
        for offset in 1..=ordered.len() {
            let rank = (cursor_rank + offset * step) % ordered.len();
            let source = ordered[rank];
            let Some(entry) = self.state.browser.entry_at(depth, source) else {
                continue;
            };
            if fold_for_search(&entry.display_name).contains(&folded) {
                self.keyboard_navigation();
                self.state.browser.select(depth, source);
                self.state.browser.focus_active();
                if self.view_mode() != BrowserMode::Columns {
                    self.state
                        .mode_views
                        .borrow()
                        .reveal_selected_entry(depth, source);
                }
                self.schedule_find_highlights();
                return true;
            }
        }
        false
    }

    pub(in crate::ui) fn modal_focus_restore(&self) -> Rc<dyn Fn()> {
        let previous = self
            .widget()
            .root()
            .and_then(|root| root.focus())
            .map(|widget| widget.downgrade());
        let view = self.downgrade();
        Rc::new(move || {
            let previous = previous.as_ref().and_then(|widget| widget.upgrade());
            // Hiding a footer prompt unmaps its entry, and GTK then focuses the
            // shortcuts button before the error dialog exists. Putting focus
            // back on that button leaves the listing with no cursor.
            let footer = previous.as_ref().is_some_and(widget_in_shortcut_footer);
            if let Some(previous) = previous.as_ref()
                && !footer
                && previous.is_mapped()
                && previous.is_sensitive()
            {
                if let Some(text) = previous.downcast_ref::<gtk::Text>() {
                    text.grab_focus_without_selecting();
                } else if let Some(entry) = previous.downcast_ref::<gtk::Entry>() {
                    entry.grab_focus_without_selecting();
                } else {
                    previous.grab_focus();
                }
            } else if (footer || super::preferences::PreferenceManager::shared().minimal_mode())
                && let Some(view) = view.upgrade()
            {
                view.restore_file_view_focus();
            }
        })
    }

    /// Returns keyboard focus to the active listing or search row after a
    /// prompt, dialog, or overlay closes. Runs on idle so GTK can unmap the
    /// previous widget without reentering a live `RefCell` borrow.
    ///
    /// An open modal keeps the focus it just took; its close handler schedules
    /// another restore after the layer is gone.
    pub(in crate::ui) fn restore_file_view_focus(&self) {
        let view = self.clone();
        glib::idle_add_local_once(move || {
            if view.modal_layer_is_open() {
                return;
            }
            let _ = view.restore_file_view_focus_now();
            let view = view.clone();
            glib::idle_add_local_once(move || {
                if view.modal_layer_is_open() || view.file_view_has_cursor_focus() {
                    return;
                }
                let _ = view.restore_file_view_focus_now();
                if view.file_view_has_cursor_focus() {
                    return;
                }
                let view = view.clone();
                glib::timeout_add_local_once(Duration::from_millis(50), move || {
                    if view.modal_layer_is_open() {
                        return;
                    }
                    let _ = view.restore_file_view_focus_now();
                });
            });
        });
    }

    fn modal_layer_is_open(&self) -> bool {
        self.state
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .is_some_and(|window| crate::ui::window::visible_modal_layer(&window).is_some())
    }

    fn restore_file_view_focus_now(&self) -> bool {
        if self.state.overlay.root().is_none() {
            return true;
        }
        // A newer text interaction wins over queued teardown focus.
        if self
            .state
            .overlay
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focused| {
                crate::ui::focus_navigation::in_popover_menu(&focused)
                    || (focused.is_mapped()
                        && (focused.is::<gtk::Text>()
                            || focused.is::<gtk::Entry>()
                            || focused.is::<gtk::TextView>()))
            })
        {
            return true;
        }
        if self.state.find_prompt_active.get() {
            return true;
        }
        let mode_views = self.state.mode_views.borrow();
        let mode = mode_views.mode();
        let search = (mode != BrowserMode::Columns)
            .then(|| mode_views.clone_inline_search())
            .flatten();
        drop(mode_views);
        if search.is_some_and(|search| search.has_results() && search.focus_selected_or_first_row())
        {
            self.set_file_view_focus_visible();
            return self.file_view_has_cursor_focus();
        }
        self.set_file_view_focus_visible();
        if mode != BrowserMode::Columns {
            let Some(depth) = self.state.browser.active_depth() else {
                return true;
            };
            let mode_views = self.state.mode_views.borrow();
            mode_views.focus_visible_pane(depth);
            drop(mode_views);
            return self.file_view_has_cursor_focus();
        }
        let Some(depth) = self.state.browser.active_depth() else {
            return true;
        };
        let columns = self.state.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return true;
        };
        let _ = column.list.grab_focus();
        if let Some((focused_depth, position, _)) = self.state.browser.focused_item()
            && focused_depth == depth
            && let Some(view_pos) = column.map.view_position(position)
        {
            columns::restore_column_cursor(column, view_pos);
        }
        drop(columns);
        self.file_view_has_cursor_focus()
    }

    fn set_file_view_focus_visible(&self) {
        if let Some(window) = self.state.overlay.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
    }

    fn file_view_has_cursor_focus(&self) -> bool {
        let Some(focused) = self.state.overlay.root().and_then(|root| root.focus()) else {
            return false;
        };
        if crate::ui::focus_navigation::in_popover_menu(&focused) {
            return true;
        }
        if focused.is::<gtk::Stack>() {
            return false;
        }
        let in_icons_or_list = self.state.mode_views.borrow().item_view_has_focus();
        let in_column = self.state.columns.borrow().iter().any(|column| {
            focused == *column.list.upcast_ref::<gtk::Widget>() || focused.is_ancestor(&column.list)
        });
        in_icons_or_list
            || in_column
            || focused.is::<gtk::ListBoxRow>()
            || focused.ancestor(gtk::ListBox::static_type()).is_some()
    }

    /// Focuses the first search result for minimal `s`, if a recursive
    /// search is showing. Falls back to the first listing item.
    pub fn focus_first_search_result(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            let search = self.state.mode_views.borrow().clone_inline_search();
            if search
                .is_some_and(|search| search.has_results() && search.focus_selected_or_first_row())
            {
                return true;
            }
        } else {
            let column = {
                let columns = self.state.columns.borrow();
                self.state
                    .destination_depth()
                    .and_then(|depth| columns.get(depth).cloned())
                    .or_else(|| {
                        columns
                            .iter()
                            .find(|column| column.search_session.is_active())
                            .cloned()
                    })
            };
            if let Some(column) = column
                && column.search_session.is_active()
                && column.search_model.n_items() > 0
            {
                column.selection.select_item(0, true);
                column.list.grab_focus();
                return true;
            }
        }
        self.jump_selection(-1)
    }

    /// Moves Icons/List InlineSearch rows. Columns results live in the
    /// ListView and return false so the caller can use listing motion.
    pub fn move_inline_search_results(&self, direction: i32) -> bool {
        if self.view_mode() == BrowserMode::Columns {
            return false;
        }
        self.inline_search()
            .is_some_and(|search| search.move_selection(direction))
    }

    /// Jumps to the first or last Icons/List InlineSearch row. Columns
    /// returns false so the caller can use the ListView jump.
    pub fn jump_inline_search_results(&self, direction: i32) -> bool {
        if self.view_mode() == BrowserMode::Columns {
            return false;
        }
        self.inline_search()
            .is_some_and(|search| search.jump_selection(direction))
    }

    /// The miller column whose ListView is showing `index_filter` rows: a
    /// listing `f` overlay or recursive `s`. Hit-list selection walks this
    /// model.
    fn search_overlay_column(&self) -> Option<ColumnView> {
        if self.view_mode() != BrowserMode::Columns {
            return None;
        }
        let columns = self.state.columns.borrow();
        self.state
            .destination_depth()
            .and_then(|depth| columns.get(depth).cloned())
            .filter(|column| column.search_session.is_active())
            .or_else(|| {
                columns
                    .iter()
                    .find(|column| column.search_session.is_active())
                    .cloned()
            })
    }

    fn inline_search(&self) -> Option<crate::ui::inline_search::InlineSearch> {
        if self.view_mode() == BrowserMode::Columns {
            return None;
        }
        self.state.mode_views.borrow().clone_inline_search()
    }

    /// Cursor into the recursive `s` hit list.
    pub fn search_hit_index(&self) -> Option<u32> {
        if let Some(column) = self.search_overlay_column() {
            return collection::bitset_positions(&column.selection.selection())
                .last()
                .copied();
        }
        self.inline_search()
            .and_then(|search| search.current_index())
    }

    pub fn search_hit_count(&self) -> u32 {
        if let Some(column) = self.search_overlay_column() {
            return column.search_model.n_items();
        }
        self.inline_search()
            .map(|search| search.hit_count())
            .unwrap_or(0)
    }

    pub fn search_hit_selected_count(&self) -> u32 {
        if let Some(column) = self.search_overlay_column() {
            return collection::bitset_positions(&column.selection.selection()).len() as u32;
        }
        self.inline_search()
            .map(|search| search.selected_count())
            .unwrap_or(0)
    }

    pub fn search_hit_is_selected(&self, index: u32) -> bool {
        if let Some(column) = self.search_overlay_column() {
            return column.selection.is_selected(index);
        }
        self.inline_search()
            .is_some_and(|search| search.index_is_selected(index))
    }

    pub fn focus_search_hit(&self, index: u32, replace_selection: bool) -> bool {
        if let Some(column) = self.search_overlay_column() {
            if index >= column.search_model.n_items() {
                return false;
            }
            column.syncing_selection.set(true);
            if replace_selection {
                column.selection.select_item(index, true);
            }
            column.syncing_selection.set(false);
            column.list.grab_focus();
            column
                .list
                .scroll_to(index, gtk::ListScrollFlags::FOCUS, None);
            return true;
        }
        self.inline_search()
            .is_some_and(|search| search.focus_index(index, replace_selection))
    }

    pub fn toggle_search_hit(&self, index: u32) -> bool {
        if let Some(column) = self.search_overlay_column() {
            if index >= column.search_model.n_items() {
                return false;
            }
            column.syncing_selection.set(true);
            if column.selection.is_selected(index) {
                column.selection.unselect_item(index);
            } else {
                column.selection.select_item(index, false);
            }
            column.syncing_selection.set(false);
            return true;
        }
        self.inline_search()
            .is_some_and(|search| search.toggle_index(index))
    }

    pub fn select_search_hit_range(&self, from: u32, to: u32) -> bool {
        if let Some(column) = self.search_overlay_column() {
            let count = column.search_model.n_items();
            if count == 0 {
                return false;
            }
            let last = count - 1;
            let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
            let lo = lo.min(last);
            let hi = hi.min(last);
            let positions: Vec<u32> = (lo..=hi).collect();
            column.syncing_selection.set(true);
            collection::apply_selection_plan(&column.selection, count, &positions);
            column.syncing_selection.set(false);
            return true;
        }
        self.inline_search()
            .is_some_and(|search| search.select_index_range(from, to))
    }

    pub fn unselect_search_hit_range(&self, from: u32, to: u32) -> bool {
        if let Some(column) = self.search_overlay_column() {
            let count = column.search_model.n_items();
            if count == 0 {
                return false;
            }
            let last = count - 1;
            let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
            column.syncing_selection.set(true);
            for index in lo.min(last)..=hi.min(last) {
                column.selection.unselect_item(index);
            }
            column.syncing_selection.set(false);
            return true;
        }
        self.inline_search()
            .is_some_and(|search| search.unselect_index_range(from, to))
    }

    /// Complements the result-list selection among currently displayed `f`
    /// matches or `s` hits. Returns false when that overlay is not showing so
    /// the listing invert can run instead.
    pub fn invert_search_hits(&self) -> bool {
        if let Some(column) = self.search_overlay_column() {
            let count = column.search_model.n_items();
            if count == 0 {
                return false;
            }
            let selected = collection::bitset_positions(&column.selection.selection());
            let inverted: Vec<u32> = (0..count)
                .filter(|index| !selected.contains(index))
                .collect();
            column.syncing_selection.set(true);
            collection::apply_selection_plan(&column.selection, count, &inverted);
            column.syncing_selection.set(false);
            return true;
        }
        self.inline_search()
            .is_some_and(|search| search.invert_selection())
    }

    /// Minimal `g Space` path: navigates a typed path/URI through the same
    /// location entry validation (mount dialogs included). The caller clears
    /// the footer entry so credentials never linger.
    pub fn submit_footer_location(&self, input: &str) -> bool {
        self.state.location_entry.set_text(input);
        self.state.submit_location();
        true
    }

    pub fn jump_parked_selection(&self, direction: i32) -> bool {
        if !self.item_view_has_focus()
            || !self
                .state
                .overlay
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focused| focused.is::<gtk::Stack>())
        {
            return false;
        }
        let order = self
            .state
            .browser
            .active_depth()
            .map(|depth| self.state.mode_views.borrow().visual_order(depth))
            .filter(|order| !order.is_empty());
        self.state
            .browser
            .page_along(direction, usize::MAX, order.as_deref());
        true
    }

    pub fn dismiss_empty_focused_filter(&self) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let empty = self.state.mode_views.borrow().empty_filter_has_focus()
            || self.state.columns.borrow().iter().any(|column| {
                column.filter_entry.text().is_empty()
                    && focused.as_ref().is_some_and(|focused| {
                        focused == column.filter_entry.upcast_ref::<gtk::Widget>()
                            || focused.is_ancestor(&column.filter_entry)
                    })
            });
        empty && self.dismiss_focused_filter()
    }

    pub fn dismiss_focused_filter(&self) -> bool {
        if self.state.mode_views.borrow().dismiss_focused_filter() {
            return true;
        }
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let columns = self.state.columns.borrow();
        let Some(column) = columns.iter().find(|column| {
            column.filter_entry.has_focus()
                || focused.as_ref().is_some_and(|focused| {
                    focused == column.filter_entry.upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(&column.filter_entry)
                })
        }) else {
            return false;
        };
        column.filter_button.set_active(false);
        column.list.grab_focus();
        true
    }

    pub(super) fn open_focused_context_menu(&self) -> bool {
        let focused = self.state.browser.focused_item();
        let depth = focused
            .as_ref()
            .map(|(depth, ..)| *depth)
            .or_else(|| self.state.browser.active_depth());
        let Some(depth) = depth else {
            return false;
        };
        let position = focused.map(|(_, position, _)| position).filter(|position| {
            self.state
                .browser
                .selected_positions(depth)
                .contains(position)
        });

        let target = if self.view_mode() == BrowserMode::Columns {
            self.columns_context_menu_target(depth, position)
        } else {
            self.mode_views_context_menu_target(depth, position)
        };

        let Some((trigger, x, y)) = target else {
            return false;
        };
        trigger(x, y);
        true
    }

    fn columns_context_menu_target(
        &self,
        depth: usize,
        position: Option<usize>,
    ) -> Option<ContextMenuTarget> {
        self.state
            .columns
            .borrow()
            .get(depth)?
            .context_menu_target(position)
    }

    fn mode_views_context_menu_target(
        &self,
        depth: usize,
        position: Option<usize>,
    ) -> Option<ContextMenuTarget> {
        self.state
            .mode_views
            .borrow()
            .context_menu_target(depth, position)
    }

    pub fn dismiss_filter_on_outside_click(&self, root: &gtk::Widget, x: f64, y: f64) {
        if root
            .root()
            .and_downcast::<gtk::Window>()
            .is_some_and(|window| crate::ui::window::visible_modal_layer(&window).is_some())
        {
            return;
        }
        if self.view_mode() != BrowserMode::Columns {
            return;
        }
        let picked = root.pick(x, y, gtk::PickFlags::DEFAULT);
        let filter_button = {
            let columns = self.state.columns.borrow();
            let Some(column) = columns.iter().find(|c| c.filter_button.is_active()) else {
                return;
            };
            let inside = picked.as_ref().is_some_and(|p| {
                p == column.shell.upcast_ref::<gtk::Widget>() || p.is_ancestor(&column.shell)
            });
            if inside {
                return;
            }
            column.filter_button.clone()
        };
        filter_button.set_active(false);
    }
}

impl ViewState {
    pub(in crate::ui) fn find_highlight_query(&self) -> String {
        self.find_highlight_query.borrow().clone()
    }

    pub(in crate::ui) fn apply_find_highlights(&self) {
        let query = self.find_highlight_query.borrow().clone();
        if query.is_empty() && !self.find_highlights_on.get() {
            return;
        }
        for column in self.columns.borrow().iter() {
            for bound in column.bound_rows.borrow().iter() {
                if let Some(label) = bound.rename_label.upgrade() {
                    find_highlight::apply_to_label(&label, &query);
                }
            }
        }
        self.mode_views.borrow().apply_find_highlights(&query);
        self.find_highlights_on.set(!query.is_empty());
    }

    pub(in crate::ui) fn notify_visible_listing(&self) {
        if self.notifying_visible_listing.get() {
            return;
        }
        self.notifying_visible_listing.set(true);
        for observer in self.visible_listing_observers.borrow().iter() {
            observer();
        }
        self.apply_find_highlights();
        self.notifying_visible_listing.set(false);
    }

    pub(super) fn notify_search_selection_changed(&self) {
        let handlers = self.search_selection_handlers.borrow().clone();
        for handler in handlers {
            handler();
        }
    }

    pub(in crate::ui::browser) fn stop_drag_autoscroll(&self) {
        if let Some(tracker) = self.drag_autoscroll.borrow().as_ref() {
            tracker.stop();
        }
    }

    pub(in crate::ui) fn cancel_peek(&self) {
        cancel_source(&self.pending_peek);
        self.browser.close_peek();
    }

    fn begin_global_activity(self: &Rc<Self>, label: impl Into<String>) -> GlobalActivity {
        let label = label.into();
        let id = self.global_activity.borrow_mut().begin(label.clone());
        self.global_activity_spinner.set_tooltip_text(Some(&label));
        self.global_activity_spinner.set_visible(true);
        self.global_activity_spinner.start();
        GlobalActivity {
            state: Rc::downgrade(self),
            id,
        }
    }

    fn finish_global_activity(&self, id: u64) {
        let current = {
            let mut activity = self.global_activity.borrow_mut();
            activity.finish(id);
            activity.current_label().map(str::to_owned)
        };
        if let Some(label) = current {
            self.global_activity_spinner.set_tooltip_text(Some(&label));
        } else {
            self.global_activity_spinner.stop();
            self.global_activity_spinner.set_visible(false);
            self.global_activity_spinner
                .set_tooltip_text(Some("Working…"));
        }
    }

    pub(super) fn set_auto_refresh_interval(self: &Rc<Self>, secs: u32) {
        if let Some(source) = self.auto_refresh.take() {
            source.remove();
        }
        if secs == 0 {
            return;
        }
        let weak_state = Rc::downgrade(self);
        let source = glib::timeout_add_local(Duration::from_secs(u64::from(secs)), move || {
            let Some(state) = weak_state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.pending_new_entry.borrow().is_some()
                || state.active_rename.borrow().is_some()
                || state.rename_operation_pending()
                || state.mode_views.borrow().rename_is_active()
            {
                return glib::ControlFlow::Continue;
            }
            state.refresh_browser();
            glib::ControlFlow::Continue
        });
        self.auto_refresh.replace(Some(source));
    }

    fn refresh_browser(&self) {
        if self.mode.get() == BrowserMode::Columns {
            self.browser.refresh_all();
        } else {
            self.browser.reload_active();
        }
    }

    fn sync_mode_selection(&self) {
        if self.mode.get() == BrowserMode::Columns {
            if let Some(depth) = self.focused_column_depth() {
                self.browser.set_active_column(depth);
            }
            return;
        }
        let Some((depth, positions)) = self.mode_views.borrow().selected_positions() else {
            return;
        };
        let focused = self
            .mode_views
            .borrow()
            .focused_position()
            .filter(|(focused_depth, _)| *focused_depth == depth)
            .map(|(_, position)| position)
            .or_else(|| positions.last().copied());
        self.browser.set_selection(depth, &positions, focused);
    }

    fn install_input_ownership(self: &Rc<Self>) {
        self.overlay.add_css_class("keyboard-navigation");
        let motion = gtk::EventControllerMotion::new();
        motion.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |controller, x, y| {
            let Some(state) = weak.upgrade() else { return };
            let Some(event) = controller
                .current_event()
                .filter(|event| event.event_type() == gtk::gdk::EventType::MotionNotify)
            else {
                return;
            };
            let Some(position) = event.position() else {
                return;
            };
            if !state.input_ownership.borrow_mut().pointer_motion(position) {
                return;
            }
            state.hovered_column.set(state.column_depth_at(x, y));
            state.overlay.remove_css_class("keyboard-navigation");
            state.refresh_destination_style();
        });
        self.overlay.add_controller(motion);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            if let Some(state) = weak.upgrade() {
                state.hovered_column.set(state.column_depth_at(x, y));
                state.pointer_navigation();
            }
        });
        self.overlay.add_controller(click);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.pointer_navigation();
            }
            glib::Propagation::Proceed
        });
        self.overlay.add_controller(scroll);
    }

    fn column_depth_at(&self, x: f64, y: f64) -> Option<usize> {
        let picked = self.overlay.pick(x, y, gtk::PickFlags::DEFAULT)?;
        self.columns.borrow().iter().position(|column| {
            picked == column.shell.upcast_ref::<gtk::Widget>().clone()
                || picked.is_ancestor(&column.shell)
        })
    }

    fn pointer_navigation(&self) {
        self.input_ownership.borrow_mut().pointer_action();
        self.overlay.remove_css_class("keyboard-navigation");
        self.refresh_destination_style();
    }

    fn destination_depth(&self) -> Option<usize> {
        if self.mode.get() != BrowserMode::Columns {
            return self.browser.active_depth();
        }
        if let Some(depth) = self.context_menu_column.get()
            && depth < self.columns.borrow().len()
        {
            return Some(depth);
        }
        self.input_ownership.borrow().destination(
            self.hovered_column.get(),
            self.focused_column_depth(),
            self.browser.active_depth(),
            self.columns.borrow().len(),
        )
    }

    fn refresh_destination_style(&self) {
        let destination = self.destination_depth();
        let focused_column = self.focused_column_depth();
        let focused_item = self
            .browser
            .focused_item()
            .map(|(depth, position, _)| (depth, position));
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            let show_actions = destination == Some(depth);
            if show_actions && !self.suppress_column_header_focus.get() {
                column.shell.add_css_class("active-column");
            } else {
                column.shell.remove_css_class("active-column");
            }
            if !show_actions
                && self
                    .overlay
                    .root()
                    .and_then(|root| root.focus())
                    .is_some_and(|focused| {
                        focused == *column.header_actions.upcast_ref::<gtk::Widget>()
                            || focused.is_ancestor(&column.header_actions)
                    })
            {
                // Do not leave keyboard focus inside controls hidden by pointer navigation.
                if !column.list.grab_focus() {
                    column.presentation.stack.grab_focus();
                }
            }
            column
                .header_actions_stack
                .set_visible_child_name(if show_actions { "actions" } else { "hidden" });
            let cursor = focused_item
                .filter(|(item_depth, _)| *item_depth == depth)
                .filter(|_| focused_column == Some(depth))
                .and_then(|(_, position)| column.map.view_position(position));
            column.bound_rows.borrow_mut().retain(|bound| {
                let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                    return false;
                };
                if cursor == Some(item.position()) {
                    row.add_css_class("keyboard-cursor");
                } else {
                    row.remove_css_class("keyboard-cursor");
                }
                true
            });
            let active = destination == Some(depth)
                && self.browser.location_at(depth).is_some_and(|location| {
                    !is_trash_location(&location) && !location.is_recent_location()
                });
            if active {
                column.shell.add_css_class("destination-column");
            } else {
                column.shell.remove_css_class("destination-column");
            }
        }
    }

    fn focused_column_depth(&self) -> Option<usize> {
        let focused = self.overlay.root()?.focus()?;
        self.columns.borrow().iter().position(|column| {
            focused == column.shell.clone().upcast::<gtk::Widget>()
                || focused.is_ancestor(&column.shell)
        })
    }

    fn select_all(&self, depth: usize) {
        if self.mode.get() != BrowserMode::Columns {
            self.browser.select_all(depth);
            return;
        }
        if let Some(column) = self.columns.borrow().get(depth) {
            column.selection.select_all();
            column.list.grab_focus();
        }
    }
}

fn widget_in_shortcut_footer(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class("shortcut-footer") {
            return true;
        }
        current = widget.parent();
    }
    false
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

fn vim_focus_direction(key: gtk::gdk::Key) -> Option<gtk::DirectionType> {
    match key {
        gtk::gdk::Key::h => Some(gtk::DirectionType::Left),
        gtk::gdk::Key::j => Some(gtk::DirectionType::Down),
        gtk::gdk::Key::k => Some(gtk::DirectionType::Up),
        gtk::gdk::Key::l => Some(gtk::DirectionType::Right),
        _ => None,
    }
}

mod chooser_context;

#[cfg(test)]
mod tests;
