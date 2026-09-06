// SPDX-License-Identifier: GPL-3.0-or-later

//! Browser composition and public commands. Feature modules share this view's state and the
//! application controller; they must not create independent navigation or operation state.

use crate::app::Browser;
use crate::model::{FileEntry, Location};
use crate::services::{FileSource, LoadHandle, OperationProvider};
use crate::ui::browser::clipboard::{copy_locations, register_cut_view};
use crate::ui::browser::collection::cancel_source;
use crate::ui::browser::columns::{COLUMN_WIDTH, ColumnView};
use crate::ui::browser::desktop::selected_terminal_location;
use crate::ui::browser::inline_edit::{ActiveNewEntry, ActiveRename};
use crate::ui::browser::location::{MountCredentials, is_breadcrumb_button_target};
use crate::ui::browser::paths::{can_pin_entry, is_trash_location};
use crate::ui::browser::peek::{PeekAnchor, PeekView};
use crate::ui::browser::progress::FileProgressView;
use crate::ui::browser::transfer::duplicate_transfer;
use crate::ui::browser::trash::TrashLoadingView;
use crate::ui::browser_modes::{BrowserDensity, BrowserMode, ClickActivation, ModeViews};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

mod archive;
mod clipboard;
mod collection;
mod columns;
mod context_menu;
mod customization;
mod desktop;
mod destination;
mod entry;
mod events;
mod inline_edit;
mod location;
mod pane_header;
mod paths;
mod peek;
mod presentation;
mod progress;
mod properties;
mod transfer;
mod trash;

