// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ffi::OsString,
    future::Future,
    path::Path,
    pin::Pin,
    process::{Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::location_for_file,
    app::{Browser, BrowserEvent},
    model::{EntryKind, FileEntry, Location, SortDirection, SortKey},
    services::{
        ArchiveFormat, FileSource, LoadHandle, LocationValidationError, OperationProvider,
        PasteItem, PreviewContent, SearchEvent, TransferConflict, UriCredentials,
        backend_unavailable_message, content_family, has_plain_text_extension, index_tree,
        is_extensionless_dotfile, sanitize_uri_credentials, validate_basename,
    },
};

use super::{
    blur::BlurBin,
    browser_modes::{BrowserDensity, BrowserMode, ClickActivation, ClickCount, ModeViews},
    controls::{
        ModalTone, form_check_button, form_entry, form_label, form_password_entry, menu_option,
        message_dialog_description, message_dialog_layout, modal_layout, modal_layout_with_tone,
        segmented_control, wrap_dialog_text,
    },
    motion::{animations_enabled, emphasized_deceleration},
};

const COLUMN_WIDTH: i32 = 300;
const COLUMN_OFFSET: i32 = 24;
const COLUMN_TRANSITION: Duration = Duration::from_millis(220);
const PEEK_WIDTH: i32 = 256;
const PEEK_GAP: f32 = 8.0;

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
    header_actions: gtk::Box,
    filter_entry: gtk::Entry,
    filter_button: gtk::ToggleButton,
    selection: gtk::MultiSelection,
    syncing_selection: Rc<Cell<bool>>,
    list: gtk::ListView,
    marquee: super::marquee::Marquee,
    bound_rows: Rc<RefCell<Vec<BoundRow>>>,
    entry_count: Rc<Cell<usize>>,
    spinner: gtk::Spinner,
    truncated_hint: gtk::Image,
    empty_trash_button: Option<gtk::Button>,
    new_entry_row: gtk::Box,
    new_entry_icon: gtk::Image,
    new_entry_entry: gtk::Entry,
    show_hidden: Rc<Cell<bool>>,
    filter: gtk::CustomFilter,
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

const FILE_PROGRESS_DELAY: Duration = Duration::from_millis(350);
const IMMEDIATE_PROGRESS_ITEM_COUNT: usize = 16;

fn should_show_progress_immediately(total: usize) -> bool {
    total == 0 || total >= IMMEDIATE_PROGRESS_ITEM_COUNT
}

struct DeleteProgressView {
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
    progress: gtk::ProgressBar,
    status: gtk::Label,
}

struct TrashLoadingView {
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
}

struct PeekAnchor {
    widget: gtk::Widget,
    origin_depth: usize,
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

pub(super) fn loading_skeleton() -> gtk::Box {
    let skeleton = gtk::Box::new(gtk::Orientation::Vertical, 9);
    skeleton.add_css_class("loading-skeleton");
    for width in [168, 124, 192, 148, 176, 112] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("skeleton-row");
        row.set_size_request(width, 10);
        row.set_halign(gtk::Align::Start);
        skeleton.append(&row);
    }
    skeleton
}

impl LoadPresentation {
    fn new(content: &impl IsA<gtk::Widget>, retry: Option<gtk::Button>) -> Self {
        let skeleton = loading_skeleton();

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
            fade_duration: Duration::from_millis(150),
            item_limit: 8,
        }
    }
}

type PinHandler = Rc<dyn Fn(Location, String)>;
type PinStatusHandler = Rc<dyn Fn(&Location) -> PinStatus>;

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

pub(super) struct ViewState {
    overlay: gtk::Overlay,
    location_control: gtk::Box,
    location_stack: gtk::Stack,
    global_activity_spinner: gtk::Spinner,
    global_activity: RefCell<GlobalActivityState>,
    breadcrumbs: gtk::Box,
    location_entry: gtk::Entry,
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
    peek_anchor: RefCell<Option<PeekAnchor>>,
    peek_behavior: PeekBehavior,
    peek_enabled: Cell<bool>,
    single_click_previews: Cell<bool>,
    columns_click_activation: Cell<ClickActivation>,
    active_rename: RefCell<Option<ActiveRename>>,
    active_new_entry: RefCell<Option<ActiveNewEntry>>,
    delete_progress: RefCell<Option<DeleteProgressView>>,
    pending_file_progress: RefCell<Option<glib::SourceId>>,
    file_operation_progress: Cell<(usize, usize)>,
    pin_handler: RefCell<Option<PinHandler>>,
    pin_status_handler: RefCell<Option<PinStatusHandler>>,
    pending_select: RefCell<Vec<String>>,
    pending_extract_retry: RefCell<Option<(FileEntry, Location)>>,
    pending_navigate: RefCell<Option<Location>>,
    pending_location_credentials: RefCell<Option<MountCredentials>>,
    pending_trash_summary: RefCell<Option<LoadHandle>>,
    pending_empty_trash: RefCell<Option<LoadHandle>>,
    trash_loading: RefCell<Option<TrashLoadingView>>,
    auto_refresh: RefCell<Option<glib::SourceId>>,
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
        scroller.add_css_class("fixed-scrollbar");
        let overlay = gtk::Overlay::new();

