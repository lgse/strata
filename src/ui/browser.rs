// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    future::Future,
    path::Path,
    pin::Pin,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    app::{Browser, BrowserEvent},
    model::{EntryKind, FileEntry, Location, SortDirection, SortKey},
    services::{
        FileSource, OperationProvider, PasteItem, PreviewContent, TransferConflict, content_family,
        has_plain_text_extension, validate_basename,
    },
};

use super::{
    blur::BlurBin,
    browser_modes::{BrowserDensity, BrowserMode, ModeViews},
    motion::{animations_enabled, emphasized_deceleration},
};

const COLUMN_WIDTH: i32 = 300;
const COLUMN_OFFSET: i32 = 24;
const COLUMN_TRANSITION: Duration = Duration::from_millis(220);

#[derive(Clone)]
struct LoadPresentation {
    stack: gtk::Stack,
    skeleton: gtk::Box,
    feedback: gtk::Box,
    message: gtk::Label,
    retry: Option<gtk::Button>,
}

struct BoundRow {
    item: glib::WeakRef<gtk::ListItem>,
    row: glib::WeakRef<gtk::Box>,
}

#[derive(Clone)]
struct ColumnView {
    shell: gtk::Box,
    animation_generation: Rc<Cell<u64>>,
    presentation: LoadPresentation,
    model: gtk::StringList,
    filtered_model: gtk::FilterListModel,
    filter_entry: gtk::Entry,
    filter_button: gtk::ToggleButton,
    selection: gtk::MultiSelection,
    syncing_selection: Rc<Cell<bool>>,
    list: gtk::ListView,
    marquee: gtk::Box,
    bound_rows: Rc<RefCell<Vec<BoundRow>>>,
    entry_count: Rc<Cell<usize>>,
    spinner: gtk::Spinner,
    new_entry_row: gtk::Box,
    new_entry_icon: gtk::Image,
    new_entry_entry: gtk::Entry,
}

struct ActiveRename {
    entry: FileEntry,
    field: gtk::Entry,
    label: gtk::Label,
    spacer: gtk::Box,
}

struct ActiveNewEntry {
    location: Location,
    is_directory: bool,
    row: gtk::Box,
    field: gtk::Entry,
}

struct DeleteProgressView {
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
    progress: gtk::ProgressBar,
    status: gtk::Label,
}

struct PeekView {
    revealer: gtk::Revealer,
    location: Location,
    presentation: LoadPresentation,
    model: gtk::StringList,
    entries: Rc<RefCell<Vec<FileEntry>>>,
    entry_count: Rc<Cell<usize>>,
    spinner: gtk::Spinner,
}

impl LoadPresentation {
    fn new(content: &impl IsA<gtk::Widget>, retry: Option<gtk::Button>) -> Self {
        let skeleton = gtk::Box::new(gtk::Orientation::Vertical, 9);
        skeleton.add_css_class("loading-skeleton");
        for width in [168, 124, 192, 148, 176, 112] {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.add_css_class("skeleton-row");
            row.set_size_request(width, 10);
            row.set_halign(gtk::Align::Start);
            skeleton.append(&row);
        }

        let feedback = gtk::Box::new(gtk::Orientation::Vertical, 8);
        feedback.add_css_class("directory-feedback");
        feedback.set_halign(gtk::Align::Center);
        feedback.set_valign(gtk::Align::Center);
        let message = gtk::Label::new(None);
        message.add_css_class("status-message");
        message.set_justify(gtk::Justification::Center);
        message.set_wrap(true);
        feedback.append(&message);
        if let Some(button) = retry.as_ref() {
            button.set_halign(gtk::Align::Center);
            feedback.append(button);
        }

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(100)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(content, Some("content"));
        stack.add_named(&skeleton, Some("loading"));
        stack.add_named(&feedback, Some("feedback"));
        stack.set_visible_child_name("loading");

        Self {
            stack,
            skeleton,
            feedback,
            message,
            retry,
        }
    }

    fn show_loading(&self) {
        self.skeleton.set_visible(true);
        self.feedback.set_visible(true);
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(false);
        }
        self.stack.set_visible_child_name("loading");
    }

    fn show_content(&self) {
        self.stack.set_visible_child_name("content");
    }

    fn show_empty(&self) {
        self.message.set_text("This directory is empty");
        self.message.remove_css_class("error");
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(false);
        }
        self.stack.set_visible_child_name("feedback");
    }

    fn show_error(&self, message: &str) {
        self.message.set_text(message);
        self.message.add_css_class("error");
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(true);
        }
        self.stack.set_visible_child_name("feedback");
    }
}

#[derive(Clone, Copy)]
pub struct PeekBehavior {
    pub open_delay: Duration,
    pub close_delay: Duration,
    pub fade_duration: Duration,
    pub item_limit: usize,
}

impl Default for PeekBehavior {
    fn default() -> Self {
        Self {
            open_delay: Duration::from_millis(180),
            close_delay: Duration::from_millis(80),
            fade_duration: Duration::from_millis(100),
            item_limit: 8,
        }
    }
}

type PinHandler = Rc<dyn Fn(Location, String)>;

pub(super) struct ViewState {
    overlay: gtk::Overlay,
    location_stack: gtk::Stack,
    breadcrumbs: gtk::Box,
    location_entry: gtk::Entry,
    location_error: gtk::Label,
    columns_widget: gtk::Box,
    scroller: gtk::ScrolledWindow,
    mode_views: RefCell<ModeViews>,
    columns: RefCell<Vec<ColumnView>>,
    hovered_column: Cell<Option<usize>>,
    cut_locations: RefCell<Vec<Location>>,
    horizontal_scroll_generation: Rc<Cell<u64>>,
    peek: RefCell<Option<PeekView>>,
    pending_peek: RefCell<Option<glib::SourceId>>,
    pending_close: RefCell<Option<glib::SourceId>>,
    peek_anchor: RefCell<Option<gtk::Widget>>,
    peek_behavior: PeekBehavior,
    peek_enabled: Cell<bool>,
    single_click_previews: Cell<bool>,
    active_rename: RefCell<Option<ActiveRename>>,
    active_new_entry: RefCell<Option<ActiveNewEntry>>,
    delete_progress: RefCell<Option<DeleteProgressView>>,
    pin_handler: RefCell<Option<PinHandler>>,
    browser: Rc<Browser>,
}

#[derive(Clone)]
pub struct BrowserView {
    state: Rc<ViewState>,
}