pub(super) use crate::ui::browser::clipboard::file_drag_content;
pub(crate) use crate::ui::browser::clipboard::{file_drop_action, locations_from_file_list_value};
pub(crate) use crate::ui::browser::collection::{
    activate_recursive_search_result, debounce_filter_entry, detach_collection_view,
    focus_collection_item_when_allocated, focus_filter_entry, notify_filter_query,
    recursive_search_activation_key, scroll_collection_when_allocated,
    search_result_navigation_position,
};
pub(super) use crate::ui::browser::columns::max_child_natural_width;
pub(super) use crate::ui::browser::context_menu::{
    install_folder_context_menu, install_item_context_menu,
};
pub(super) use crate::ui::browser::desktop::{launch_terminal, open_location};
pub(super) use crate::ui::browser::entry::{
    FOLDER_TYPE_GROUP, entry_filter, entry_icon, entry_model_value, format_file_size,
    metadata_needs_fill, model_type_group,
};
pub(super) use crate::ui::browser::inline_edit::{rename_stem_end, update_basename_validation};
pub(super) use crate::ui::browser::pane_header::{
    column_sort_direction_toggle, column_sort_menu, empty_trash_button, pane_new_folder_button,
    pane_refresh_button,
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
    input_ownership: RefCell<super::input_ownership::InputOwnership>,
    horizontal_scroll_generation: Rc<Cell<u64>>,
    source_generation: Rc<Cell<u64>>,
    peek: RefCell<Option<PeekView>>,
    pending_peek: RefCell<Option<glib::SourceId>>,
    pending_close: RefCell<Option<glib::SourceId>>,
    peek_anchor: RefCell<Option<PeekAnchor>>,
    peek_behavior: PeekBehavior,
    peek_enabled: Cell<bool>,
    single_click_previews: Cell<bool>,
    multiple_selection: Rc<Cell<bool>>,
    interactive: bool,
    columns_click_activation: Cell<ClickActivation>,
    active_rename: RefCell<Option<ActiveRename>>,
    active_new_entry: RefCell<Option<ActiveNewEntry>>,
    file_progress_view: RefCell<Option<FileProgressView>>,
    pending_file_progress: RefCell<Option<glib::SourceId>>,
    file_operation_progress: Cell<(usize, usize)>,
    transfer_progress: Cell<Option<(usize, u64, Option<u64>)>>,
    pin_handler: RefCell<Option<PinHandler>>,
    unpin_handler: RefCell<Option<UnpinHandler>>,
    pin_status_handler: RefCell<Option<PinStatusHandler>>,
    print_handler: RefCell<Option<PrintHandler>>,
    pending_select: RefCell<Vec<String>>,
    /// Set when the pending selection came from a properties request, so the
    /// dialog opens once the entry it describes is actually loaded.
    pending_select_properties: Cell<bool>,
    pending_extract_retry: RefCell<Option<(FileEntry, Location)>>,
    /// The entries a just-dispatched, non-permanent delete requested,
    /// snapshotted so a `CompletedWithErrors` response naming entries that
    /// failed only because the location doesn't support Trash can offer a
    /// permanent-delete retry for exactly those entries.
    pending_delete_entries: RefCell<Vec<FileEntry>>,
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
            .overlay_scrolling(false)
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
        breadcrumb_scroller.add_css_class("fixed-scrollbar");
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
        let source_generation = Rc::new(Cell::new(0u64));
        let multiple_selection = Rc::new(Cell::new(multiple));
        let mode_views = ModeViews::new(&scroller, browser.clone(), multiple_selection.clone());
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
            input_ownership: RefCell::new(super::input_ownership::InputOwnership::default()),
            horizontal_scroll_generation: Rc::new(Cell::new(0)),
            source_generation,
            peek: RefCell::new(None),
            pending_peek: RefCell::new(None),
            pending_close: RefCell::new(None),
            peek_anchor: RefCell::new(None),
            peek_behavior,
            peek_enabled: Cell::new(true),
            single_click_previews: Cell::new(true),
            multiple_selection,
            interactive,
            columns_click_activation: Cell::new(ClickActivation::default()),
            active_rename: RefCell::new(None),
            active_new_entry: RefCell::new(None),
            file_progress_view: RefCell::new(None),
            pending_file_progress: RefCell::new(None),
            file_operation_progress: Cell::new((0, 0)),
            transfer_progress: Cell::new(None),
            pin_handler: RefCell::new(None),
            unpin_handler: RefCell::new(None),
            pin_status_handler: RefCell::new(None),
            print_handler: RefCell::new(None),
            pending_select: RefCell::new(Vec::new()),
            pending_select_properties: Cell::new(false),
            pending_extract_retry: RefCell::new(None),
            pending_delete_entries: RefCell::new(Vec::new()),
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
        register_cut_view(&state);
        state.install_input_ownership();

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
                move |destination, sources, move_sources| {
                    if let Some(state) = weak_state.upgrade() {
                        state.start_transfer(destination, sources, move_sources);
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

        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.overlay.clone().upcast()
    }

    pub fn navigate_location(&self, location: Location) {
        self.state.browser.navigate(location);
    }

    pub fn start_transfer(
        &self,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        self.state
            .start_transfer(destination, sources, move_sources);
    }

    /// Selects `names` in the active column once it finishes loading,
    /// optionally opening the properties dialog for the focused one.
    pub fn select_after_load(&self, names: Vec<String>, properties: bool) {
        self.state.pending_select.borrow_mut().extend(names);
        self.state.pending_select_properties.set(properties);
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

    pub(super) fn set_print_handler(&self, handler: PrintHandler) {
        self.state.print_handler.replace(Some(handler));
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

    pub fn connect_view_mode_changed(&self, handler: impl Fn(BrowserMode) + 'static) {
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
                handler(mode);
            });
    }

    pub fn set_view_mode(&self, mode: BrowserMode) {
        let previous = self.state.mode_views.borrow().mode();
        if mode == previous {
            return;
        }
        self.state.mode_views.borrow().show_mode(mode);
        self.state.mode_views.borrow_mut().prepare_mode(mode);
        if mode == BrowserMode::Columns {
            self.state.rebuild_columns();
        } else if let Some(depth) = self.state.browser.active_depth() {
            self.state.mode_views.borrow().focus_visible_pane(depth);
        }
        match previous {
            BrowserMode::Columns => self.state.truncate(0),
            BrowserMode::Icons | BrowserMode::List => self
                .state
                .mode_views
                .borrow_mut()
                .clear_inactive_mode(previous),
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

    pub fn synchronize_native_selection(&self, extend: bool) {
        let position = self.state.mode_views.borrow().focused_position();
        if let Some((depth, focused)) = position {
            self.select_native_target(depth, focused, extend);
        }
    }

    pub fn cross_type_group(&self, direction: gtk::DirectionType, extend: bool) -> bool {
        let target = self
            .state
            .mode_views
            .borrow()
            .group_boundary_target(direction);
        if let Some((depth, focused)) = target {
            self.select_native_target(depth, focused, extend);
            true
        } else {
            false
        }
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
        let moved = focus_header_action(&column.header_actions, direction);
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

    pub fn paste(&self) {
        self.state.sync_mode_selection();
        let selected = self.state.browser.selected_entries();
        let column = self
            .state
            .destination_depth()
            .and_then(|depth| self.state.browser.location_at(depth));
        if let Some(location) = paste_destination(&selected, column) {
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

    pub fn duplicate_selection(&self) -> bool {
        self.state.sync_mode_selection();
        let entries = self.state.browser.selected_entries();
        let Some((destination, sources)) = duplicate_transfer(&entries) else {
            return false;
        };
        self.state.start_transfer(destination, sources, false);
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
        self.keyboard_navigation();
        if self.view_mode() == BrowserMode::Columns {
            if let Some(depth) = self
                .state
                .focused_column_depth()
                .or_else(|| self.state.browser.active_depth())
            {
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
        let entries = if self.view_mode() == BrowserMode::Columns {
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
        self.state.request_delete(entries, permanent || in_trash);
        true
    }

    pub fn undo_last_operation(&self) -> bool {
        if let Some((generation, records)) = self.state.browser.pending_undo_move() {
            return self.state.undo_move(generation, records);
        }
        self.state.browser.undo_last_trash()
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
        let depth = self
            .state
            .focused_column_depth()
            .or_else(|| self.state.browser.active_depth());
        let columns = self.state.columns.borrow();
        let Some(column) = depth.and_then(|depth| columns.get(depth)) else {
            return false;
        };
        column.filter_button.set_active(true);
        focus_filter_entry(&column.filter_entry, query);
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
                    focused == column.presentation.stack.upcast_ref::<gtk::Widget>()
                        || focused == column.list.upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(&column.list)
                })
            })
    }

    /// Moves the focus by about one viewport of the focused view, so `Page Up` and
    /// `Page Down` act on the active pane or column only.
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
        self.state.browser.page_selection(direction, page.items);
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
        self.state.browser.page_selection(direction, usize::MAX);
        super::scrolling::reveal_jump(&view, &scroll, direction);
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
            state.refresh_browser();
            glib::ControlFlow::Continue
        });
        self.auto_refresh.replace(Some(source));
    }

    fn refresh_browser(&self) {
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            self.browser.refresh_all();
        } else {
            self.browser.reload_active();
        }
    }

    fn sync_mode_selection(&self) {
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            if let Some(depth) = self.focused_column_depth() {
                self.browser.set_active_column(depth);
            }
            return;
        }
        let Some((depth, positions)) = self.mode_views.borrow().selected_positions() else {
            return;
        };
        let focused = positions.last().copied();
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
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self.browser.active_depth();
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
        let pointer = self.input_ownership.borrow().last_navigation
            == super::input_ownership::NavigationInput::Pointer
            && self.hovered_column.get() == destination;
        let focused_column = self.focused_column_depth();
        let focused_item = self
            .browser
            .focused_item()
            .map(|(depth, position, _)| (depth, position));
        for (depth, column) in self.columns.borrow().iter().enumerate() {
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
            let active = destination == Some(depth);
            if active {
                column.shell.add_css_class("destination-column");
            } else {
                column.shell.remove_css_class("destination-column");
            }
            column.destination_hint.set_label(if !active {
                ""
            } else if pointer {
                "Pointer · Paste here"
            } else {
                "Keyboard · Paste here"
            });
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
        if let Some(column) = self.columns.borrow().get(depth) {
            column.selection.select_all();
            column.list.grab_focus();
        }
    }
}

fn paste_destination(selected: &[FileEntry], column: Option<Location>) -> Option<Location> {
    match selected {
        [folder] if folder.is_directory() => Some(folder.location.clone()),
        _ => column,
    }
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