        let location_entry = gtk::Entry::builder()
            .hexpand(true)
            .width_chars(48)
            .placeholder_text("Enter an absolute path")
            .tooltip_text("Location (Ctrl+L)")
            .build();
        location_entry.add_css_class("location-entry");
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
            location_control,
            location_stack,
            global_activity_spinner,
            global_activity: RefCell::new(GlobalActivityState::default()),
            breadcrumbs,
            location_entry,
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
            columns_click_activation: Cell::new(ClickActivation::default()),
            active_rename: RefCell::new(None),
            active_new_entry: RefCell::new(None),
            delete_progress: RefCell::new(None),
            pending_file_progress: RefCell::new(None),
            file_operation_progress: Cell::new((0, 0)),
            pin_handler: RefCell::new(None),
            pin_status_handler: RefCell::new(None),
            pending_select: RefCell::new(Vec::new()),
            pending_extract_retry: RefCell::new(None),
            pending_navigate: RefCell::new(None),
            pending_location_credentials: RefCell::new(None),
            pending_trash_summary: RefCell::new(None),
            pending_empty_trash: RefCell::new(None),
            trash_loading: RefCell::new(None),
            auto_refresh: RefCell::new(None),
            browser,
        });

        // Columns are laid out from the start edge, so the blank strip beside the last
        // one is the natural place to begin a marquee that runs into it.
        let weak_state = Rc::downgrade(&state);
        super::marquee::install_shared_origin_surface(&state.scroller, move |surface, _, x, _| {
            let state = weak_state.upgrade()?;
            let laid_out = state.columns_widget.compute_bounds(surface)?;
            if x < f64::from(laid_out.x() + laid_out.width()) {
                return None;
            }
            let columns = state.columns.borrow();
            columns.last().map(|column| column.marquee.clone())
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

    pub(super) fn set_pin_handlers(&self, handler: PinHandler, status_handler: PinStatusHandler) {
        self.state.pin_handler.replace(Some(handler));
        self.state.pin_status_handler.replace(Some(status_handler));
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

    /// Lets a marquee drag begin on blank chrome beside the file panes — the sidebar —
    /// and run into whichever view the current mode shows. The pane nearest the start
    /// edge is the target, since that is the one such a drag runs into.
    pub(super) fn add_marquee_origin(&self, surface: &impl IsA<gtk::Widget>) {
        let weak_state = Rc::downgrade(&self.state);
        super::marquee::install_shared_origin_surface(surface, move |_, _, _, _| {
            let state = weak_state.upgrade()?;
            let mode = state.mode_views.borrow().mode();
            if mode == BrowserMode::Columns {
                return state
                    .columns
                    .borrow()
                    .first()
                    .map(|column| column.marquee.clone());
            }
            state.mode_views.borrow().leading_marquee()
        });
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

    pub fn first_column_has_focus(&self) -> bool {
        self.view_mode() == BrowserMode::Columns && self.state.focused_column_depth() == Some(0)
    }

    pub fn focus_header_from_top_item(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return false;
        }
        let Some((depth, position, _)) = self.state.browser.focused_item() else {
            return false;
        };
        let columns = self.state.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return false;
        };
        if filtered_position_for_source(column, position) != Some(0) {
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
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        self.state.columns.borrow().iter().any(|column| {
            focused.as_ref().is_some_and(|focused| {
                focused == column.header_actions.upcast_ref::<gtk::Widget>()
                    || focused.is_ancestor(&column.header_actions)
            })
        })
    }

    pub fn move_header_focus(&self, direction: gtk::DirectionType) -> bool {
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
        let moved = focus_header_action(&column.header_actions, direction);
        if moved && let Some(window) = self.state.overlay.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        moved
    }

    pub fn focus_items_from_header(&self) -> bool {
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

    pub fn copy_path(&self) -> bool {
        self.state.sync_mode_selection();
        let entries = self.state.browser.selected_entries();
        if entries.is_empty() {
            let Some(entry) = self.state.browser.focused_entry() else {
                return false;
            };
            copy_locations(&[entry]);
        } else {
            copy_locations(&entries);
        }
        true
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
        if self.view_mode() == BrowserMode::Columns {
            if let Some(depth) = self.state.columns.borrow().len().checked_sub(1) {
                self.state.select_all(depth);
            }
        } else if let Some(depth) = self.state.browser.active_depth() {
            self.state.browser.select_all(depth);
        }
    }

    pub fn open_terminal(&self) {
        self.state.sync_mode_selection();
        let selected = self.state.browser.selected_entries();
        let location = selected_terminal_location(&selected).or_else(|| {
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
        self.state.sync_mode_selection();
        let entries = self.state.browser.deletion_entries();
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
        self.state.request_delete(entries, permanent || in_trash);
        true
    }

    pub fn undo_last_trash(&self) -> bool {
        self.state.browser.undo_last_trash()
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

    pub fn item_view_has_focus(&self) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        self.state.mode_views.borrow().item_view_has_focus()
            || self.state.columns.borrow().iter().any(|column| {
                focused.as_ref().is_some_and(|focused| {
                    focused == column.list.upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(&column.list)
                })
            })
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
}

impl ViewState {
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
        let weak_browser = Rc::downgrade(&self.browser);
        let source = glib::timeout_add_local(Duration::from_secs(u64::from(secs)), move || {
            let Some(state) = weak_state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.active_rename.borrow().is_some()
                || state.active_new_entry.borrow().is_some()
                || state.mode_views.borrow().rename_is_active()
                || state.mode_views.borrow().new_entry_is_active()
            {
                return glib::ControlFlow::Continue;
            }
            let Some(browser) = weak_browser.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.mode_views.borrow().mode() == BrowserMode::Columns {
                browser.refresh_all();
            } else {
                browser.reload_active();
            }
            glib::ControlFlow::Continue
        });
        self.auto_refresh.replace(Some(source));
    }

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
        self.resolve_transfer_collisions(destination, collisions, accepted, move_sources);
    }

    fn resolve_transfer_collisions(
        self: &Rc<Self>,
        destination: Location,
        mut collisions: Vec<Location>,
        accepted: Vec<PasteItem>,
        move_sources: bool,
    ) {
        if collisions.is_empty() {
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
        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            "File already exists",
            &name,
            "Replace",
            ModalTone::Danger,
        );
        let explanation = message_dialog_description(&format!(
            "An item named “{name}” already exists in {}. Replacing it will overwrite its contents.",
            compact_display_path(&destination)
        ));
        layout.body.append(&explanation);
        let apply_all = form_check_button("Apply this choice to all remaining conflicts");
        apply_all.set_visible(!collisions.is_empty());
        layout.body.append(&apply_all);
        let skip = gtk::Button::with_label("Skip");
        skip.add_css_class("action-dialog-cancel");
        layout
            .actions
            .insert_child_after(&skip, Some(&layout.cancel));
        let content = layout.content;
        let cancel = layout.cancel;
        let replace = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
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
            );
        });

        let escape = gtk::EventControllerKey::new();
        escape.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        let enter_replace = replace.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                enter_replace.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        replace.grab_focus();
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
        retain_untransferred(&mut self.cut_locations.borrow_mut(), transferred);
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
        let cut_lookup: HashSet<_> = cut.iter().collect();
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
                .is_some_and(|entry| cut_lookup.contains(&entry.location));
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
                .filter_map(|file| location_for_file(&file))
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
        let layout = modal_layout(
            if move_sources {
                crate::assets::icons::FOLDER
            } else {
                crate::assets::icons::COPY
            },
            if move_sources { "Move to" } else { "Copy to" },
            &format!(
                "Choose a destination for {}",
                item_count_label(entries.len())
            ),
            if move_sources {
                "Move here"
            } else {
                "Copy here"
            },
        );
        layout.content.add_css_class("wide");
        let field_label = form_label("Destination folder");
        let field = form_entry();
        field.set_hexpand(true);
        field.set_placeholder_text(Some("Search for a folder…"));
        field.set_text(&folder_input_path(&base));
        field.set_position(-1);
        layout.body.append(&field_label);
        layout.body.append(&field);

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
        layout.body.append(&suggestion_scroll);
        let error = gtk::Label::new(None);
        error.add_css_class("form-message");
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        layout.body.append(&error);
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let confirm = layout.confirm;

        let generation = Rc::new(Cell::new(0_u64));
        let pending_creation = Rc::new(RefCell::new(None::<std::path::PathBuf>));
        let creating_destination = Rc::new(Cell::new(false));
        let suggestions_box = suggestions.clone();
        let suggestions_error = error.clone();
        let changed_confirm = confirm.clone();
        let changed_creation = pending_creation.clone();
        setup_transfer_search(
            &field,
            &suggestions_box,
            &generation,
            base.clone(),
            move |field| {
                field.remove_css_class("error");
                suggestions_error.set_visible(false);
                suggestions_error.remove_css_class("warning");
                suggestions_error.add_css_class("error");
                changed_creation.borrow_mut().take();
                changed_confirm.set_label(if move_sources {
                    "Move here"
                } else {
                    "Copy here"
                });
            },
        );

        let initial_text = folder_input_path(&base);
        let dirty_field = field.clone();
        let dirty_creating = creating_destination.clone();
        let layer = modal_layer(
            &content,
            &window_overlay,
            blurred_root.clone(),
            Some(Rc::new(move || {
                dirty_creating.get() || dirty_field.text() != initial_text
            })),
        );
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
                confirm_error.remove_css_class("warning");
                confirm_error.add_css_class("error");
                confirm_error.set_text("The destination exists, but it is not a folder.");
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                confirm_field.grab_focus();
                return;
            }
            if !path.exists() && confirm_creation.borrow().as_ref() != Some(&path) {
                confirm_creation.replace(Some(path.clone()));
                confirm_error.remove_css_class("error");
                confirm_error.add_css_class("warning");
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
                transfer_state
                    .pending_navigate
                    .replace(Some(Location::local(path.clone())));
                let names: Vec<String> = sources
                    .iter()
                    .filter_map(|s| s.native_path()?.file_name()?.to_str().map(String::from))
                    .collect();
                transfer_state.pending_select.borrow_mut().extend(names);
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
                        created_state
                            .pending_navigate
                            .replace(Some(Location::local(path.clone())));
                        let names: Vec<String> = created_sources
                            .iter()
                            .filter_map(|s| {
                                s.native_path()?.file_name()?.to_str().map(String::from)
                            })
                            .collect();
                        created_state.pending_select.borrow_mut().extend(names);
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
                        created_error.remove_css_class("warning");
                        created_error.add_css_class("error");
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
                        created_error.remove_css_class("warning");
                        created_error.add_css_class("error");
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

        field.emit_by_name::<()>("changed", &[]);
        field.grab_focus();
    }

    fn show_file_operation_progress(
        self: &Rc<Self>,
        total: usize,
        icon: &str,
        title_text: &str,
        subtitle_text: &str,
        on_cancel: Rc<dyn Fn()>,
    ) {
        self.dismiss_delete_progress();
        self.file_operation_progress.set((0, total));
        if should_show_progress_immediately(total) {
            self.present_file_operation_progress(icon, title_text, subtitle_text, on_cancel);
            return;
        }

        let weak = Rc::downgrade(self);
        let icon = icon.to_owned();
        let title_text = title_text.to_owned();
        let subtitle_text = subtitle_text.to_owned();
        let source = glib::timeout_add_local_once(FILE_PROGRESS_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.pending_file_progress.borrow_mut().take();
            state.present_file_operation_progress(&icon, &title_text, &subtitle_text, on_cancel);
        });
        self.pending_file_progress.replace(Some(source));
    }

    fn present_file_operation_progress(
        self: &Rc<Self>,
        icon: &str,
        title_text: &str,
        subtitle_text: &str,
        on_cancel: Rc<dyn Fn()>,
    ) {
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

        let layout = modal_layout(icon, title_text, subtitle_text, "Cancel");
        layout.content.add_css_class("compact");
        layout.close.set_visible(false);
        layout.cancel.set_visible(false);
        let status = gtk::Label::new(Some("0%"));
        status.add_css_class("modal-progress-status");
        status.set_xalign(0.0);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("modal-progress");
        progress.set_fraction(0.0);
        layout.body.append(&status);
        layout.body.append(&progress);
        let content = layout.content;
        let cancel = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        self.delete_progress.replace(Some(DeleteProgressView {
            layer,
            overlay: window_overlay,
            blurred_root,
            progress,
            status,
        }));
        let cancel_action = on_cancel.clone();
        cancel.connect_clicked(move |_| cancel_action());
        let escape = gtk::EventControllerKey::new();
        let escape_action = on_cancel;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                escape_action();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        if let Some(progress) = self.delete_progress.borrow().as_ref() {
            progress.layer.add_controller(escape);
        }
        cancel.grab_focus();
        let (completed, total) = self.file_operation_progress.get();
        self.update_delete_progress(completed, total);
    }

    fn update_delete_progress(&self, completed: usize, total: usize) {
        self.file_operation_progress.set((completed, total));
        let progress_view = self.delete_progress.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        let pct = if total > 0 {
            (completed as f64 / total as f64 * 100.0) as usize
        } else {
            0
        };
        view.status.set_text(&format!("{pct}%"));
        view.progress
            .set_fraction(completed as f64 / total.max(1) as f64);
    }

    fn update_archive_progress(&self, completed: usize, total: usize) {
        let progress_view = self.delete_progress.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        if total == 0 {
            view.status.set_text(&format!("{completed} files"));
            view.progress.pulse();
        } else {
            view.status
                .set_text(&format!("{completed} / {total} files"));
            view.progress.set_fraction(completed as f64 / total as f64);
        }
    }

    fn dismiss_delete_progress(&self) {
        if let Some(source) = self.pending_file_progress.take() {
            source.remove();
        }
        self.file_operation_progress.set((0, 0));
        if let Some(view) = self.delete_progress.take() {
            dismiss_modal_layer(&view.layer, &view.overlay, view.blurred_root.as_ref());
        }
    }

    /// The total item count isn't known upfront -- entries are deleted as they're enumerated,
    /// one bounded batch at a time -- so this pulses rather than fills to a fraction.
    fn show_empty_trash_progress(self: &Rc<Self>, on_cancel: Rc<dyn Fn()>) {
        self.show_file_operation_progress(
            0,
            crate::assets::icons::TRASH,
            "Emptying Trash",
            "This may take a moment",
            on_cancel,
        );
        self.update_empty_trash_progress(0);
    }

    fn update_empty_trash_progress(&self, processed: usize) {
        let progress_view = self.delete_progress.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        view.status
            .set_text(&format!("{} deleted", item_count_label(processed)));
        view.progress.pulse();
    }

    /// Safe to call more than once: whichever of cancel or completion runs first leaves the
    /// other a no-op.
    fn clear_empty_trash(&self) {
        self.pending_empty_trash.borrow_mut().take();
        self.dismiss_delete_progress();
    }

    fn load_trash_summary(self: &Rc<Self>) {
        let trash_empty = self
            .columns
            .borrow()
            .iter()
            .find(|column| column.empty_trash_button.is_some())
            .is_some_and(|column| column.entry_count.get() == 0);
        if trash_empty {
            return;
        }
        self.show_trash_loading_indicator();
        let weak = Rc::downgrade(self);
        let started = Instant::now();
        let task = glib::MainContext::default().spawn_local(async move {
            // Let GTK paint the loading dialog before beginning a walk whose GIO futures may be
            // immediately ready for long stretches on a fast local trash backend.
            glib::timeout_future(Duration::from_millis(16)).await;
            let trash = gio::File::for_uri("trash:///");
            match summarize_trash(&trash).await {
                Ok(summary) if summary.item_count > 0 => {
                    if summary.truncated {
                        tracing::warn!(
                            item_count = summary.item_count,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "trash summary truncated"
                        );
                    } else {
                        tracing::info!(
                            item_count = summary.item_count,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "trash summary built"
                        );
                    }
                    if let Some(state) = weak.upgrade() {
                        state.clear_trash_loading();
                        state.show_empty_trash_confirmation(summary);
                    }
                }
                Ok(_) => {
                    if let Some(state) = weak.upgrade() {
                        state.clear_trash_loading();
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        error_domain = ?error.domain(),
                        error_code = error.code(),
                        "trash summary failed"
                    );
                    if let Some(state) = weak.upgrade() {
                        state.clear_trash_loading();
                        show_error_dialog(
                            &state.overlay,
                            "Unable to read Trash",
                            &error.to_string(),
                        );
                    }
                }
            }
        });
        self.pending_trash_summary
            .replace(Some(LoadHandle::new(move || {
                tracing::debug!("trash summary cancelled");
                task.abort();
            })));
    }

    /// The walk is bounded but can still take a few seconds on a large trash, hence the indicator.
    fn show_trash_loading_indicator(self: &Rc<Self>) {
        self.dismiss_trash_loading();
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

        let layout = modal_layout(
            crate::assets::icons::TRASH,
            "Measuring Trash…",
            "",
            "Empty Trash",
        );
        layout.set_loading(true, Some("Measuring Trash…"));
        layout.subtitle.set_visible(false);
        layout.confirm.set_visible(false);
        let explanation = message_dialog_description(
            "Calculating the number and size of items. This may take a few seconds.",
        );
        layout.body.append(&explanation);
        let content = layout.content;
        let cancel = layout.cancel;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        self.trash_loading.replace(Some(TrashLoadingView {
            layer,
            overlay: window_overlay,
            blurred_root,
        }));

        let weak = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.clear_trash_loading();
            }
        });
        let escape = gtk::EventControllerKey::new();
        let weak_escape = Rc::downgrade(self);
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(state) = weak_escape.upgrade() {
                    state.clear_trash_loading();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        if let Some(view) = self.trash_loading.borrow().as_ref() {
            view.layer.add_controller(escape);
        }
        cancel.grab_focus();
    }

    fn dismiss_trash_loading(&self) {
        let Some(view) = self.trash_loading.take() else {
            return;
        };
        dismiss_modal_layer(&view.layer, &view.overlay, view.blurred_root.as_ref());
    }

    /// Safe to call more than once: whichever of cancel or completion runs first leaves the
    /// other a no-op.
    fn clear_trash_loading(&self) {
        self.pending_trash_summary.borrow_mut().take();
        self.dismiss_trash_loading();
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

        let layout = message_dialog_layout(
            crate::assets::icons::TRASH,
            "Empty Trash?",
            &format!(
                "{}{} · {}{} will be reclaimed",
                if summary.truncated { "At least " } else { "" },
                item_count_label(summary.item_count),
                if summary.truncated { "at least " } else { "" },
                format_file_size(summary.total_size)
            ),
            "Empty Trash",
            ModalTone::Danger,
        );
        let explanation = message_dialog_description(
            "Everything in Trash will be permanently deleted. This action cannot be undone.",
        );
        layout.body.append(&explanation);
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let empty = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        let cancel_browser = self.browser.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
            cancel_browser.focus_active();
        });
        let close_layer = layer.clone();
        let close_overlay = window_overlay.clone();
        let close_root = blurred_root.clone();
        let close_browser = self.browser.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
            close_browser.focus_active();
        });
        let empty_layer = layer.clone();
        let empty_overlay = window_overlay.clone();
        let empty_root = blurred_root.clone();
        let browser = self.browser.clone();
        let error_overlay = self.overlay.clone();
        let weak_ui = Rc::downgrade(self);
        empty.connect_clicked(move |_| {
            dismiss_modal_layer(&empty_layer, &empty_overlay, empty_root.as_ref());
            browser.focus_active();

            let cancel_ui = weak_ui.clone();
            let on_cancel: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(ui) = cancel_ui.upgrade() {
                    ui.clear_empty_trash();
                }
            });
            if let Some(ui) = weak_ui.upgrade() {
                ui.show_empty_trash_progress(on_cancel);
            }

            let error_overlay = error_overlay.clone();
            let progress_ui = weak_ui.clone();
            let finish_ui = weak_ui.clone();
            let finish_browser = browser.clone();
            let task = glib::MainContext::default().spawn_local(async move {
                let trash = gio::File::for_uri("trash:///");
                let result = empty_trash(&trash, move |processed| {
                    if let Some(ui) = progress_ui.upgrade() {
                        ui.update_empty_trash_progress(processed);
                    }
                })
                .await;
                if let Some(ui) = finish_ui.upgrade() {
                    ui.clear_empty_trash();
                }
                match result {
                    Ok(outcome) => {
                        // A wholesale refresh rather than tracking each deleted location, to
                        // keep this operation's own state bounded too.
                        finish_browser.refresh_columns_at(&Location::uri("trash:///"));
                        if outcome.failed > 0 {
                            show_error_dialog(
                                &error_overlay,
                                "Completed with errors",
                                &empty_trash_error_summary(&outcome),
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            error_domain = ?error.domain(),
                            error_code = error.code(),
                            "empty trash failed"
                        );
                        show_error_dialog(
                            &error_overlay,
                            "Unable to empty Trash",
                            &error.to_string(),
                        );
                    }
                }
            });
            if let Some(ui) = weak_ui.upgrade() {
                ui.pending_empty_trash
                    .replace(Some(LoadHandle::new(move || {
                        tracing::debug!("empty trash cancelled");
                        task.abort();
                    })));
            }
        });
        let keys = gtk::EventControllerKey::new();
        let escape_layer = layer.clone();
        let focused_layer = layer.clone();
        let escape_overlay = window_overlay.clone();
        let escape_root = blurred_root.clone();
        let escape_browser = self.browser.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
                escape_browser.focus_active();
                glib::Propagation::Stop
            } else if !modifiers
                .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
                && let Some(direction) = vim_focus_direction(key)
            {
                focused_layer.child_focus(direction);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(keys);
        let initial_focus = cancel.clone();
        glib::idle_add_local_once(move || {
            initial_focus.grab_focus();
            if let Some(window) = initial_focus.root().and_downcast::<gtk::Window>() {
                window.set_focus_visible(true);
            }
        });
    }

    fn request_delete(self: &Rc<Self>, entries: Vec<FileEntry>, permanent: bool) {
        if permanent {
            self.show_delete_confirmation(entries);
        } else {
            self.browser.delete(entries, false);
            self.browser.focus_active();
        }
    }

    fn show_delete_confirmation(self: &Rc<Self>, entries: Vec<FileEntry>) {
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
        let title = format!("Permanently delete {}?", item_count_label(count));
        let confirm_label = format!("Permanently delete {}", item_count_label(count));
        let layout = message_dialog_layout(
            crate::assets::icons::TRASH,
            &title,
            &entry_kind_summary(&entries),
            &confirm_label,
            ModalTone::Danger,
        );
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
        layout.body.append(&file_scroller);
        let explanation = message_dialog_description(
            "These items will be permanently deleted. This action cannot be undone.",
        );
        layout.body.append(&explanation);
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let confirm = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        let cancelled_layer = layer.clone();
        let cancelled_overlay = window_overlay.clone();
        let cancelled_root = blurred_root.clone();
        let cancelled_browser = self.browser.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(
                &cancelled_layer,
                &cancelled_overlay,
                cancelled_root.as_ref(),
            );
            cancelled_browser.focus_active();
        });
        let closed_layer = layer.clone();
        let closed_overlay = window_overlay.clone();
        let closed_root = blurred_root.clone();
        let closed_browser = self.browser.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&closed_layer, &closed_overlay, closed_root.as_ref());
            closed_browser.focus_active();
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
            browser.delete(entries.clone(), true);
            browser.focus_active();
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        let escaped_browser = self.browser.clone();
        let focused_cancel = cancel.clone();
        let focused_confirm = confirm.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                escaped_browser.focus_active();
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                focused_confirm.emit_clicked();
                glib::Propagation::Stop
            } else if !modifiers
                .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
            {
                match delete_confirmation_focus_target(key) {
                    Some(DeleteConfirmationFocus::Cancel) => focused_cancel.grab_focus(),
                    Some(DeleteConfirmationFocus::Confirm) => focused_confirm.grab_focus(),
                    None => return glib::Propagation::Proceed,
                };
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(keys);
        let initial_focus = confirm.clone();
        glib::idle_add_local_once(move || {
            initial_focus.grab_focus();
            if let Some(window) = initial_focus.root().and_downcast::<gtk::Window>() {
                window.set_focus_visible(true);
            }
        });
    }

    fn show_folder_properties(self: &Rc<Self>, location: &Location) {
        self.show_properties(location.clone(), None);
    }

    fn show_entry_properties(self: &Rc<Self>, entry: FileEntry) {
        self.show_properties(entry.location.clone(), Some(entry));
    }

    fn build_archive_modal(
        self: &Rc<Self>,
        title: &str,
        subtitle: &str,
        confirm_label: &str,
        block_dismiss: Option<Rc<dyn Fn() -> bool>>,
    ) -> (gtk::Box, gtk::Button, Rc<dyn Fn()>) {
        let Some(window_overlay) = self
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.child())
            .and_downcast::<gtk::Overlay>()
        else {
            return (gtk::Box::default(), gtk::Button::default(), Rc::new(|| {}));
        };
        let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = blurred_root.as_ref() {
            root.set_blurred(true);
        }

        let layout = modal_layout(
            crate::assets::icons::FILE_ARCHIVE,
            title,
            subtitle,
            confirm_label,
        );
        let layer = modal_layer(
            &layout.content,
            &window_overlay,
            blurred_root.clone(),
            block_dismiss,
        );
        window_overlay.add_overlay(&layer);

        let dismiss: Rc<dyn Fn()> = Rc::new({
            let layer = layer.clone();
            let overlay = window_overlay.clone();
            let root = blurred_root.clone();
            move || dismiss_modal_layer(&layer, &overlay, root.as_ref())
        });
        let dismiss_for_cancel = dismiss.clone();
        layout.cancel.connect_clicked(move |_| dismiss_for_cancel());
        let dismiss_for_close = dismiss.clone();
        layout.close.connect_clicked(move |_| dismiss_for_close());
        let escape = gtk::EventControllerKey::new();
        let dismiss_for_escape = dismiss.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_for_escape();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        (layout.body, layout.confirm, dismiss)
    }

    fn start_compression(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        destination: Location,
        archive_name: String,
        format: ArchiveFormat,
        password: Option<String>,
    ) {
        let final_name = format!("{archive_name}.{}", format.extension());
        if !archive_has_collision(&destination, &final_name) {
            self.browser.compress(
                entries,
                destination,
                archive_name,
                TransferConflict::FailIfExists,
                format,
                password,
            );
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
        let layout = message_dialog_layout(
            crate::assets::icons::FILE_ARCHIVE,
            "File already exists",
            &final_name,
            "Replace",
            ModalTone::Danger,
        );
        layout.body.append(&message_dialog_description(&format!(
            "An archive named “{final_name}” already exists in {}. Replacing it will overwrite its contents.",
            compact_display_path(&destination)
        )));
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let replace = layout.confirm;
        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);

        for button in [&close, &cancel] {
            let dismissed_layer = layer.clone();
            let dismissed_overlay = window_overlay.clone();
            let dismissed_root = blurred_root.clone();
            let browser = self.browser.clone();
            button.connect_clicked(move |_| {
                dismiss_modal_layer(
                    &dismissed_layer,
                    &dismissed_overlay,
                    dismissed_root.as_ref(),
                );
                browser.focus_active();
            });
        }

        let replaced_layer = layer.clone();
        let replaced_overlay = window_overlay.clone();
        let replaced_root = blurred_root.clone();
        let browser = self.browser.clone();
        replace.connect_clicked(move |_| {
            dismiss_modal_layer(&replaced_layer, &replaced_overlay, replaced_root.as_ref());
            browser.compress(
                entries.clone(),
                destination.clone(),
                archive_name.clone(),
                TransferConflict::ReplaceExisting,
                format,
                password.clone(),
            );
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        let enter_replace = replace.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                enter_replace.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(keys);
        replace.grab_focus();
    }

    fn show_compress_dialog(self: &Rc<Self>, entries: Vec<FileEntry>) {
        if entries.is_empty() {
            return;
        }
        let destination = self
            .browser
            .active_location()
            .unwrap_or_else(|| Location::local(glib::home_dir()));

        let default_name = if entries.len() == 1 {
            entries[0].display_name.clone()
        } else {
            "archive".to_owned()
        };

        let title = format!("Compress {}", item_count_label(entries.len()));
        let subtitle = entry_kind_summary(&entries);

        let name_entry = form_entry();
        name_entry.set_text(&default_name);
        name_entry.connect_changed(|field| {
            update_basename_validation(field);
        });
        let password_entry = form_password_entry();
        password_entry.set_show_peek_icon(true);
        let confirm_entry = form_password_entry();
        confirm_entry.set_show_peek_icon(true);
        let compress_default_name = default_name.clone();
        let dirty_name = name_entry.clone();
        let dirty_password = password_entry.clone();
        let dirty_confirm = confirm_entry.clone();
        let (body, confirm, dismiss) = self.build_archive_modal(
            &title,
            &subtitle,
            "Compress",
            Some(Rc::new(move || {
                dirty_name.text() != compress_default_name
                    || !dirty_password.text().is_empty()
                    || !dirty_confirm.text().is_empty()
            })),
        );

        let name_label = form_label("Archive name");
        body.append(&name_label);
        body.append(&name_entry);

        let format_label = form_label("Format");
        let (format_control, format_options) =
            segmented_control(&["ZIP", "7Z", "TAR.GZ", "TAR"], 0);
        let selected_format = Rc::new(Cell::new(ArchiveFormat::Zip));
        body.append(&format_label);
        body.append(&format_control);

        let protection_label = form_label("Protection");
        let (protection_control, protection_options) =
            segmented_control(&["No password", "Password protected"], 0);
        let no_password = protection_options[0].clone();
        let password_protected = protection_options[1].clone();

        let password_label = form_label("Password");
        let confirm_label = form_label("Confirm password");
        let password_fields = gtk::Box::new(gtk::Orientation::Vertical, 6);
        password_fields.append(&password_label);
        password_fields.append(&password_entry);
        password_fields.append(&confirm_label);
        password_fields.append(&confirm_entry);
        password_fields.set_visible(false);

        let protection_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        protection_box.append(&protection_label);
        protection_box.append(&protection_control);
        protection_box.append(&password_fields);
        body.append(&protection_box);

        let fields_for_protection = password_fields.clone();
        let password_for_focus = password_entry.clone();
        password_protected.connect_toggled(move |option| {
            fields_for_protection.set_visible(option.is_active());
            if option.is_active() {
                password_for_focus.grab_focus();
            }
        });

        for (option, format) in format_options.into_iter().zip([
            ArchiveFormat::Zip,
            ArchiveFormat::SevenZ,
            ArchiveFormat::TarGz,
            ArchiveFormat::Tar,
        ]) {
            let selected_format = selected_format.clone();
            let protection_for_format = protection_box.clone();
            let no_password_for_format = no_password.clone();
            option.connect_toggled(move |option| {
                if !option.is_active() {
                    return;
                }
                selected_format.set(format);
                let supported = format.supports_password();
                protection_for_format.set_visible(supported);
                if !supported {
                    no_password_for_format.set_active(true);
                }
            });
        }

        let state = Rc::downgrade(self);
        let confirm_entries = entries.clone();
        let confirm_destination = destination.clone();
        let name_for_confirm = name_entry.clone();
        let format_for_confirm = selected_format.clone();
        let password_for_confirm = password_entry.clone();
        let confirm_for_confirm = confirm_entry.clone();
        let protected_for_confirm = password_protected.clone();
        let overlay_for_error = self.overlay.clone();
        let dismiss_for_confirm = dismiss.clone();
        confirm.connect_clicked(move |_| {
            let name = name_for_confirm.text().to_string();
            let format = format_for_confirm.get();
            let archive_name = normalized_archive_name(&name, format);
            if let Err(message) = validate_basename(&archive_name) {
                name_for_confirm.add_css_class("error");
                name_for_confirm.set_tooltip_text(Some(message));
                name_for_confirm.grab_focus();
                return;
            }
            let password = if format.supports_password() && protected_for_confirm.is_active() {
                let pw = password_for_confirm.text().to_string();
                if pw.is_empty() {
                    show_error_dialog(
                        &overlay_for_error,
                        "Password required",
                        "Enter a password or choose No password.",
                    );
                    return;
                }
                let confirm_pw = confirm_for_confirm.text().to_string();
                if pw != confirm_pw {
                    show_error_dialog(
                        &overlay_for_error,
                        "Passwords do not match",
                        "Please enter the same password in both fields.",
                    );
                    return;
                }
                Some(pw)
            } else {
                None
            };
            dismiss_for_confirm();
            if let Some(state) = state.upgrade() {
                state.start_compression(
                    confirm_entries.clone(),
                    confirm_destination.clone(),
                    archive_name,
                    format,
                    password,
                );
            }
        });
        name_entry.grab_focus();
    }

    fn extract_entry(self: &Rc<Self>, entry: FileEntry) {
        let Some(parent) = entry.location.parent() else {
            show_error_dialog(
                &self.overlay,
                "Cannot extract",
                "This archive has no parent directory.",
            );
            return;
        };
        let format = ArchiveFormat::from_extension(&entry.display_name);
        if format.map(|f| f.supports_password()).unwrap_or(false) {
            self.pending_extract_retry
                .replace(Some((entry.clone(), parent.clone())));
        }
        self.browser.extract(entry, parent, None);
    }

    fn show_extract_to_dialog(self: &Rc<Self>, entry: FileEntry) {
        let base = entry
            .location
            .parent()
            .and_then(|p| p.native_path().map(Path::to_path_buf))
            .unwrap_or_else(glib::home_dir);
        let field = form_entry();
        field.set_hexpand(true);
        field.set_placeholder_text(Some("Search for a folder…"));
        field.set_text(&folder_input_path(&base));
        field.set_position(-1);
        let extract_initial_text = folder_input_path(&base);
        let dirty_field = field.clone();
        let (body, confirm, dismiss) = self.build_archive_modal(
            "Extract to",
            &entry.display_name,
            "Extract here",
            Some(Rc::new(move || dirty_field.text() != extract_initial_text)),
        );
        let field_label = form_label("Destination folder");
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
        error.add_css_class("form-message");
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        body.append(&error);

        let generation = Rc::new(Cell::new(0_u64));
        let suggestions_box = suggestions.clone();
        let extract_error = error.clone();
        setup_transfer_search(
            &field,
            &suggestions_box,
            &generation,
            base.clone(),
            move |field| {
                field.remove_css_class("error");
                extract_error.set_visible(false);
            },
        );

        let extract_state = self.clone();
        let confirm_field = field.clone();
        let confirm_error = error.clone();
        let confirm_base = base.clone();
        let extract_entry = entry.clone();
        let dismiss_for_confirm = dismiss.clone();
        confirm.connect_clicked(move |_| {
            let path =
                resolve_destination_path(&confirm_field.text(), &confirm_base, &glib::home_dir());
            if path.exists() && !path.is_dir() {
                confirm_error.set_text("The destination exists, but it is not a folder.");
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                confirm_field.grab_focus();
                return;
            }
            if !path.exists()
                && let Err(e) = std::fs::create_dir_all(&path)
            {
                confirm_error.set_text(&format!("Could not create folder: {e}"));
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                return;
            }
            let dest = Location::local(path);
            let format = ArchiveFormat::from_extension(&extract_entry.display_name);
            if format.map(|f| f.supports_password()).unwrap_or(false) {
                extract_state
                    .pending_extract_retry
                    .replace(Some((extract_entry.clone(), dest.clone())));
            }
            extract_state.pending_navigate.replace(Some(dest.clone()));
            extract_state
                .browser
                .extract(extract_entry.clone(), dest, None);
            dismiss_for_confirm();
        });

        field.grab_focus();
    }

    fn show_extract_password_dialog(self: &Rc<Self>, entry: FileEntry, destination: Location) {
        let password_entry = form_password_entry();
        password_entry.set_show_peek_icon(true);
        let dirty_password = password_entry.clone();
        let (body, confirm, dismiss) = self.build_archive_modal(
            "Extract",
            &entry.display_name,
            "Extract",
            Some(Rc::new(move || !dirty_password.text().is_empty())),
        );

        let password_label = form_label("Password");
        body.append(&password_label);
        body.append(&password_entry);

        let browser = self.browser.clone();
        let password_for_confirm = password_entry.clone();
        let dismiss_for_confirm = dismiss.clone();
        confirm.connect_clicked(move |_| {
            let pw = password_for_confirm.text().to_string();
            let password = if pw.is_empty() { None } else { Some(pw) };
            dismiss_for_confirm();
            browser.extract(entry.clone(), destination.clone(), password);
        });
        password_entry.grab_focus();
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

        let layout = modal_layout(
            icon_name,
            &name,
            if is_directory { "Folder" } else { "File" },
            "Close",
        );
        layout.content.add_css_class("properties-content");
        layout
            .title
            .set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        layout.cancel.set_visible(false);
        layout.confirm.set_visible(false);
        let kind = layout.subtitle.clone();
        let close = layout.close.clone();

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
        let modified = properties_row(&details, "MODIFIED", "—");
        crate::util::set_modified_date(&modified, entry.as_ref(), "—");
        let opens_with = properties_row(&details, "OPENS WITH", "—");
        let hidden = properties_row(
            &details,
            "HIDDEN",
            if name.starts_with('.') { "Yes" } else { "No" },
        );
        let pin_status = self
            .pin_status_handler
            .borrow()
            .as_ref()
            .map_or(PinStatus::Unavailable, |handler| handler(&location));
        let _pinned = properties_row(
            &details,
            "PINNED",
            if pin_status == PinStatus::Pinned {
                "Yes"
            } else {
                "No"
            },
        );
        layout.body.append(&details);

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
        layout.body.append(&permissions);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.add_css_class("properties-actions");
        let open = properties_action(crate::assets::icons::EXTERNAL_LINK, "Open");
        let rename = properties_action(crate::assets::icons::PENCIL, "Rename");
        rename.set_sensitive(entry.is_some());
        let pin = properties_action(crate::assets::icons::PIN, "Pin");
        let pin_handler = self.pin_handler.borrow().clone();
        pin.set_sensitive(
            is_directory
                && !is_trash_location(&location)
                && pin_handler.is_some()
                && pin_status == PinStatus::Available,
        );
        let copy_path = properties_action(crate::assets::icons::COPY, "Copy path");
        actions.append(&open);
        actions.append(&rename);
        actions.append(&pin);
        actions.append(&copy_path);
        layout.actions.prepend(&actions);
        let content = layout.content;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
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
        let pinning_layer = layer.clone();
        let pinning_overlay = window_overlay.clone();
        let pinning_root = blurred_root.clone();
        let pinning_location = location.clone();
        let pinning_name = name.clone();
        pin.connect_clicked(move |_| {
            if let Some(handler) = pin_handler.as_ref() {
                handler(pinning_location.clone(), pinning_name.clone());
            }
            dismiss_modal_layer(&pinning_layer, &pinning_overlay, pinning_root.as_ref());
        });
        let copied_location = location.clone();
        copy_path.connect_clicked(move |button| {
            if let Some(display) = gtk::gdk::Display::default() {
                display
                    .clipboard()
                    .set_text(&copy_path_text(&copied_location, true));
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
                        let prefix = if summary.truncated { "≥ " } else { "" };
                        size.set_text(&format!("{prefix}{}", format_file_size(summary.total_size)));
                        size.set_tooltip_text(Some(&format!(
                            "{prefix}{}",
                            item_count_label(summary.item_count)
                        )));
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
        self.location_stack.set_visible_child_name("entry");
        self.location_entry.grab_focus();
        self.location_entry.select_region(0, -1);
    }

    fn cancel_location_edit(&self) {
        self.restore_location_text();
        self.location_stack.set_visible_child_name("breadcrumbs");
        self.browser.focus_active();
    }

    fn submit_location(self: &Rc<Self>) {
        let input = self.location_entry.text();
        let (input, credentials) = match credentials_from_location_input(input.as_str()) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.restore_location_text();
                self.location_stack.set_visible_child_name("breadcrumbs");
                show_error_dialog(&self.overlay, "Unable to open location", &error.to_string());
                return;
            }
        };
        if credentials.is_some() {
            self.location_entry.set_text(&input);
        }
        self.pending_location_credentials.replace(credentials);
        match self.browser.navigate_input(&input) {
            Ok(()) => {
                self.location_stack.set_visible_child_name("breadcrumbs");
                self.browser.focus_active();
            }
            Err(LocationValidationError::NotMounted(location)) => {
                let credentials = self.pending_location_credentials.take();
                self.mount_then_navigate_with_credentials(
                    location,
                    MountStrategy::EnclosingVolume,
                    credentials,
                );
            }
            Err(LocationValidationError::Mountable(location)) => {
                let credentials = self.pending_location_credentials.take();
                self.mount_then_navigate_with_credentials(
                    location,
                    MountStrategy::Mountable,
                    credentials,
                );
            }
            Err(error) => {
                self.pending_location_credentials.take();
                self.restore_location_text();
                self.location_stack.set_visible_child_name("breadcrumbs");
                show_error_dialog(&self.overlay, "Unable to open location", &error.to_string());
            }
        }
    }

    fn handle_navigation_rejected(
        self: &Rc<Self>,
        parent_depth: usize,
        error: LocationValidationError,
    ) {
        match error {
            LocationValidationError::NotMounted(location) => {
                self.mount_then_descend(parent_depth, location, MountStrategy::EnclosingVolume);
            }
            LocationValidationError::Mountable(location) => {
                self.mount_then_descend(parent_depth, location, MountStrategy::Mountable);
            }
            error => {
                show_error_dialog(
                    &self.overlay,
                    "Unable to open directory",
                    &error.to_string(),
                );
            }
        }
    }

    fn mount_then_navigate_with_credentials(
        self: &Rc<Self>,
        location: Location,
        strategy: MountStrategy,
        credentials: Option<MountCredentials>,
    ) {
        self.mount_location(
            location.clone(),
            strategy,
            credentials,
            move |state, outcome| match outcome {
                MountOutcome::Mounted => {
                    state.browser.navigate(location.clone());
                    state.location_stack.set_visible_child_name("breadcrumbs");
                    state.browser.focus_active();
                }
                MountOutcome::NeedsCredentials { attempted, details } => {
                    state.prompt_to_retry_navigation(location.clone(), strategy, attempted, details)
                }
                MountOutcome::Cancelled => {
                    state.restore_location_text();
                    state.location_stack.set_visible_child_name("breadcrumbs");
                    state.browser.focus_active();
                }
                MountOutcome::Failed(message) => {
                    state.restore_location_text();
                    state.location_stack.set_visible_child_name("breadcrumbs");
                    show_error_dialog(&state.overlay, "Unable to connect", &message);
                }
            },
        );
    }

    fn mount_then_descend(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        strategy: MountStrategy,
    ) {
        self.mount_then_descend_with_credentials(parent_depth, location, strategy, None);
    }

    fn mount_then_descend_with_credentials(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        strategy: MountStrategy,
        credentials: Option<MountCredentials>,
    ) {
        self.mount_location(
            location.clone(),
            strategy,
            credentials,
            move |state, outcome| match outcome {
                MountOutcome::Mounted => state.browser.descend(parent_depth, location.clone()),
                MountOutcome::NeedsCredentials { attempted, details } => state
                    .prompt_to_retry_descend(
                        parent_depth,
                        location.clone(),
                        strategy,
                        attempted,
                        details,
                    ),
                MountOutcome::Cancelled => {}
                MountOutcome::Failed(message) => {
                    show_error_dialog(&state.overlay, "Unable to connect", &message);
                }
            },
        );
    }

    fn prompt_to_retry_navigation(
        self: &Rc<Self>,
        location: Location,
        strategy: MountStrategy,
        previous_credentials: Option<MountCredentials>,
        prompt_details: Option<MountPromptDetails>,
    ) {
        let weak = Rc::downgrade(self);
        let cancel_weak = weak.clone();
        let prompt_location = location.clone();
        self.show_mount_retry_prompt(
            &prompt_location,
            previous_credentials,
            prompt_details,
            move |credentials| {
                if let Some(state) = weak.upgrade() {
                    state.mount_then_navigate_with_credentials(
                        location.clone(),
                        strategy,
                        Some(credentials),
                    );
                }
            },
            move || {
                if let Some(state) = cancel_weak.upgrade() {
                    state.restore_location_text();
                    state.location_stack.set_visible_child_name("breadcrumbs");
                    state.browser.focus_active();
                }
            },
        );
    }

    fn prompt_to_retry_descend(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        strategy: MountStrategy,
        previous_credentials: Option<MountCredentials>,
        prompt_details: Option<MountPromptDetails>,
    ) {
        let weak = Rc::downgrade(self);
        let prompt_location = location.clone();
        self.show_mount_retry_prompt(
            &prompt_location,
            previous_credentials,
            prompt_details,
            move |credentials| {
                if let Some(state) = weak.upgrade() {
                    state.mount_then_descend_with_credentials(
                        parent_depth,
                        location.clone(),
                        strategy,
                        Some(credentials),
                    );
                }
            },
            || {},
        );
    }

    fn show_mount_retry_prompt(
        &self,
        location: &Location,
        previous_credentials: Option<MountCredentials>,
        prompt_details: Option<MountPromptDetails>,
        retry: impl Fn(MountCredentials) + 'static,
        cancelled: impl Fn() + 'static,
    ) {
        let authentication_failed = previous_credentials.is_some();
        let details = prompt_details.unwrap_or_else(|| MountPromptDetails::fallback(location));
        let defaults = previous_credentials.unwrap_or_else(|| {
            let mut defaults = MountCredentials::default_for_prompt(details.flags);
            if !details.default_user.is_empty() {
                defaults.username.clone_from(&details.default_user);
            }
            if !details.default_domain.is_empty() {
                defaults.domain.clone_from(&details.default_domain);
            }
            defaults
        });
        let _prompt = show_authentication_dialog(
            &self.overlay,
            None,
            &details.message,
            (&defaults.username, &defaults.domain),
            details.flags,
            authentication_failed,
            MountDialogHandlers {
                submitted: Some(Rc::new(retry)),
                cancelled: Some(Rc::new(cancelled)),
            },
        );
    }

    fn mount_location(
        self: &Rc<Self>,
        location: Location,
        strategy: MountStrategy,
        credentials: Option<MountCredentials>,
        on_result: impl Fn(&Rc<Self>, MountOutcome) + 'static,
    ) {
        if self.overlay.root().and_downcast::<gtk::Window>().is_none() {
            return;
        }
        log_mount_started(&location, strategy);
        let activity = BrowserView {
            state: self.clone(),
        }
        .begin_global_activity("Connecting…");
        let file = gio_file_for_location(&location);
        let operation = gio::MountOperation::new();
        let attempt = Rc::new(RefCell::new(MountAttempt::new(credentials)));
        let active_prompt = Rc::new(RefCell::new(None::<gtk::Box>));

        let prompt_overlay = self.overlay.clone();
        let prompt_for_signal = active_prompt.clone();
        let attempt_for_password = attempt.clone();
        operation.connect_ask_password(
            move |operation, message, default_user, default_domain, flags| {
                let request = attempt_for_password
                    .borrow_mut()
                    .ask_password(MountPromptDetails {
                        message: message.to_owned(),
                        default_user: default_user.to_owned(),
                        default_domain: default_domain.to_owned(),
                        flags,
                    });
                let retry = match request {
                    PasswordRequest::Reply => {
                        if let Some(credentials) =
                            attempt_for_password.borrow().attempted.as_ref().cloned()
                        {
                            apply_mount_credentials(operation, &credentials);
                        }
                        operation.reply(gio::MountOperationResult::Handled);
                        return;
                    }
                    PasswordRequest::Ask { retry } => retry,
                };
                if let Some(previous) = prompt_for_signal.borrow_mut().take() {
                    dismiss_authentication_prompt(&prompt_overlay, &previous);
                }
                let prompt = show_authentication_dialog(
                    &prompt_overlay,
                    Some(operation),
                    message,
                    (default_user, default_domain),
                    flags,
                    retry,
                    MountDialogHandlers {
                        submitted: Some(Rc::new({
                            let attempt = attempt_for_password.clone();
                            move |credentials| attempt.borrow_mut().submitted(credentials)
                        })),
                        cancelled: None,
                    },
                );
                prompt_for_signal.replace(prompt);
            },
        );
        let question_overlay = self.overlay.clone();
        let question_prompt = active_prompt.clone();
        let attempt_for_question = attempt.clone();
        // `ask-question` carries a GStrv, which gir cannot wrap, so the backend's
        // trust decisions are taken from the raw signal rather than dropped.
        operation.connect_local("ask-question", false, move |values| {
            let operation = values
                .first()
                .and_then(|value| value.get::<gio::MountOperation>().ok())?;
            let message = values
                .get(1)
                .and_then(|value| value.get::<String>().ok())
                .unwrap_or_default();
            let choices = values
                .get(2)
                .and_then(|value| value.get::<Vec<String>>().ok())
                .unwrap_or_default();
            let details = MountQuestionDetails { message, choices };
            attempt_for_question
                .borrow_mut()
                .ask_question(details.clone());
            if let Some(previous) = question_prompt.borrow_mut().take() {
                dismiss_authentication_prompt(&question_overlay, &previous);
            }
            let prompt = show_mount_question_dialog(&question_overlay, &operation, &details, {
                let attempt = attempt_for_question.clone();
                move |decision| attempt.borrow_mut().decide(decision)
            });
            question_prompt.replace(prompt);
            None
        });

        let weak = Rc::downgrade(self);
        let result_overlay = self.overlay.clone();
        glib::MainContext::default().spawn_local(async move {
            let _activity = activity;
            let result = match strategy {
                MountStrategy::EnclosingVolume => {
                    file.mount_enclosing_volume_future(gio::MountMountFlags::NONE, Some(&operation))
                        .await
                }
                MountStrategy::Mountable => file
                    .mount_mountable_future(gio::MountMountFlags::NONE, Some(&operation))
                    .await
                    .map(|_| ()),
            };
            log_mount_finished(&location, &result);
            if let Some(prompt) = active_prompt.borrow_mut().take() {
                dismiss_authentication_prompt(&result_overlay, &prompt);
            }
            if let Some(state) = weak.upgrade() {
                let outcome = attempt.borrow().outcome(&location, result);
                on_result(&state, outcome);
            }
        });
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
                let copied_path = copy_path_text(location, true);
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

    fn handle(self: &Rc<Self>, event: BrowserEvent) {
        self.mode_views.borrow_mut().handle(&event);
        match event {
            BrowserEvent::Reset => {
                self.pending_location_credentials.take();
                self.truncate(0);
            }
            BrowserEvent::ColumnsTruncated { len } => {
                self.truncate(len);
                self.sync_active_location();
            }
            BrowserEvent::ColumnAdded { depth, location } => {
                self.set_location(&location);
                self.append_column(depth, &location);
            }
            BrowserEvent::EntriesInserted { depth, insertions } => {
                let render_started = Instant::now();
                let entry_count = insertions
                    .iter()
                    .map(|insertion| insertion.entries.len())
                    .sum();
                if let Some(column) = self.columns.borrow().get(depth).cloned() {
                    if entry_count > 0 && !column.spinner.is_spinning() {
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
                    update_empty_trash_sensitivity(&column, count);
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
                    update_empty_trash_sensitivity(&column, entries.len());
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
                    update_empty_trash_sensitivity(column, count);
                }
            }
            BrowserEvent::ColumnReloaded { depth } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    column.syncing_selection.set(true);
                    column.selection.set_model(None::<&gio::ListModel>);
                    column.filtered_model.set_model(None::<&gio::ListModel>);
                    column.model.splice(0, column.model.n_items(), &[]);
                    column.entry_count.set(0);
                    set_filter_placeholder(column, 0);
                    column.truncated_hint.set_visible(false);
                    column.spinner.set_visible(true);
                    column.spinner.start();
                    column.presentation.show_loading();
                }
            }
            BrowserEvent::HiddenToggled { show_hidden } => {
                for column in self.columns.borrow().iter() {
                    column.show_hidden.set(show_hidden);
                    column.filter.changed(gtk::FilterChange::Different);
                }
                self.mode_views.borrow().set_show_hidden(show_hidden);
            }
            BrowserEvent::LoadFinished { depth, truncated } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    if column.selection.model().is_none() {
                        column.filtered_model.set_model(Some(&column.model));
                        column.selection.set_model(Some(&column.filtered_model));
                        column.syncing_selection.set(false);
                    }
                    column.spinner.stop();
                    column.spinner.set_visible(false);
                    column.truncated_hint.set_visible(truncated);
                    let count = column.entry_count.get();
                    if count == 0 {
                        column.presentation.show_empty();
                    } else {
                        column.presentation.show_content();
                    }
                    update_empty_trash_sensitivity(column, count);
                }
                if self.browser.active_depth() == Some(depth) {
                    let names = self.pending_select.take();
                    if !names.is_empty() {
                        let weak = Rc::downgrade(self);
                        glib::idle_add_local_once(move || {
                            if let Some(state) = weak.upgrade() {
                                state.browser.select_entries_by_name(&names);
                            }
                        });
                    }
                }
            }
            BrowserEvent::LoadFailed { depth, message } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    if column.selection.model().is_none() {
                        column.filtered_model.set_model(Some(&column.model));
                        column.selection.set_model(Some(&column.filtered_model));
                        column.syncing_selection.set(false);
                    }
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
                take_focus,
            } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    let filtered_positions: Vec<_> = positions
                        .into_iter()
                        .filter_map(|position| filtered_position_for_source(column, position))
                        .collect();
                    set_column_selections(column, &filtered_positions);
                    // A background batch delivered for a column that already has a
                    // selection re-fires this event; don't let it steal focus from
                    // an in-progress New Folder/File prompt or rename (visible for
                    // slow network directories that stream many batches).
                    if self.active_rename.borrow().is_none()
                        && self.active_new_entry.borrow().is_none()
                    {
                        if let Some(focused) = filtered_position_for_source(column, focused) {
                            column
                                .list
                                .scroll_to(focused, gtk::ListScrollFlags::FOCUS, None);
                        }
                        if take_focus && self.mode_views.borrow().mode() == BrowserMode::Columns {
                            column.list.grab_focus();
                        }
                    }
                }
            }
            BrowserEvent::FocusChanged { depth, position } => {
                if let Some(column) = self.columns.borrow().get(depth) {
                    let editing = self.active_rename.borrow().is_some()
                        || self.active_new_entry.borrow().is_some();
                    if let Some(filtered_position) =
                        position.and_then(|position| filtered_position_for_source(column, position))
                    {
                        set_column_selection(column, filtered_position);
                        if !editing {
                            column.list.scroll_to(
                                filtered_position,
                                gtk::ListScrollFlags::FOCUS,
                                None,
                            );
                        }
                    }
                    if !editing && self.mode_views.borrow().mode() == BrowserMode::Columns {
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
            BrowserEvent::TransferStarted { total, moving } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    total,
                    if moving {
                        crate::assets::icons::FOLDER
                    } else {
                        crate::assets::icons::COPY
                    },
                    if moving {
                        "Moving items"
                    } else {
                        "Copying items"
                    },
                    "Cancelling will not undo completed changes",
                    Rc::new(move || browser.cancel_file_operation()),
                );
            }
            BrowserEvent::TransferProgress { completed, total } => {
                self.update_delete_progress(completed, total);
            }
            BrowserEvent::TransferFinished { moved_locations } => {
                if !moved_locations.is_empty() {
                    self.complete_cut_transfer(&moved_locations);
                }
                self.dismiss_delete_progress();
            }
            BrowserEvent::DeletionStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    total,
                    crate::assets::icons::TRASH,
                    "Deleting items",
                    "Cancelling will not undo completed changes",
                    Rc::new(move || browser.cancel_file_operation()),
                );
            }
            BrowserEvent::DeletionProgress { completed, total } => {
                self.update_delete_progress(completed, total);
            }
            BrowserEvent::DeletionFinished => self.dismiss_delete_progress(),
            BrowserEvent::RestorationStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    total,
                    crate::assets::icons::FOLDER,
                    "Restoring items",
                    "Cancelling will not undo completed changes",
                    Rc::new(move || browser.cancel_file_operation()),
                );
            }
            BrowserEvent::RestorationProgress { completed, total } => {
                self.update_delete_progress(completed, total);
            }
            BrowserEvent::RestorationFinished => self.dismiss_delete_progress(),
            BrowserEvent::OperationFailed { message } => {
                self.dismiss_delete_progress();
                let retry = self.pending_extract_retry.take();
                if let Some((entry, dest)) = retry {
                    let lower = message.to_lowercase();
                    if lower.contains("password") || lower.contains("encrypt") {
                        self.show_extract_password_dialog(entry, dest);
                        return;
                    }
                }
                show_error_dialog(&self.overlay, "Unable to complete operation", &message);
            }
            BrowserEvent::OperationCompletedWithErrors { message } => {
                show_error_dialog(&self.overlay, "Completed with errors", &message);
            }
            BrowserEvent::OperationCancelled {
                completed,
                failed,
                not_attempted,
                affected_locations,
            } => {
                let message = format!(
                    "{} completed, {} failed, and {} not attempted.\n\nCompleted changes were not reverted.",
                    item_count_label(completed),
                    item_count_label(failed),
                    item_count_label(not_attempted),
                );
                let browser = self.browser.clone();
                show_error_dialog_after_close(
                    &self.overlay,
                    "Operation cancelled",
                    &message,
                    Rc::new(move || browser.refresh_after_cancellation(&affected_locations)),
                );
            }
            BrowserEvent::NavigationRejected {
                parent_depth,
                error,
            } => {
                self.handle_navigation_rejected(parent_depth, error);
            }
            BrowserEvent::EmptyTrashRequested => {
                self.load_trash_summary();
            }
            BrowserEvent::LocationNavigationRejected { error } => {
                let credentials = self.pending_location_credentials.take();
                match error {
                    LocationValidationError::NotMounted(location) => {
                        self.mount_then_navigate_with_credentials(
                            location,
                            MountStrategy::EnclosingVolume,
                            credentials,
                        );
                    }
                    LocationValidationError::Mountable(location) => {
                        self.mount_then_navigate_with_credentials(
                            location,
                            MountStrategy::Mountable,
                            credentials,
                        );
                    }
                    error => show_error_dialog(
                        &self.overlay,
                        "Unable to open location",
                        &error.to_string(),
                    ),
                }
            }
            BrowserEvent::ArchiveStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    total,
                    crate::assets::icons::FILE_ARCHIVE,
                    "Working",
                    "This may take a moment",
                    Rc::new(move || browser.cancel_file_operation()),
                );
            }
            BrowserEvent::ArchiveProgress { completed, total } => {
                self.update_archive_progress(completed, total);
            }
            BrowserEvent::ArchiveCompleted { select_name, .. } => {
                self.dismiss_delete_progress();
                self.pending_extract_retry.replace(None);
                if !select_name.is_empty() {
                    self.pending_select.borrow_mut().push(select_name);
                }
                if let Some(dest) = self.pending_navigate.take() {
                    self.browser.navigate(dest);
                } else {
                    self.browser.reload_active();
                }
            }
            BrowserEvent::TransferCompleted => {
                if let Some(dest) = self.pending_navigate.take() {
                    self.browser.navigate(dest);
                }
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
        let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        heading_box.set_hexpand(true);
        let heading = gtk::Label::new(Some(&location.display_name()));
        heading.set_xalign(0.0);
        heading.set_tooltip_text(Some(&location.display_path()));
        let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
        truncated_hint.set_tooltip_text(Some(
            "This directory has more entries than could be loaded; showing a partial listing.",
        ));
        truncated_hint.set_visible(false);
        heading_box.append(&heading);
        heading_box.append(&truncated_hint);
        let spinner = gtk::Spinner::new();
        spinner.start();
        header.append(&heading_box);
        header.append(&spinner);
        let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header_actions.add_css_class("column-header-actions");
        let empty_trash = empty_trash_button(&self.browser);
        let is_trash = is_trash_root(location);
        empty_trash.set_visible(is_trash);
        empty_trash.set_sensitive(false);
        header_actions.append(&empty_trash);
        header_actions.append(&pane_refresh_button(&self.browser, depth));
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
        filter_button.set_child(Some(&crate::assets::primary_icon(
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
            close.set_child(Some(&crate::assets::primary_icon(
                crate::assets::icons::X,
                16,
            )));
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
        let initial_show_hidden = self
            .browser
            .column_preferences(depth)
            .map_or_else(|| self.browser.preferences().show_hidden, |p| p.show_hidden);
        let show_hidden = Rc::new(Cell::new(initial_show_hidden));
        let filter = entry_filter(show_hidden.clone(), filter_query.clone());
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
        let filter_for_column = filter.clone();
        selection.connect_selection_changed(move |selection, position, count| {
            if syncing_selection_changed.get() {
                return;
            }
            let filtered_positions = bitset_positions(&selection.selection());
            let mapped_positions = source_positions_for_filtered(
                &source_for_selection,
                &filtered_for_selection,
                &filtered_positions,
            );
            let source_positions: Vec<_> = mapped_positions
                .iter()
                .map(|(_, source_position)| *source_position)
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
                mapped_positions
                    .iter()
                    .find_map(|(filtered, source)| (*filtered == position).then_some(*source))
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
                    let entry = state.browser.entry_at(depth, source_position);
                    if entry.as_ref().is_some_and(|entry| {
                        should_activate_single_click(
                            press_count,
                            entry.is_directory(),
                            state.columns_click_activation.get(),
                            control,
                            shift,
                            preserve_group,
                        )
                    }) {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        state.browser.activate(depth, source_position);
                    } else if should_preview_pointer_press(
                        press_count,
                        control,
                        shift,
                        preserve_group,
                    ) && entry.as_ref().is_some_and(|entry| {
                        entry_responds_to_preview_click(entry, state.single_click_previews.get())
                    }) {
                        state.browser.preview(depth, source_position);
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
            size.set_visible(!size_text.is_empty());
        });
        factory.connect_unbind(|_, item| super::thumbnail::cancel_list_item_thumbnails(item));

        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("file-list");
        list.set_enable_rubberband(false);
        list.set_single_click_activate(false);
        list.set_vexpand(true);

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
        scroll.add_css_class("fixed-scrollbar");
        let rows_for_marquee = bound_rows.clone();
        let marquee = super::marquee::install(super::marquee::MarqueeSetup {
            view: list.clone().upcast(),
            scroll: scroll.clone(),
            overlay: self.overlay.clone(),
            selection: selection.clone(),
            visit_items: Rc::new(move |visit| {
                rows_for_marquee.borrow_mut().retain(|bound| {
                    let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade())
                    else {
                        return false;
                    };
                    visit(item.position(), row.upcast_ref());
                    true
                });
            }),
            is_item: Rc::new(|widget| is_file_row_target(widget.clone())),
        });
        marquee.add_origin_surface(&header);

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
        let last_press = Rc::new(Cell::new(0u64));
        let shell_for_resize_start = shell.clone();
        let shell_for_autofit = shell.clone();
        let column_for_autofit = column.clone();
        let resize_start_for_begin = resize_start.clone();
        let pointer_start_for_begin = pointer_start.clone();
        let last_press_for_begin = last_press.clone();
        resize.connect_drag_begin(move |gesture, _, _| {
            let now = glib::monotonic_time() as u64;
            let prev = last_press_for_begin.get();
            last_press_for_begin.set(now);
            if now.wrapping_sub(prev) <= 400_000 {
                let max_natural =
                    max_child_natural_width(column_for_autofit.upcast_ref::<gtk::Widget>());
                shell_for_autofit.set_size_request(max_natural.max(COLUMN_WIDTH), -1);
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
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
            header_actions,
            filter_entry,
            filter_button,
            selection,
            syncing_selection,
            list,
            marquee,
            bound_rows,
            entry_count,
            spinner,
            truncated_hint,
            empty_trash_button: is_trash.then_some(empty_trash),
            new_entry_row,
            new_entry_icon,
            new_entry_entry,
            show_hidden,
            filter: filter_for_column,
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
        if self.browser.is_open_child(origin_depth, &location) {
            self.peek_anchor.take();
            self.browser.close_peek();
            return;
        }
        if self
            .peek
            .borrow()
            .as_ref()
            .is_some_and(|peek| peek.location == location)
        {
            return;
        }
        self.peek_anchor.replace(Some(PeekAnchor {
            widget: anchor,
            origin_depth,
        }));

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
        let Some(row_bounds) = anchor.widget.compute_bounds(&self.overlay) else {
            self.browser.close_peek();
            return;
        };
        let source_bounds = match peek_origin_bounds(self.mode_views.borrow().mode()) {
            PeekOriginBounds::Anchor => row_bounds,
            PeekOriginBounds::Column => {
                let Some(bounds) = self
                    .columns
                    .borrow()
                    .get(anchor.origin_depth)
                    .and_then(|column| column.shell.compute_bounds(&self.overlay))
                else {
                    self.browser.close_peek();
                    return;
                };
                bounds
            }
        };
        let Some(placement) = peek_horizontal_placement(
            source_bounds.x(),
            source_bounds.width(),
            self.overlay.width() as f32,
        ) else {
            self.browser.close_peek();
            return;
        };

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_size_request(PEEK_WIDTH, -1);
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

        content.add_css_class("peek-popover");
        let transition_duration = self
            .peek_behavior
            .fade_duration
            .as_millis()
            .min(u128::from(u32::MAX)) as u32;
        let transition_type = peek_transition(placement.side);
        let (halign, margin_start, margin_end) =
            peek_horizontal_layout(placement, self.overlay.width() as f32);
        let revealer = gtk::Revealer::builder()
            .child(&content)
            .transition_type(transition_type)
            .transition_duration(transition_duration)
            .reveal_child(false)
            .halign(halign)
            .valign(gtk::Align::Start)
            .margin_start(margin_start)
            .margin_end(margin_end)
            .margin_top(row_bounds.y().round().max(0.0) as i32)
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
            self.overlay.remove_overlay(&column.marquee.band());
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
        row.add_css_class("file-row");
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
    factory.connect_unbind(|_, item| super::thumbnail::cancel_list_item_thumbnails(item));
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
    source_positions_for_filtered(source, filtered, &[filtered_position])
        .first()
        .map(|(_, source)| *source)
}

fn source_positions_for_filtered(
    source: &gtk::StringList,
    filtered: &gtk::FilterListModel,
    filtered_positions: &[u32],
) -> Vec<(u32, usize)> {
    let mut source_position = 0;
    filtered_positions
        .iter()
        .filter_map(|filtered_position| {
            let item = filtered.item(*filtered_position)?;
            while source_position < source.n_items() {
                let candidate_position = source_position;
                source_position += 1;
                if source
                    .item(candidate_position)
                    .is_some_and(|candidate| candidate == item)
                {
                    return Some((*filtered_position, candidate_position as usize));
                }
            }
            None
        })
        .collect()
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

    let new_folder = context_menu_option(
        crate::assets::icons::FOLDER_PLUS,
        "New Folder",
        "Ctrl+Shift+N",
    );
    let new_file = context_menu_option(crate::assets::icons::FILE_PLUS, "New File", "");
    let open_terminal =
        context_menu_option(crate::assets::icons::TERMINAL, "Open in Terminal", "Ctrl+T");
    let paste = context_menu_option(crate::assets::icons::CLIPBOARD_PASTE, "Paste", "Ctrl+V");
    let select_all = context_menu_option(crate::assets::icons::LIST_CHECKS, "Select All", "Ctrl+A");
    let refresh = context_menu_option(crate::assets::icons::REFRESH, "Refresh", "F5");
    let properties = context_menu_option(crate::assets::icons::INFO, "Properties", "");
    content.append(&new_folder);
    content.append(&new_file);
    content.append(&open_terminal);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&paste);
    content.append(&select_all);
    content.append(&refresh);
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
    let refresh_popover = popover.downgrade();
    refresh.connect_clicked(move |_| {
        if let Some(popover) = refresh_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.browser.retry_column(depth);
        }
    });
    let weak = Rc::downgrade(state);
    let properties_popover = popover.downgrade();
    let properties_location = location.clone();
    properties.connect_clicked(move |_| {
        if let Some(popover) = properties_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.show_folder_properties(&properties_location);
        }
    });
    let weak = Rc::downgrade(state);
    let terminal_popover = popover.downgrade();
    let terminal_location = location.clone();
    open_terminal.connect_clicked(move |_| {
        if let Some(popover) = terminal_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            launch_terminal(&terminal_location, &state.overlay);
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
        open_terminal.set_sensitive(can_open_terminal(&location));
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

const ITEM_CONTEXT_SUMMARY_MAX_CHARS: i32 = 60;

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
    heading.set_max_width_chars(ITEM_CONTEXT_SUMMARY_MAX_CHARS);
    heading.set_xalign(0.0);
    let summary = gtk::Label::new(None);
    summary.add_css_class("item-context-summary");
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_max_width_chars(ITEM_CONTEXT_SUMMARY_MAX_CHARS);
    summary.set_xalign(0.0);
    header.append(&heading);
    header.append(&summary);
    content.append(&header);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let single = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let open = item_context_option(crate::assets::icons::EXTERNAL_LINK, "Open", "↵");
    let open_terminal =
        item_context_option(crate::assets::icons::TERMINAL, "Open in Terminal", "Ctrl+T");
    let preview = item_context_option(crate::assets::icons::EYE, "Quick preview", "Space");
    let restore = item_context_option(crate::assets::icons::FOLDER, "Restore", "");
    restore.set_visible(in_trash);
    let pin = item_context_option(crate::assets::icons::PIN, "Pin to sidebar", "P");
    let copy = item_context_option(crate::assets::icons::COPY, "Copy", "Ctrl+C");
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
    let move_to_trash = if in_trash {
        let option = item_context_danger_option(crate::assets::icons::TRASH, delete_label, "Del");
        option.add_css_class("danger");
        option
    } else {
        item_context_option(crate::assets::icons::TRASH, delete_label, "Del")
    };
    let permanent_delete = item_context_danger_option(
        crate::assets::icons::TRASH,
        "Permanently delete",
        "Shift+Del",
    );
    permanent_delete.add_css_class("danger");
    let properties = item_context_option(crate::assets::icons::INFO, "Properties", "Alt+Enter");
    let compress = item_context_option(crate::assets::icons::FILE_ARCHIVE, "Compress…", "");
    let extract = item_context_option(crate::assets::icons::FILE_ARCHIVE, "Extract here", "");
    let extract_to = item_context_option(crate::assets::icons::FILE_ARCHIVE, "Extract to…", "");
    single.append(&open);
    single.append(&open_terminal);
    single.append(&preview);
    single.append(&restore);
    single.append(&extract);
    single.append(&extract_to);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&pin);
    single.append(&cut);
    single.append(&copy);
    single.append(&copy_path);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&move_to);
    single.append(&copy_to);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&rename);
    single.append(&compress);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&properties);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&move_to_trash);
    single.append(&permanent_delete);
    content.append(&single);

    let multiple = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let restore_multiple = item_context_option(crate::assets::icons::FOLDER, "Restore items", "");
    restore_multiple.set_visible(in_trash);
    let copy_multiple = item_context_option(crate::assets::icons::COPY, "Copy", "Ctrl+C");
    let copy_paths = item_context_option(crate::assets::icons::COPY, "Copy paths", "Y");
    let move_multiple = item_context_option(crate::assets::icons::FOLDER, "Move to…", "");
    let copy_to_multiple = item_context_option(crate::assets::icons::COPY, "Copy to…", "");
    let cut_multiple = item_context_option(crate::assets::icons::SCISSORS, "Cut", "Ctrl+X");
    let trash_multiple = if in_trash {
        let option = item_context_danger_option(crate::assets::icons::TRASH, delete_label, "Del");
        option.add_css_class("danger");
        option
    } else {
        item_context_option(crate::assets::icons::TRASH, delete_label, "Del")
    };
    let permanent_delete_multiple = item_context_danger_option(
        crate::assets::icons::TRASH,
        "Permanently delete",
        "Shift+Del",
    );
    permanent_delete_multiple.add_css_class("danger");
    let compress_multiple =
        item_context_option(crate::assets::icons::FILE_ARCHIVE, "Compress…", "");
    multiple.append(&restore_multiple);
    multiple.append(&cut_multiple);
    multiple.append(&copy_multiple);
    multiple.append(&copy_paths);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&move_multiple);
    multiple.append(&copy_to_multiple);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&compress_multiple);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&trash_multiple);
    multiple.append(&permanent_delete_multiple);
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
    let terminal_target = target.clone();
    let terminal_popover = popover.downgrade();
    open_terminal.connect_clicked(move |_| {
        if let Some(popover) = terminal_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = terminal_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade() {
            launch_terminal(&entry.location, &state.overlay);
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
    connect_context_transfer(&copy_to_multiple, &popover, state, &target, false);
    connect_context_cut(&cut, &popover, state, &target);
    connect_context_cut(&cut_multiple, &popover, state, &target);
    connect_context_copy(&copy, &popover, state, &target);
    connect_context_copy(&copy_multiple, &popover, state, &target);
    connect_context_trash(&move_to_trash, &popover, state, &target, in_trash);
    connect_context_trash(&trash_multiple, &popover, state, &target, in_trash);
    connect_context_trash(&permanent_delete, &popover, state, &target, true);
    connect_context_trash(&permanent_delete_multiple, &popover, state, &target, true);
    connect_context_compress(&compress, &popover, state, &target);
    connect_context_compress(&compress_multiple, &popover, state, &target);
    connect_context_extract(&extract, &popover, state, &target, false);
    connect_context_extract(&extract_to, &popover, state, &target, true);
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
        target.replace(Some((resolved_position, entry.clone())));
        let entries = state.browser.selected_entries();
        preview.set_visible(entry_supports_quick_preview(&entry));
        open_terminal.set_visible(entry.is_directory() && can_open_terminal(&entry.location));
        permanent_delete.set_visible(!in_trash);
        permanent_delete_multiple.set_visible(!in_trash);
        pin.set_visible(entry.is_directory() && !is_trash_location(&entry.location));
        pin.set_sensitive(
            state
                .pin_status_handler
                .borrow()
                .as_ref()
                .is_some_and(|handler| handler(&entry.location) == PinStatus::Available),
        );
        extract.set_visible(ArchiveFormat::from_extension(&entry.display_name).is_some());
        extract_to.set_visible(ArchiveFormat::from_extension(&entry.display_name).is_some());
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

fn entry_responds_to_preview_click(entry: &FileEntry, previews_enabled: bool) -> bool {
    previews_enabled && !entry.is_directory() && entry_supports_quick_preview(entry)
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
        || is_extensionless_dotfile(&entry.native_name)
}

struct TrashSummary {
    item_count: usize,
    total_size: u64,
    /// `true` if measurement did not cover the full trash tree; `item_count`/`total_size` are
    /// then a lower bound.
    truncated: bool,
}

const TRASH_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-symlink,standard::size,time::modified";

const MAX_TRASH_ENTRIES: usize = 200_000;
const MAX_TRASH_DEPTH: usize = 64;
const TRASH_TIME_BUDGET: Duration = Duration::from_secs(5);

async fn summarize_trash(root: &gio::File) -> Result<TrashSummary, glib::Error> {
    summarize_trash_with_budget(root, MAX_TRASH_ENTRIES, MAX_TRASH_DEPTH, TRASH_TIME_BUDGET).await
}

async fn summarize_trash_with_budget(
    root: &gio::File,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> Result<TrashSummary, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut item_count = 0_usize;
    let mut total_size = 0_u64;
    let mut truncated = false;
    let visited = Rc::new(Cell::new(0_usize));
    let deadline = Instant::now() + time_budget;
    'root: loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        glib::timeout_future(Duration::from_millis(1)).await;
        for info in children {
            if visited.get() >= max_entries || Instant::now() >= deadline {
                truncated = true;
                break 'root;
            }
            let (count, size, entry_truncated) = measure_trash_entry(
                root.child(info.name()),
                info,
                0,
                visited.clone(),
                deadline,
                max_entries,
                max_depth,
            )
            .await?;
            item_count = item_count.saturating_add(count);
            total_size = total_size.saturating_add(size);
            truncated |= entry_truncated;
        }
        // Stop only when the shared budget is actually spent -- a child's own `truncated` (depth
        // cap, discarded error) is branch-local and must not cut off its unrelated siblings.
        if visited.get() >= max_entries || Instant::now() >= deadline {
            truncated = true;
            break;
        }
    }
    Ok(TrashSummary {
        item_count,
        total_size,
        truncated,
    })
}

struct EmptyTrashOutcome {
    deleted: usize,
    failed: usize,
    /// Capped at 8 messages regardless of `failed`, so a trash full of failures can't grow this
    /// without bound.
    errors: Vec<String>,
}

/// Empties `root` by enumerating and deleting one batch at a time -- unlike a listing that
/// collects every top-level entry into a `Vec<FileEntry>` first, no per-entry list is ever
/// retained here; only a running count and a capped error list, so memory stays flat no matter
/// how large the trash is.
async fn empty_trash(
    root: &gio::File,
    mut on_progress: impl FnMut(usize),
) -> Result<EmptyTrashOutcome, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut outcome = EmptyTrashOutcome {
        deleted: 0,
        failed: 0,
        errors: Vec::new(),
    };
    loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        for info in children {
            let file = root.child(info.name());
            match file.delete_future(glib::Priority::DEFAULT).await {
                Ok(_) => outcome.deleted += 1,
                Err(error) => {
                    outcome.failed += 1;
                    if outcome.errors.len() < 8 {
                        outcome
                            .errors
                            .push(format!("{}: {error}", info.display_name()));
                    }
                }
            }
        }
        on_progress(outcome.deleted + outcome.failed);
    }
    Ok(outcome)
}