impl BrowserView {
    pub fn new(source: Rc<dyn FileSource>, peek_behavior: PeekBehavior) -> Self {
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
        let overlay = gtk::Overlay::new();

        let location_entry = gtk::Entry::builder()
            .hexpand(true)
            .width_chars(48)
            .placeholder_text("Enter an absolute path")
            .tooltip_text("Location (Ctrl+L)")
            .build();
        location_entry.add_css_class("location-entry");
        let location_error = gtk::Label::new(None);
        location_error.add_css_class("location-error");
        location_error.set_visible(false);
        location_error.set_xalign(0.0);
        let confirm_location = gtk::Button::builder()
            .tooltip_text("Navigate (Enter)")
            .build();
        confirm_location.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::CHECK,
            16,
        )));
        confirm_location.add_css_class("location-action");
        let cancel_location = gtk::Button::builder()
            .tooltip_text("Cancel (Escape)")
            .build();
        cancel_location.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            16,
        )));
        cancel_location.add_css_class("location-action");
        let entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        entry_row.append(&location_entry);
        entry_row.append(&confirm_location);
        entry_row.append(&cancel_location);
        let entry_control = gtk::Box::new(gtk::Orientation::Vertical, 0);
        entry_control.append(&entry_row);
        entry_control.append(&location_error);

        let breadcrumbs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        breadcrumbs.add_css_class("breadcrumbs");
        let breadcrumb_scroller = gtk::ScrolledWindow::builder()
            .child(&breadcrumbs)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .build();
        let location_stack = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(100)
            .build();
        location_stack.add_named(&breadcrumb_scroller, Some("breadcrumbs"));
        location_stack.add_named(&entry_control, Some("entry"));
        location_stack.set_visible_child_name("breadcrumbs");
        location_stack.add_css_class("location-control");
        location_stack.set_hexpand(true);
        location_stack.set_valign(gtk::Align::Center);

        let preferences = super::theme::ThemeManager::shared();
        let browser = Browser::with_preferences(source, preferences.sort_preferences());
        let preferences_for_sorting = preferences.clone();
        browser.observe_preferences(move |sorting| {
            preferences_for_sorting.set_sort_preferences(sorting);
        });
        let mode_views = ModeViews::new(&scroller, browser.clone());
        overlay.set_child(Some(&mode_views.widget()));
        let state = Rc::new(ViewState {
            overlay,
            location_stack,
            breadcrumbs,
            location_entry,
            location_error,
            columns_widget,
            scroller,
            mode_views: RefCell::new(mode_views),
            columns: RefCell::new(Vec::new()),
            hovered_column: Cell::new(None),
            cut_locations: RefCell::new(Vec::new()),
            horizontal_scroll_generation: Rc::new(Cell::new(0)),
            peek: RefCell::new(None),
            pending_peek: RefCell::new(None),
            pending_close: RefCell::new(None),
            peek_anchor: RefCell::new(None),
            peek_behavior,
            peek_enabled: Cell::new(true),
            single_click_previews: Cell::new(true),
            active_rename: RefCell::new(None),
            active_new_entry: RefCell::new(None),
            delete_progress: RefCell::new(None),
            pin_handler: RefCell::new(None),
            browser,
        });

        let weak_state = Rc::downgrade(&state);
        state.mode_views.borrow().set_transfer_handler(Rc::new(
            move |destination, sources, move_sources| {
                if let Some(state) = weak_state.upgrade() {
                    state.start_transfer(destination, sources, move_sources);
                }
            },
        ));
        state
            .mode_views
            .borrow()
            .set_context_state(Rc::downgrade(&state));

        // The observer owns the view state while its window is alive. The window clears
        // the observer on destruction to break this deliberate lifecycle cycle.
        let observer_state = state.clone();
        state
            .browser
            .observe(move |event| observer_state.handle(event));

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
                .is_some_and(is_breadcrumb_target);
            if !clicked_button && let Some(state) = weak_state.upgrade() {
                state.begin_location_edit();
            }
        });
        breadcrumb_scroller.add_controller(edit_location);

        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.overlay.clone().upcast()
    }

    pub fn navigate(&self, path: impl AsRef<Path>) {
        self.state
            .browser
            .navigate(Location::local(path.as_ref().to_path_buf()));
    }

    pub fn browser(&self) -> Rc<Browser> {
        self.state.browser.clone()
    }

    pub(super) fn set_pin_handler(&self, handler: PinHandler) {
        self.state.pin_handler.replace(Some(handler));
    }

    pub fn set_operation_provider(&self, provider: Rc<dyn OperationProvider>) {
        self.state.browser.set_operation_provider(provider);
    }

    pub fn begin_rename(&self) -> bool {
        self.state.begin_rename()
    }

    pub fn cancel_rename(&self) -> bool {
        self.state.cancel_rename()
    }

    pub fn cancel_new_entry(&self) -> bool {
        self.state.cancel_new_entry() || self.state.mode_views.borrow().cancel_new_entry()
    }

    pub fn rename_is_active(&self) -> bool {
        self.state.active_rename.borrow().is_some()
            || self.state.mode_views.borrow().rename_is_active()
    }

    pub fn new_entry_is_active(&self) -> bool {
        self.state.active_new_entry.borrow().is_some()
            || self.state.mode_views.borrow().new_entry_is_active()
    }

    pub fn preview_occupied_width(&self) -> i32 {
        if self.view_mode() != BrowserMode::Columns {
            return single_pane_preview_reservation(self.state.overlay.width());
        }
        self.state
            .columns
            .borrow()
            .iter()
            .map(|column| column.shell.width().max(COLUMN_WIDTH))
            .fold(0, i32::saturating_add)
    }

    pub fn view_mode(&self) -> BrowserMode {
        self.state.mode_views.borrow().mode()
    }

    pub fn set_view_mode(&self, mode: BrowserMode) {
        let previous = self.state.mode_views.borrow().mode();
        self.state.mode_views.borrow_mut().set_mode(mode);
        if mode == BrowserMode::Columns && previous != BrowserMode::Columns {
            self.state.sync_column_models();
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

    pub fn activate_focused(&self) {
        if self.view_mode() != BrowserMode::Columns {
            self.state.browser.activate_focused_in_place();
        } else {
            self.state.browser.activate_focused();
        }
    }

    pub fn navigate_left(&self) {
        if self.view_mode() != BrowserMode::Columns {
            self.state.browser.parent();
        } else {
            self.state.browser.focus_parent();
        }
    }

    pub fn navigate_up(&self) {
        self.state.browser.parent();
    }

    pub fn location_widget(&self) -> gtk::Widget {
        self.state.location_stack.clone().upcast()
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

    pub fn create_new_folder(&self) {
        let mode = self.view_mode();
        let depth = if mode == BrowserMode::Columns {
            new_folder_destination_depth(
                self.state.hovered_column.get(),
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

    pub fn paste(&self) {
        let columns = self.state.columns.borrow();
        let depth = paste_destination_depth(self.state.hovered_column.get(), columns.len());
        drop(columns);
        if let Some(location) = depth.and_then(|depth| self.state.browser.location_at(depth)) {
            self.state.paste_into(location);
        }
    }

    pub fn copy_selection(&self) -> bool {
        self.state.sync_mode_selection();
        let entries = self.state.browser.selected_entries();
        if entries.is_empty() {
            return false;
        }
        self.state.copy_entries(&entries);
        true
    }

    pub fn cut_selection(&self) -> bool {
        self.state.sync_mode_selection();
        let entries = self.state.browser.selected_entries();
        if entries.is_empty() {
            return false;
        }
        self.state.cut_entries(&entries);
        true
    }

    pub fn select_all(&self) {
        if self.view_mode() == BrowserMode::Columns {
            if let Some(depth) = self.state.columns.borrow().len().checked_sub(1) {
                self.state.select_all(depth);
            }
        } else if let Some(depth) = self.state.browser.active_depth() {
            self.state.browser.select_all(depth);
        }
    }

    pub fn show_location_properties(&self, location: &Location) {
        self.state.show_folder_properties(location);
    }

    pub fn show_focused_properties(&self) -> bool {
        self.state.sync_mode_selection();
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
        let entries = self.state.browser.selected_entries();
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
        self.state
            .show_delete_confirmation(entries, permanent || in_trash);
        true
    }

    pub fn show_filter(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().show_filter();
        }
        let depth = self
            .state
            .focused_column_depth()
            .or_else(|| self.state.browser.active_depth());
        let columns = self.state.columns.borrow();
        let Some(column) = depth.and_then(|depth| columns.get(depth)) else {
            return false;
        };
        column.filter_button.set_active(true);
        column.filter_entry.grab_focus();
        true
    }

    pub fn filter_has_focus(&self) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        self.state.mode_views.borrow().filter_has_focus()
            || self.state.columns.borrow().iter().any(|column| {
                column.filter_entry.has_focus()
                    || focused.as_ref().is_some_and(|focused| {
                        focused == column.filter_entry.upcast_ref::<gtk::Widget>()
                            || focused.is_ancestor(&column.filter_entry)
                    })
            })
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
}

impl ViewState {
    fn sync_mode_selection(&self) {
        let Some((depth, positions)) = self.mode_views.borrow().selected_positions() else {
            return;
        };
        let focused = positions.last().copied();
        self.browser.set_selection(depth, &positions, focused);
    }

    fn focused_column_depth(&self) -> Option<usize> {
        let focused = self.overlay.root()?.focus()?;
        self.columns.borrow().iter().position(|column| {
            focused == column.shell.clone().upcast::<gtk::Widget>()
                || focused.is_ancestor(&column.shell)
        })
    }

    fn select_all(&self, depth: usize) {
        if let Some(column) = self.columns.borrow().get(depth) {
            column.selection.select_all();
            column.list.grab_focus();
        }
    }

    fn begin_new_entry(self: &Rc<Self>, depth: usize, location: Location, is_directory: bool) {
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            self.cancel_new_entry();
            self.mode_views
                .borrow()
                .begin_new_entry(depth, is_directory);
            return;
        }
        self.cancel_new_entry();
        self.cancel_rename();
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        let icon_name = if is_directory {
            crate::assets::icons::FOLDER
        } else {
            crate::assets::icons::DOCUMENTS
        };
        crate::assets::set_primary_icon(&column.new_entry_icon, icon_name);
        column.new_entry_entry.set_text("");
        column.new_entry_entry.remove_css_class("error");
        column.new_entry_entry.set_tooltip_text(None);
        column.new_entry_row.set_visible(true);
        self.active_new_entry.replace(Some(ActiveNewEntry {
            location,
            is_directory,
            row: column.new_entry_row.clone(),
            field: column.new_entry_entry.clone(),
        }));
        column.new_entry_entry.grab_focus();
    }

    fn submit_new_entry(self: &Rc<Self>, field: &gtk::Entry) {
        if !self
            .active_new_entry
            .borrow()
            .as_ref()
            .is_some_and(|active| active.field == *field)
        {
            return;
        }
        let name = field.text().to_string();
        if !update_basename_validation(field) {
            field.grab_focus();
            return;
        }
        let Some(active) = self.active_new_entry.take() else {
            return;
        };
        active.row.set_visible(false);
        field.set_text("");
        if active.is_directory {
            self.browser.create_directory(active.location, name);
        } else {
            self.browser.create_file(active.location, name);
        }
    }

    fn cancel_new_entry(&self) -> bool {
        let Some(active) = self.active_new_entry.take() else {
            return false;
        };
        active.field.set_text("");
        active.field.remove_css_class("error");
        active.field.set_tooltip_text(None);
        active.row.set_visible(false);
        true
    }

    fn start_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        let clear_cut = move_sources && same_locations(&sources, &self.cut_locations.borrow());
        let mut accepted = Vec::new();
        let mut collisions = Vec::new();
        for source in sources {
            if transfer_has_collision(&source, &destination) {
                collisions.push(source);
            } else {
                accepted.push(PasteItem {
                    source,
                    conflict: TransferConflict::FailIfExists,
                });
            }
        }
        self.resolve_transfer_collisions(
            destination,
            collisions,
            accepted,
            move_sources,
            clear_cut,
        );
    }

    fn resolve_transfer_collisions(
        self: &Rc<Self>,
        destination: Location,
        mut collisions: Vec<Location>,
        accepted: Vec<PasteItem>,
        move_sources: bool,
        clear_cut: bool,
    ) {
        if collisions.is_empty() {
            if clear_cut {
                self.complete_cut_transfer(
                    &accepted
                        .iter()
                        .map(|item| item.source.clone())
                        .collect::<Vec<_>>(),
                );
            }
            self.browser.transfer(destination, accepted, move_sources);
            return;
        }
        let source = collisions.remove(0);
        let Some(window_overlay) = self
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.child())
            .and_downcast::<gtk::Overlay>()
        else {
            return;
        };
        let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = blurred_root.as_ref() {
            root.set_blurred(true);
        }

        let name = source.display_name();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("delete-confirmation");
        content.add_css_class("delete-confirmation-content");
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("delete-confirmation-header");
        let symbol = gtk::CenterBox::new();
        symbol.add_css_class("delete-confirmation-symbol");
        symbol.set_size_request(40, 40);
        symbol.set_center_widget(Some(&crate::assets::danger_icon(
            crate::assets::icons::COPY,
            20,
        )));
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
        heading.set_hexpand(true);
        let title = gtk::Label::new(Some("File already exists"));
        title.add_css_class("delete-confirmation-title");
        title.set_xalign(0.0);
        let subtitle = gtk::Label::new(Some(&name));
        subtitle.add_css_class("delete-confirmation-subtitle");
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        subtitle.set_xalign(0.0);
        heading.append(&title);
        heading.append(&subtitle);
        header.append(&symbol);
        header.append(&heading);
        content.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.add_css_class("delete-confirmation-body");
        let explanation = gtk::Label::new(Some(&format!(
            "An item named “{name}” already exists in {}. Replacing it will overwrite its contents.",
            compact_display_path(&destination)
        )));
        explanation.add_css_class("delete-confirmation-explanation");
        explanation.set_max_width_chars(64);
        explanation.set_wrap(true);
        explanation.set_xalign(0.0);
        body.append(&explanation);
        let apply_all =
            gtk::CheckButton::with_label("Apply this choice to all remaining conflicts");
        apply_all.add_css_class("collision-apply-all");
        apply_all.set_visible(!collisions.is_empty());
        body.append(&apply_all);
        content.append(&body);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("delete-confirmation-actions");
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("delete-confirmation-cancel");
        let skip = gtk::Button::with_label("Skip");
        skip.add_css_class("delete-confirmation-cancel");
        let replace = gtk::Button::with_label("Replace");
        replace.add_css_class("delete-confirmation-delete");
        actions.append(&spacer);
        actions.append(&cancel);
        actions.append(&skip);
        actions.append(&replace);
        content.append(&actions);

        let layer = modal_layer(&content);
        window_overlay.add_overlay(&layer);
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
        });

        let skipped_layer = layer.clone();
        let skipped_overlay = window_overlay.clone();
        let skipped_root = blurred_root.clone();
        let skipped_state = self.clone();
        let skipped_destination = destination.clone();
        let skipped_collisions = collisions.clone();
        let skipped_accepted = accepted.clone();
        let skip_all = apply_all.clone();
        skip.connect_clicked(move |_| {
            dismiss_modal_layer(&skipped_layer, &skipped_overlay, skipped_root.as_ref());
            skipped_state.resolve_transfer_collisions(
                skipped_destination.clone(),
                if skip_all.is_active() {
                    Vec::new()
                } else {
                    skipped_collisions.clone()
                },
                skipped_accepted.clone(),
                move_sources,
                clear_cut,
            );
        });

        let replaced_layer = layer.clone();
        let replaced_overlay = window_overlay.clone();
        let replaced_root = blurred_root.clone();
        let replaced_state = self.clone();
        let replace_all = apply_all;
        replace.connect_clicked(move |_| {
            dismiss_modal_layer(&replaced_layer, &replaced_overlay, replaced_root.as_ref());
            let mut accepted = accepted.clone();
            accepted.push(PasteItem {
                source: source.clone(),
                conflict: TransferConflict::ReplaceExisting,
            });
            let remaining = if replace_all.is_active() {
                accepted.extend(collisions.iter().cloned().map(|source| PasteItem {
                    source,
                    conflict: TransferConflict::ReplaceExisting,
                }));
                Vec::new()
            } else {
                collisions.clone()
            };
            replaced_state.resolve_transfer_collisions(
                destination.clone(),
                remaining,
                accepted,
                move_sources,
                clear_cut,
            );
        });

        let escape = gtk::EventControllerKey::new();
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        cancel.grab_focus();
    }

    fn copy_entries(&self, entries: &[FileEntry]) {
        if set_files_clipboard(entries) {
            self.clear_cut();
        }
    }

    fn cut_entries(&self, entries: &[FileEntry]) {
        if set_files_clipboard(entries) {
            self.cut_locations
                .replace(entries.iter().map(|entry| entry.location.clone()).collect());
            self.refresh_cut_rows();
        }
    }

    fn clear_cut(&self) {
        if self.cut_locations.borrow().is_empty() {
            return;
        }
        self.cut_locations.borrow_mut().clear();
        self.refresh_cut_rows();
    }

    fn complete_cut_transfer(&self, transferred: &[Location]) {
        self.cut_locations
            .borrow_mut()
            .retain(|location| !transferred.contains(location));
        let remaining = self.cut_locations.borrow().clone();
        if remaining.is_empty() {
            if let Some(display) = gtk::gdk::Display::default() {
                let _result = display
                    .clipboard()
                    .set_content(None::<&gtk::gdk::ContentProvider>);
            }
        } else {
            let _set = set_location_files_clipboard(&remaining);
        }
        self.refresh_cut_rows();
    }

    fn refresh_cut_rows(&self) {
        let cut = self.cut_locations.borrow();
        self.mode_views.borrow().set_cut_locations(&cut);
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            column.bound_rows.borrow_mut().retain(|bound| {
                let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                    return false;
                };
                let is_cut = source_position_for_filtered(
                    &column.model,
                    &column.filtered_model,
                    item.position(),
                )
                .and_then(|position| self.browser.entry_at(depth, position))
                .is_some_and(|entry| cut.contains(&entry.location));
                set_cut_path_style(&row, is_cut);
                true
            });
        }
    }

    fn paste_into(self: &Rc<Self>, destination: Location) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let clipboard = display.clipboard();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = clipboard
                .read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT)
                .await;
            let files = match result {
                Ok(value) => match value.get::<gtk::gdk::FileList>() {
                    Ok(files) => files.files(),
                    Err(_) => return,
                },
                Err(_) => return,
            };
            let sources = files
                .into_iter()
                .map(|file| {
                    file.path()
                        .map(Location::local)
                        .unwrap_or_else(|| Location::uri(file.uri()))
                })
                .collect::<Vec<_>>();
            if let Some(state) = weak.upgrade() {
                let move_sources = same_locations(&sources, &state.cut_locations.borrow());
                state.start_transfer(destination, sources, move_sources);
            }
        });
    }

    fn show_transfer_dialog(self: &Rc<Self>, entries: Vec<FileEntry>, move_sources: bool) {
        if entries.is_empty() {
            return;
        }
        let Some(window_overlay) = self
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.child())
            .and_downcast::<gtk::Overlay>()
        else {
            return;
        };
        let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = blurred_root.as_ref() {
            root.set_blurred(true);
        }

        let base = self
            .browser
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf))
            .unwrap_or_else(glib::home_dir);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("transfer-dialog");
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("transfer-header");
        let symbol = gtk::CenterBox::new();
        symbol.add_css_class("transfer-symbol");
        symbol.set_center_widget(Some(&crate::assets::primary_icon(
            if move_sources {
                crate::assets::icons::FOLDER
            } else {
                crate::assets::icons::COPY
            },
            20,
        )));
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
        heading.set_hexpand(true);
        let title = gtk::Label::new(Some(if move_sources { "Move to" } else { "Copy to" }));
        title.add_css_class("transfer-title");
        title.set_xalign(0.0);
        let subtitle = gtk::Label::new(Some(&format!(
            "Choose a destination for {}",
            item_count_label(entries.len())
        )));
        subtitle.add_css_class("transfer-subtitle");
        subtitle.set_xalign(0.0);
        heading.append(&title);
        heading.append(&subtitle);
        let close = gtk::Button::new();
        close.add_css_class("transfer-close");
        close.set_tooltip_text(Some("Cancel"));
        close.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            16,
        )));
        header.append(&symbol);
        header.append(&heading);
        header.append(&close);
        content.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.add_css_class("transfer-body");
        let field_label = gtk::Label::new(Some("DESTINATION FOLDER"));
        field_label.add_css_class("transfer-field-label");
        field_label.set_xalign(0.0);
        let field = gtk::Entry::new();
        field.add_css_class("transfer-field");
        field.set_hexpand(true);
        field.set_placeholder_text(Some("Type a folder path…"));
        field.set_text(&folder_input_path(&base));
        field.set_position(-1);
        body.append(&field_label);
        body.append(&field);

        let suggestions = gtk::Box::new(gtk::Orientation::Vertical, 2);
        suggestions.add_css_class("transfer-suggestions");
        let suggestion_scroll = gtk::ScrolledWindow::builder()
            .child(&suggestions)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(150)
            .max_content_height(220)
            .propagate_natural_height(true)
            .build();
        suggestion_scroll.add_css_class("transfer-suggestion-scroll");
        body.append(&suggestion_scroll);
        let error = gtk::Label::new(None);
        error.add_css_class("transfer-error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        body.append(&error);
        content.append(&body);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("transfer-actions");
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("transfer-cancel");
        let confirm = gtk::Button::with_label(if move_sources {
            "Move here"
        } else {
            "Copy here"
        });
        confirm.add_css_class("transfer-confirm");
        actions.append(&spacer);
        actions.append(&cancel);
        actions.append(&confirm);
        content.append(&actions);

        let generation = Rc::new(Cell::new(0_u64));
        let pending_creation = Rc::new(RefCell::new(None::<std::path::PathBuf>));
        let creating_destination = Rc::new(Cell::new(false));
        let suggestions_box = suggestions.clone();
        let suggestions_generation = generation.clone();
        let suggestions_base = base.clone();
        let suggestions_error = error.clone();
        let changed_confirm = confirm.clone();
        let changed_creation = pending_creation.clone();
        field.connect_changed(move |field| {
            field.remove_css_class("error");
            suggestions_error.set_visible(false);
            suggestions_error.remove_css_class("transfer-warning");
            suggestions_error.add_css_class("transfer-error");
            changed_creation.borrow_mut().take();
            changed_confirm.set_label(if move_sources {
                "Move here"
            } else {
                "Copy here"
            });
            refresh_transfer_suggestions(
                field,
                &suggestions_box,
                &suggestions_generation,
                suggestions_base.clone(),
            );
        });

        let layer = modal_layer(&content);
        window_overlay.add_overlay(&layer);
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        let cancel_creating = creating_destination.clone();
        cancel.connect_clicked(move |_| {
            if !cancel_creating.get() {
                dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
            }
        });
        let close_layer = layer.clone();
        let close_overlay = window_overlay.clone();
        let close_root = blurred_root.clone();
        let close_creating = creating_destination.clone();
        close.connect_clicked(move |_| {
            if !close_creating.get() {
                dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
            }
        });
        let confirm_layer = layer.clone();
        let confirm_overlay = window_overlay.clone();
        let confirm_root = blurred_root.clone();
        let transfer_state = self.clone();
        let confirm_field = field.clone();
        let confirm_error = error.clone();
        let confirm_base = base.clone();
        let confirm_creation = pending_creation;
        let confirm_creating = creating_destination.clone();
        let confirm_cancel = cancel.clone();
        let confirm_close = close.clone();
        let sources = entries
            .iter()
            .map(|entry| entry.location.clone())
            .collect::<Vec<_>>();
        confirm.connect_clicked(move |button| {
            let path =
                resolve_destination_path(&confirm_field.text(), &confirm_base, &glib::home_dir());
            if path.exists() && !path.is_dir() {
                confirm_error.remove_css_class("transfer-warning");
                confirm_error.add_css_class("transfer-error");
                confirm_error.set_text("The destination exists, but it is not a folder.");
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                confirm_field.grab_focus();
                return;
            }
            if !path.exists() && confirm_creation.borrow().as_ref() != Some(&path) {
                confirm_creation.replace(Some(path.clone()));
                confirm_error.remove_css_class("transfer-error");
                confirm_error.add_css_class("transfer-warning");
                confirm_error.set_text(&format!(
                    "{} does not exist. It will be created before the items are transferred.",
                    compact_native_path(&path)
                ));
                confirm_error.set_visible(true);
                button.set_label(if move_sources {
                    "Create and move"
                } else {
                    "Create and copy"
                });
                button.grab_focus();
                return;
            }
            if path.is_dir() {
                transfer_state.start_transfer(Location::local(path), sources.clone(), move_sources);
                dismiss_modal_layer(&confirm_layer, &confirm_overlay, confirm_root.as_ref());
                return;
            }

            confirm_creating.set(true);
            button.set_sensitive(false);
            button.set_label("Creating folder…");
            confirm_field.set_sensitive(false);
            confirm_cancel.set_sensitive(false);
            confirm_close.set_sensitive(false);
            let created_state = transfer_state.clone();
            let created_sources = sources.clone();
            let created_layer = confirm_layer.clone();
            let created_overlay = confirm_overlay.clone();
            let created_root = confirm_root.clone();
            let created_button = button.clone();
            let created_field = confirm_field.clone();
            let created_error = confirm_error.clone();
            let created_creating = confirm_creating.clone();
            let created_cancel = confirm_cancel.clone();
            let created_close = confirm_close.clone();
            glib::MainContext::default().spawn_local(async move {
                let created_path = path.clone();
                let result =
                    gio::spawn_blocking(move || std::fs::create_dir_all(&created_path)).await;
                match result {
                    Ok(Ok(())) => {
                        created_state.start_transfer(
                            Location::local(path),
                            created_sources,
                            move_sources,
                        );
                        dismiss_modal_layer(
                            &created_layer,
                            &created_overlay,
                            created_root.as_ref(),
                        );
                    }
                    Ok(Err(error)) => {
                        created_creating.set(false);
                        created_cancel.set_sensitive(true);
                        created_close.set_sensitive(true);
                        created_error.remove_css_class("transfer-warning");
                        created_error.add_css_class("transfer-error");
                        created_error.set_text(&format!("Unable to create that folder: {error}"));
                        created_error.set_visible(true);
                        created_field.add_css_class("error");
                        created_field.set_sensitive(true);
                        created_field.grab_focus();
                        created_button.set_sensitive(true);
                        created_button.set_label(if move_sources {
                            "Move here"
                        } else {
                            "Copy here"
                        });
                    }
                    Err(_) => {
                        created_creating.set(false);
                        created_cancel.set_sensitive(true);
                        created_close.set_sensitive(true);
                        created_error.remove_css_class("transfer-warning");
                        created_error.add_css_class("transfer-error");
                        created_error.set_text("Unable to create that folder.");
                        created_error.set_visible(true);
                        created_field.add_css_class("error");
                        created_field.set_sensitive(true);
                        created_field.grab_focus();
                        created_button.set_sensitive(true);
                        created_button.set_label(if move_sources {
                            "Move here"
                        } else {
                            "Copy here"
                        });
                    }
                }
            });
        });
        let activate_confirm = confirm.clone();
        field.connect_activate(move |_| activate_confirm.emit_clicked());
        let escape = gtk::EventControllerKey::new();
        let escape_layer = layer.clone();
        let escape_overlay = window_overlay;
        let escape_root = blurred_root;
        let escape_creating = creating_destination;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if escape_creating.get() {
                    return glib::Propagation::Stop;
                }
                dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);

        refresh_transfer_suggestions(&field, &suggestions, &generation, base);
        field.grab_focus();
    }

    fn show_file_operation_progress(
        self: &Rc<Self>,
        total: usize,
        icon: &str,
        title_text: &str,
        subtitle_text: &str,
    ) {
        self.dismiss_delete_progress();
        let Some(window_overlay) = self
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.child())
            .and_downcast::<gtk::Overlay>()
        else {
            return;
        };
        let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = blurred_root.as_ref() {
            root.set_blurred(true);
        }

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("transfer-dialog");
        content.add_css_class("delete-progress-dialog");
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("transfer-header");
        let symbol = gtk::CenterBox::new();
        symbol.add_css_class("transfer-symbol");
        symbol.set_center_widget(Some(&crate::assets::primary_icon(icon, 20)));
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let title = gtk::Label::new(Some(title_text));
        title.add_css_class("transfer-title");
        title.set_xalign(0.0);
        let subtitle = gtk::Label::new(Some(subtitle_text));
        subtitle.add_css_class("transfer-subtitle");
        subtitle.set_xalign(0.0);
        heading.append(&title);
        heading.append(&subtitle);
        header.append(&symbol);
        header.append(&heading);
        content.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.add_css_class("transfer-body");
        let status = gtk::Label::new(Some(&format!("0 of {total} items")));
        status.add_css_class("delete-progress-status");
        status.set_xalign(0.0);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("delete-progress-bar");
        progress.set_fraction(0.0);
        body.append(&status);
        body.append(&progress);
        content.append(&body);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("transfer-actions");
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("transfer-cancel");
        actions.append(&spacer);
        actions.append(&cancel);
        content.append(&actions);

        let layer = modal_layer(&content);
        window_overlay.add_overlay(&layer);
        self.delete_progress.replace(Some(DeleteProgressView {
            layer,
            overlay: window_overlay,
            blurred_root,
            progress,
            status,
        }));
        let browser = self.browser.clone();
        cancel.connect_clicked(move |_| browser.cancel_file_operation());
        let escape = gtk::EventControllerKey::new();
        let escape_browser = self.browser.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                escape_browser.cancel_file_operation();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        if let Some(progress) = self.delete_progress.borrow().as_ref() {
            progress.layer.add_controller(escape);
        }
        cancel.grab_focus();
    }

    fn update_delete_progress(&self, completed: usize, total: usize) {
        let progress_view = self.delete_progress.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        view.status
            .set_text(&format!("{completed} of {total} items"));
        view.progress
            .set_fraction(completed as f64 / total.max(1) as f64);
    }

    fn dismiss_delete_progress(&self) {
        let Some(view) = self.delete_progress.take() else {
            return;
        };
        dismiss_modal_layer(&view.layer, &view.overlay, view.blurred_root.as_ref());
    }

    fn load_trash_summary(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let trash = gio::File::for_uri("trash:///");
            match summarize_trash(&trash).await {
                Ok(summary) if !summary.entries.is_empty() => {
                    if let Some(state) = weak.upgrade() {
                        state.show_empty_trash_confirmation(summary);
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    if let Some(state) = weak.upgrade() {
                        show_error_dialog(
                            &state.overlay,
                            "Unable to read Trash",
                            &error.to_string(),
                        );
                    }
                }
            }
        });
    }

    fn show_empty_trash_confirmation(self: &Rc<Self>, summary: TrashSummary) {
        let Some(window_overlay) = self
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.child())
            .and_downcast::<gtk::Overlay>()
        else {
            return;
        };
        let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = blurred_root.as_ref() {
            root.set_blurred(true);
        }

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("delete-confirmation");
        content.add_css_class("delete-confirmation-content");
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("delete-confirmation-header");
        let symbol = gtk::CenterBox::new();
        symbol.add_css_class("delete-confirmation-symbol");
        symbol.set_size_request(40, 40);
        symbol.set_center_widget(Some(&crate::assets::danger_icon(
            crate::assets::icons::TRASH,
            21,
        )));
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
        heading.set_hexpand(true);
        let title = gtk::Label::new(Some("Empty Trash?"));
        title.add_css_class("delete-confirmation-title");
        title.set_xalign(0.0);
        let subtitle = gtk::Label::new(Some(&format!(
            "{} · {} will be reclaimed",
            item_count_label(summary.item_count),
            format_file_size(summary.total_size)
        )));
        subtitle.add_css_class("delete-confirmation-subtitle");
        subtitle.set_xalign(0.0);
        heading.append(&title);
        heading.append(&subtitle);
        let close = gtk::Button::new();
        close.add_css_class("delete-confirmation-close");
        close.set_tooltip_text(Some("Cancel"));
        close.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            16,
        )));
        header.append(&symbol);
        header.append(&heading);
        header.append(&close);
        content.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("delete-confirmation-body");
        let explanation = gtk::Label::new(Some(
            "Everything in Trash will be permanently deleted. This action cannot be undone.",
        ));
        explanation.add_css_class("delete-confirmation-explanation");
        explanation.set_wrap(true);
        explanation.set_xalign(0.0);
        body.append(&explanation);
        content.append(&body);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("delete-confirmation-actions");
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("delete-confirmation-cancel");
        let empty = gtk::Button::with_label("Empty Trash");
        empty.add_css_class("delete-confirmation-delete");
        actions.append(&spacer);
        actions.append(&cancel);
        actions.append(&empty);
        content.append(&actions);

        let layer = modal_layer(&content);
        window_overlay.add_overlay(&layer);
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
        });
        let close_layer = layer.clone();
        let close_overlay = window_overlay.clone();
        let close_root = blurred_root.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
        });
        let empty_layer = layer.clone();
        let empty_overlay = window_overlay.clone();
        let empty_root = blurred_root.clone();
        let browser = self.browser.clone();
        empty.connect_clicked(move |_| {
            dismiss_modal_layer(&empty_layer, &empty_overlay, empty_root.as_ref());
            browser.delete(summary.entries.clone(), true);
        });
        let escape = gtk::EventControllerKey::new();
        let escape_layer = layer.clone();
        let escape_overlay = window_overlay;
        let escape_root = blurred_root;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        cancel.grab_focus();
    }

    fn show_delete_confirmation(self: &Rc<Self>, entries: Vec<FileEntry>, permanent: bool) {
        let Some(window_overlay) = self
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.child())
            .and_downcast::<gtk::Overlay>()
        else {
            return;
        };
        let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = blurred_root.as_ref() {
            root.set_blurred(true);
        }

        let count = entries.len();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("delete-confirmation");
        content.add_css_class("delete-confirmation-content");
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("delete-confirmation-header");
        let symbol = gtk::CenterBox::new();
        symbol.add_css_class("delete-confirmation-symbol");
        symbol.set_size_request(40, 40);
        symbol.set_hexpand(false);
        let symbol_icon = crate::assets::danger_icon(crate::assets::icons::TRASH, 21);
        symbol.set_center_widget(Some(&symbol_icon));
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
        heading.set_hexpand(true);
        let question = gtk::Label::new(Some(&if permanent {
            format!("Permanently delete {}?", item_count_label(count))
        } else {
            format!("Move {} to trash?", item_count_label(count))
        }));
        question.add_css_class("delete-confirmation-title");
        question.set_xalign(0.0);
        let subtitle = gtk::Label::new(Some(&entry_kind_summary(&entries)));
        subtitle.add_css_class("delete-confirmation-subtitle");
        subtitle.set_xalign(0.0);
        heading.append(&question);
        heading.append(&subtitle);
        let close = gtk::Button::new();
        close.add_css_class("delete-confirmation-close");
        close.set_tooltip_text(Some("Cancel"));
        close.set_child(Some(&crate::assets::text_icon(crate::assets::icons::X, 16)));
        header.append(&symbol);
        header.append(&heading);
        header.append(&close);
        content.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.add_css_class("delete-confirmation-body");
        let files = gtk::Box::new(gtk::Orientation::Vertical, 3);
        files.add_css_class("delete-confirmation-files");
        for entry in &entries {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("delete-confirmation-file");
            let icon = crate::assets::primary_icon(entry_icon(entry), 16);
            let name = gtk::Label::new(Some(&entry.display_name));
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_hexpand(true);
            name.set_xalign(0.0);
            name.set_tooltip_text(Some(&entry.location.display_path()));
            let metadata = gtk::Label::new(Some(&if entry.is_directory() {
                "Folder".to_owned()
            } else {
                match entry.size {
                    crate::model::MetadataValue::Known(size) => format_file_size(size),
                    crate::model::MetadataValue::Unknown
                    | crate::model::MetadataValue::Unavailable => "—".to_owned(),
                }
            }));
            metadata.add_css_class("delete-confirmation-file-metadata");
            row.append(&icon);
            row.append(&name);
            row.append(&metadata);
            files.append(&row);
        }
        let file_scroller = gtk::ScrolledWindow::builder()
            .child(&files)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(if count > 10 {
                gtk::PolicyType::Automatic
            } else {
                gtk::PolicyType::Never
            })
            .max_content_height(256)
            .propagate_natural_height(true)
            .build();
        file_scroller.add_css_class("delete-confirmation-list");
        body.append(&file_scroller);
        let explanation = gtk::Label::new(Some(if permanent {
            "These items will be permanently deleted. This action cannot be undone."
        } else {
            "The items will be moved to trash. You can restore them later."
        }));
        explanation.add_css_class("delete-confirmation-explanation");
        explanation.set_wrap(true);
        explanation.set_xalign(0.0);
        body.append(&explanation);
        content.append(&body);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("delete-confirmation-actions");
        let action_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        action_spacer.set_hexpand(true);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("delete-confirmation-cancel");
        let confirm_label = if permanent {
            format!("Permanently delete {}", item_count_label(count))
        } else {
            format!("Move {}", item_count_label(count))
        };
        let confirm_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        confirm_content.append(&crate::assets::danger_icon(crate::assets::icons::TRASH, 15));
        confirm_content.append(&gtk::Label::new(Some(&confirm_label)));
        let confirm = gtk::Button::builder().child(&confirm_content).build();
        confirm.add_css_class("delete-confirmation-delete");
        actions.append(&action_spacer);
        actions.append(&cancel);
        actions.append(&confirm);
        content.append(&actions);

        let layer = modal_layer(&content);
        window_overlay.add_overlay(&layer);
        let cancelled_layer = layer.clone();
        let cancelled_overlay = window_overlay.clone();
        let cancelled_root = blurred_root.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(
                &cancelled_layer,
                &cancelled_overlay,
                cancelled_root.as_ref(),
            );
        });
        let closed_layer = layer.clone();
        let closed_overlay = window_overlay.clone();
        let closed_root = blurred_root.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&closed_layer, &closed_overlay, closed_root.as_ref());
        });
        let confirmed_layer = layer.clone();
        let confirmed_overlay = window_overlay.clone();
        let confirmed_root = blurred_root.clone();
        let browser = self.browser.clone();
        confirm.connect_clicked(move |_| {
            dismiss_modal_layer(
                &confirmed_layer,
                &confirmed_overlay,
                confirmed_root.as_ref(),
            );
            browser.delete(entries.clone(), permanent);
        });
        let escape = gtk::EventControllerKey::new();
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        if permanent {
            cancel.grab_focus();
        } else {
            confirm.grab_focus();
        }
    }

    fn show_folder_properties(self: &Rc<Self>, location: &Location) {
        self.show_properties(location.clone(), None);
    }

    fn show_entry_properties(self: &Rc<Self>, entry: FileEntry) {
        self.show_properties(entry.location.clone(), Some(entry));
    }

    fn show_properties(self: &Rc<Self>, location: Location, entry: Option<FileEntry>) {
        let Some(window_overlay) = self
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.child())
            .and_downcast::<gtk::Overlay>()
        else {
            return;
        };
        let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = blurred_root.as_ref() {
            root.set_blurred(true);
        }
        let is_directory = entry.as_ref().is_none_or(FileEntry::is_directory);
        let name = entry
            .as_ref()
            .map(|entry| entry.display_name.clone())
            .unwrap_or_else(|| location.display_name());
        let icon_name = entry
            .as_ref()
            .map(entry_icon)
            .unwrap_or(crate::assets::icons::FOLDER);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("properties-dialog");
        content.add_css_class("properties-content");
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("properties-header");
        let icon = crate::assets::primary_icon(icon_name, 30);
        icon.add_css_class("properties-icon");
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
        heading.set_hexpand(true);
        let title = gtk::Label::new(Some(&name));
        title.add_css_class("properties-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_xalign(0.0);
        let kind = gtk::Label::new(Some(if is_directory { "Folder" } else { "File" }));
        kind.add_css_class("properties-kind");
        kind.set_xalign(0.0);
        heading.append(&title);
        heading.append(&kind);
        let close = gtk::Button::new();
        close.add_css_class("properties-close");
        close.set_tooltip_text(Some("Close properties"));
        close.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            15,
        )));
        header.append(&icon);
        header.append(&heading);
        header.append(&close);
        content.append(&header);

        let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details.add_css_class("properties-details");
        let location_value = properties_row(&details, "LOCATION", &compact_display_path(&location));
        location_value.set_tooltip_text(Some(&location.display_path()));
        let trash_root = is_trash_root(&location);
        let initial_size = if trash_root {
            "Calculating…".to_owned()
        } else {
            entry
                .as_ref()
                .and_then(|entry| match entry.size {
                    crate::model::MetadataValue::Known(size) => Some(format_file_size(size)),
                    crate::model::MetadataValue::Unknown
                    | crate::model::MetadataValue::Unavailable => None,
                })
                .unwrap_or_else(|| "—".to_owned())
        };
        let size = properties_row(&details, "SIZE", &initial_size);
        let modified = properties_row(
            &details,
            "MODIFIED",
            &entry
                .as_ref()
                .map(metadata_modified)
                .unwrap_or_else(|| "—".to_owned()),
        );
        let opens_with = properties_row(&details, "OPENS WITH", "—");
        let hidden = properties_row(
            &details,
            "HIDDEN",
            if name.starts_with('.') { "Yes" } else { "No" },
        );
        let _pinned = properties_row(&details, "PINNED", "No");
        content.append(&details);

        let permissions = gtk::Box::new(gtk::Orientation::Vertical, 8);
        permissions.add_css_class("properties-permissions");
        let permissions_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let permissions_title = gtk::Label::new(Some("PERMISSIONS"));
        permissions_title.add_css_class("properties-section-title");
        permissions_title.set_xalign(0.0);
        permissions_title.set_hexpand(true);
        let permissions_mode = gtk::Label::new(Some("—"));
        permissions_mode.add_css_class("properties-mode");
        permissions_header.append(&permissions_title);
        permissions_header.append(&permissions_mode);
        permissions.append(&permissions_header);
        let owner = permission_row(&permissions, "Owner");
        let group = permission_row(&permissions, "Group");
        let others = permission_row(&permissions, "Others");
        content.append(&permissions);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.add_css_class("properties-actions");
        let open = properties_action(crate::assets::icons::EXTERNAL_LINK, "Open");
        let rename = properties_action(crate::assets::icons::PENCIL, "Rename");
        rename.set_sensitive(entry.is_some());
        let pin = properties_action(crate::assets::icons::PIN, "Pin");
        pin.set_sensitive(false);
        pin.set_tooltip_text(Some("Pinned locations are planned"));
        let copy_path = properties_action(crate::assets::icons::COPY, "Copy path");
        actions.append(&open);
        actions.append(&rename);
        actions.append(&pin);
        actions.append(&copy_path);
        content.append(&actions);

        let layer = modal_layer(&content);
        window_overlay.add_overlay(&layer);
        let closing_layer = layer.clone();
        let closing_overlay = window_overlay.clone();
        let closing_root = blurred_root.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&closing_layer, &closing_overlay, closing_root.as_ref());
        });
        let opening_layer = layer.clone();
        let opening_overlay = window_overlay.clone();
        let opening_root = blurred_root.clone();
        let opening_location = location.clone();
        open.connect_clicked(move |_| {
            open_location(&opening_location, &opening_layer);
            dismiss_modal_layer(&opening_layer, &opening_overlay, opening_root.as_ref());
        });
        let renamed_layer = layer.clone();
        let renamed_overlay = window_overlay.clone();
        let renamed_root = blurred_root.clone();
        let weak = Rc::downgrade(self);
        rename.connect_clicked(move |_| {
            dismiss_modal_layer(&renamed_layer, &renamed_overlay, renamed_root.as_ref());
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    state.begin_rename();
                }
            });
        });
        let copied_location = location.clone();
        copy_path.connect_clicked(move |button| {
            if let Some(display) = gtk::gdk::Display::default() {
                display
                    .clipboard()
                    .set_text(&copied_location.display_path());
                button.set_label("Copied");
            }
        });
        let escape = gtk::EventControllerKey::new();
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay.clone();
        let escaped_root = blurred_root.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        layer.grab_focus();

        if trash_root {
            let weak_size = size.downgrade();
            glib::MainContext::default().spawn_local(async move {
                let summary = summarize_trash(&gio::File::for_uri("trash:///")).await;
                let Some(size) = weak_size.upgrade() else {
                    return;
                };
                match summary {
                    Ok(summary) => {
                        size.set_text(&format_file_size(summary.total_size));
                        size.set_tooltip_text(Some(&item_count_label(summary.item_count)));
                    }
                    Err(_) => size.set_text("Unavailable"),
                }
            });
        }

        let file = gio_file_for_location(&location);
        glib::MainContext::default().spawn_local(async move {
            let Ok(info) = file
                .query_info_future(
                    "standard::content-type,standard::is-hidden,standard::size,time::modified,unix::mode,owner::user,owner::group",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                )
                .await
            else {
                return;
            };
            if !is_directory {
                size.set_text(&format_file_size(info.size().max(0) as u64));
            }
            if let Some(time) = info.modification_date_time() {
                modified.set_text(
                    &time
                        .format("%Y-%m-%d %H:%M")
                        .map(|value| value.to_string())
                        .unwrap_or_else(|_| "—".to_owned()),
                );
            }
            hidden.set_text(if info.is_hidden() { "Yes" } else { "No" });
            if let Some(content_type) = info.content_type() {
                kind.set_text(&gio::content_type_get_description(&content_type));
                if let Some(app) = gio::AppInfo::default_for_type(&content_type, false) {
                    opens_with.set_text(&app.display_name());
                }
            }
            let mode = info.attribute_uint32("unix::mode");
            if mode != 0 {
                permissions_mode.set_text(&format_permissions(mode));
                set_permission_row(&owner, mode, 6);
                set_permission_row(&group, mode, 3);
                set_permission_row(&others, mode, 0);
            }
            owner.0.set_text(info.attribute_string("owner::user").as_deref().unwrap_or("—"));
            group.0.set_text(info.attribute_string("owner::group").as_deref().unwrap_or("—"));
        });
    }

    fn begin_rename(self: &Rc<Self>) -> bool {
        self.cancel_new_entry();
        self.sync_mode_selection();
        let Some((depth, source_position, entry)) = self.browser.rename_item() else {
            return false;
        };
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self
                .mode_views
                .borrow()
                .begin_rename(depth, source_position, &entry);
        }
        self.cancel_rename();
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return false;
        };
        let Some(filtered_position) = filtered_position_for_source(column, source_position) else {
            return false;
        };
        let row = column.bound_rows.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == filtered_position).then(|| bound.row.upgrade())?
        });
        let Some(row) = row else {
            return false;
        };
        let Some(icon) = row.first_child() else {
            return false;
        };
        let Some(middle) = icon.next_sibling().and_downcast::<gtk::Overlay>() else {
            return false;
        };
        let Some(editor) = middle.child().and_downcast::<gtk::Box>() else {
            return false;
        };
        let Some(label) = editor.first_child().and_downcast::<gtk::Label>() else {
            return false;
        };
        let Some(field) = label.next_sibling().and_downcast::<gtk::Entry>() else {
            return false;
        };
        let Some(spacer) = field.next_sibling().and_downcast::<gtk::Box>() else {
            return false;
        };
        field.remove_css_class("error");
        field.set_tooltip_text(None);
        field.set_sensitive(true);
        field.set_text(&entry.display_name);
        label.set_visible(false);
        spacer.set_visible(false);
        field.set_visible(true);
        field.grab_focus();
        field.select_region(0, rename_stem_end(&entry.display_name));
        self.active_rename.replace(Some(ActiveRename {
            entry,
            field,
            label,
            spacer,
        }));
        true
    }

    fn cancel_rename(&self) -> bool {
        if self.mode_views.borrow().cancel_rename() {
            return true;
        }
        let Some(rename) = self.active_rename.take() else {
            return false;
        };
        rename.field.remove_css_class("error");
        rename.field.set_tooltip_text(None);
        rename.field.set_visible(false);
        rename.field.set_sensitive(true);
        rename.label.set_visible(true);
        rename.spacer.set_visible(true);
        true
    }

    fn submit_rename(self: &Rc<Self>, field: &gtk::Entry) {
        let mut active = self.active_rename.borrow_mut();
        let Some(rename) = active.as_mut().filter(|rename| rename.field == *field) else {
            return;
        };
        let new_name = field.text().to_string();
        if new_name == rename.entry.display_name {
            drop(active);
            self.cancel_rename();
            self.browser.focus_active();
            return;
        }
        field.remove_css_class("error");
        field.set_tooltip_text(None);
        field.set_sensitive(false);
        self.browser.rename(rename.entry.clone(), new_name);
    }

    fn begin_location_edit(&self) {
        self.clear_location_error();
        self.location_stack.set_visible_child_name("entry");
        self.location_entry.grab_focus();
        self.location_entry.select_region(0, -1);
    }

    fn cancel_location_edit(&self) {
        self.restore_location_text();
        self.clear_location_error();
        self.location_stack.set_visible_child_name("breadcrumbs");
        self.browser.focus_active();
    }

    fn submit_location(self: &Rc<Self>) {
        let input = self.location_entry.text();
        match self.browser.navigate_input(input.as_str()) {
            Ok(()) => {
                self.clear_location_error();
                self.location_stack.set_visible_child_name("breadcrumbs");
                self.browser.focus_active();
            }
            Err(error) => {
                self.location_entry.add_css_class("error");
                self.location_error.set_text(&error.to_string());
                self.location_error.set_visible(true);
                self.location_entry.grab_focus();
            }
        }
    }

    fn restore_location_text(&self) {
        if let Some(location) = self.browser.active_location() {
            self.location_entry.set_text(&location.display_path());
        }
    }

    fn sync_active_location(self: &Rc<Self>) {
        if let Some(location) = self.browser.active_location() {
            self.set_location(&location);
        }
    }

    fn set_location(self: &Rc<Self>, location: &Location) {
        self.location_entry.set_text(&location.display_path());
        while let Some(child) = self.breadcrumbs.first_child() {
            self.breadcrumbs.remove(&child);
        }

        let home = Location::local(glib::home_dir());
        let mut locations = location.breadcrumbs();
        if let Some(home_index) = locations.iter().position(|crumb| crumb == &home) {
            locations.drain(..home_index);
        }
        let starts_at_root = locations
            .first()
            .and_then(Location::native_path)
            .is_some_and(|path| path == Path::new("/"));
        let last = locations.len().saturating_sub(1);
        for (index, crumb) in locations.into_iter().enumerate() {
            if index > 0 && !(starts_at_root && index == 1) {
                let separator = gtk::Label::new(Some("/"));
                separator.add_css_class("breadcrumb-separator");
                self.breadcrumbs.append(&separator);
            }

            let label = if crumb == home {
                "~".to_owned()
            } else {
                crumb.display_name()
            };
            if index == last {
                let current = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                current.add_css_class("current-breadcrumb");
                let current_label = gtk::Label::new(Some(&label));
                current_label.add_css_class("breadcrumb");
                current_label.add_css_class("current");
                current_label.set_tooltip_text(Some(&crumb.display_path()));
                let copy = gtk::Button::builder().tooltip_text("Copy path").build();
                let copy_icon = crate::assets::primary_icon(crate::assets::icons::COPY, 16);
                copy.set_child(Some(&copy_icon));
                copy.add_css_class("copy-path");
                copy.set_has_frame(false);
                copy.set_cursor_from_name(Some("pointer"));
                let copied_path = location.display_path();
                let feedback_generation = Rc::new(Cell::new(0_u64));
                copy.connect_clicked(move |button| {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&copied_path);
                    }
                    let generation = feedback_generation.get().saturating_add(1);
                    feedback_generation.set(generation);
                    crate::assets::set_primary_icon(&copy_icon, crate::assets::icons::CHECK);
                    button.set_tooltip_text(Some("Path copied"));
                    let button = button.clone();
                    let copy_icon = copy_icon.clone();
                    let feedback_generation = feedback_generation.clone();
                    glib::timeout_add_local_once(Duration::from_secs(2), move || {
                        if feedback_generation.get() == generation {
                            crate::assets::set_primary_icon(&copy_icon, crate::assets::icons::COPY);
                            button.set_tooltip_text(Some("Copy path"));
                        }
                    });
                });
                current.append(&current_label);
                current.append(&copy);
                self.breadcrumbs.append(&current);
            } else {
                let button = gtk::Button::with_label(&label);
                button.add_css_class("breadcrumb");
                if crumb
                    .native_path()
                    .is_some_and(|path| path == Path::new("/"))
                {
                    button.add_css_class("breadcrumb-root");
                }
                button.set_has_frame(false);
                button.set_tooltip_text(Some(&crumb.display_path()));
                button.set_cursor_from_name(Some("pointer"));
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(state) = weak.upgrade() {
                        state.browser.navigate(crumb.clone());
                    }
                });
                self.breadcrumbs.append(&button);
            }
        }
        self.location_stack.set_visible_child_name("breadcrumbs");
    }

    fn clear_location_error(&self) {
        self.location_entry.remove_css_class("error");
        self.location_error.set_visible(false);
        self.location_error.set_text("");
    }

    fn handle(self: &Rc<Self>, event: BrowserEvent) {
        self.mode_views.borrow_mut().handle(&event);
        match event {
            BrowserEvent::Reset => {
                self.truncate(0);
                self.clear_location_error();
            }
            BrowserEvent::ColumnsTruncated { len } => {
                self.truncate(len);
                self.sync_active_location();
            }
            BrowserEvent::ColumnAdded { depth, location } => {
                self.set_location(&location);
                self.clear_location_error();
                self.append_column(depth, &location);
            }
            BrowserEvent::EntriesInserted { depth, insertions } => {
                let render_started = Instant::now();
                let entry_count = insertions
                    .iter()
                    .map(|insertion| insertion.entries.len())
                    .sum();
                if let Some(column) = self.columns.borrow().get(depth).cloned() {
                    if entry_count > 0 {
                        column.presentation.show_content();
                    }
                    for insertion in insertions {
                        let labels: Vec<_> =
                            insertion.entries.iter().map(entry_model_value).collect();
                        let labels: Vec<_> = labels.iter().map(String::as_str).collect();
                        column.model.splice(insertion.position as u32, 0, &labels);
                    }
                    let count = column.entry_count.get() + entry_count;
                    column.entry_count.set(count);
                    set_filter_placeholder(&column, count);
                    crate::metrics::mark_batch_rendered(entry_count, render_started);
                }
            }
            BrowserEvent::EntriesReplaced { depth, entries } => {
                if self.mode_views.borrow().mode() == BrowserMode::Columns
                    && let Some(column) = self.columns.borrow().get(depth).cloned()
                {
                    if !entries.is_empty() {
                        column.presentation.show_content();
                    }
                    let labels: Vec<_> = entries.iter().map(entry_model_value).collect();
                    let labels: Vec<_> = labels.iter().map(String::as_str).collect();
                    column.model.splice(0, column.model.n_items(), &labels);
                    column.entry_count.set(entries.len());
                    set_filter_placeholder(&column, entries.len());
                }
            }
            BrowserEvent::SortingStarted { depth } => {
                self.overlay.set_cursor_from_name(Some("wait"));
                if let Some(column) = self.columns.borrow().get(depth) {
                    column.spinner.set_tooltip_text(Some("Sorting…"));
                    column.spinner.set_visible(true);
                    column.spinner.start();
                }
            }
            BrowserEvent::SortingFinished { depth } => {
                self.overlay.set_cursor(None::<&gtk::gdk::Cursor>);
                if let Some(column) = self.columns.borrow().get(depth) {
                    column.spinner.stop();
                    column.spinner.set_visible(false);
                    column.spinner.set_tooltip_text(None);
                }
            }
            BrowserEvent::EntriesSpliced {
                depth,
                splices,
                selected,
            } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    let mut count = column.entry_count.get();
                    for splice in splices {
                        let labels: Vec<_> = splice.entries.iter().map(entry_model_value).collect();
                        let labels: Vec<_> = labels.iter().map(String::as_str).collect();
                        column
                            .model
                            .splice(splice.position as u32, splice.removed as u32, &labels);
                        count = count
                            .saturating_sub(splice.removed)
                            .saturating_add(splice.entries.len());
                    }
                    column.entry_count.set(count);
                    set_filter_placeholder(column, count);
                    set_column_selection(
                        column,
                        selected
                            .and_then(|position| filtered_position_for_source(column, position))
                            .unwrap_or(gtk::INVALID_LIST_POSITION),
                    );
                    if count == 0 {
                        column.presentation.show_empty();
                    } else {
                        column.presentation.show_content();
                    }
                }
            }
            BrowserEvent::ColumnReloaded { depth } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    column.model.splice(0, column.model.n_items(), &[]);
                    column.entry_count.set(0);
                    set_filter_placeholder(column, 0);
                    column.spinner.set_visible(true);
                    column.spinner.start();
                    column.presentation.show_loading();
                }
            }
            BrowserEvent::LoadFinished { depth } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    column.spinner.stop();
                    column.spinner.set_visible(false);
                    if column.entry_count.get() == 0 {
                        column.presentation.show_empty();
                    } else {
                        column.presentation.show_content();
                    }
                }
            }
            BrowserEvent::LoadFailed { depth, message } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    column.spinner.stop();
                    column.spinner.set_visible(false);
                    column
                        .presentation
                        .show_error(&format!("Unable to read this directory\n{message}"));
                }
            }
            BrowserEvent::PeekStarted { location } => self.append_peek(&location),
            BrowserEvent::PeekEntriesAdded { entries } => {
                if let Some(peek) = self.peek.borrow().as_ref() {
                    if !entries.is_empty() {
                        peek.presentation.show_content();
                    }
                    append_peek_entries(peek, entries, self.peek_behavior.item_limit);
                }
            }
            BrowserEvent::PeekFinished => {
                if let Some(peek) = self.peek.borrow().as_ref() {
                    peek.spinner.stop();
                    peek.spinner.set_visible(false);
                    if peek.entry_count.get() == 0 {
                        peek.presentation.show_empty();
                    } else {
                        peek.presentation.show_content();
                    }
                }
            }
            BrowserEvent::PeekFailed { message } => {
                if let Some(peek) = self.peek.borrow().as_ref() {
                    peek.spinner.stop();
                    peek.spinner.set_visible(false);
                    peek.presentation
                        .show_error(&format!("Unable to read this directory\n{message}"));
                }
            }
            BrowserEvent::PeekClosed => self.close_peek_visual(),
            BrowserEvent::SelectionSetChanged {
                depth,
                positions,
                focused,
            } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    let filtered_positions: Vec<_> = positions
                        .into_iter()
                        .filter_map(|position| filtered_position_for_source(column, position))
                        .collect();
                    set_column_selections(column, &filtered_positions);
                    if let Some(focused) = filtered_position_for_source(column, focused) {
                        column
                            .list
                            .scroll_to(focused, gtk::ListScrollFlags::FOCUS, None);
                    }
                    if self.mode_views.borrow().mode() == BrowserMode::Columns {
                        column.list.grab_focus();
                    }
                }
            }
            BrowserEvent::FocusChanged { depth, position } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    if let Some(filtered_position) =
                        position.and_then(|position| filtered_position_for_source(column, position))
                    {
                        set_column_selection(column, filtered_position);
                        column
                            .list
                            .scroll_to(filtered_position, gtk::ListScrollFlags::FOCUS, None);
                    }
                    if self.mode_views.borrow().mode() == BrowserMode::Columns {
                        column.list.grab_focus();
                    }
                }
            }
            BrowserEvent::PreviewRequested { .. } => {}
            BrowserEvent::OpenRequested { location } => {
                open_location(&location, &self.overlay);
            }
            BrowserEvent::RenameCompleted => {
                self.cancel_rename();
                self.browser.focus_active();
            }
            BrowserEvent::RenameFailed { message } => {
                if let Some(rename) = self.active_rename.borrow().as_ref() {
                    rename.field.set_sensitive(true);
                    rename.field.add_css_class("error");
                    rename.field.set_tooltip_text(Some(&message));
                    rename.field.grab_focus();
                }
            }
            BrowserEvent::DeletionStarted { total } => self.show_file_operation_progress(
                total,
                crate::assets::icons::TRASH,
                "Deleting items",
                "This may take a moment",
            ),
            BrowserEvent::DeletionProgress { completed, total } => {
                self.update_delete_progress(completed, total);
            }
            BrowserEvent::DeletionFinished => self.dismiss_delete_progress(),
            BrowserEvent::RestorationStarted { total } => self.show_file_operation_progress(
                total,
                crate::assets::icons::FOLDER,
                "Restoring items",
                "Items are being returned to their original locations",
            ),
            BrowserEvent::RestorationProgress { completed, total } => {
                self.update_delete_progress(completed, total);
            }
            BrowserEvent::RestorationFinished => self.dismiss_delete_progress(),
            BrowserEvent::OperationFailed { message } => {
                show_error_dialog(&self.overlay, "Unable to complete operation", &message);
            }
            BrowserEvent::OperationCompletedWithErrors { message } => {
                show_error_dialog(&self.overlay, "Completed with errors", &message);
            }
            BrowserEvent::NavigationRejected { message } => {
                show_error_dialog(&self.overlay, "Unable to open directory", &message);
            }
        }
        self.refresh_active_path_rows();
    }

    fn sync_column_models(&self) {
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            let Some(snapshot) = self.browser.column_snapshot(depth) else {
                continue;
            };
            let labels = snapshot
                .entries
                .iter()
                .map(entry_model_value)
                .collect::<Vec<_>>();
            let labels = labels.iter().map(String::as_str).collect::<Vec<_>>();
            column.model.splice(0, column.model.n_items(), &labels);
            column.entry_count.set(snapshot.entries.len());
            set_filter_placeholder(column, snapshot.entries.len());
            let positions = snapshot
                .selected_positions
                .into_iter()
                .filter_map(|position| filtered_position_for_source(column, position))
                .collect::<Vec<_>>();
            set_column_selections(column, &positions);
        }
    }

    fn refresh_active_path_rows(&self) {
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            let active = self
                .browser
                .active_child_position(depth)
                .and_then(|position| filtered_position_for_source(column, position));
            column.bound_rows.borrow_mut().retain(|bound| {
                let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                    return false;
                };
                set_active_path_style(&row, active == Some(item.position()));
                true
            });
        }
    }

    fn append_column(self: &Rc<Self>, depth: usize, location: &Location) {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("directory-column");
        column.set_hexpand(true);
        column.set_vexpand(true);
        let pane_motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        pane_motion.connect_enter(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.hovered_column.set(Some(depth));
            }
        });
        let weak = Rc::downgrade(self);
        pane_motion.connect_leave(move |_| {
            if let Some(state) = weak.upgrade()
                && state.hovered_column.get() == Some(depth)
            {
                state.hovered_column.set(None);
            }
        });
        column.add_controller(pane_motion);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("column-header");
        let heading = gtk::Label::new(Some(&location.display_name()));
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        heading.set_tooltip_text(Some(&location.display_path()));
        let spinner = gtk::Spinner::new();
        spinner.start();
        header.append(&heading);
        header.append(&spinner);
        let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header_actions.add_css_class("column-header-actions");
        header_actions.append(&column_sort_direction_toggle(&self.browser, depth));
        header_actions.append(&column_sort_menu(&self.browser, depth));

        let filter_entry = gtk::Entry::builder()
            .placeholder_text("Filter 0 items…")
            .has_frame(false)
            .hexpand(true)
            .build();
        filter_entry.add_css_class("column-filter-entry");
        let filter_icon = crate::assets::primary_icon(crate::assets::icons::FUNNEL, 16);
        let filter_control = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        filter_control.add_css_class("column-filter");
        filter_control.append(&filter_icon);
        filter_control.append(&filter_entry);
        let filter_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .child(&filter_control)
            .build();
        let filter_button = gtk::ToggleButton::builder()
            .tooltip_text("Filter this pane (Ctrl+F)")
            .build();
        filter_button.set_child(Some(&crate::assets::text_icon(
            crate::assets::icons::FUNNEL,
            16,
        )));
        filter_button.add_css_class("column-header-action");
        let shown_filter = filter_revealer.clone();
        let focused_filter = filter_entry.clone();
        filter_button.connect_toggled(move |button| {
            shown_filter.set_reveal_child(button.is_active());
            if button.is_active() {
                focused_filter.grab_focus();
            } else {
                focused_filter.set_text("");
            }
        });
        header_actions.append(&filter_button);
        if depth > 0 {
            let close = gtk::Button::builder()
                .tooltip_text("Close this pane")
                .build();
            close.set_child(Some(&crate::assets::text_icon(crate::assets::icons::X, 16)));
            close.add_css_class("column-header-action");
            let weak_browser = Rc::downgrade(&self.browser);
            close.connect_clicked(move |_| {
                if let Some(browser) = weak_browser.upgrade() {
                    browser.close_column(depth);
                }
            });
            header_actions.append(&close);
        }
        header.append(&header_actions);
        column.append(&header);
        column.append(&filter_revealer);

        let entry_count = Rc::new(Cell::new(0));
        let model = gtk::StringList::new(&[]);
        let filter_query = Rc::new(RefCell::new(String::new()));
        let query = filter_query.clone();
        let filter = gtk::CustomFilter::new(move |item| {
            let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
                return false;
            };
            let query = query.borrow();
            query.is_empty()
                || model_display_name(&item.string())
                    .to_lowercase()
                    .contains(query.as_str())
        });
        let filtered_model = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
        let selection = gtk::MultiSelection::new(Some(filtered_model.clone()));
        let syncing_selection = Rc::new(Cell::new(false));
        let modified_selection = Rc::new(Cell::new(false));
        let focused_filtered = Rc::new(Cell::new(None::<u32>));
        let weak_browser = Rc::downgrade(&self.browser);
        let source_for_selection = model.clone();
        let filtered_for_selection = filtered_model.clone();
        let syncing_selection_changed = syncing_selection.clone();
        let focused_filtered_changed = focused_filtered.clone();
        selection.connect_selection_changed(move |selection, position, count| {
            if syncing_selection_changed.get() {
                return;
            }
            let filtered_positions = bitset_positions(&selection.selection());
            let source_positions: Vec<_> = filtered_positions
                .iter()
                .filter_map(|position| {
                    source_position_for_filtered(
                        &source_for_selection,
                        &filtered_for_selection,
                        *position,
                    )
                })
                .collect();
            let changed_end = position.saturating_add(count);
            let focused = filtered_positions
                .iter()
                .rev()
                .copied()
                .find(|candidate| *candidate >= position && *candidate < changed_end)
                .or_else(|| {
                    focused_filtered_changed
                        .get()
                        .filter(|candidate| filtered_positions.contains(candidate))
                })
                .or_else(|| filtered_positions.last().copied());
            focused_filtered_changed.set(focused);
            let focused_source = focused.and_then(|position| {
                source_position_for_filtered(
                    &source_for_selection,
                    &filtered_for_selection,
                    position,
                )
            });
            if let Some(browser) = weak_browser.upgrade() {
                browser.set_selection(depth, &source_positions, focused_source);
            }
        });
        filter_entry.connect_changed(move |entry| {
            *filter_query.borrow_mut() = entry.text().to_lowercase();
            filter.changed(gtk::FilterChange::Different);
        });

        let factory = gtk::SignalListItemFactory::new();
        let bound_rows: Rc<RefCell<Vec<BoundRow>>> = Rc::new(RefCell::new(Vec::new()));
        let rows_for_setup = bound_rows.clone();
        let weak_state = Rc::downgrade(self);
        let modified_selection_for_rows = modified_selection.clone();
        let selection_for_rows = selection.clone();
        let mouse_selection_anchor = Rc::new(Cell::new(None::<u32>));
        let source_for_hover = model.clone();
        let filtered_for_hover = filtered_model.clone();
        let mouse_selection_anchor_for_background = mouse_selection_anchor.clone();
        factory.connect_setup(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("file-row");
            let icon = gtk::Image::new();
            icon.add_css_class("file-icon");
            icon.set_pixel_size(17);
            let label = gtk::Label::builder()
                .halign(gtk::Align::Fill)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            let rename = gtk::Entry::new();
            rename.add_css_class("inline-rename");
            rename.set_hexpand(true);
            rename.set_visible(false);
            rename.connect_changed(|field| {
                update_basename_validation(field);
            });
            let weak_state_for_rename = weak_state.clone();
            rename.connect_activate(move |field| {
                if let Some(state) = weak_state_for_rename.upgrade() {
                    state.submit_rename(field);
                }
            });
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.add_css_class("file-row-spacer");
            let editor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            editor.append(&label);
            editor.append(&rename);
            editor.append(&spacer);
            let size = gtk::Label::new(None);
            size.add_css_class("file-size");
            size.set_halign(gtk::Align::End);
            size.set_valign(gtk::Align::Center);
            size.set_xalign(1.0);
            let middle = gtk::Overlay::new();
            middle.set_hexpand(true);
            middle.set_child(Some(&editor));
            middle.add_overlay(&size);
            let chevron = crate::assets::primary_icon(crate::assets::icons::CHEVRON_RIGHT, 15);
            chevron.add_css_class("file-chevron");
            row.append(&icon);
            row.append(&middle);
            row.append(&chevron);
            let motion = gtk::EventControllerMotion::new();
            let list_item = item.clone();
            let anchor: gtk::Widget = row.clone().upcast();
            let weak_state_for_enter = weak_state.clone();
            let source_for_enter = source_for_hover.clone();
            let filtered_for_enter = filtered_for_hover.clone();
            motion.connect_enter(move |_, _, _| {
                if let Some(state) = weak_state_for_enter.upgrade() {
                    let source_position = source_position_for_filtered(
                        &source_for_enter,
                        &filtered_for_enter,
                        list_item.position(),
                    );
                    let entry = source_position
                        .and_then(|position| state.browser.entry_at(depth, position));
                    if let Some(entry) = entry {
                        if entry.is_directory() {
                            state.schedule_peek(depth, entry.location, anchor.clone());
                        } else {
                            cancel_source(&state.pending_peek);
                            state.browser.close_peek();
                        }
                    }
                }
            });
            let weak_state_for_leave = weak_state.clone();
            motion.connect_leave(move |_| {
                if let Some(state) = weak_state_for_leave.upgrade() {
                    state.schedule_close_peek();
                }
            });
            row.add_controller(motion);

            let drag = gtk::DragSource::builder()
                .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
                .build();
            let weak_state_for_drag = weak_state.clone();
            let dragged_item = item.clone();
            let source_for_drag = source_for_hover.clone();
            let filtered_for_drag = filtered_for_hover.clone();
            drag.connect_prepare(move |source, x, y| {
                let state = weak_state_for_drag.upgrade()?;
                let source_position = source_position_for_filtered(
                    &source_for_drag,
                    &filtered_for_drag,
                    dragged_item.position(),
                )?;
                let entry = state.browser.entry_at(depth, source_position)?;
                let selected = state.browser.selected_entries();
                let entries = if selected
                    .iter()
                    .any(|selected| selected.location == entry.location)
                {
                    selected
                } else {
                    vec![entry]
                };
                let paintable = gtk::WidgetPaintable::new(source.widget().as_ref());
                source.set_icon(Some(&paintable), x.round() as i32, y.round() as i32);
                file_drag_content(&entries)
            });
            let dragged_row = row.clone();
            drag.connect_drag_begin(move |_, _| dragged_row.add_css_class("dragging"));
            let dragged_row = row.clone();
            drag.connect_drag_end(move |_, _, _| dragged_row.remove_css_class("dragging"));
            row.add_controller(drag);

            let drop = gtk::DropTarget::new(
                gtk::gdk::FileList::static_type(),
                gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
            );
            drop.connect_enter(|target, _, _| file_drop_action(target));
            drop.connect_motion(|target, _, _| file_drop_action(target));
            let weak_state_for_accept = weak_state.clone();
            let accepted_item = item.clone();
            let source_for_accept = source_for_hover.clone();
            let filtered_for_accept = filtered_for_hover.clone();
            drop.connect_accept(move |_, offered| {
                let Some(state) = weak_state_for_accept.upgrade() else {
                    return false;
                };
                let entry = source_position_for_filtered(
                    &source_for_accept,
                    &filtered_for_accept,
                    accepted_item.position(),
                )
                .and_then(|position| state.browser.entry_at(depth, position));
                entry.is_some_and(|entry| {
                    entry.is_directory()
                        && offered
                            .formats()
                            .contains_type(gtk::gdk::FileList::static_type())
                })
            });
            let weak_state_for_drop = weak_state.clone();
            let dropped_item = item.clone();
            let source_for_drop = source_for_hover.clone();
            let filtered_for_drop = filtered_for_hover.clone();
            drop.connect_drop(move |target, value, _, _| {
                let Some(state) = weak_state_for_drop.upgrade() else {
                    return false;
                };
                let Some(destination) = source_position_for_filtered(
                    &source_for_drop,
                    &filtered_for_drop,
                    dropped_item.position(),
                )
                .and_then(|position| state.browser.entry_at(depth, position))
                .filter(FileEntry::is_directory)
                .map(|entry| entry.location) else {
                    return false;
                };
                transfer_dropped_files(&state, target, value, destination)
            });
            row.add_controller(drop);

            let selection_click = gtk::GestureClick::new();
            selection_click.set_button(1);
            selection_click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let clicked_item = item.clone();
            let selection_for_click = selection_for_rows.clone();
            let selection_anchor_for_click = mouse_selection_anchor.clone();
            let modified_for_click = modified_selection_for_rows.clone();
            let weak_state_for_click = weak_state.clone();
            let source_for_click = source_for_hover.clone();
            let filtered_for_click = filtered_for_hover.clone();
            selection_click.connect_pressed(move |gesture, press_count, _, _| {
                let position = clicked_item.position();
                if position == gtk::INVALID_LIST_POSITION {
                    return;
                }
                let modifiers = gesture.current_event_state();
                let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
                let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
                let preserve_group = !control
                    && !shift
                    && should_preserve_drag_selection(
                        selection_for_click.is_selected(position),
                        selection_for_click.selection().size(),
                    );
                modified_for_click.set(control || shift);
                if shift {
                    let anchor = selection_anchor_for_click.get().unwrap_or(position);
                    let start = anchor.min(position);
                    let count = anchor.max(position).saturating_sub(start) + 1;
                    selection_for_click.select_range(start, count, true);
                } else if control {
                    selection_anchor_for_click.set(Some(position));
                    if selection_for_click.is_selected(position) {
                        selection_for_click.unselect_item(position);
                    } else {
                        selection_for_click.select_item(position, false);
                    }
                } else {
                    selection_anchor_for_click.set(Some(position));
                    if !preserve_group {
                        selection_for_click.select_item(position, true);
                    }
                }
                if control || shift {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
                modified_for_click.set(false);

                let source_position =
                    source_position_for_filtered(&source_for_click, &filtered_for_click, position);
                if let (Some(state), Some(source_position)) =
                    (weak_state_for_click.upgrade(), source_position)
                {
                    if press_count == 2 {
                        state.browser.activate(depth, source_position);
                    } else if !control && !shift && !preserve_group {
                        let entry = state.browser.entry_at(depth, source_position);
                        if entry.as_ref().is_some_and(|entry| {
                            entry_responds_to_single_click(entry, state.single_click_previews.get())
                        }) {
                            state.browser.preview(depth, source_position);
                        }
                    }
                }
            });
            row.add_controller(selection_click);
            item.set_child(Some(&row));
            let weak_item = glib::WeakRef::new();
            weak_item.set(Some(item));
            let weak_row = glib::WeakRef::new();
            weak_row.set(Some(&row));
            rows_for_setup.borrow_mut().push(BoundRow {
                item: weak_item,
                row: weak_row,
            });
        });
        let source_for_bind = model.clone();
        let filtered_for_bind = filtered_model.clone();
        let weak_state_for_bind = Rc::downgrade(self);
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(value) = item.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            let Some(row) = item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(icon) = row.first_child().and_downcast::<gtk::Image>() else {
                return;
            };
            let Some(middle) = icon.next_sibling().and_downcast::<gtk::Overlay>() else {
                return;
            };
            let Some(editor) = middle.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(label) = editor.first_child().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(rename) = label.next_sibling().and_downcast::<gtk::Entry>() else {
                return;
            };
            let Some(spacer) = rename.next_sibling().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(size) = middle.last_child().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(chevron) = middle.next_sibling().and_downcast::<gtk::Image>() else {
                return;
            };
            label.set_label(model_display_name(&value.string()));
            rename.set_visible(false);
            label.set_visible(true);
            spacer.set_visible(true);
            let source_position =
                source_position_for_filtered(&source_for_bind, &filtered_for_bind, item.position());
            let state = weak_state_for_bind.upgrade();
            let browser = state.as_ref().map(|state| &state.browser);
            let entry = source_position.and_then(|position| browser?.entry_at(depth, position));
            let active = source_position.is_some_and(|position| {
                browser
                    .as_ref()
                    .and_then(|browser| browser.active_child_position(depth))
                    == Some(position)
            });
            set_active_path_style(&row, active);
            set_cut_path_style(
                &row,
                entry.as_ref().is_some_and(|entry| {
                    state
                        .as_ref()
                        .is_some_and(|state| state.cut_locations.borrow().contains(&entry.location))
                }),
            );
            if let Some(entry) = entry.as_ref() {
                super::thumbnail::set_thumbnail_or_icon(&icon, entry, entry_icon(entry), 17, 17);
                icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
                chevron.set_visible(entry.is_directory());
            } else {
                super::thumbnail::show_fallback_icon(&icon, crate::assets::icons::DOCUMENTS, 17);
                icon.set_opacity(0.72);
                chevron.set_visible(false);
            }
            let size_text = entry
                .filter(|entry| !entry.is_directory())
                .and_then(|entry| match entry.size {
                    crate::model::MetadataValue::Known(bytes) => Some(format_file_size(bytes)),
                    crate::model::MetadataValue::Unknown
                    | crate::model::MetadataValue::Unavailable => None,
                })
                .unwrap_or_default();
            size.set_label(&size_text);
        });

        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("file-list");
        list.set_enable_rubberband(false);
        list.set_single_click_activate(false);
        list.set_vexpand(true);

        let marquee_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        marquee_box.add_css_class("file-marquee");
        marquee_box.set_can_target(false);
        marquee_box.set_halign(gtk::Align::Start);
        marquee_box.set_valign(gtk::Align::Start);
        marquee_box.set_visible(false);
        self.overlay.add_overlay(&marquee_box);

        let marquee_active = Rc::new(Cell::new(false));
        let marquee_origin = Rc::new(Cell::new((0.0, 0.0)));
        let marquee_initial = Rc::new(RefCell::new(gtk::Bitset::new_empty()));
        let marquee_modifiers = Rc::new(Cell::new((false, false)));
        let marquee = gtk::GestureDrag::new();
        marquee.set_button(1);
        marquee.set_propagation_phase(gtk::PropagationPhase::Capture);
        let active_for_begin = marquee_active.clone();
        let origin_for_begin = marquee_origin.clone();
        let initial_for_begin = marquee_initial.clone();
        let modifiers_for_begin = marquee_modifiers.clone();
        let selection_for_begin = selection.clone();
        let marquee_box_for_begin = marquee_box.clone();
        marquee.connect_drag_begin(move |gesture, x, y| {
            let starts_on_row = gesture
                .widget()
                .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
                .is_some_and(is_file_row_target);
            let force_marquee = gesture
                .current_event_state()
                .contains(gtk::gdk::ModifierType::ALT_MASK);
            let can_start = force_marquee || !starts_on_row;
            active_for_begin.set(can_start);
            if !can_start {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            marquee_box_for_begin.set_visible(true);
            origin_for_begin.set((x, y));
            initial_for_begin.replace(selection_for_begin.selection().copy());
            let modifiers = gesture.current_event_state();
            modifiers_for_begin.set((
                modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK),
                modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
            ));
        });
        let active_for_update = marquee_active.clone();
        let origin_for_update = marquee_origin.clone();
        let initial_for_update = marquee_initial.clone();
        let modifiers_for_update = marquee_modifiers.clone();
        let selection_for_marquee = selection.clone();
        let rows_for_marquee = bound_rows.clone();
        let list_for_marquee = list.clone();
        let overlay_for_marquee = self.overlay.clone();
        let marquee_box_for_update = marquee_box.clone();
        marquee.connect_drag_update(move |_, offset_x, offset_y| {
            if !active_for_update.get() {
                return;
            }
            let (origin_x, origin_y) = origin_for_update.get();
            let current_x = origin_x + offset_x;
            let current_y = origin_y + offset_y;
            let left = origin_x.min(current_x);
            let right = origin_x.max(current_x);
            let top = origin_y.min(current_y);
            let bottom = origin_y.max(current_y);
            if let Some(list_bounds) = list_for_marquee.compute_bounds(&overlay_for_marquee) {
                marquee_box_for_update
                    .set_margin_start((f64::from(list_bounds.x()) + left).round().max(0.0) as i32);
                marquee_box_for_update
                    .set_margin_top((f64::from(list_bounds.y()) + top).round().max(0.0) as i32);
                marquee_box_for_update.set_size_request(
                    (right - left).round().max(1.0) as i32,
                    (bottom - top).round().max(1.0) as i32,
                );
            }
            let initial = initial_for_update.borrow();
            let (control, shift) = modifiers_for_update.get();
            let selected = if control || shift {
                initial.copy()
            } else {
                gtk::Bitset::new_empty()
            };
            rows_for_marquee.borrow_mut().retain(|bound| {
                let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                    return false;
                };
                let Some(bounds) = row.compute_bounds(&list_for_marquee) else {
                    return true;
                };
                let intersects = f64::from(bounds.x()) < right
                    && f64::from(bounds.x() + bounds.width()) > left
                    && f64::from(bounds.y()) < bottom
                    && f64::from(bounds.y() + bounds.height()) > top;
                let position = item.position();
                if intersects && position != gtk::INVALID_LIST_POSITION {
                    if control && initial.contains(position) {
                        selected.remove(position);
                    } else {
                        selected.add(position);
                    }
                }
                true
            });
            let mask = gtk::Bitset::new_range(0, selection_for_marquee.n_items());
            selection_for_marquee.set_selection(&selected, &mask);
        });
        let active_for_end = marquee_active.clone();
        let marquee_box_for_end = marquee_box.clone();
        marquee.connect_drag_end(move |_, _, _| {
            active_for_end.set(false);
            marquee_box_for_end.set_visible(false);
        });
        let clear_selection = gtk::GestureClick::new();
        clear_selection.set_button(1);
        let background_press = Rc::new(Cell::new((0.0, 0.0)));
        let background_press_start = background_press.clone();
        clear_selection.connect_pressed(move |_, _, x, y| background_press_start.set((x, y)));
        let selection_for_background = selection.clone();
        clear_selection.connect_released(move |gesture, _, x, y| {
            let (start_x, start_y) = background_press.get();
            if (x - start_x).abs() > 3.0 || (y - start_y).abs() > 3.0 {
                return;
            }
            let target = gesture
                .widget()
                .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT));
            if !target.is_some_and(is_file_row_target) {
                selection_for_background.unselect_all();
                mouse_selection_anchor_for_background.set(None);
            }
        });
        list.add_controller(marquee);
        list.add_controller(clear_selection);
        let selection_keys = gtk::EventControllerKey::new();
        selection_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let modified_for_key = modified_selection.clone();
        selection_keys.connect_key_pressed(move |_, _, _, modifiers| {
            modified_for_key.set(
                modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                    || modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
            );
            glib::Propagation::Proceed
        });
        let modified_for_key = modified_selection.clone();
        selection_keys.connect_key_released(move |_, _, _, _| {
            modified_for_key.set(false);
        });
        list.add_controller(selection_keys);

        let weak_browser = Rc::downgrade(&self.browser);
        let source_for_activation = model.clone();
        let filtered_for_activation = filtered_model.clone();
        list.connect_activate(move |_, position| {
            let source_position = source_position_for_filtered(
                &source_for_activation,
                &filtered_for_activation,
                position,
            );
            if let (Some(browser), Some(source_position)) =
                (weak_browser.upgrade(), source_position)
            {
                browser.activate(depth, source_position);
            }
        });

        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let retry = gtk::Button::with_label("Retry");
        retry.add_css_class("retry-button");
        let weak_browser = Rc::downgrade(&self.browser);
        retry.connect_clicked(move |_| {
            if let Some(browser) = weak_browser.upgrade() {
                browser.retry_column(depth);
            }
        });
        let new_entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        new_entry_row.add_css_class("file-row");
        new_entry_row.add_css_class("new-entry-row");
        new_entry_row.set_visible(false);
        let new_entry_icon = crate::assets::primary_icon(crate::assets::icons::FOLDER, 17);
        new_entry_icon.add_css_class("file-icon");
        let new_entry_entry = gtk::Entry::new();
        new_entry_entry.add_css_class("inline-rename");
        new_entry_entry.set_hexpand(true);
        new_entry_entry.connect_changed(|field| {
            update_basename_validation(field);
        });
        new_entry_row.append(&new_entry_icon);
        new_entry_row.append(&new_entry_entry);
        let weak_state = Rc::downgrade(self);
        new_entry_entry.connect_activate(move |field| {
            if let Some(state) = weak_state.upgrade() {
                state.submit_new_entry(field);
            }
        });
        let new_entry_focus = gtk::EventControllerFocus::new();
        let weak_state = Rc::downgrade(self);
        let field = new_entry_entry.clone();
        new_entry_focus.connect_leave(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.submit_new_entry(&field);
            }
        });
        new_entry_entry.add_controller(new_entry_focus);

        let presentation = LoadPresentation::new(&scroll, Some(retry));
        install_directory_drop_target(self, &presentation.stack, location.clone());
        install_folder_context_menu(
            self,
            presentation.stack.upcast_ref(),
            &selection,
            Rc::new(|picked| is_file_row_target(picked.clone())),
            depth,
            location.clone(),
        );
        let rows_for_context = bound_rows.clone();
        let pick_position = Rc::new(move |picked: &gtk::Widget| {
            let picked = file_row_target(picked.clone())?;
            rows_for_context.borrow().iter().find_map(|bound| {
                let row = bound.row.upgrade()?;
                let item = bound.item.upgrade()?;
                (row == picked).then_some(item.position())
            })
        });
        let source_for_context = model.clone();
        let filtered_for_context = filtered_model.clone();
        let source_position = Rc::new(move |position| {
            source_position_for_filtered(&source_for_context, &filtered_for_context, position)
        });
        install_item_context_menu(
            self,
            list.upcast_ref(),
            &selection,
            pick_position,
            source_position,
            depth,
        );
        column.append(&new_entry_row);
        column.append(&presentation.stack);

        let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        shell.set_size_request(COLUMN_WIDTH, -1);
        shell.set_vexpand(true);
        shell.set_overflow(gtk::Overflow::Hidden);
        let column_overlay = gtk::Overlay::new();
        column_overlay.set_child(Some(&column));
        column_overlay.set_hexpand(true);
        column_overlay.set_vexpand(true);
        shell.append(&column_overlay);
        let resize_handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        resize_handle.add_css_class("column-resize-handle");
        resize_handle.set_width_request(7);
        resize_handle.set_cursor_from_name(Some("col-resize"));
        let resize = gtk::GestureDrag::new();
        resize.set_button(1);
        let resize_start = Rc::new(Cell::new(COLUMN_WIDTH));
        let pointer_start = Rc::new(Cell::new(None));
        let shell_for_resize_start = shell.clone();
        let resize_start_for_begin = resize_start.clone();
        let pointer_start_for_begin = pointer_start.clone();
        resize.connect_drag_begin(move |gesture, _, _| {
            resize_start_for_begin.set(shell_for_resize_start.width().max(COLUMN_WIDTH));
            if let Some((pointer_x, _)) = gesture.current_event().and_then(|event| event.position())
            {
                pointer_start_for_begin.set(Some(pointer_x));
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        let shell_for_resize = shell.clone();
        resize.connect_drag_update(move |gesture, fallback_offset_x, _| {
            let pointer_x = gesture
                .current_event()
                .and_then(|event| event.position())
                .map(|(pointer_x, _)| pointer_x);
            let offset_x = pointer_start
                .get()
                .zip(pointer_x)
                .map_or(fallback_offset_x, |(start, current)| current - start);
            shell_for_resize
                .set_size_request(resized_column_width(resize_start.get(), offset_x), -1);
        });
        resize_handle.add_controller(resize);
        resize_handle.set_halign(gtk::Align::End);
        resize_handle.set_valign(gtk::Align::Fill);
        column_overlay.add_overlay(&resize_handle);
        let animation_generation = Rc::new(Cell::new(0));
        let previous = depth
            .checked_sub(1)
            .and_then(|previous| self.columns.borrow().get(previous).cloned())
            .map(|column| column.shell);
        self.columns_widget
            .insert_child_after(&shell, previous.as_ref());
        self.columns.borrow_mut().push(ColumnView {
            shell: shell.clone(),
            animation_generation: animation_generation.clone(),
            presentation,
            model,
            filtered_model,
            filter_entry,
            filter_button,
            selection,
            syncing_selection,
            list,
            marquee: marquee_box,
            bound_rows,
            entry_count,
            spinner,
            new_entry_row,
            new_entry_icon,
            new_entry_entry,
        });

        self.refresh_active_path_rows();
        animate_column_entry(&shell, &column, &animation_generation);
        self.reveal_column(shell);
    }

    fn reveal_column(self: &Rc<Self>, shell: gtk::Box) {
        let animation_id = self.horizontal_scroll_generation.get().saturating_add(1);
        self.horizontal_scroll_generation.set(animation_id);
        let weak = Rc::downgrade(self);
        let measured_shell = shell;
        let _tick = self.scroller.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.horizontal_scroll_generation.get() != animation_id
                || measured_shell.parent().is_none()
            {
                return glib::ControlFlow::Break;
            }
            let adjustment = state.scroller.hadjustment();
            if measured_shell.width() <= 0 || adjustment.page_size() <= 0.0 {
                return glib::ControlFlow::Continue;
            }
            let Some(bounds) = measured_shell.compute_bounds(&state.columns_widget) else {
                return glib::ControlFlow::Continue;
            };
            let target = horizontal_reveal_target(
                adjustment.value(),
                adjustment.page_size(),
                adjustment.lower(),
                adjustment.upper(),
                f64::from(bounds.x()),
                f64::from(bounds.x() + bounds.width()),
            );
            animate_horizontal_scroll(
                &state.scroller,
                &adjustment,
                target,
                &state.horizontal_scroll_generation,
                animation_id,
            );
            glib::ControlFlow::Break
        });
    }

    pub(super) fn schedule_peek(
        self: &Rc<Self>,
        origin_depth: usize,
        location: Location,
        anchor: gtk::Widget,
    ) {
        if !self.peek_enabled.get() {
            return;
        }
        cancel_source(&self.pending_peek);
        cancel_source(&self.pending_close);
        if self
            .peek
            .borrow()
            .as_ref()
            .is_some_and(|peek| peek.location == location)
        {
            return;
        }
        self.peek_anchor.replace(Some(anchor));

        let weak_state = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(self.peek_behavior.open_delay, move || {
            if let Some(state) = weak_state.upgrade() {
                state.pending_peek.take();
                state.browser.begin_peek(origin_depth, location);
            }
        });
        self.pending_peek.replace(Some(source));
    }

    pub(super) fn schedule_close_peek(self: &Rc<Self>) {
        cancel_source(&self.pending_peek);
        cancel_source(&self.pending_close);

        let weak_state = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(self.peek_behavior.close_delay, move || {
            if let Some(state) = weak_state.upgrade() {
                state.pending_close.take();
                state.browser.close_peek();
            }
        });
        self.pending_close.replace(Some(source));
    }

    fn append_peek(self: &Rc<Self>, location: &Location) {
        let anchor = self.peek_anchor.take();
        self.close_peek_visual();
        let Some(anchor) = anchor else {
            self.browser.close_peek();
            return;
        };

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_size_request(256, -1);
        content.set_overflow(gtk::Overflow::Hidden);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("column-header");
        let heading = gtk::Label::new(Some(&location.display_name()));
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        let spinner = gtk::Spinner::new();
        spinner.start();
        header.append(&heading);
        header.append(&spinner);
        content.append(&header);

        let entry_count = Rc::new(Cell::new(0));
        let entries = Rc::new(RefCell::new(Vec::new()));
        let model = gtk::StringList::new(&[]);
        let selection = gtk::NoSelection::new(Some(model.clone()));
        let factory = peek_label_factory(entries.clone());
        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.add_css_class("file-list");
        let weak_browser = Rc::downgrade(&self.browser);
        list.connect_activate(move |_, _| {
            if let Some(browser) = weak_browser.upgrade() {
                browser.commit_peek();
            }
        });
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(240)
            .propagate_natural_height(true)
            .build();
        let presentation = LoadPresentation::new(&scroll, None);
        presentation.stack.set_size_request(-1, 120);
        content.append(&presentation.stack);

        let motion = gtk::EventControllerMotion::new();
        let weak_state = Rc::downgrade(self);
        motion.connect_enter(move |_, _, _| {
            if let Some(state) = weak_state.upgrade() {
                cancel_source(&state.pending_close);
            }
        });
        let weak_state = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.schedule_close_peek();
            }
        });
        content.add_controller(motion);

        let click = gtk::GestureClick::new();
        let weak_browser = Rc::downgrade(&self.browser);
        click.connect_released(move |_, _, _, _| {
            if let Some(browser) = weak_browser.upgrade() {
                browser.commit_peek();
            }
        });
        content.add_controller(click);

        let Some(bounds) = anchor.compute_bounds(&self.overlay) else {
            self.browser.close_peek();
            return;
        };
        content.add_css_class("peek-popover");
        let right = bounds.x() + bounds.width() + 4.0;
        let left = (bounds.x() - 260.0).max(0.0);
        let x = if right + 256.0 <= self.overlay.width() as f32 {
            right
        } else {
            left
        };
        let transition_duration = self
            .peek_behavior
            .fade_duration
            .as_millis()
            .min(u128::from(u32::MAX)) as u32;
        let revealer = gtk::Revealer::builder()
            .child(&content)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .transition_duration(transition_duration)
            .reveal_child(false)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start(x.round() as i32)
            .margin_top(bounds.y().round().max(0.0) as i32)
            .build();
        self.overlay.add_overlay(&revealer);
        self.peek.replace(Some(PeekView {
            revealer: revealer.clone(),
            location: location.clone(),
            presentation,
            model,
            entries,
            entry_count,
            spinner,
        }));
        glib::idle_add_local_once(move || revealer.set_reveal_child(true));
    }

    fn close_peek_visual(&self) {
        cancel_source(&self.pending_peek);
        cancel_source(&self.pending_close);
        if let Some(peek) = self.peek.take() {
            peek.revealer.set_can_target(false);
            peek.revealer.set_reveal_child(false);
            let overlay = self.overlay.clone();
            let revealer = peek.revealer;
            let delay = Duration::from_millis(u64::from(revealer.transition_duration()));
            glib::timeout_add_local_once(delay, move || overlay.remove_overlay(&revealer));
        }
    }

    fn truncate(self: &Rc<Self>, len: usize) {
        self.close_peek_visual();
        if self.hovered_column.get().is_some_and(|depth| depth >= len) {
            self.hovered_column.set(None);
        }
        self.cancel_rename();
        self.cancel_new_entry();
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().saturating_add(1));
        while self.columns.borrow().len() > len {
            let Some(column) = self.columns.borrow_mut().pop() else {
                break;
            };
            column
                .animation_generation
                .set(column.animation_generation.get().saturating_add(1));
            self.columns_widget.remove(&column.shell);
            self.overlay.remove_overlay(&column.marquee);
        }
        let retained = self
            .columns
            .borrow()
            .last()
            .map(|column| column.shell.clone());
        if let Some(retained) = retained {
            self.reveal_column(retained);
        }
    }
}