fn empty_trash_error_summary(outcome: &EmptyTrashOutcome) -> String {
    let mut summary = format!(
        "{} could not be deleted. The remaining items were processed.",
        item_count_label(outcome.failed)
    );
    for error in &outcome.errors {
        summary.push_str("\n\n• ");
        summary.push_str(error);
    }
    if outcome.failed > outcome.errors.len() {
        summary.push_str(&format!(
            "\n\n…and {} more",
            outcome.failed - outcome.errors.len()
        ));
    }
    summary
}

type TrashMeasurementFuture =
    Pin<Box<dyn Future<Output = Result<(usize, u64, bool), glib::Error>>>>;

/// `visited` is shared across the whole walk, so the entry budget applies tree-wide rather than
/// per-branch, and `deadline` is a fixed point so descending deeper can't reset the time budget.
fn measure_trash_entry(
    file: gio::File,
    info: gio::FileInfo,
    depth: usize,
    visited: Rc<Cell<usize>>,
    deadline: Instant,
    max_entries: usize,
    max_depth: usize,
) -> TrashMeasurementFuture {
    Box::pin(async move {
        visited.set(visited.get() + 1);
        let mut count = 1_usize;
        let mut size = if info.file_type() == gio::FileType::Regular {
            info.size().max(0) as u64
        } else {
            0
        };
        let mut truncated = false;
        if info.file_type() == gio::FileType::Directory && !info.is_symlink() {
            let budget_exhausted =
                depth >= max_depth || visited.get() >= max_entries || Instant::now() >= deadline;
            if budget_exhausted {
                truncated = true;
            } else {
                // A directory can become unreadable or disappear before we measure it; degrade
                // this branch to truncated rather than failing the whole walk.
                match enumerate_trash_directory(
                    &file,
                    depth,
                    visited.clone(),
                    deadline,
                    max_entries,
                    max_depth,
                )
                .await
                {
                    Ok((child_count, child_size, child_truncated)) => {
                        count = count.saturating_add(child_count);
                        size = size.saturating_add(child_size);
                        truncated |= child_truncated;
                    }
                    Err(_) => truncated = true,
                }
            }
        }
        Ok((count, size, truncated))
    })
}