fn peek_label_factory(entries: Rc<RefCell<Vec<FileEntry>>>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = gtk::Image::new();
        icon.add_css_class("file-icon");
        icon.set_pixel_size(17);
        let label = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let chevron = crate::assets::primary_icon(crate::assets::icons::CHEVRON_RIGHT, 15);
        chevron.add_css_class("file-chevron");
        row.append(&icon);
        row.append(&label);
        row.append(&chevron);
        item.set_child(Some(&row));
    });
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(value) = item.item().and_downcast::<gtk::StringObject>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = row.first_child().and_downcast::<gtk::Image>() else {
            return;
        };
        let Some(label) = icon.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(chevron) = label.next_sibling().and_downcast::<gtk::Image>() else {
            return;
        };
        let value = value.string();
        let name = model_display_name(&value);
        let directory = model_is_directory(&value);
        label.set_label(name);
        if let Some(entry) = entries.borrow().get(item.position() as usize) {
            super::thumbnail::set_thumbnail_or_icon(&icon, entry, entry_icon(entry), 17, 24);
        } else {
            super::thumbnail::show_fallback_icon(&icon, icon_for_name(name), 17);
        }
        icon.set_opacity(if directory { 1.0 } else { 0.82 });
        chevron.set_visible(directory);
    });
    factory
}

pub(super) fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    if bytes < 1_000 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1_000.0 && unit < UNITS.len() - 1 {
        value /= 1_000.0;
        unit += 1;
    }
    let formatted = format!("{value:.1}");
    format!("{} {}", formatted.trim_end_matches(".0"), UNITS[unit])
}

fn set_filter_placeholder(column: &ColumnView, count: usize) {
    let noun = if count == 1 { "item" } else { "items" };
    column
        .filter_entry
        .set_placeholder_text(Some(&format!("Filter {count} {noun}…")));
}

fn source_position_for_filtered(
    source: &gtk::StringList,
    filtered: &gtk::FilterListModel,
    filtered_position: u32,
) -> Option<usize> {
    let item = filtered.item(filtered_position)?;
    (0..source.n_items())
        .find(|position| {
            source
                .item(*position)
                .is_some_and(|candidate| candidate == item)
        })
        .map(|position| position as usize)
}

fn set_column_selection(column: &ColumnView, position: u32) {
    column.syncing_selection.set(true);
    column.selection.unselect_all();
    if position != gtk::INVALID_LIST_POSITION {
        column.selection.select_item(position, true);
    }
    column.syncing_selection.set(false);
}

fn set_column_selections(column: &ColumnView, positions: &[u32]) {
    column.syncing_selection.set(true);
    column.selection.unselect_all();
    for position in positions {
        column.selection.select_item(*position, false);
    }
    column.syncing_selection.set(false);
}