async fn enumerate_trash_directory(
    file: &gio::File,
    depth: usize,
    visited: Rc<Cell<usize>>,
    deadline: Instant,
    max_entries: usize,
    max_depth: usize,
) -> Result<(usize, u64, bool), glib::Error> {
    let enumerator = file
        .enumerate_children_future(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut count = 0_usize;
    let mut size = 0_u64;
    let mut truncated = false;
    'this_directory: loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        glib::timeout_future(Duration::from_millis(1)).await;
        for child in children {
            if visited.get() >= max_entries || Instant::now() >= deadline {
                truncated = true;
                break 'this_directory;
            }
            let (child_count, child_size, child_truncated) = measure_trash_entry(
                file.child(child.name()),
                child,
                depth + 1,
                visited.clone(),
                deadline,
                max_entries,
                max_depth,
            )
            .await?;
            count = count.saturating_add(child_count);
            size = size.saturating_add(child_size);
            truncated |= child_truncated;
        }
        // Stop only when the shared budget is actually spent -- a child's own `truncated` (depth
        // cap, discarded error) is branch-local and must not cut off its unrelated siblings.
        if visited.get() >= max_entries || Instant::now() >= deadline {
            truncated = true;
            break;
        }
    }
    Ok((count, size, truncated))
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
    let max_chars = ITEM_CONTEXT_SUMMARY_MAX_CHARS as usize;
    if names.chars().count() > max_chars {
        names = names.chars().take(max_chars - 1).collect();
        names.push('…');
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
            state.request_delete(entries, permanent);
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

fn connect_context_copy(
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
            state.copy_entries(&context_entries(&state, &target));
        }
    });
}