fn bitset_positions(bitset: &gtk::Bitset) -> Vec<u32> {
    let Some((iterator, first)) = gtk::BitsetIter::init_first(bitset) else {
        return Vec::new();
    };
    std::iter::once(first).chain(iterator).collect()
}

fn filtered_position_for_source(column: &ColumnView, source_position: usize) -> Option<u32> {
    let item = column.model.item(source_position as u32)?;
    (0..column.filtered_model.n_items()).find(|position| {
        column
            .filtered_model
            .item(*position)
            .is_some_and(|candidate| candidate == item)
    })
}

pub(super) fn install_folder_context_menu(
    state: &Rc<ViewState>,
    parent: &gtk::Widget,
    selection: &gtk::MultiSelection,
    is_item_target: Rc<dyn Fn(&gtk::Widget) -> bool>,
    depth: usize,
    location: Location,
) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("folder-context-menu");
    let popover = gtk::Popover::builder()
        .child(&content)
        .autohide(true)
        .has_arrow(false)
        .build();
    popover.add_css_class("folder-context-popover");

    let new_folder = context_menu_option("New Folder", Some("Ctrl+Shift+N"));
    let new_file = context_menu_option("New File", None);
    let paste = context_menu_option("Paste", Some("Ctrl+V"));
    let select_all = context_menu_option("Select All", Some("Ctrl+A"));
    let properties = context_menu_option("Properties", None);
    content.append(&new_folder);
    content.append(&new_file);
    content.append(&paste);
    content.append(&select_all);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&properties);

    let pending_new_entry = Rc::new(Cell::new(None));
    let pending_for_click = pending_new_entry.clone();
    let new_folder_popover = popover.downgrade();
    new_folder.connect_clicked(move |_| {
        pending_for_click.set(Some(true));
        if let Some(popover) = new_folder_popover.upgrade() {
            popover.popdown();
        }
    });
    let pending_for_click = pending_new_entry.clone();
    let new_file_popover = popover.downgrade();
    new_file.connect_clicked(move |_| {
        pending_for_click.set(Some(false));
        if let Some(popover) = new_file_popover.upgrade() {
            popover.popdown();
        }
    });
    let weak = Rc::downgrade(state);
    let folder = location.clone();
    popover.connect_closed(move |popover| {
        popover.unparent();
        let Some(is_directory) = pending_new_entry.take() else {
            return;
        };
        let weak = weak.clone();
        let folder = folder.clone();
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.begin_new_entry(depth, folder, is_directory);
            }
        });
    });
    let weak = Rc::downgrade(state);
    let folder = location.clone();
    let paste_popover = popover.downgrade();
    paste.connect_clicked(move |_| {
        if let Some(popover) = paste_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.paste_into(folder.clone());
        }
    });
    let weak = Rc::downgrade(state);
    let select_popover = popover.downgrade();
    select_all.connect_clicked(move |_| {
        if let Some(popover) = select_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.select_all(depth);
        }
    });
    let weak = Rc::downgrade(state);
    let properties_popover = popover.downgrade();
    properties.connect_clicked(move |_| {
        if let Some(popover) = properties_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.show_folder_properties(&location);
        }
    });

    let menu_click = gtk::GestureClick::new();
    menu_click.set_button(3);
    let selection = selection.clone();
    let popover_for_click = popover.clone();
    menu_click.connect_pressed(move |gesture, _, x, y| {
        let over_item = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|picked| is_item_target(&picked));
        if over_item {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        paste.set_sensitive(gtk::gdk::Display::default().is_some_and(|display| {
            display
                .clipboard()
                .formats()
                .contains_type(gtk::gdk::FileList::static_type())
        }));
        select_all.set_sensitive(selection.n_items() > 0);
        if popover_for_click.parent().is_none()
            && let Some(parent) = gesture.widget()
        {
            popover_for_click.set_parent(&parent);
        }
        popover_for_click.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            x.round() as i32,
            y.round() as i32,
            1,
            1,
        )));
        popover_for_click.popup();
    });
    parent.add_controller(menu_click);
}

pub(super) type ContextPickPosition = Rc<dyn Fn(&gtk::Widget) -> Option<u32>>;
pub(super) type ContextSourcePosition = Rc<dyn Fn(u32) -> Option<usize>>;

pub(super) fn install_item_context_menu(
    state: &Rc<ViewState>,
    widget: &gtk::Widget,
    selection: &gtk::MultiSelection,
    pick_position: ContextPickPosition,
    source_position: ContextSourcePosition,
    depth: usize,
) {
    let in_trash = state
        .browser
        .location_at(depth)
        .as_ref()
        .is_some_and(is_trash_location);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("item-context-menu");
    let header = gtk::Box::new(gtk::Orientation::Vertical, 2);
    header.add_css_class("item-context-header");
    let heading = gtk::Label::new(None);
    heading.add_css_class("item-context-title");
    heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    heading.set_xalign(0.0);
    let summary = gtk::Label::new(None);
    summary.add_css_class("item-context-summary");
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_xalign(0.0);
    header.append(&heading);
    header.append(&summary);
    content.append(&header);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let single = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let open = item_context_option(crate::assets::icons::EXTERNAL_LINK, "Open", "↵");
    let preview = item_context_option(crate::assets::icons::EYE, "Quick preview", "Space");
    let restore = item_context_option(crate::assets::icons::FOLDER, "Restore", "");
    restore.set_visible(in_trash);
    let pin = item_context_option(crate::assets::icons::PIN, "Pin to sidebar", "P");
    let copy_path = item_context_option(crate::assets::icons::COPY, "Copy path", "Y");
    let move_to = item_context_option(crate::assets::icons::FOLDER, "Move to…", "");
    let copy_to = item_context_option(crate::assets::icons::COPY, "Copy to…", "");
    let rename = item_context_option(crate::assets::icons::PENCIL, "Rename", "F2");
    let cut = item_context_option(crate::assets::icons::SCISSORS, "Cut", "Ctrl+X");
    let delete_label = if in_trash {
        "Permanently delete"
    } else {
        "Move to Trash"
    };
    let move_to_trash =
        item_context_danger_option(crate::assets::icons::TRASH, delete_label, "Del");
    move_to_trash.add_css_class("danger");
    let properties = item_context_option(crate::assets::icons::INFO, "Properties", "Alt+Enter");
    single.append(&open);
    single.append(&preview);
    single.append(&restore);
    single.append(&pin);
    single.append(&copy_path);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&move_to);
    single.append(&copy_to);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&rename);
    single.append(&cut);
    single.append(&move_to_trash);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&properties);
    content.append(&single);

    let multiple = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let restore_multiple = item_context_option(crate::assets::icons::FOLDER, "Restore items", "");
    restore_multiple.set_visible(in_trash);
    let copy_paths = item_context_option(crate::assets::icons::COPY, "Copy paths", "Y");
    let move_multiple = item_context_option(crate::assets::icons::FOLDER, "Move to…", "");
    let copy_multiple = item_context_option(crate::assets::icons::COPY, "Copy to…", "");
    let cut_multiple = item_context_option(crate::assets::icons::SCISSORS, "Cut", "Ctrl+X");
    let trash_multiple =
        item_context_danger_option(crate::assets::icons::TRASH, delete_label, "Del");
    trash_multiple.add_css_class("danger");
    multiple.append(&restore_multiple);
    multiple.append(&copy_paths);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&move_multiple);
    multiple.append(&copy_multiple);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&cut_multiple);
    multiple.append(&trash_multiple);
    multiple.set_visible(false);
    content.append(&multiple);

    let popover = gtk::Popover::builder()
        .child(&content)
        .autohide(true)
        .has_arrow(false)
        .build();
    popover.add_css_class("folder-context-popover");
    popover.set_parent(widget);

    let target = Rc::new(RefCell::new(None::<(usize, FileEntry)>));
    let weak = Rc::downgrade(state);
    let open_target = target.clone();
    let open_popover = popover.downgrade();
    open.connect_clicked(move |_| {
        if let Some(popover) = open_popover.upgrade() {
            popover.popdown();
        }
        let Some((position, _)) = open_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade() {
            if state.mode_views.borrow().mode() == BrowserMode::Columns {
                state.browser.activate(depth, position);
            } else {
                state.browser.activate_in_place(depth, position);
            }
        }
    });
    let weak = Rc::downgrade(state);
    let preview_target = target.clone();
    let preview_popover = popover.downgrade();
    preview.connect_clicked(move |_| {
        if let Some(popover) = preview_popover.upgrade() {
            popover.popdown();
        }
        let Some((position, entry)) = preview_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade()
            && !entry.is_directory()
        {
            state.browser.preview(depth, position);
        }
    });
    let weak = Rc::downgrade(state);
    let pin_target = target.clone();
    let pin_popover = popover.downgrade();
    pin.connect_clicked(move |_| {
        if let Some(popover) = pin_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = pin_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade()
            && entry.is_directory()
            && let Some(handler) = state.pin_handler.borrow().as_ref()
        {
            handler(entry.location, entry.display_name);
        }
    });
    let weak = Rc::downgrade(state);
    let copy_target = target.clone();
    let copy_popover = popover.downgrade();
    copy_path.connect_clicked(move |_| {
        if let Some(popover) = copy_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = copy_target.borrow().clone() else {
            return;
        };
        if weak.upgrade().is_some() {
            copy_locations(&[entry]);
        }
    });
    let weak = Rc::downgrade(state);
    let rename_target = target.clone();
    let rename_popover = popover.downgrade();
    rename.connect_clicked(move |_| {
        if let Some(popover) = rename_popover.upgrade() {
            popover.popdown();
        }
        let Some((position, _)) = rename_target.borrow().clone() else {
            return;
        };
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.browser.select(depth, position);
                state.begin_rename();
            }
        });
    });
    connect_context_restore(&restore, &popover, state, &target);
    connect_context_restore(&restore_multiple, &popover, state, &target);
    connect_context_transfer(&move_to, &popover, state, &target, true);
    connect_context_transfer(&copy_to, &popover, state, &target, false);
    connect_context_transfer(&move_multiple, &popover, state, &target, true);
    connect_context_transfer(&copy_multiple, &popover, state, &target, false);
    connect_context_cut(&cut, &popover, state, &target);
    connect_context_cut(&cut_multiple, &popover, state, &target);
    connect_context_trash(&move_to_trash, &popover, state, &target, in_trash);
    connect_context_trash(&trash_multiple, &popover, state, &target, in_trash);
    let weak = Rc::downgrade(state);
    let properties_target = target.clone();
    let properties_popover = popover.downgrade();
    properties.connect_clicked(move |_| {
        if let Some(popover) = properties_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = properties_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade() {
            state.show_entry_properties(entry);
        }
    });
    let weak = Rc::downgrade(state);
    let paths_target = target.clone();
    let paths_popover = popover.downgrade();
    copy_paths.connect_clicked(move |_| {
        if let Some(popover) = paths_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            copy_locations(&context_entries(&state, &paths_target));
        }
    });

    let click = gtk::GestureClick::new();
    click.set_button(3);
    let weak_state = Rc::downgrade(state);
    let weak_popover = popover.downgrade();
    let selection = selection.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(picked) = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
        else {
            return;
        };
        let Some(filtered_position) = pick_position(&picked) else {
            return;
        };
        let Some(resolved_position) = source_position(filtered_position) else {
            return;
        };
        let Some(state) = weak_state.upgrade() else {
            return;
        };
        let Some(entry) = state.browser.entry_at(depth, resolved_position) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if !selection.is_selected(filtered_position) {
            selection.select_item(filtered_position, true);
        }
        let selected_positions = bitset_positions(&selection.selection())
            .into_iter()
            .filter_map(|position| source_position(position))
            .collect::<Vec<_>>();
        state
            .browser
            .set_selection(depth, &selected_positions, Some(resolved_position));
        target.replace(Some((resolved_position, entry.clone())));
        let entries = state.browser.selected_entries();
        preview.set_visible(entry_supports_quick_preview(&entry));
        pin.set_visible(entry.is_directory() && !is_trash_location(&entry.location));
        if entries.len() > 1 {
            heading.set_text(&format!("{} items selected", entries.len()));
            summary.set_text(&selected_items_summary(&entries));
            single.set_visible(false);
            multiple.set_visible(true);
        } else {
            heading.set_text(&entry.display_name);
            summary.set_text(&compact_display_path(&entry.location));
            single.set_visible(true);
            multiple.set_visible(false);
        }
        let Some(popover) = weak_popover.upgrade() else {
            return;
        };
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            x.round() as i32,
            y.round() as i32,
            1,
            1,
        )));
        popover.popup();
    });
    widget.add_controller(click);
}