fn connect_context_compress(
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
            let entries = context_entries(&state, &target);
            state.show_compress_dialog(entries);
        }
    });
}

fn connect_context_extract(
    button: &gtk::Button,
    popover: &gtk::Popover,
    state: &Rc<ViewState>,
    target: &Rc<RefCell<Option<(usize, FileEntry)>>>,
    pick_destination: bool,
) {
    let weak = Rc::downgrade(state);
    let target = target.clone();
    let popover = popover.downgrade();
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            let Some((_, entry)) = target.borrow().clone() else {
                return;
            };
            if pick_destination {
                state.show_extract_to_dialog(entry);
            } else {
                state.extract_entry(entry);
            }
        }
    });
}

fn render_transfer_suggestions(
    suggestions: &gtk::Box,
    items: Vec<crate::services::SearchItem>,
    field: &gtk::Entry,
) {
    while let Some(child) = suggestions.first_child() {
        suggestions.remove(&child);
    }
    let mut dirs: Vec<_> = items.into_iter().filter(|item| item.is_directory).collect();
    dirs.sort_by_key(|item| item.path.ancestors().count());
    dirs.truncate(8);
    if dirs.is_empty() {
        let empty = gtk::Label::new(Some("No matching folders"));
        empty.add_css_class("transfer-suggestions-empty");
        empty.set_xalign(0.0);
        suggestions.append(&empty);
        return;
    }
    for item in dirs {
        append_suggestion(suggestions, &item.path, field);
    }
}