fn entry_responds_to_single_click(entry: &FileEntry, previews_enabled: bool) -> bool {
    entry.is_directory() || (previews_enabled && entry_supports_quick_preview(entry))
}

pub(super) fn entry_supports_quick_preview(entry: &FileEntry) -> bool {
    if !matches!(entry.kind, EntryKind::File | EntryKind::FileSymbolicLink) {
        return false;
    }

    let (content_type, _) =
        gio::content_type_guess(Some(Path::new(&entry.native_name)), None::<&[u8]>);
    !matches!(content_family(&content_type), PreviewContent::Unsupported)
        || gio::content_type_is_a(&content_type, "text/plain")
        || has_plain_text_extension(&entry.native_name)
}

struct TrashSummary {
    entries: Vec<FileEntry>,
    item_count: usize,
    total_size: u64,
}

const TRASH_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-symlink,standard::size,time::modified";

async fn summarize_trash(root: &gio::File) -> Result<TrashSummary, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut entries = Vec::new();
    let mut item_count = 0_usize;
    let mut total_size = 0_u64;
    loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        for info in children {
            let file = root.child(info.name());
            let (count, size) = measure_trash_entry(file.clone(), info.clone()).await?;
            item_count = item_count.saturating_add(count);
            total_size = total_size.saturating_add(size);
            entries.push(trash_file_entry(file, &info));
        }
    }
    Ok(TrashSummary {
        entries,
        item_count,
        total_size,
    })
}

type TrashMeasurementFuture = Pin<Box<dyn Future<Output = Result<(usize, u64), glib::Error>>>>;

fn measure_trash_entry(file: gio::File, info: gio::FileInfo) -> TrashMeasurementFuture {
    Box::pin(async move {
        let mut count = 1_usize;
        let mut size = if info.file_type() == gio::FileType::Regular {
            info.size().max(0) as u64
        } else {
            0
        };
        if info.file_type() == gio::FileType::Directory && !info.is_symlink() {
            let enumerator = file
                .enumerate_children_future(
                    TRASH_ATTRIBUTES,
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                )
                .await?;
            loop {
                let children = enumerator
                    .next_files_future(64, glib::Priority::DEFAULT)
                    .await?;
                if children.is_empty() {
                    break;
                }
                for child in children {
                    let (child_count, child_size) =
                        measure_trash_entry(file.child(child.name()), child).await?;
                    count = count.saturating_add(child_count);
                    size = size.saturating_add(child_size);
                }
            }
        }
        Ok((count, size))
    })
}

fn trash_file_entry(file: gio::File, info: &gio::FileInfo) -> FileEntry {
    let kind = match (info.file_type(), info.is_symlink()) {
        (gio::FileType::Directory, true) => EntryKind::DirectorySymbolicLink,
        (gio::FileType::Regular, true) => EntryKind::FileSymbolicLink,
        (gio::FileType::Directory, false) => EntryKind::Directory,
        (gio::FileType::Regular, false) => EntryKind::File,
        (gio::FileType::SymbolicLink, _) => EntryKind::SymbolicLink,
        _ => EntryKind::Other,
    };
    FileEntry {
        location: location_for_gio_file(&file),
        native_name: info.name().into_os_string(),
        display_name: info.display_name().to_string(),
        kind,
        size: if matches!(kind, EntryKind::File | EntryKind::FileSymbolicLink) {
            crate::model::MetadataValue::Known(info.size().max(0) as u64)
        } else {
            crate::model::MetadataValue::Unknown
        },
        modified_unix_seconds: info
            .modification_date_time()
            .map(|time| crate::model::MetadataValue::Known(time.to_unix()))
            .unwrap_or(crate::model::MetadataValue::Unavailable),
    }
}

fn selected_items_summary(entries: &[FileEntry]) -> String {
    let mut names = entries
        .iter()
        .take(3)
        .map(|entry| entry.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if entries.len() > 3 {
        names.push_str(", …");
    }
    names
}

fn context_entries(
    state: &ViewState,
    target: &RefCell<Option<(usize, FileEntry)>>,
) -> Vec<FileEntry> {
    let entries = state.browser.selected_entries();
    if entries.is_empty() {
        target
            .borrow()
            .as_ref()
            .map(|(_, entry)| vec![entry.clone()])
            .unwrap_or_default()
    } else {
        entries
    }
}

fn connect_context_trash(
    button: &gtk::Button,
    popover: &gtk::Popover,
    state: &Rc<ViewState>,
    target: &Rc<RefCell<Option<(usize, FileEntry)>>>,
    permanent: bool,
) {
    let weak = Rc::downgrade(state);
    let target = target.clone();
    let popover = popover.downgrade();
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            let entries = context_entries(&state, &target);
            state.show_delete_confirmation(entries, permanent);
        }
    });
}

fn connect_context_restore(
    button: &gtk::Button,
    popover: &gtk::Popover,
    state: &Rc<ViewState>,
    target: &Rc<RefCell<Option<(usize, FileEntry)>>>,
) {
    let weak = Rc::downgrade(state);
    let target = target.clone();
    let popover = popover.downgrade();
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.browser.restore(context_entries(&state, &target));
        }
    });
}

fn connect_context_transfer(
    button: &gtk::Button,
    popover: &gtk::Popover,
    state: &Rc<ViewState>,
    target: &Rc<RefCell<Option<(usize, FileEntry)>>>,
    move_sources: bool,
) {
    let weak = Rc::downgrade(state);
    let target = target.clone();
    let popover = popover.downgrade();
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.show_transfer_dialog(context_entries(&state, &target), move_sources);
        }
    });
}

fn connect_context_cut(
    button: &gtk::Button,
    popover: &gtk::Popover,
    state: &Rc<ViewState>,
    target: &Rc<RefCell<Option<(usize, FileEntry)>>>,
) {
    let weak = Rc::downgrade(state);
    let target = target.clone();
    let popover = popover.downgrade();
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.cut_entries(&context_entries(&state, &target));
        }
    });
}

fn refresh_transfer_suggestions(
    field: &gtk::Entry,
    suggestions: &gtk::Box,
    generation: &Rc<Cell<u64>>,
    base: std::path::PathBuf,
) {
    let request = generation.get().saturating_add(1);
    generation.set(request);
    let input = field.text().to_string();
    let home = glib::home_dir();
    let field = field.clone();
    let suggestions = suggestions.clone();
    let generation = generation.clone();
    glib::MainContext::default().spawn_local(async move {
        let matches =
            gio::spawn_blocking(move || directory_suggestions(&input, &base, &home)).await;
        if generation.get() != request {
            return;
        }
        while let Some(child) = suggestions.first_child() {
            suggestions.remove(&child);
        }
        let Ok(matches) = matches else {
            return;
        };
        if matches.is_empty() {
            let empty = gtk::Label::new(Some("No matching folders"));
            empty.add_css_class("transfer-suggestions-empty");
            empty.set_xalign(0.0);
            suggestions.append(&empty);
        }
        for path in matches {
            let option = gtk::Button::new();
            option.add_css_class("transfer-suggestion");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            row.append(&crate::assets::primary_icon(
                crate::assets::icons::FOLDER,
                16,
            ));
            let label = gtk::Label::new(Some(&compact_native_path(&path)));
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            row.append(&label);
            option.set_child(Some(&row));
            option.set_tooltip_text(Some(&path.to_string_lossy()));
            let selected_field = field.clone();
            option.connect_clicked(move |_| {
                selected_field.remove_css_class("error");
                selected_field.set_text(&folder_input_path(&path));
                selected_field.set_position(-1);
                selected_field.grab_focus();
            });
            suggestions.append(&option);
        }
    });
}

fn folder_input_path(path: &Path) -> String {
    let path = compact_native_path(path);
    if path.ends_with(std::path::MAIN_SEPARATOR) {
        path
    } else {
        format!("{path}{}", std::path::MAIN_SEPARATOR)
    }
}

fn resolve_destination_path(input: &str, base: &Path, home: &Path) -> std::path::PathBuf {
    let input = input.trim();
    if input == "~" {
        home.to_path_buf()
    } else if let Some(relative) = input.strip_prefix("~/") {
        home.join(relative)
    } else {
        let path = Path::new(input);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }
}

fn directory_suggestions(input: &str, base: &Path, home: &Path) -> Vec<std::path::PathBuf> {
    let resolved = resolve_destination_path(input, base, home);
    let trailing_separator = input.trim_end().ends_with(std::path::MAIN_SEPARATOR);
    let (directory, prefix) = if trailing_separator {
        (resolved, String::new())
    } else {
        (
            resolved.parent().unwrap_or(base).to_path_buf(),
            resolved
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
        )
    };
    let Ok(children) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut matches = children
        .filter_map(Result::ok)
        .map(|child| child.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy().to_lowercase();
                (prefix.starts_with('.') || !name.starts_with('.')) && name.starts_with(&prefix)
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    matches.truncate(8);
    matches
}

fn install_directory_drop_target(
    state: &Rc<ViewState>,
    widget: &impl IsA<gtk::Widget>,
    destination: Location,
) {
    widget.add_css_class("file-drop-zone");
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    drop.connect_enter(|target, _, _| file_drop_action(target));
    drop.connect_motion(|target, _, _| file_drop_action(target));
    let weak = Rc::downgrade(state);
    drop.connect_drop(move |target, value, _, _| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        transfer_dropped_files(&state, target, value, destination.clone())
    });
    widget.add_controller(drop);
}

fn transfer_has_collision(source: &Location, destination: &Location) -> bool {
    let source = gio_file_for_location(source);
    let destination = gio_file_for_location(destination);
    let Some(name) = source.basename() else {
        return false;
    };
    let target = destination.child(name);
    if source.equal(&target) || source.equal(&destination) || destination.has_prefix(&source) {
        return false;
    }
    target.query_exists(None::<&gio::Cancellable>)
}

fn transfer_dropped_files(
    state: &Rc<ViewState>,
    target: &gtk::DropTarget,
    value: &glib::Value,
    destination: Location,
) -> bool {
    let Some(sources) = locations_from_file_list_value(value) else {
        return false;
    };
    if sources.is_empty() {
        return false;
    }
    let move_sources = file_drop_action(target) == gtk::gdk::DragAction::MOVE;
    state.start_transfer(destination, sources, move_sources);
    true
}

pub(super) fn file_drop_action(target: &gtk::DropTarget) -> gtk::gdk::DragAction {
    let Some(drop) = target.current_drop() else {
        return gtk::gdk::DragAction::empty();
    };
    preferred_file_drop_action(drop.actions(), drop.drag().is_some())
}

fn preferred_file_drop_action(actions: gtk::gdk::DragAction, local: bool) -> gtk::gdk::DragAction {
    if actions.contains(gtk::gdk::DragAction::MOVE)
        && (local || !actions.contains(gtk::gdk::DragAction::COPY))
    {
        gtk::gdk::DragAction::MOVE
    } else if actions.contains(gtk::gdk::DragAction::COPY) {
        gtk::gdk::DragAction::COPY
    } else {
        gtk::gdk::DragAction::empty()
    }
}

pub(super) fn locations_from_file_list_value(value: &glib::Value) -> Option<Vec<Location>> {
    let files = value.get::<gtk::gdk::FileList>().ok()?;
    let locations = files
        .files()
        .iter()
        .map(location_for_gio_file)
        .collect::<Vec<_>>();
    (!locations.is_empty()).then_some(locations)
}

pub(super) fn location_for_gio_file(file: &gio::File) -> Location {
    file.path()
        .map(Location::local)
        .unwrap_or_else(|| Location::uri(file.uri().as_str()))
}

pub(super) fn file_drag_content(entries: &[FileEntry]) -> Option<gtk::gdk::ContentProvider> {
    let files = entries
        .iter()
        .map(|entry| gio_file_for_location(&entry.location))
        .collect::<Vec<_>>();
    if files.is_empty() {
        return None;
    }
    let file_list =
        gtk::gdk::ContentProvider::for_value(&gtk::gdk::FileList::from_array(&files).to_value());
    let uri_list = files
        .iter()
        .map(|file| file.uri())
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n";
    let uri_list = gtk::gdk::ContentProvider::for_bytes(
        "text/uri-list",
        &glib::Bytes::from_owned(uri_list.into_bytes()),
    );
    Some(gtk::gdk::ContentProvider::new_union(&[file_list, uri_list]))
}

fn copy_locations(entries: &[FileEntry]) {
    let text = entries
        .iter()
        .map(|entry| entry.location.display_path())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(&text);
    }
}

fn set_files_clipboard(entries: &[FileEntry]) -> bool {
    set_location_files_clipboard(
        &entries
            .iter()
            .map(|entry| entry.location.clone())
            .collect::<Vec<_>>(),
    )
}

fn set_location_files_clipboard(locations: &[Location]) -> bool {
    let files = locations
        .iter()
        .map(gio_file_for_location)
        .collect::<Vec<_>>();
    if files.is_empty() {
        return false;
    }
    gtk::gdk::Display::default().is_some_and(|display| {
        display
            .clipboard()
            .set_content(Some(&gtk::gdk::ContentProvider::for_value(
                &gtk::gdk::FileList::from_array(&files).to_value(),
            )))
            .is_ok()
    })
}

fn should_preserve_drag_selection(clicked_selected: bool, selected_count: u64) -> bool {
    clicked_selected && selected_count > 1
}

fn paste_destination_depth(hovered: Option<usize>, pane_count: usize) -> Option<usize> {
    hovered
        .filter(|depth| *depth < pane_count)
        .or_else(|| pane_count.checked_sub(1))
}

fn new_folder_destination_depth(
    hovered: Option<usize>,
    focused: Option<usize>,
    active: Option<usize>,
    pane_count: usize,
) -> Option<usize> {
    hovered
        .filter(|depth| *depth < pane_count)
        .or_else(|| focused.filter(|depth| *depth < pane_count))
        .or_else(|| active.filter(|depth| *depth < pane_count))
        .or_else(|| pane_count.checked_sub(1))
}

fn same_locations(left: &[Location], right: &[Location]) -> bool {
    !left.is_empty()
        && left.len() == right.len()
        && left.iter().all(|location| right.contains(location))
}

fn item_context_option(icon: &str, label: &str, accelerator: &str) -> gtk::Button {
    item_context_option_with_icon(crate::assets::text_icon(icon, 15), label, accelerator)
}

fn item_context_danger_option(icon: &str, label: &str, accelerator: &str) -> gtk::Button {
    item_context_option_with_icon(crate::assets::danger_icon(icon, 15), label, accelerator)
}

fn item_context_option_with_icon(icon: gtk::Image, label: &str, accelerator: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("item-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    icon.add_css_class("item-context-icon");
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&icon);
    row.append(&title);
    if !accelerator.is_empty() {
        let shortcut = gtk::Label::new(Some(accelerator));
        shortcut.add_css_class("item-context-shortcut");
        row.append(&shortcut);
    }
    button.set_child(Some(&row));
    button
}

fn context_menu_option(label: &str, accelerator: Option<&str>) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("folder-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&title);
    if let Some(accelerator) = accelerator {
        let shortcut = gtk::Label::new(Some(accelerator));
        shortcut.add_css_class("folder-context-shortcut");
        row.append(&shortcut);
    }
    button.set_child(Some(&row));
    button
}