fn append_suggestion(suggestions: &gtk::Box, path: &Path, field: &gtk::Entry) {
    let option = gtk::Button::new();
    option.add_css_class("transfer-suggestion");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    row.append(&crate::assets::primary_icon(
        crate::assets::icons::FOLDER,
        16,
    ));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let label = gtk::Label::new(Some(&name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    if let Some(parent) = path.parent().map(compact_native_path) {
        let parent_label = gtk::Label::new(Some(&parent));
        parent_label.add_css_class("transfer-suggestion-parent");
        parent_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        parent_label.set_xalign(1.0);
        row.append(&parent_label);
    }
    option.set_child(Some(&row));
    option.set_tooltip_text(Some(&path.to_string_lossy()));
    let selected_field = field.clone();
    let path = path.to_path_buf();
    option.connect_clicked(move |_| {
        selected_field.remove_css_class("error");
        selected_field.set_text(&folder_input_path(&path));
        selected_field.set_position(-1);
        selected_field.grab_focus();
    });
    suggestions.append(&option);
}

fn setup_transfer_search(
    field: &gtk::Entry,
    suggestions: &gtk::Box,
    generation: &Rc<Cell<u64>>,
    base: std::path::PathBuf,
    on_changed: impl Fn(&gtk::Entry) + 'static,
) {
    let (search_handle, search_receiver) = index_tree(glib::home_dir());
    let search_handle = Rc::new(search_handle);
    let query_handle = Rc::downgrade(&search_handle);
    let search_mode = Rc::new(Cell::new(false));
    let poll_suggestions = suggestions.downgrade();
    let poll_field = field.downgrade();
    let poll_mode = search_mode.clone();
    let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
        let _keep_search_alive = &search_handle;
        let (Some(suggestions), Some(field)) = (poll_suggestions.upgrade(), poll_field.upgrade())
        else {
            return glib::ControlFlow::Break;
        };
        if field.root().is_none() {
            return glib::ControlFlow::Break;
        }
        if !poll_mode.get() {
            return glib::ControlFlow::Continue;
        }
        let mut latest = None;
        for _ in 0..8 {
            match search_receiver.try_recv() {
                Ok(event) => latest = Some(event),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return glib::ControlFlow::Break;
                }
            }
        }
        if let Some(SearchEvent::Results { query, items, .. }) = latest
            && query == field.text().trim()
        {
            render_transfer_suggestions(&suggestions, items, &field);
        }
        glib::ControlFlow::Continue
    });
    let suggestions_clone = suggestions.clone();
    let generation_clone = generation.clone();
    field.connect_changed(move |field| {
        on_changed(field);
        let input = field.text().to_string();
        let request = generation_clone.get().saturating_add(1);
        generation_clone.set(request);
        let looks_like_path = input.trim().contains(std::path::MAIN_SEPARATOR)
            || input.trim().starts_with('~')
            || input.trim().is_empty();
        if looks_like_path {
            search_mode.set(false);
            let gen_check = generation_clone.clone();
            let home = glib::home_dir();
            let base = base.clone();
            let field_clone = field.clone();
            let suggestions_clone = suggestions_clone.clone();
            glib::MainContext::default().spawn_local(async move {
                let matches =
                    gio::spawn_blocking(move || path_suggestions(&input, &base, &home)).await;
                if gen_check.get() != request {
                    return;
                }
                while let Some(child) = suggestions_clone.first_child() {
                    suggestions_clone.remove(&child);
                }
                let Ok(paths) = matches else { return };
                if paths.is_empty() {
                    let empty = gtk::Label::new(Some("No matching folders"));
                    empty.add_css_class("transfer-suggestions-empty");
                    empty.set_xalign(0.0);
                    suggestions_clone.append(&empty);
                }
                for path in paths {
                    append_suggestion(&suggestions_clone, &path, &field_clone);
                }
            });
        } else {
            search_mode.set(true);
            if let Some(search_handle) = query_handle.upgrade() {
                search_handle.query(&input);
            }
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

fn path_suggestions(input: &str, base: &Path, home: &Path) -> Vec<std::path::PathBuf> {
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

fn normalized_archive_name(name: &str, format: ArchiveFormat) -> String {
    name.strip_suffix(&format!(".{}", format.extension()))
        .unwrap_or(name)
        .to_owned()
}

fn archive_has_collision(destination: &Location, archive_name: &str) -> bool {
    gio_file_for_location(destination)
        .child(archive_name)
        .query_exists(None::<&gio::Cancellable>)
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
        .filter_map(location_for_file)
        .collect::<Vec<_>>();
    (!locations.is_empty()).then_some(locations)
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
        .map(|entry| copy_path_text(&entry.location, entry.is_directory()))
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(&text);
    }
}

fn can_pin_entry(entry: &FileEntry, status: PinStatus) -> bool {
    entry.is_directory() && !is_trash_location(&entry.location) && status == PinStatus::Available
}

fn copy_path_text(location: &Location, is_directory: bool) -> String {
    match location.native_path() {
        Some(path) => {
            let mut path = shell_escape_path(path);
            if is_directory && !path.ends_with(std::path::MAIN_SEPARATOR) {
                path.push(std::path::MAIN_SEPARATOR);
            }
            path
        }
        None => location.display_path(),
    }
}

fn shell_escape_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if path.contains('\n') {
        return format!("'{}'", path.replace('\'', "'\\''"));
    }

    let mut escaped = String::new();
    for c in path.chars() {
        if needs_shell_escape(c) {
            escaped.push('\\');
            escaped.push(c);
        } else {
            escaped.push(c);
        }
    }
    escaped
}

fn needs_shell_escape(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '"' | '\''
                | '\\'
                | '$'
                | '`'
                | '!'
                | '#'
                | '&'
                | '*'
                | ';'
                | '<'
                | '>'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '('
                | ')'
                | '|'
                | '~'
        )
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

fn should_activate_single_click(
    press_count: i32,
    is_directory: bool,
    activation: ClickActivation,
    control: bool,
    shift: bool,
    preserve_group: bool,
) -> bool {
    let configured = if is_directory {
        activation.folders
    } else {
        activation.files
    };
    press_count == 1 && configured == ClickCount::One && !control && !shift && !preserve_group
}

fn should_preview_pointer_press(
    press_count: i32,
    control: bool,
    shift: bool,
    preserve_group: bool,
) -> bool {
    press_count == 1 && !control && !shift && !preserve_group
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
    if left.is_empty() || left.len() != right.len() {
        return false;
    }
    let left: HashSet<_> = left.iter().collect();
    let right: HashSet<_> = right.iter().collect();
    left.len() == right.len() && left == right
}

fn retain_untransferred(cut: &mut Vec<Location>, transferred: &[Location]) {
    let transferred: HashSet<_> = transferred.iter().collect();
    cut.retain(|location| !transferred.contains(location));
}

fn item_context_option(icon: &str, label: &str, accelerator: &str) -> gtk::Button {
    item_context_option_with_icon(crate::assets::primary_icon(icon, 15), label, accelerator)
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

fn context_menu_option(icon: &str, label: &str, accelerator: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("folder-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = crate::assets::primary_icon(icon, 15);
    icon.add_css_class("folder-context-icon");
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&icon);
    row.append(&title);
    if !accelerator.is_empty() {
        let shortcut = gtk::Label::new(Some(accelerator));
        shortcut.add_css_class("folder-context-shortcut");
        row.append(&shortcut);
    }
    button.set_child(Some(&row));
    button
}

pub(super) fn pane_refresh_button(browser: &Rc<Browser>, depth: usize) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text("Refresh (F5)").build();
    button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::REFRESH,
        16,
    )));
    button.add_css_class("column-header-action");
    let weak_browser = Rc::downgrade(browser);
    button.connect_clicked(move |_| {
        if let Some(browser) = weak_browser.upgrade() {
            browser.retry_column(depth);
        }
    });
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
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .halign(gtk::Align::End)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.add_css_class("column-popover");
    let popover_weak = popover.downgrade();
    for (label, key) in [
        ("Name", SortKey::Name),
        ("Size", SortKey::Size),
        ("Modified", SortKey::Modified),
        ("Type", SortKey::Type),
    ] {
        let (option, check) = menu_option(label, preferences.sort_key == key);
        selected_checks.borrow_mut().push((key, check));
        let checks = selected_checks.clone();
        let weak_browser = Rc::downgrade(browser);
        let popover_weak = popover_weak.clone();
        option.connect_clicked(move |_| {
            for (check_key, check) in checks.borrow().iter() {
                check.set_visible(*check_key == key);
            }
            if let Some(browser) = weak_browser.upgrade() {
                browser.set_sort_key(depth, key);
            }
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
        content.append(&option);
    }

    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let (folders_first, folders_check) = menu_option("Folders first", preferences.folders_first);
    let folders_enabled = Rc::new(Cell::new(preferences.folders_first));
    let weak_browser = Rc::downgrade(browser);
    let folders_enabled_for_click = folders_enabled.clone();
    let folders_check_for_click = folders_check.clone();
    let popover_weak = popover_weak.clone();
    folders_first.connect_clicked(move |_| {
        let enabled = !folders_enabled_for_click.get();
        folders_enabled_for_click.set(enabled);
        folders_check_for_click.set_visible(enabled);
        if let Some(browser) = weak_browser.upgrade() {
            browser.set_folders_first(depth, enabled);
        }
        if let Some(popover) = popover_weak.upgrade() {
            popover.popdown();
        }
    });
    content.append(&folders_first);

    popover.set_child(Some(&content));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let dismissed_popover = popover.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
        {
            return glib::Propagation::Proceed;
        }
        if key == gtk::gdk::Key::BackSpace {
            dismissed_popover.popdown();
            glib::Propagation::Stop
        } else if let Some(direction) = match key {
            gtk::gdk::Key::h => Some(gtk::DirectionType::Left),
            gtk::gdk::Key::j => Some(gtk::DirectionType::Down),
            gtk::gdk::Key::k => Some(gtk::DirectionType::Up),
            gtk::gdk::Key::l => Some(gtk::DirectionType::Right),
            _ => None,
        } {
            dismissed_popover.child_focus(direction);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    popover.add_controller(keys);
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
    button.set_child(Some(&crate::assets::primary_icon(
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
    let icon = crate::assets::primary_icon(crate::assets::icons::ARROW_UP_NARROW_WIDE, 16);
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
    crate::assets::set_primary_icon(
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

fn update_empty_trash_sensitivity(column: &ColumnView, count: usize) {
    if let Some(button) = &column.empty_trash_button {
        button.set_sensitive(count > 0);
    }
}

pub(super) fn empty_trash_button(browser: &Rc<Browser>) -> gtk::Button {
    let button = gtk::Button::builder()
        .tooltip_text("Empty Trash")
        .visible(false)
        .build();
    button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::TRASH,
        16,
    )));
    button.add_css_class("column-header-action");
    let weak_browser = Rc::downgrade(browser);
    button.connect_clicked(move |_| {
        if let Some(browser) = weak_browser.upgrade() {
            browser.request_empty_trash();
        }
    });
    button
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

/// Whether a name currently typed into a field should be visually flagged as
/// an error. An empty name is left unstyled: it's the normal starting state
/// (opening, cancelling, or succeeding a prompt all clear the field) rather
/// than a mistake the user made, even though it still can't be submitted.
/// Kept separate from `update_basename_validation` so it can be unit tested
/// without constructing a real GTK widget.
fn basename_field_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        None
    } else {
        validate_basename(name).err()
    }
}

/// Validates a name field live as it changes, including the programmatic
/// clears that happen when a prompt opens, cancels, or succeeds.
pub(super) fn update_basename_validation(field: &gtk::Entry) -> bool {
    let text = field.text();
    match basename_field_error(text.as_str()) {
        None => {
            field.remove_css_class("error");
            field.set_tooltip_text(None);
            !text.is_empty()
        }
        Some(message) => {
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

pub(super) fn entry_model_value(entry: &FileEntry) -> String {
    let kind = if entry.is_broken_symbolic_link() {
        'x'
    } else if entry.is_directory() {
        'd'
    } else if entry.is_symbolic_link() {
        's'
    } else {
        'f'
    };
    let hidden = if entry.is_hidden { 'h' } else { 'v' };
    format!("{kind}{hidden}\t{}", entry.display_name)
}

fn model_display_name(value: &str) -> &str {
    value.split_once('\t').map_or(value, |(_, name)| name)
}

fn model_is_directory(value: &str) -> bool {
    value.starts_with("d")
}

pub(super) fn model_is_hidden(value: &str) -> bool {
    value.as_bytes().get(1) == Some(&b'h')
}

pub(super) fn entry_filter(
    show_hidden: Rc<Cell<bool>>,
    filter_query: Rc<RefCell<String>>,
) -> gtk::CustomFilter {
    gtk::CustomFilter::new(move |item| {
        let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
            return false;
        };
        let value = item.string();
        if !show_hidden.get() && model_is_hidden(&value) {
            return false;
        }
        let query = filter_query.borrow();
        query.is_empty()
            || model_display_name(&value)
                .to_lowercase()
                .contains(query.as_str())
    })
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PeekOriginBounds {
    Anchor,
    Column,
}

fn peek_origin_bounds(mode: BrowserMode) -> PeekOriginBounds {
    match mode {
        BrowserMode::Columns => PeekOriginBounds::Column,
        BrowserMode::Grid | BrowserMode::Explorer => PeekOriginBounds::Anchor,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PeekSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PeekPlacement {
    x: f32,
    side: PeekSide,
}

fn peek_transition(side: PeekSide) -> gtk::RevealerTransitionType {
    match side {
        PeekSide::Left => gtk::RevealerTransitionType::SlideLeft,
        PeekSide::Right => gtk::RevealerTransitionType::SlideRight,
    }
}

fn peek_horizontal_layout(placement: PeekPlacement, viewport_width: f32) -> (gtk::Align, i32, i32) {
    match placement.side {
        PeekSide::Right => (gtk::Align::Start, placement.x.round() as i32, 0),
        PeekSide::Left => (
            gtk::Align::End,
            0,
            (viewport_width - placement.x - PEEK_WIDTH as f32)
                .max(0.0)
                .round() as i32,
        ),
    }
}

fn peek_horizontal_placement(
    source_x: f32,
    source_width: f32,
    viewport_width: f32,
) -> Option<PeekPlacement> {
    let right = source_x + source_width + PEEK_GAP;
    if right + PEEK_WIDTH as f32 <= viewport_width {
        return Some(PeekPlacement {
            x: right,
            side: PeekSide::Right,
        });
    }

    let left = source_x - PEEK_GAP - PEEK_WIDTH as f32;
    (left >= 0.0).then_some(PeekPlacement {
        x: left,
        side: PeekSide::Left,
    })
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

pub(super) fn max_child_natural_width(widget: &gtk::Widget) -> i32 {
    let (_, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
    let mut max_natural = natural;
    let mut child = widget.first_child();
    while let Some(c) = child {
        let child_max = max_child_natural_width(&c);
        if child_max > max_natural {
            max_natural = child_max;
        }
        child = c.next_sibling();
    }
    max_natural
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeleteConfirmationFocus {
    Cancel,
    Confirm,
}

fn delete_confirmation_focus_target(key: gtk::gdk::Key) -> Option<DeleteConfirmationFocus> {
    match key {
        gtk::gdk::Key::Left | gtk::gdk::Key::h => Some(DeleteConfirmationFocus::Cancel),
        gtk::gdk::Key::Right | gtk::gdk::Key::l => Some(DeleteConfirmationFocus::Confirm),
        _ => None,
    }
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

#[expect(
    deprecated,
    reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
)]
pub(super) fn modal_layer(
    content: &impl IsA<gtk::Widget>,
    overlay: &gtk::Overlay,
    root: Option<BlurBin>,
    block_dismiss: Option<Rc<dyn Fn() -> bool>>,
) -> gtk::Box {
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
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    left.set_hexpand(true);
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    right.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(&left);
    row.append(content);
    row.append(&right);
    layer.append(&top);
    layer.append(&row);
    layer.append(&bottom);

    let click = gtk::GestureClick::new();
    let weak_layer = layer.downgrade();
    let weak_content = content.downgrade();
    let overlay = overlay.clone();
    let root = root.clone();
    let block = block_dismiss.clone();
    click.connect_pressed(move |_, _, x, y| {
        if let Some(block) = block.as_ref()
            && block()
        {
            return;
        }
        let Some(layer) = weak_layer.upgrade() else {
            return;
        };
        let Some(content) = weak_content.upgrade() else {
            return;
        };
        let on_dialog = content
            .translate_coordinates(&layer, 0.0, 0.0)
            .is_some_and(|(cx, cy)| {
                let alloc = content.allocation();
                x >= cx
                    && x < cx + alloc.width() as f64
                    && y >= cy
                    && y < cy + alloc.height() as f64
            });
        if !on_dialog {
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
        }
    });
    layer.add_controller(click);
    animate_in(&layer);
    layer
}

pub(super) fn animate_in(layer: &gtk::Box) {
    layer.remove_css_class("dismissing");
    layer.set_sensitive(true);
    layer.add_css_class("modal-hidden");
    let weak = layer.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(16), move || {
        if let Some(layer) = weak.upgrade() {
            layer.remove_css_class("modal-hidden");
        }
    });
}

pub(super) fn animate_out(layer: &gtk::Box, on_done: impl FnOnce() + 'static) {
    layer.add_css_class("modal-hidden");
    glib::timeout_add_local_once(Duration::from_millis(200), on_done);
}

pub(super) fn dismiss_modal_layer(
    layer: &gtk::Box,
    overlay: &gtk::Overlay,
    root: Option<&BlurBin>,
) {
    if layer.has_css_class("dismissing") {
        return;
    }
    layer.add_css_class("dismissing");
    layer.set_sensitive(false);
    let overlay = overlay.clone();
    let layer_for_anim = layer.clone();
    let layer = layer.clone();
    let root = root.cloned();
    animate_out(&layer_for_anim, move || {
        overlay.remove_overlay(&layer);
        if let Some(root) = root
            && !overlay_has_modal_layer(&overlay)
        {
            root.set_blurred(false);
        }
    });
}

fn overlay_has_modal_layer(overlay: &gtk::Overlay) -> bool {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        if widget.is_visible() && widget.has_css_class("app-modal-layer") {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn gio_file_for_location(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}

type MountCredentialsHandler = Rc<dyn Fn(MountCredentials)>;
type MountCancelledHandler = Rc<dyn Fn()>;

struct MountDialogHandlers {
    submitted: Option<MountCredentialsHandler>,
    cancelled: Option<MountCancelledHandler>,
}

#[derive(Clone)]
struct MountPromptDetails {
    message: String,
    default_user: String,
    default_domain: String,
    flags: gio::AskPasswordFlags,
}

impl MountPromptDetails {
    fn fallback(location: &Location) -> Self {
        Self {
            message: format!("Enter your credentials for “{}”.", location.display_path()),
            default_user: String::new(),
            default_domain: String::new(),
            flags: fallback_ask_password_flags(location),
        }
    }
}

/// Chooses which credential fields to offer when a mount fails before the
/// backend ever asked for any, so an SFTP retry does not present SMB's domain
/// and anonymous options.
fn fallback_ask_password_flags(location: &Location) -> gio::AskPasswordFlags {
    let mut flags = gio::AskPasswordFlags::NEED_USERNAME
        | gio::AskPasswordFlags::NEED_PASSWORD
        | gio::AskPasswordFlags::SAVING_SUPPORTED;
    match location_scheme(location).as_str() {
        "smb" => {
            flags |=
                gio::AskPasswordFlags::NEED_DOMAIN | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED;
        }
        "ftp" | "ftps" => flags |= gio::AskPasswordFlags::ANONYMOUS_SUPPORTED,
        _ => {}
    }
    flags
}

fn location_scheme(location: &Location) -> String {
    location
        .uri_value()
        .and_then(|uri| uri.split("://").next())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// SFTP asks for an encrypted key's passphrase through the same password
/// prompt, so the secret field is labelled from the backend's own wording.
fn password_field_label(message: &str) -> &'static str {
    if message.to_ascii_lowercase().contains("passphrase") {
        "Passphrase"
    } else {
        "Password"
    }
}

fn authentication_dialog_heading(message: &str) -> (&'static str, &'static str) {
    if password_field_label(message) == "Passphrase" {
        (
            "Passphrase required",
            "Unlock the key this connection needs",
        )
    } else {
        (
            "Authentication required",
            "Sign in to access this network location",
        )
    }
}

/// Names only the fields the prompt is actually showing, so an SFTP retry does
/// not tell the user to check a domain it never asked for.
fn authentication_failure_hint(flags: gio::AskPasswordFlags, secret_label: &str) -> String {
    let mut fields: Vec<String> = Vec::new();
    if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) {
        fields.push("username".to_owned());
    }
    if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
        fields.push("domain".to_owned());
    }
    if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
        fields.push(secret_label.to_ascii_lowercase());
    }
    let Some(last) = fields.pop() else {
        return "That attempt wasn’t accepted. Try again.".to_owned();
    };
    let listed = if fields.is_empty() {
        last
    } else {
        format!("{} and {last}", fields.join(", "))
    };
    format!("That attempt wasn’t accepted. Check the {listed}, then try again.")
}

const AUTHENTICATION_TEXT_WIDTH_CHARS: i32 = 64;

fn show_authentication_dialog(
    browser_overlay: &gtk::Overlay,
    operation: Option<&gio::MountOperation>,
    message: &str,
    defaults: (&str, &str),
    flags: gio::AskPasswordFlags,
    authentication_failed: bool,
    handlers: MountDialogHandlers,
) -> Option<gtk::Box> {
    let MountDialogHandlers {
        submitted,
        cancelled,
    } = handlers;
    let Some(window_overlay) = browser_overlay
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
    else {
        if let Some(operation) = operation {
            operation.reply(gio::MountOperationResult::Unhandled);
        }
        return None;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let secret_label = password_field_label(message);
    let (heading, subheading) = authentication_dialog_heading(message);
    let layout = modal_layout(crate::assets::icons::KEY, heading, subheading, "Connect");
    layout.content.add_css_class("wide");
    layout.body.add_css_class("authentication-body");
    let explanation_text =
        wrap_dialog_text(message.trim(), AUTHENTICATION_TEXT_WIDTH_CHARS as usize);
    let explanation = gtk::Label::new(Some(&explanation_text));
    explanation.add_css_class("authentication-explanation");
    explanation.set_max_width_chars(AUTHENTICATION_TEXT_WIDTH_CHARS);
    explanation.set_wrap(true);
    explanation.set_xalign(0.0);
    layout.body.append(&explanation);
    if authentication_failed {
        let error_text = wrap_dialog_text(
            &authentication_failure_hint(flags, secret_label),
            AUTHENTICATION_TEXT_WIDTH_CHARS as usize,
        );
        let error = gtk::Label::new(Some(&error_text));
        error.add_css_class("authentication-error");
        error.set_max_width_chars(AUTHENTICATION_TEXT_WIDTH_CHARS);
        error.set_wrap(true);
        error.set_xalign(0.0);
        layout.body.append(&error);
    }

    let credentials = gtk::Box::new(gtk::Orientation::Vertical, 10);
    credentials.add_css_class("authentication-fields");

    let username = form_entry();
    username.set_text(defaults.0);
    if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) {
        append_authentication_field(&credentials, "Username", &username);
    }

    let domain = form_entry();
    domain.set_text(defaults.1);
    if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
        append_authentication_field(&credentials, "Domain", &domain);
    }

    let password = form_password_entry();
    password.set_show_peek_icon(true);
    if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
        append_authentication_field(&credentials, secret_label, &password);
    }

    let (connect_as_control, connect_as_buttons) =
        segmented_control(&["Registered user", "Anonymous"], 0);
    let anonymous = connect_as_buttons[1].clone();
    if flags.contains(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED) {
        let connect_as = gtk::Box::new(gtk::Orientation::Vertical, 7);
        connect_as.append(&form_label("Connect as"));
        connect_as.append(&connect_as_control);
        layout.body.append(&connect_as);
    }
    layout.body.append(&credentials);

    let (remember, remember_buttons) =
        segmented_control(&["Don't remember", "Until logout", "Forever"], 0);
    if flags.contains(gio::AskPasswordFlags::SAVING_SUPPORTED) {
        let remember_field = gtk::Box::new(gtk::Orientation::Vertical, 5);
        remember_field.append(&form_label("Password storage"));
        remember_field.append(&remember);
        layout.body.append(&remember_field);
    }
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let connect = layout.confirm;

    let credential_widgets = [
        username.clone().upcast::<gtk::Widget>(),
        domain.clone().upcast(),
        password.clone().upcast(),
        remember.clone().upcast(),
    ];
    anonymous.connect_toggled(move |anonymous| {
        for widget in &credential_widgets {
            widget.set_sensitive(!anonymous.is_active());
        }
    });

    let auth_user = username.clone();
    let auth_domain = domain.clone();
    let auth_password = password.clone();
    let layer = modal_layer(
        &content,
        &window_overlay,
        blurred_root.clone(),
        Some(Rc::new(move || {
            !auth_user.text().is_empty()
                || !auth_domain.text().is_empty()
                || !auth_password.text().is_empty()
        })),
    );
    window_overlay.add_overlay(&layer);

    let cancel_operation = operation.cloned();
    let cancel_handler = cancelled.clone();
    let cancel_layer = layer.clone();
    let cancel_overlay = window_overlay.clone();
    let cancel_root = blurred_root.clone();
    cancel.connect_clicked(move |_| {
        dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
        if let Some(operation) = cancel_operation.as_ref() {
            operation.reply(gio::MountOperationResult::Aborted);
        } else if let Some(cancelled) = cancel_handler.as_ref() {
            cancelled();
        }
    });

    let close_operation = operation.cloned();
    let close_handler = cancelled.clone();
    let close_layer = layer.clone();
    let close_overlay = window_overlay.clone();
    let close_root = blurred_root.clone();
    close.connect_clicked(move |_| {
        dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
        if let Some(operation) = close_operation.as_ref() {
            operation.reply(gio::MountOperationResult::Aborted);
        } else if let Some(cancelled) = close_handler.as_ref() {
            cancelled();
        }
    });

    let connect_operation = operation.cloned();
    let connect_layer = layer.clone();
    let connect_overlay = window_overlay.clone();
    let connect_root = blurred_root.clone();
    let connect_username = username.clone();
    let connect_domain = domain.clone();
    let connect_password = password.clone();
    let connect_anonymous = anonymous.clone();
    let connect_remember = remember_buttons;
    connect.connect_clicked(move |_| {
        let selected = connect_remember
            .iter()
            .position(gtk::ToggleButton::is_active)
            .unwrap_or_default() as u32;
        let credentials = MountCredentials {
            anonymous: connect_anonymous.is_active(),
            username: connect_username.text().to_string(),
            domain: connect_domain.text().to_string(),
            password: connect_password.text().to_string(),
            save: password_save_for_selection(selected),
        };
        if let Some(operation) = connect_operation.as_ref() {
            apply_mount_credentials(operation, &credentials);
        }
        dismiss_modal_layer(&connect_layer, &connect_overlay, connect_root.as_ref());
        if let Some(operation) = connect_operation.as_ref() {
            operation.reply(gio::MountOperationResult::Handled);
        }
        if let Some(submitted) = submitted.as_ref() {
            submitted(credentials);
        }
    });

    for entry in [&username, &domain] {
        let submit = connect.clone();
        entry.connect_activate(move |_| submit.emit_clicked());
    }
    let submit = connect.clone();
    password.connect_activate(move |_| submit.emit_clicked());

    let escape = gtk::EventControllerKey::new();
    let escape_operation = operation.cloned();
    let escape_handler = cancelled;
    let escape_layer = layer.clone();
    let escape_overlay = window_overlay;
    let escape_root = blurred_root;
    escape.connect_key_pressed(move |_, key, _, _| {
        if key != gtk::gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
        if let Some(operation) = escape_operation.as_ref() {
            operation.reply(gio::MountOperationResult::Aborted);
        } else if let Some(cancelled) = escape_handler.as_ref() {
            cancelled();
        }
        glib::Propagation::Stop
    });
    layer.add_controller(escape);

    if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) && defaults.0.is_empty() {
        username.grab_focus();
    } else if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
        password.grab_focus();
    } else {
        connect.grab_focus();
    }
    Some(layer)
}

fn dismiss_authentication_prompt(browser_overlay: &gtk::Overlay, layer: &gtk::Box) {
    if layer.parent().is_none() {
        return;
    }
    let Some(window_overlay) = browser_overlay
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
    else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    dismiss_modal_layer(layer, &window_overlay, blurred_root.as_ref());
}

fn append_authentication_field(fields: &gtk::Box, label_text: &str, field: &impl IsA<gtk::Widget>) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 5);
    group.append(&form_label(label_text));
    group.append(field);
    fields.append(&group);
}

fn password_save_for_selection(selected: u32) -> gio::PasswordSave {
    match selected {
        1 => gio::PasswordSave::ForSession,
        2 => gio::PasswordSave::Permanently,
        _ => gio::PasswordSave::Never,
    }
}

fn credentials_from_location_input(
    input: &str,
) -> Result<(String, Option<MountCredentials>), LocationValidationError> {
    if !input.contains("://") {
        return Ok((input.to_owned(), None));
    }
    let (sanitized, credentials) = sanitize_uri_credentials(input)?;
    let credentials = credentials.map(|credentials: UriCredentials| MountCredentials {
        anonymous: false,
        username: credentials.username,
        domain: String::new(),
        password: credentials.password,
        save: gio::PasswordSave::Never,
    });
    Ok((sanitized, credentials))
}

#[derive(Clone)]
struct MountCredentials {
    anonymous: bool,
    username: String,
    domain: String,
    password: String,
    save: gio::PasswordSave,
}

impl MountCredentials {
    fn default_for_prompt(flags: gio::AskPasswordFlags) -> Self {
        Self {
            anonymous: false,
            username: glib::user_name().to_string_lossy().into_owned(),
            domain: if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
                "WORKGROUP".to_owned()
            } else {
                String::new()
            },
            password: String::new(),
            save: gio::PasswordSave::Never,
        }
    }
}

fn apply_mount_credentials(operation: &gio::MountOperation, credentials: &MountCredentials) {
    operation.set_anonymous(credentials.anonymous);
    if credentials.anonymous {
        return;
    }
    operation.set_username(Some(&credentials.username));
    if !credentials.domain.is_empty() {
        operation.set_domain(Some(&credentials.domain));
    }
    operation.set_password(Some(&credentials.password));
    operation.set_password_save(credentials.save);
}

/// A `GMountOperation::ask-question` request: a trust decision the backend
/// refuses to make on its own, such as an unrecognized or changed SSH host key.
#[derive(Clone, Debug, Eq, PartialEq)]
struct MountQuestionDetails {
    message: String,
    choices: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MountQuestionKind {
    UnknownHostKey,
    ChangedHostKey,
    Other,
}

impl MountQuestionKind {
    fn classify(message: &str) -> Self {
        let message = message.to_ascii_lowercase();
        let about_host_key = ["host key", "identity of the remote computer", "fingerprint"]
            .iter()
            .any(|marker| message.contains(marker));
        if !about_host_key {
            return Self::Other;
        }
        if ["changed", "man-in-the-middle", "nasty", "eavesdropping"]
            .iter()
            .any(|marker| message.contains(marker))
        {
            Self::ChangedHostKey
        } else {
            Self::UnknownHostKey
        }
    }

    fn heading(self) -> (&'static str, &'static str) {
        match self {
            Self::UnknownHostKey => (
                "Unrecognized host key",
                "This computer has not been verified before",
            ),
            Self::ChangedHostKey => (
                "Host key changed",
                "This computer is not presenting the key it used before",
            ),
            Self::Other => (
                "Confirmation required",
                "The connection needs a decision before it can continue",
            ),
        }
    }

    fn tone(self) -> ModalTone {
        match self {
            Self::ChangedHostKey => ModalTone::Danger,
            Self::UnknownHostKey | Self::Other => ModalTone::Accent,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::ChangedHostKey => crate::assets::icons::TRIANGLE_ALERT,
            Self::UnknownHostKey | Self::Other => crate::assets::icons::KEY,
        }
    }
}

/// Recognizes the choice that declines, so it can be the focused default. A
/// trust decision is never pre-selected in the accepting direction.
fn choice_declines(label: &str) -> bool {
    let label = label.to_ascii_lowercase();
    ["cancel", "abort", "no", "don't", "do not", "reject", "deny"]
        .iter()
        .any(|marker| label.contains(marker))
}

fn safest_choice(choices: &[String]) -> usize {
    choices
        .iter()
        .position(|choice| choice_declines(choice))
        .unwrap_or_else(|| choices.len().saturating_sub(1))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MountQuestionDecision {
    Chose(usize),
    Declined,
}

fn show_mount_question_dialog(
    browser_overlay: &gtk::Overlay,
    operation: &gio::MountOperation,
    details: &MountQuestionDetails,
    decided: impl Fn(MountQuestionDecision) + 'static,
) -> Option<gtk::Box> {
    if details.choices.is_empty() {
        operation.reply(gio::MountOperationResult::Aborted);
        decided(MountQuestionDecision::Declined);
        return None;
    }
    let Some(window_overlay) = browser_overlay
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
    else {
        operation.reply(gio::MountOperationResult::Aborted);
        decided(MountQuestionDecision::Declined);
        return None;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let kind = MountQuestionKind::classify(&details.message);
    let (heading, subheading) = kind.heading();
    let layout = modal_layout_with_tone(kind.icon(), heading, subheading, "Continue", kind.tone());
    layout.content.add_css_class("wide");
    layout.body.add_css_class("authentication-body");
    let explanation = gtk::Label::new(Some(&wrap_dialog_text(
        details.message.trim(),
        AUTHENTICATION_TEXT_WIDTH_CHARS as usize,
    )));
    explanation.add_css_class("authentication-explanation");
    explanation.set_max_width_chars(AUTHENTICATION_TEXT_WIDTH_CHARS);
    explanation.set_selectable(true);
    explanation.set_wrap(true);
    explanation.set_xalign(0.0);
    layout.body.append(&explanation);

    // The backend owns the wording of every option, so the stock Cancel and
    // Continue buttons are replaced by one button per offered choice.
    while let Some(child) = layout.actions.first_child() {
        layout.actions.remove(&child);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    layout.actions.append(&spacer);

    let content = layout.content;
    let layer = modal_layer(
        &content,
        &window_overlay,
        blurred_root.clone(),
        Some(Rc::new(|| true)),
    );
    window_overlay.add_overlay(&layer);
    let decided = Rc::new(decided);
    let answered = Rc::new(Cell::new(false));

    let default_choice = safest_choice(&details.choices);
    let mut default_button = None;
    for (index, choice) in details.choices.iter().enumerate() {
        let button = gtk::Button::with_label(choice);
        if choice_declines(choice) {
            button.add_css_class("action-dialog-cancel");
        } else {
            button.add_css_class("action-dialog-confirm");
            if kind.tone() == ModalTone::Danger {
                button.add_css_class("danger");
            }
        }
        let operation = operation.clone();
        let decided = decided.clone();
        let answered = answered.clone();
        let layer = layer.clone();
        let overlay = window_overlay.clone();
        let root = blurred_root.clone();
        button.connect_clicked(move |_| {
            if answered.replace(true) {
                return;
            }
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
            operation.set_choice(index as i32);
            operation.reply(gio::MountOperationResult::Handled);
            decided(MountQuestionDecision::Chose(index));
        });
        layout.actions.append(&button);
        if index == default_choice {
            default_button = Some(button);
        }
    }

    let decline = {
        let operation = operation.clone();
        let decided = decided.clone();
        let answered = answered.clone();
        let layer = layer.clone();
        let overlay = window_overlay.clone();
        let root = blurred_root.clone();
        Rc::new(move || {
            if answered.replace(true) {
                return;
            }
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
            operation.reply(gio::MountOperationResult::Aborted);
            decided(MountQuestionDecision::Declined);
        })
    };

    let close_decline = decline.clone();
    layout.close.connect_clicked(move |_| close_decline());

    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key != gtk::gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        decline();
        glib::Propagation::Stop
    });
    layer.add_controller(escape);

    if let Some(button) = default_button {
        button.grab_focus();
    }
    Some(layer)
}

/// How a finished mount attempt should be reported to the caller.
enum MountOutcome {
    Mounted,
    /// The user backed out, so the browser returns to where it already was.
    Cancelled,
    NeedsCredentials {
        attempted: Option<MountCredentials>,
        details: Option<MountPromptDetails>,
    },
    Failed(String),
}

/// What a backend password request should do, given what the attempt has
/// already asked the user.
#[derive(Debug, Eq, PartialEq)]
enum PasswordRequest {
    /// Credentials the user already supplied are replayed without a prompt.
    Reply,
    /// The user must be asked; `retry` marks a second or later request, which
    /// only happens after the backend rejected the previous answer.
    Ask { retry: bool },
}

/// Tracks what a single mount attempt has already asked the user, so the
/// credential prompt is shown once per backend request rather than looping.
#[derive(Default)]
struct MountAttempt {
    supplied: Option<MountCredentials>,
    attempted: Option<MountCredentials>,
    prompted: bool,
    details: Option<MountPromptDetails>,
    question: Option<MountQuestionDetails>,
    declined: bool,
}

impl MountAttempt {
    fn new(credentials: Option<MountCredentials>) -> Self {
        Self {
            prompted: credentials.is_some(),
            supplied: credentials.clone(),
            attempted: credentials,
            ..Self::default()
        }
    }

    fn ask_password(&mut self, details: MountPromptDetails) -> PasswordRequest {
        self.details = Some(details);
        if self.supplied.take().is_some() {
            return PasswordRequest::Reply;
        }
        PasswordRequest::Ask {
            retry: std::mem::replace(&mut self.prompted, true),
        }
    }

    fn submitted(&mut self, credentials: MountCredentials) {
        self.attempted = Some(credentials);
    }

    fn ask_question(&mut self, details: MountQuestionDetails) {
        self.question = Some(details);
    }

    fn decide(&mut self, decision: MountQuestionDecision) {
        self.declined = decision == MountQuestionDecision::Declined;
    }

    fn outcome(&self, location: &Location, result: Result<(), glib::Error>) -> MountOutcome {
        let error = match result {
            Ok(()) => return MountOutcome::Mounted,
            Err(error) if error.matches(gio::IOErrorEnum::AlreadyMounted) => {
                return MountOutcome::Mounted;
            }
            Err(error) => error,
        };
        if self.declined {
            return MountOutcome::Cancelled;
        }
        if mount_error_is_authentication_failure(location, &error) {
            return MountOutcome::NeedsCredentials {
                attempted: self.attempted.clone(),
                details: self.details.clone(),
            };
        }
        match mount_failure_message(location, &error) {
            Some(message) => MountOutcome::Failed(message),
            None => MountOutcome::Cancelled,
        }
    }
}

fn log_mount_started(location: &Location, strategy: MountStrategy) {
    tracing::info!(
        backend = %location.backend_name(),
        strategy = strategy.name(),
        "mount requested"
    );
    tracing::debug!(location = %location.diagnostic_path(), "mount location");
}

fn log_mount_finished(location: &Location, result: &Result<(), glib::Error>) {
    match result {
        Ok(()) => tracing::info!(backend = %location.backend_name(), "mount finished"),
        Err(error) => tracing::info!(
            backend = %location.backend_name(),
            error_domain = ?error.domain(),
            error_code = error.code(),
            "mount failed"
        ),
    }
}

#[derive(Clone, Copy)]
enum MountStrategy {
    /// The location itself is accessible but sits on an unmounted volume.
    EnclosingVolume,
    /// The location is itself the mountable target (an SMB share, a
    /// "Connect to Server" bookmark, ...).
    Mountable,
}

impl MountStrategy {
    fn name(self) -> &'static str {
        match self {
            Self::EnclosingVolume => "enclosing-volume",
            Self::Mountable => "mountable",
        }
    }
}

fn mount_error_is_authentication_failure(location: &Location, error: &glib::Error) -> bool {
    if location.uri_value().is_none() {
        return false;
    }
    if mount_error_is_host_key_rejection(error) {
        return false;
    }
    if error.matches(gio::IOErrorEnum::PermissionDenied) {
        return true;
    }

    // GVfs' SMB backend reports rejected credentials as G_IO_ERROR_FAILED on
    // some versions, preserving the useful distinction only in its message.
    let message = error.message().to_ascii_lowercase();
    [
        "permission denied",
        "authentication failed",
        "authentication required",
        "auth fail",
        "logon failure",
        "invalid credentials",
        "too many authentication failures",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

/// Host-key problems reach us as generic failures whose message is the only
/// signal. They must not be replayed as rejected credentials, or declining a
/// key would immediately reopen the sign-in prompt.
fn mount_error_is_host_key_rejection(error: &glib::Error) -> bool {
    let message = error.message().to_ascii_lowercase();
    [
        "host key",
        "host identification",
        "fingerprint",
        "known_hosts",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

/// Decides what, if anything, to tell the user about a failed mount attempt.
/// A user-initiated cancel (the GTK credential dialog's Cancel button, or a
/// backend that already reported the failure to the operation itself) should
/// quietly return to the prior state rather than surface an alarming error,
/// per lgse/strata#20's "cancelling authentication returns to the prior
/// committed location" requirement.
fn mount_failure_message(location: &Location, error: &glib::Error) -> Option<String> {
    if error.matches(gio::IOErrorEnum::Cancelled) || error.matches(gio::IOErrorEnum::FailedHandled)
    {
        return None;
    }
    if error.matches(gio::IOErrorEnum::NotSupported) {
        return Some(backend_unavailable_message(
            location.uri_value().unwrap_or_default(),
        ));
    }
    if mount_error_is_host_key_rejection(error) {
        return Some(
            "The remote computer’s host key could not be verified, so the connection was \
             refused. Confirm the new key with whoever runs the server and remove the stale \
             entry from your known_hosts file before trying again."
                .to_owned(),
        );
    }
    if let Some(message) = transport_failure_message(error) {
        return Some(message);
    }
    Some(sanitize_failure_message(&error.to_string()))
}

/// Turns the transport failures a remote mount can hit into guidance the user
/// can act on, instead of the backend's own terse wording.
fn transport_failure_message(error: &glib::Error) -> Option<String> {
    if error.matches(gio::IOErrorEnum::HostNotFound) {
        return Some(
            "That host couldn’t be found. Check the address for typos and confirm the name \
             resolves on this network."
                .to_owned(),
        );
    }
    if error.matches(gio::IOErrorEnum::ConnectionRefused) {
        return Some(
            "The host refused the connection. Check that the service is running and that you \
             used the right port."
                .to_owned(),
        );
    }
    if error.matches(gio::IOErrorEnum::TimedOut) {
        return Some(
            "The connection timed out. Check that the host is reachable and that a firewall \
             isn’t blocking the port."
                .to_owned(),
        );
    }
    if error.matches(gio::IOErrorEnum::HostUnreachable)
        || error.matches(gio::IOErrorEnum::NetworkUnreachable)
    {
        return Some(
            "That host is unreachable from this network. Check your connection, then try again."
                .to_owned(),
        );
    }
    None
}

/// Strips URI user-info secrets from a backend message, so a password typed
/// into the address bar cannot resurface in an error dialog.
fn sanitize_failure_message(message: &str) -> String {
    message
        .split_inclusive(char::is_whitespace)
        .map(|token| {
            let Some((scheme, rest)) = token.split_once("://") else {
                return token.to_owned();
            };
            let authority_end = rest.find('/').unwrap_or(rest.len());
            let Some(userinfo_end) = rest[..authority_end].rfind('@') else {
                return token.to_owned();
            };
            let user = rest[..userinfo_end]
                .split([':', ';'])
                .next()
                .unwrap_or_default();
            let remainder = &rest[userinfo_end + 1..];
            if user.is_empty() {
                format!("{scheme}://{remainder}")
            } else {
                format!("{scheme}://{user}@{remainder}")
            }
        })
        .collect()
}

pub(super) fn is_trash_root(location: &Location) -> bool {
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

fn can_open_terminal(location: &Location) -> bool {
    location.native_path().is_some() && !is_trash_location(location)
}

fn selected_terminal_location(entries: &[FileEntry]) -> Option<Location> {
    let [entry] = entries else {
        return None;
    };
    entry.is_directory().then(|| entry.location.clone())
}

fn terminal_directory_argument(path: &Path) -> OsString {
    let mut argument = OsString::from("--dir=");
    argument.push(path);
    argument
}

pub(super) fn launch_terminal(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let Some(path) = location.native_path() else {
        show_error_dialog(
            parent,
            "Unable to open terminal",
            "This location is not a local folder",
        );
        return;
    };
    if is_trash_location(location) {
        show_error_dialog(
            parent,
            "Unable to open terminal",
            "Terminal cannot be opened in Trash",
        );
        return;
    }
    let path = path.to_path_buf();
    tracing::debug!(
        location = %location.diagnostic_path(),
        "opening terminal"
    );
    let result = Command::new("xdg-terminal-exec")
        .arg(terminal_directory_argument(&path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(error) = result {
        tracing::warn!(%error, "unable to launch terminal");
        show_error_dialog(parent, "Unable to open terminal", &error.to_string());
    }
}

pub(super) fn show_error_dialog(parent: &impl IsA<gtk::Widget>, message: &str, detail: &str) {
    show_error_dialog_after_close(parent, message, detail, Rc::new(|| {}));
}

fn show_error_dialog_after_close(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    detail: &str,
    on_close: Rc<dyn Fn()>,
) {
    let Some(window_overlay) = parent
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
    else {
        on_close();
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let layout = message_dialog_layout(
        crate::assets::icons::X,
        message,
        if message == "Completed with errors" {
            "Some items could not be processed"
        } else {
            "The operation could not be completed"
        },
        "Close",
        ModalTone::Danger,
    );
    layout.cancel.set_visible(false);
    let explanation = message_dialog_description(detail);
    explanation.set_selectable(true);
    layout.body.append(&explanation);
    let content = layout.content;
    let close_icon = layout.close;
    let close = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    window_overlay.add_overlay(&layer);
    let close_layer = layer.clone();
    let close_overlay = window_overlay.clone();
    let close_root = blurred_root.clone();
    let dismissed = Rc::new(Cell::new(false));
    let dismiss = move || {
        if dismissed.replace(true) {
            return;
        }
        dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
        let on_close = on_close.clone();
        glib::timeout_add_local_once(Duration::from_millis(250), move || on_close());
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