pub(super) fn column_sort_menu(browser: &Rc<Browser>, depth: usize) -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.add_css_class("column-menu");
    let heading = gtk::Label::new(Some("SORT BY"));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    content.append(&heading);

    let preferences = browser.column_preferences(depth).unwrap_or_default();
    let selected_checks: Rc<RefCell<Vec<(SortKey, gtk::Image)>>> =
        Rc::new(RefCell::new(Vec::new()));
    for (label, key) in [
        ("Name", SortKey::Name),
        ("Size", SortKey::Size),
        ("Modified", SortKey::Modified),
        ("Type", SortKey::Type),
    ] {
        let (option, check) = column_menu_option(label, preferences.sort_key == key);
        selected_checks.borrow_mut().push((key, check));
        let checks = selected_checks.clone();
        let weak_browser = Rc::downgrade(browser);
        option.connect_clicked(move |_| {
            for (check_key, check) in checks.borrow().iter() {
                check.set_visible(*check_key == key);
            }
            if let Some(browser) = weak_browser.upgrade() {
                browser.set_sort_key(depth, key);
            }
        });
        content.append(&option);
    }

    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let (folders_first, folders_check) =
        column_menu_option("Folders first", preferences.folders_first);
    let folders_enabled = Rc::new(Cell::new(preferences.folders_first));
    let weak_browser = Rc::downgrade(browser);
    let folders_enabled_for_click = folders_enabled.clone();
    let folders_check_for_click = folders_check.clone();
    folders_first.connect_clicked(move |_| {
        let enabled = !folders_enabled_for_click.get();
        folders_enabled_for_click.set(enabled);
        folders_check_for_click.set_visible(enabled);
        if let Some(browser) = weak_browser.upgrade() {
            browser.set_folders_first(depth, enabled);
        }
    });
    content.append(&folders_first);

    let popover = gtk::Popover::builder()
        .child(&content)
        .has_arrow(false)
        .halign(gtk::Align::End)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.add_css_class("column-popover");
    let weak_browser = Rc::downgrade(browser);
    let checks = selected_checks.clone();
    let folders_enabled_for_map = folders_enabled.clone();
    let folders_check_for_map = folders_check.clone();
    popover.connect_map(move |_| {
        let Some(preferences) = weak_browser
            .upgrade()
            .and_then(|browser| browser.column_preferences(depth))
        else {
            return;
        };
        for (key, check) in checks.borrow().iter() {
            check.set_visible(*key == preferences.sort_key);
        }
        folders_enabled_for_map.set(preferences.folders_first);
        folders_check_for_map.set_visible(preferences.folders_first);
    });
    let button = gtk::MenuButton::builder()
        .tooltip_text("Choose sort field")
        .popover(&popover)
        .build();
    button.set_child(Some(&crate::assets::text_icon(
        crate::assets::icons::SETTINGS_2,
        16,
    )));
    button.add_css_class("column-header-action");
    button
}

pub(super) fn column_sort_direction_toggle(browser: &Rc<Browser>, depth: usize) -> gtk::Button {
    let direction = browser
        .column_preferences(depth)
        .unwrap_or_default()
        .sort_direction;
    let button = gtk::Button::new();
    let icon = crate::assets::text_icon(crate::assets::icons::ARROW_UP_NARROW_WIDE, 16);
    button.set_child(Some(&icon));
    button.add_css_class("column-header-action");
    sync_sort_direction_toggle(&button, &icon, direction);

    let weak_browser = Rc::downgrade(browser);
    let icon_for_map = icon.clone();
    button.connect_map(move |button| {
        if let Some(direction) = weak_browser
            .upgrade()
            .and_then(|browser| browser.column_preferences(depth))
            .map(|preferences| preferences.sort_direction)
        {
            sync_sort_direction_toggle(button, &icon_for_map, direction);
        }
    });
    let weak_browser = Rc::downgrade(browser);
    button.connect_clicked(move |button| {
        let Some(browser) = weak_browser.upgrade() else {
            return;
        };
        let direction = match browser
            .column_preferences(depth)
            .unwrap_or_default()
            .sort_direction
        {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        };
        sync_sort_direction_toggle(button, &icon, direction);
        browser.set_sort_direction(depth, direction);
    });
    button
}

fn sync_sort_direction_toggle(button: &gtk::Button, icon: &gtk::Image, direction: SortDirection) {
    let descending = direction == SortDirection::Descending;
    crate::assets::set_text_icon(
        icon,
        if descending {
            crate::assets::icons::ARROW_DOWN_WIDE_NARROW
        } else {
            crate::assets::icons::ARROW_UP_NARROW_WIDE
        },
    );
    button.set_tooltip_text(Some(if descending {
        "Descending — click to reverse"
    } else {
        "Ascending — click to reverse"
    }));
}

fn column_menu_option(label: &str, selected: bool) -> (gtk::Button, gtk::Image) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let check = crate::assets::primary_icon(crate::assets::icons::CHECK, 16);
    check.set_visible(selected);
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    row.append(&check);
    let option = gtk::Button::builder().child(&row).build();
    option.add_css_class("column-menu-option");
    option.set_has_frame(false);
    (option, check)
}

fn file_row_target(mut target: gtk::Widget) -> Option<gtk::Box> {
    loop {
        if target.has_css_class("file-row") {
            return target.downcast::<gtk::Box>().ok();
        }
        if target.is::<gtk::ListView>() {
            return None;
        }
        target = target.parent()?;
    }
}

fn is_file_row_target(target: gtk::Widget) -> bool {
    file_row_target(target).is_some()
}

fn is_breadcrumb_target(mut target: gtk::Widget) -> bool {
    loop {
        if target.is::<gtk::Button>()
            || target.has_css_class("breadcrumb")
            || target.has_css_class("breadcrumb-separator")
            || target.has_css_class("current-breadcrumb")
        {
            return true;
        }
        let Some(parent) = target.parent() else {
            return false;
        };
        if parent.has_css_class("breadcrumbs") {
            return false;
        }
        target = parent;
    }
}

fn set_active_path_style(row: &gtk::Box, active: bool) {
    if active {
        row.add_css_class("active-path");
    } else {
        row.remove_css_class("active-path");
    }
}

fn set_cut_path_style(row: &gtk::Box, cut: bool) {
    if cut {
        row.add_css_class("cut");
    } else {
        row.remove_css_class("cut");
    }
}

pub(super) fn update_basename_validation(field: &gtk::Entry) -> bool {
    match validate_basename(field.text().as_str()) {
        Ok(()) => {
            field.remove_css_class("error");
            field.set_tooltip_text(None);
            true
        }
        Err(message) => {
            field.add_css_class("error");
            field.set_tooltip_text(Some(message));
            false
        }
    }
}

pub(super) fn rename_stem_end(name: &str) -> i32 {
    let end = name
        .rfind('.')
        .filter(|position| *position > 0)
        .unwrap_or(name.len());
    name[..end].chars().count().min(i32::MAX as usize) as i32
}

fn entry_model_value(entry: &FileEntry) -> String {
    let kind = if entry.is_broken_symbolic_link() {
        'x'
    } else if entry.is_directory() {
        'd'
    } else if entry.is_symbolic_link() {
        's'
    } else {
        'f'
    };
    format!("{kind}\t{}", entry.display_name)
}

fn model_display_name(value: &str) -> &str {
    value.split_once('\t').map_or(value, |(_, name)| name)
}

fn model_is_directory(value: &str) -> bool {
    value.starts_with("d\t")
}

pub(super) fn entry_icon(entry: &FileEntry) -> &'static str {
    if entry.is_broken_symbolic_link() {
        return crate::assets::icons::X;
    }
    if entry.is_directory() {
        return crate::assets::icons::FOLDER;
    }
    icon_for_name(&entry.display_name)
}

fn icon_for_name(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("sh" | "bash" | "zsh" | "fish") => crate::assets::icons::TERMINAL,
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "tif" | "tiff"
            | "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf" | "kdc" | "mef" | "mos" | "mrw"
            | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2" | "rwl" | "sr2" | "srf" | "srw"
            | "x3f",
        ) => crate::assets::icons::PICTURES,
        Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v") => crate::assets::icons::VIDEOS,
        Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst") => {
            crate::assets::icons::FILE_ARCHIVE
        }
        Some(
            "rs" | "c" | "h" | "cpp" | "go" | "py" | "rb" | "java" | "js" | "jsx" | "ts" | "tsx"
            | "lua" | "php" | "html" | "css" | "scss" | "json",
        ) => crate::assets::icons::FILE_CODE,
        _ => crate::assets::icons::DOCUMENTS,
    }
}

fn append_peek_entries(peek: &PeekView, entries: Vec<FileEntry>, limit: usize) {
    let remaining = limit.max(1).saturating_sub(peek.entry_count.get());
    let entries = entries.into_iter().take(remaining).collect::<Vec<_>>();
    let values = entries.iter().map(entry_model_value).collect::<Vec<_>>();
    peek.entry_count.set(peek.entry_count.get() + entries.len());
    peek.entries.borrow_mut().extend(entries);
    for value in values {
        peek.model.append(&value);
    }
}

fn cancel_source(source: &RefCell<Option<glib::SourceId>>) {
    if let Some(source) = source.take() {
        source.remove();
    }
}

fn animate_column_entry(shell: &gtk::Box, column: &gtk::Box, generation: &Rc<Cell<u64>>) {
    let animation_id = generation.get().saturating_add(1);
    generation.set(animation_id);
    if !animations_enabled() {
        column.set_opacity(1.0);
        column.set_margin_start(0);
        return;
    }

    column.set_opacity(0.0);
    column.set_margin_start(COLUMN_OFFSET);
    let started = Instant::now();
    let shell = shell.clone();
    let column = column.clone();
    let generation = generation.clone();
    let _tick = shell.add_tick_callback(move |_, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }
        let progress =
            (started.elapsed().as_secs_f64() / COLUMN_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_deceleration(progress);
        column.set_opacity(eased);
        column.set_margin_start((f64::from(COLUMN_OFFSET) * (1.0 - eased)).round() as i32);
        if progress >= 1.0 {
            column.set_opacity(1.0);
            column.set_margin_start(0);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn single_pane_preview_reservation(width: i32) -> i32 {
    width.max(0) / 2
}

fn resized_column_width(initial_width: i32, horizontal_offset: f64) -> i32 {
    (f64::from(initial_width) + horizontal_offset)
        .round()
        .max(f64::from(COLUMN_WIDTH)) as i32
}

fn horizontal_reveal_target(
    current: f64,
    page_size: f64,
    lower: f64,
    upper: f64,
    item_left: f64,
    item_right: f64,
) -> f64 {
    let viewport_right = current + page_size;
    let target = if item_right > viewport_right {
        item_right - page_size
    } else if item_left < current {
        item_left
    } else {
        current
    };
    target.clamp(lower, (upper - page_size).max(lower))
}

fn animate_horizontal_scroll(
    scroller: &gtk::ScrolledWindow,
    adjustment: &gtk::Adjustment,
    target: f64,
    generation: &Rc<Cell<u64>>,
    animation_id: u64,
) {
    let start = adjustment.value();
    if !animations_enabled() || (target - start).abs() < 0.5 {
        adjustment.set_value(target);
        return;
    }

    let started = Instant::now();
    let adjustment = adjustment.clone();
    let generation = generation.clone();
    let _tick = scroller.add_tick_callback(move |_, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }
        let progress =
            (started.elapsed().as_secs_f64() / COLUMN_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_deceleration(progress);
        adjustment.set_value(start + (target - start) * eased);
        if progress >= 1.0 {
            adjustment.set_value(target);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn item_count_label(count: usize) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

fn entry_kind_summary(entries: &[FileEntry]) -> String {
    let directories = entries.iter().filter(|entry| entry.is_directory()).count();
    let files = entries.len().saturating_sub(directories);
    match (files, directories) {
        (1, 0) => "1 file".to_owned(),
        (files, 0) => format!("{files} files"),
        (0, 1) => "1 folder".to_owned(),
        (0, directories) => format!("{directories} folders"),
        _ => item_count_label(entries.len()),
    }
}

pub(super) fn modal_layer(content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("app-modal-layer");
    layer.add_css_class("modal-backdrop");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top.set_vexpand(true);
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.set_vexpand(true);
    layer.append(&top);
    layer.append(content);
    layer.append(&bottom);
    layer
}

pub(super) fn dismiss_modal_layer(
    layer: &gtk::Box,
    overlay: &gtk::Overlay,
    root: Option<&BlurBin>,
) {
    overlay.remove_overlay(layer);
    if let Some(root) = root {
        root.set_blurred(false);
    }
}

fn gio_file_for_location(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}

fn is_trash_root(location: &Location) -> bool {
    location.uri_value() == Some("trash:///")
}

fn is_trash_location(location: &Location) -> bool {
    location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
}

fn compact_display_path(location: &Location) -> String {
    location
        .native_path()
        .map(compact_native_path)
        .unwrap_or_else(|| location.display_path())
}

fn compact_native_path(path: &Path) -> String {
    let home = glib::home_dir();
    if path == home {
        return "~".to_owned();
    }
    path.strip_prefix(&home)
        .ok()
        .map(|suffix| format!("~/{}", suffix.to_string_lossy()))
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn metadata_modified(entry: &FileEntry) -> String {
    let crate::model::MetadataValue::Known(seconds) = entry.modified_unix_seconds else {
        return "—".to_owned();
    };
    glib::DateTime::from_unix_local(seconds)
        .and_then(|date| date.format("%Y-%m-%d %H:%M"))
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "—".to_owned())
}

fn properties_row(parent: &gtk::Box, label: &str, value: &str) -> gtk::Label {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("properties-row");
    let label = gtk::Label::new(Some(label));
    label.add_css_class("properties-row-label");
    label.set_xalign(0.0);
    let value = gtk::Label::new(Some(value));
    value.add_css_class("properties-row-value");
    value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    value.set_hexpand(true);
    value.set_xalign(0.0);
    row.append(&label);
    row.append(&value);
    parent.append(&row);
    value
}

type PermissionRow = (gtk::Label, [gtk::Label; 3]);

fn permission_row(parent: &gtk::Box, label: &str) -> PermissionRow {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("properties-permission-row");
    let title = gtk::Label::new(Some(label));
    title.add_css_class("properties-permission-title");
    title.set_xalign(0.0);
    let identity = gtk::Label::new(Some("—"));
    identity.add_css_class("properties-permission-identity");
    identity.set_xalign(0.0);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let read = gtk::Label::new(Some("—"));
    let write = gtk::Label::new(Some("—"));
    let execute = gtk::Label::new(Some("—"));
    for permission in [&read, &write, &execute] {
        permission.add_css_class("properties-permission-bit");
        permission.set_width_chars(2);
    }
    row.append(&title);
    row.append(&identity);
    row.append(&spacer);
    row.append(&read);
    row.append(&write);
    row.append(&execute);
    parent.append(&row);
    (identity, [read, write, execute])
}

fn set_permission_row(row: &PermissionRow, mode: u32, shift: u32) {
    let value = (mode >> shift) & 0o7;
    row.1[0].set_text(if value & 0o4 != 0 { "r" } else { "—" });
    row.1[1].set_text(if value & 0o2 != 0 { "w" } else { "—" });
    row.1[2].set_text(if value & 0o1 != 0 { "x" } else { "—" });
    for (index, permission) in row.1.iter().enumerate() {
        let enabled = value & [0o4, 0o2, 0o1][index] != 0;
        if enabled {
            permission.add_css_class("enabled");
        } else {
            permission.remove_css_class("enabled");
        }
    }
}

fn format_permissions(mode: u32) -> String {
    let kind = if mode & 0o170000 == 0o040000 {
        'd'
    } else {
        '-'
    };
    let mut symbolic = String::with_capacity(10);
    symbolic.push(kind);
    for (mask, character) in [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ] {
        symbolic.push(if mode & mask != 0 { character } else { '-' });
    }
    format!("{symbolic}  {:03o}", mode & 0o777)
}

fn properties_action(icon: &str, label: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&crate::assets::primary_icon(icon, 14));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("properties-action");
    button
}

pub(super) fn open_location(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let file = gio_file_for_location(location);
    let uri = file.uri();
    if let Err(error) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
        tracing::warn!(
            backend = %location.backend_name(),
            error_domain = ?error.domain(),
            error_code = error.code(),
            "unable to open file"
        );
        tracing::debug!(
            location = %location.diagnostic_path(),
            "file open location"
        );
        show_error_dialog(parent, "Unable to open file", &error.to_string());
    }
}

fn show_error_dialog(parent: &impl IsA<gtk::Widget>, message: &str, detail: &str) {
    let Some(window_overlay) = parent
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
    else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("delete-confirmation");
    content.add_css_class("delete-confirmation-content");
    content.set_halign(gtk::Align::Center);
    content.set_valign(gtk::Align::Center);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("delete-confirmation-header");
    let symbol = gtk::CenterBox::new();
    symbol.add_css_class("delete-confirmation-symbol");
    symbol.set_size_request(40, 40);
    symbol.set_center_widget(Some(&crate::assets::danger_icon(
        crate::assets::icons::X,
        20,
    )));
    let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
    heading.set_hexpand(true);
    let title = gtk::Label::new(Some(message));
    title.add_css_class("delete-confirmation-title");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(Some(if message == "Completed with errors" {
        "Some items could not be processed"
    } else {
        "The operation could not be completed"
    }));
    subtitle.add_css_class("delete-confirmation-subtitle");
    subtitle.set_xalign(0.0);
    heading.append(&title);
    heading.append(&subtitle);
    let close_icon = gtk::Button::new();
    close_icon.add_css_class("delete-confirmation-close");
    close_icon.set_tooltip_text(Some("Close"));
    close_icon.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::X,
        16,
    )));
    header.append(&symbol);
    header.append(&heading);
    header.append(&close_icon);
    content.append(&header);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_css_class("delete-confirmation-body");
    let explanation = gtk::Label::new(Some(detail));
    explanation.add_css_class("delete-confirmation-explanation");
    explanation.set_max_width_chars(64);
    explanation.set_wrap(true);
    explanation.set_xalign(0.0);
    explanation.set_selectable(true);
    body.append(&explanation);
    content.append(&body);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("delete-confirmation-actions");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let close = gtk::Button::with_label("Close");
    close.add_css_class("delete-confirmation-cancel");
    actions.append(&spacer);
    actions.append(&close);
    content.append(&actions);

    let layer = modal_layer(&content);
    window_overlay.add_overlay(&layer);
    let close_layer = layer.clone();
    let close_overlay = window_overlay.clone();
    let close_root = blurred_root.clone();
    let dismiss = move || {
        dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
    };
    let dismiss = Rc::new(dismiss);
    let clicked_dismiss = dismiss.clone();
    close.connect_clicked(move |_| clicked_dismiss());
    let icon_dismiss = dismiss.clone();
    close_icon.connect_clicked(move |_| icon_dismiss());
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            dismiss();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    close.grab_focus();
}

#[cfg(test)]
mod tests;
