// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    env,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{
    gdk::ModifierType,
    gio,
    gio::prelude::VfsExt as _,
    glib,
    prelude::*,
};

use crate::{
    adapters::{
        LocalFileSource, LocalOperationProvider, RevealRequest, location_for_file,
        mount_can_unplug, mount_lanes, probes, volume_can_unplug,
    },
    app::{Browser, BrowserEvent},
    model::Location,
    services::LocationValidationError,
};

use super::{
    browser::{
        BrowserView, DropHandler, PeekBehavior, PinStatus, PreparedFileDrop, WeakBrowserView,
        arm_spring_load_navigation, attach_drop_target, commit_dropped_value,
        file_drag_hover_target, file_drop_action, locations_from_file_list_value,
        prepare_file_drop_target, show_error_dialog,
    },
    browser_modes::BrowserMode,
    controls::{ModalTone, focus_button, message_dialog_description, message_dialog_layout},
    keys::{
        KeyPress,
        bindings::{
            BACKSPACE, DOWN, END_KEY, ENTER, FILTER_SLASH, FOCUS_NEXT, HOME_KEY, LEFT,
            PAGE_DOWN_KEY, PAGE_UP_KEY, RIGHT, SPACE, UP, VIEW_COLUMNS, VIEW_ICONS, VIEW_LIST,
        },
    },
    modal::{ModalHost, close_on_escape, dismiss_modal_layer, modal_layer},
    motion::{animations_enabled, emphasized_deceleration},
    preferences::PreferenceManager,
};

mod appearance_menu;
mod bookmarks;
mod composition;
mod density_slider;
mod device_labels;
mod device_release;
mod devices;
mod drive_dialogs;
mod drive_ops;
mod gio_devices;
mod key_routing;
mod keyboard;
mod launch;
mod modal_layers;
mod network_location;
mod open_argument;
mod pinned_drop;
mod places;
mod sidebar;
mod sidebar_drop_feedback;
mod sidebar_width;
mod unlock_argument;
mod volume_password;

pub(super) use appearance_menu::build_appearance_menu;
pub(crate) use gio_devices::gio_volume_is_encrypted;
pub(super) use gio_devices::gio_volume_unix_device;
use gio_devices::{gio_mount_is_encrypted, gio_volume_is_locked, orphaned_password_drives};
pub(super) use modal_layers::{foreign_modal_visible, install_modal_focus_trap, visible_modal_layer};
use modal_layers::topmost_modal_layer;
pub(super) use key_routing::{
    TypeToSearchQuery, apply_browser_mode, browser_mode_for_digit, chooser_keys,
    is_browser_navigation_key, type_to_search_query,
};
use key_routing::{
    MouseHistoryAction, SinglePaneArrow, TypeToSearch, mouse_history_action,
    sidebar_focus_direction, single_pane_arrow_action,
};
pub(in crate::ui) use sidebar_width::{RailPolicy, SidebarFit, sidebar_fits_without_preview};
pub(super) use sidebar_width::{
    MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH, bind_sidebar_text_size, preferred_sidebar_width,
    sidebar_rail_button_size, sidebar_rail_width,
};
use sidebar_width::{animate_sidebar, scaled_sidebar_width, sidebar_animation_target, sidebar_minimum_width};
pub(crate) use launch::{default_save_folder, home_directory};
pub(super) use launch::present_target;
pub use launch::{present, present_reveal};
use launch::{
    bind_update_notice_preferences, browser_for_window, navigate_to_gio_file, open_checked,
    open_default_folder, schedule_after_first_paint,
};
use places::{
    STANDARD_PLACE_IDS, is_standard_place_location, pin_keycap, place_keycap, resolve_place_order,
    should_show_standard_place, sidebar_standard_place_visible, standard_place,
    standard_place_chord_key,
};

#[cfg(test)]
pub(super) use devices::offer_removable_destinations_for_test;
pub(super) use devices::{
    RemovableDestination, RemovableWatch, removable_destination_root, removable_destinations,
    send_to_recent_folders, watch_removable_destinations,
};
pub(in crate::ui) use keyboard::{ChooserKeys, ChooserPolicy, edit_focused_text};

pub use open_argument::present_open;
pub use unlock_argument::{UnlockTarget, present_unlock};

use sidebar::PlaceNavigation;
pub(super) use sidebar::build_sidebar;

const PINNED_DRAG_PREFIX: &str = "pinned:";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecentAvailability {
    platform_tracking_enabled: bool,
    runtime_backend_supported: bool,
}

impl RecentAvailability {
    fn from_runtime() -> Self {
        let platform_tracking_enabled =
            gtk::Settings::default().is_some_and(|settings| settings.is_gtk_recent_files_enabled());
        let runtime_backend_supported = gio::Vfs::default()
            .supported_uri_schemes()
            .iter()
            .any(|scheme| scheme.as_str().eq_ignore_ascii_case("recent"));
        Self {
            platform_tracking_enabled,
            runtime_backend_supported,
        }
    }

    fn is_available(self) -> bool {
        self.platform_tracking_enabled && self.runtime_backend_supported
    }
}

fn should_show_recent_place(show_recent: bool, availability: RecentAvailability) -> bool {
    show_recent && availability.is_available()
}

type RailListener = Rc<dyn Fn(bool)>;
type SidebarReorder = Rc<dyn Fn(&Rc<SidebarState>, &str, bool) -> bool>;

pub(super) struct SidebarState {
    widget: gtk::Box,
    view: BrowserView,
    browser: Rc<Browser>,
    volume_monitor: gio::VolumeMonitor,
    mount_monitor: gio_unix::MountMonitor,
    preference_manager: Rc<super::preferences::PreferenceManager>,
    place_order: RefCell<Vec<&'static str>>,
    places_visibility: RefCell<[bool; 10]>,
    pinned_places: Rc<RefCell<Vec<(Location, String)>>>,
    place_rows: RefCell<Vec<(Location, gtk::Button)>>,
    trash_contents: Cell<TrashContents>,
    trash_menu_rows: RefCell<Option<TrashMenuRows>>,
    trash_monitor: RefCell<Option<crate::services::LoadHandle>>,
    trash_probe_running: Cell<bool>,
    trash_probe_pending: Cell<bool>,
    local_only: bool,
    recent_availability: Cell<RecentAvailability>,
    pending_scroll: Cell<Option<f64>>,
    rebuild_queued: Cell<bool>,
    scroll_restore_queued: Cell<bool>,
    pub(in crate::ui) rail: Cell<bool>,
    policy: RailPolicy,
    pub(in crate::ui) saved_width: Cell<Option<i32>>,
    update_label: gtk::Label,
    rail_changed: RefCell<Option<RailListener>>,
    keycaps: RefCell<Vec<gtk::Label>>,
    keycaps_shown: Cell<bool>,
    visible_pins: RefCell<Vec<Location>>,
    pinned_section: RefCell<Option<PinnedSection>>,
    devices_section: RefCell<Option<CollapsibleSection>>,
    /// The separator, heading and drop gap of a Pinned section with no pins.
    empty_pinned_section: RefCell<Vec<glib::WeakRef<gtk::Widget>>>,
    file_drag_over: Cell<bool>,
    drop_feedback: Rc<sidebar_drop_feedback::SidebarDropFeedback>,
}

/// Pinned rows are split across two revealers with the active row kept between
/// them, so collapsing folds the other rows away around the row the user is in.
struct PinnedSection {
    header: SectionHeader,
    upper_revealer: gtk::Revealer,
    upper_box: gtk::Box,
    lower_revealer: gtk::Revealer,
    lower_box: gtk::Box,
    /// Drop gaps and rows in display order, wherever each is currently parented.
    items: RefCell<Vec<gtk::Widget>>,
}

impl PinnedSection {
    fn collect_items(&self, sidebar: &gtk::Box) {
        let mut items = Vec::new();
        let mut next = self.lower_revealer.next_sibling();
        while let Some(child) = next {
            next = child.next_sibling();
            sidebar.remove(&child);
            self.upper_box.append(&child);
            items.push(child);
        }
        self.items.replace(items);
    }

    fn apply_state(&self, collapsed: bool) {
        for revealer in [&self.upper_revealer, &self.lower_revealer] {
            revealer.set_transition_duration(section_transition_duration());
            revealer.set_reveal_child(!collapsed);
        }
        self.header.apply_state(collapsed);
    }
}

struct CollapsibleSection {
    header: SectionHeader,
    revealer: gtk::Revealer,
    rows: gtk::Box,
}

impl CollapsibleSection {
    /// Returns the row count without the drop gaps between rows.
    fn collect_rows(&self, sidebar: &gtk::Box) -> usize {
        let mut count = 0;
        let mut next = self.revealer.next_sibling();
        while let Some(child) = next {
            next = child.next_sibling();
            sidebar.remove(&child);
            self.rows.append(&child);
            if !sidebar_drop_feedback::is_gap(&child) {
                count += 1;
            }
        }
        count
    }

    fn apply_state(&self, collapsed: bool) {
        self.revealer
            .set_transition_duration(section_transition_duration());
        self.revealer.set_reveal_child(!collapsed);
        self.header.apply_state(collapsed);
    }
}

struct SectionHeader {
    toggle: gtk::Button,
    badge: gtk::Label,
    item_count: Cell<usize>,
}

impl SectionHeader {
    fn set_item_count(&self, count: usize) {
        self.item_count.set(count);
        self.badge.set_text(&count.to_string());
        crate::ui::accessibility::set_description(
            &self.toggle,
            Some(&crate::i18n::count("items", count)),
        );
    }

    fn apply_state(&self, collapsed: bool) {
        if collapsed {
            self.toggle.remove_css_class("expanded");
        } else {
            self.toggle.add_css_class("expanded");
        }
        self.badge.set_visible(collapsed);
        self.toggle
            .update_state(&[gtk::accessible::State::Expanded(Some(!collapsed))]);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarSection {
    Pinned,
    Devices,
}

impl SidebarSection {
    fn title(self) -> &'static str {
        match self {
            SidebarSection::Pinned => "PINNED",
            SidebarSection::Devices => "DEVICES",
        }
    }

    fn is_collapsed(self, manager: &PreferenceManager) -> bool {
        match self {
            SidebarSection::Pinned => manager.sidebar_pinned_collapsed(),
            SidebarSection::Devices => manager.sidebar_devices_collapsed(),
        }
    }

    fn set_collapsed(self, manager: &PreferenceManager, collapsed: bool) {
        match self {
            SidebarSection::Pinned => manager.set_sidebar_pinned_collapsed(collapsed),
            SidebarSection::Devices => manager.set_sidebar_devices_collapsed(collapsed),
        }
    }
}

fn section_transition_duration() -> u32 {
    if animations_enabled() { 200 } else { 0 }
}

/// Rows of the Trash sidebar context menu that only make sense while Trash holds items.
struct TrashMenuRows {
    separator: gtk::Separator,
    empty: gtk::Button,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrashContents {
    /// Not probed yet, or the probe failed.
    Unknown,
    Empty,
    NonEmpty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrashMenuVisibility {
    separator: bool,
    empty: bool,
}

/// Destructive actions stay hidden until Trash is confirmed to hold something, and the
/// separator goes with them so Properties is not left above an empty gap.
fn trash_menu_visibility(contents: TrashContents) -> TrashMenuVisibility {
    let visible = contents == TrashContents::NonEmpty;
    TrashMenuVisibility {
        separator: visible,
        empty: visible,
    }
}

fn sync_trash_menu_rows(rows: &TrashMenuRows, contents: TrashContents) {
    let visibility = trash_menu_visibility(contents);
    rows.separator.set_visible(visibility.separator);
    rows.empty.set_visible(visibility.empty);
}

fn trash_contents_from_probe(probe: Result<bool, glib::Error>) -> TrashContents {
    match probe {
        Ok(false) => TrashContents::Empty,
        Ok(true) => TrashContents::NonEmpty,
        Err(_) => TrashContents::Unknown,
    }
}

fn event_changes_trash_contents(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::DeletionFinished { .. }
            | BrowserEvent::RestorationFinished { .. }
            | BrowserEvent::TransferFinished { .. }
            | BrowserEvent::OperationCompletedWithErrors { .. }
            | BrowserEvent::OperationCancelled { .. }
    )
}

pub(super) struct SidebarView {
    pub(super) widget: gtk::Widget,
    pub(super) state: Rc<SidebarState>,
    update_notice: gtk::Button,
    update_area: gtk::Box,
    update_label: gtk::Label,
    update_version: gtk::Label,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    mount_handler: RefCell<Option<glib::SignalHandlerId>>,
    recent_setting_handler: RefCell<Option<(gtk::Settings, glib::SignalHandlerId)>>,
    #[expect(
        dead_code,
        reason = "held so Drop keeps this window registered for pending-release rebuilds"
    )]
    release_watch: device_release::SidebarWatch,
    bookmark_watch: RefCell<Option<crate::adapters::bookmarks::BookmarkWatch>>,
}

impl SidebarView {
    // Device discovery can block on D-Bus; let the chooser paint before starting it.
    pub(in crate::ui) fn schedule_after_first_paint(&self, window: &impl IsA<gtk::Widget>) {
        let weak_state = Rc::downgrade(&self.state);
        let armed = Rc::new(Cell::new(false));
        let arm = {
            let weak_state = weak_state.clone();
            let armed = armed.clone();
            move |widget: &gtk::Widget| {
                if armed.get() {
                    return;
                }
                armed.set(true);
                let Some(clock) = widget.frame_clock() else {
                    let weak = weak_state.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(state) = weak.upgrade() {
                            state.rebuild();
                        }
                    });
                    return;
                };
                let handler = Rc::new(RefCell::new(None));
                let handler_for_paint = handler.clone();
                let weak = weak_state.clone();
                let id = clock.connect_after_paint(move |clock| {
                    if let Some(id) = handler_for_paint.borrow_mut().take() {
                        clock.disconnect(id);
                    }
                    let weak = weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(state) = weak.upgrade() {
                            state.rebuild();
                        }
                    });
                });
                handler.replace(Some(id));
            }
        };
        if window.is_mapped() {
            arm(window.upcast_ref());
        } else {
            let arm_on_map = arm.clone();
            window.connect_map(move |widget| {
                arm_on_map(widget.upcast_ref());
            });
        }
    }

    pub(super) fn disconnect(&self) {
        unparent_sidebar_popovers(self.state.widget.upcast_ref());
        self.state.trash_monitor.take();
        self.bookmark_watch.take();
        for handler in self.handlers.take() {
            self.state.volume_monitor.disconnect(handler);
        }
        if let Some(handler) = self.mount_handler.take() {
            self.state.mount_monitor.disconnect(handler);
        }
        if let Some((settings, handler)) = self.recent_setting_handler.take() {
            settings.disconnect(handler);
        }
    }
}

fn unparent_sidebar_popovers(widget: &gtk::Widget) {
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if widget.is::<gtk::Button>() && current.is::<gtk::Popover>() {
            current.unparent();
        } else {
            unparent_sidebar_popovers(&current);
        }
    }
}

impl SidebarState {
    fn queue_rebuild(self: &Rc<Self>) {
        self.capture_scroll();
        if self.rebuild_queued.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.rebuild_queued.set(false);
                state.rebuild();
            }
        });
    }

    fn rebuild(self: &Rc<Self>) {
        self.capture_scroll();
        while let Some(child) = self.widget.first_child() {
            unparent_sidebar_popovers(&child);
            self.widget.remove(&child);
        }
        self.drop_feedback.reset();
        self.place_rows.borrow_mut().clear();
        self.keycaps.borrow_mut().clear();
        self.visible_pins.borrow_mut().clear();
        self.pinned_section.replace(None);
        self.devices_section.replace(None);

        self.append_static_places();
        self.append_devices();
        self.sync_active_place();
        self.schedule_scroll_restore();
        self.sync_rail_rows();
    }

    pub(in crate::ui) fn set_rail(&self, rail: bool) {
        let changed = self.rail.replace(rail) != rail;
        if !changed && !rail {
            return;
        }
        if changed && let Some(listener) = self.rail_changed.borrow().clone() {
            listener(rail);
        }
        if let Some(scroller) = self.sidebar_scroller()
            && let Some(shell) = scroller.parent()
        {
            scroller.set_overlay_scrolling(rail);
            if rail {
                shell.add_css_class("sidebar-rail");
            } else {
                shell.remove_css_class("sidebar-rail");
            }
            if let Some(paned) = shell.parent().and_downcast::<gtk::Paned>() {
                paned.set_wide_handle(!rail);
                if let Some(handle) = paned_separator(&paned) {
                    handle.set_cursor_from_name(if rail {
                        Some("default")
                    } else {
                        Some("col-resize")
                    });
                }
            }
            scroller.set_width_request(if rail {
                sidebar_rail_width()
            } else {
                MIN_SIDEBAR_WIDTH
            });
            shell.set_size_request(
                if rail {
                    sidebar_rail_width()
                } else {
                    MIN_SIDEBAR_WIDTH
                },
                -1,
            );
        }
        let text = self.update_label.parent();
        if let Some(text) = &text {
            text.set_visible(!rail);
        }
        if let Some(content) = text
            .and_then(|text| text.parent())
            .and_downcast::<gtk::Box>()
        {
            content.set_spacing(if rail { 0 } else { 8 });
            content.set_halign(if rail {
                gtk::Align::Center
            } else {
                gtk::Align::Fill
            });
            if let Some(dot) = content.first_child() {
                dot.set_visible(!rail);
            }
        }
        self.sync_rail_rows();
    }

    fn sync_rail_rows(&self) {
        let rail = self.rail.get();
        let size = if rail { sidebar_rail_button_size() } else { -1 };
        if let Some(notice) = self
            .update_label
            .ancestor(gtk::Button::static_type())
            .and_downcast::<gtk::Button>()
        {
            notice.set_size_request(if rail { sidebar_rail_width() } else { -1 }, size);
        }
        let mut widget_child = self.widget.first_child();
        while let Some(child) = widget_child {
            widget_child = child.next_sibling();
            self.sync_rail_child(&child);
        }
        self.sync_section_states();
        self.sync_keycaps();
    }

    fn sync_rail_child(&self, child: &gtk::Widget) {
        let rail = self.rail.get();
        if let Ok(heading) = child.clone().downcast::<gtk::Label>() {
            heading.set_visible(!rail);
        } else if let Ok(button) = child.clone().downcast::<gtk::Button>() {
            if button.has_css_class("sidebar-heading-toggle") {
                button.set_visible(!rail);
            } else {
                sync_sidebar_button(&button, rail);
            }
        } else if let Ok(revealer) = child.clone().downcast::<gtk::Revealer>() {
            if let Some(inner) = revealer.child() {
                let mut row_child = inner.first_child();
                while let Some(row) = row_child {
                    row_child = row.next_sibling();
                    self.sync_rail_child(&row);
                }
            }
        } else if child.has_css_class("sidebar-device") {
            let mut dev_child = child.first_child();
            while let Some(w) = dev_child {
                dev_child = w.next_sibling();
                if let Ok(button) = w.clone().downcast::<gtk::Button>() {
                    sync_sidebar_button(&button, rail);
                } else if w.has_css_class("sidebar-device-actions") {
                    w.set_visible(!rail);
                }
            }
        }
    }

    /// Runs after the rail engages or releases, however that was decided.
    pub(in crate::ui) fn connect_rail_changed(&self, listener: impl Fn(bool) + 'static) {
        self.rail_changed.replace(Some(Rc::new(listener)));
    }

    /// The user's choice, from the footer button or the saved preference.
    pub(in crate::ui) fn set_compact(&self, compact: bool) {
        if let Some(rail) = self.policy.set_compact(compact) {
            self.apply_rail(rail);
        }
    }

    /// Feeds the fit rule; the caller applies a returned change.
    pub(in crate::ui) fn observe_fit(&self, fit: SidebarFit) -> Option<bool> {
        self.policy.observe(fit)
    }

    /// Shows the rail or the full sidebar and moves the divider with it.
    pub(in crate::ui) fn apply_rail(&self, rail: bool) {
        self.set_rail(rail);
        let Some(shell) = self
            .sidebar_scroller()
            .and_then(|scroller| scroller.parent())
        else {
            return;
        };
        let Some(paned) = shell.parent().and_downcast::<gtk::Paned>() else {
            return;
        };
        if !shell.is_visible() {
            return;
        }
        let target = if rail {
            sidebar_rail_width()
        } else {
            self.saved_width
                .get()
                .unwrap_or_else(preferred_sidebar_width)
                .max(sidebar_minimum_width(&shell))
        };
        if paned.position() != target {
            paned.set_position(target);
        }
    }
}

fn sync_sidebar_button(button: &gtk::Button, rail: bool) {
    let size = if rail { sidebar_rail_button_size() } else { -1 };
    button.set_size_request(size, size);
    if let Some(content) = button.child() {
        let mut child = content.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                if label.has_css_class("sidebar-keycap") {
                    continue;
                }
                button.set_tooltip_text(rail.then_some(label.label().as_str()));
                if rail {
                    crate::ui::accessibility::set_stand_in_label(button, &label.label());
                } else {
                    crate::ui::accessibility::clear_stand_in_label(button);
                }
                label.set_visible(!rail);
            }
        }
        content.set_halign(if rail {
            gtk::Align::Center
        } else {
            gtk::Align::Fill
        });
    }
}

fn paned_separator(paned: &gtk::Paned) -> Option<gtk::Widget> {
    let mut child = paned.first_child();
    while let Some(widget) = child {
        if widget.css_name() == "separator" {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

impl SidebarState {
    fn sidebar_scroller(&self) -> Option<gtk::ScrolledWindow> {
        self.widget
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast()
    }

    fn capture_scroll(&self) {
        // Removing all rows temporarily clamps the adjustment to zero.
        if self.pending_scroll.get().is_some() {
            return;
        }
        let value = self
            .sidebar_scroller()
            .map(|scroller| scroller.vadjustment().value())
            .unwrap_or(0.0);
        if value > 0.0 {
            self.pending_scroll.set(Some(value));
        }
    }

    fn schedule_scroll_restore(self: &Rc<Self>) {
        if self.pending_scroll.get().is_none() || self.scroll_restore_queued.replace(true) {
            return;
        }
        let Some(scroller) = self.sidebar_scroller() else {
            self.scroll_restore_queued.set(false);
            return;
        };
        let weak = Rc::downgrade(self);
        scroller.add_tick_callback(move |_, _| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    if let Some(position) = state.pending_scroll.take()
                        && let Some(scroller) = state.sidebar_scroller()
                    {
                        scroller.vadjustment().set_value(position);
                    }
                    state.scroll_restore_queued.set(false);
                }
            });
            glib::ControlFlow::Break
        });
    }

    fn append_static_places(self: &Rc<Self>) {
        for place in self.place_order.borrow().clone() {
            self.append_ordered_place(place);
        }
        self.append_pinned_places();
    }

    fn append_ordered_place(self: &Rc<Self>, place: &'static str) {
        match place {
            "home" => {
                if !self.preference_manager.sidebar_show_home() {
                    return;
                }
                let location = Location::local(home_directory());
                let row = self.append_place(
                    crate::assets::icons::HOME,
                    &crate::i18n::tr("Home"),
                    location.clone(),
                );
                self.add_place_keycap(&row, super::keys::bindings::GO_HOME);
                self.attach_place_context_menu(&row, location, |state| {
                    state.preference_manager.set_sidebar_show_home(false);
                });
                self.make_place_reorderable(&row, place);
            }
            "trash" => {
                if !self.local_only && self.preference_manager.sidebar_show_trash() {
                    self.append_trash_place();
                }
            }
            "network" => {
                if self.local_only || !self.preference_manager.sidebar_show_network() {
                    return;
                }
                let location = Location::uri("network:///");
                let row = self.append_place(
                    crate::assets::icons::NETWORK,
                    &crate::i18n::tr("Network"),
                    location.clone(),
                );
                self.add_place_keycap(&row, super::keys::bindings::GO_NETWORK);
                self.attach_place_context_menu_with(
                    &row,
                    location,
                    |state| {
                        state.preference_manager.set_sidebar_show_network(false);
                    },
                    Some(network_location::menu_extra()),
                );
                self.make_place_reorderable(&row, place);
            }
            "recent" => {
                if should_show_recent_place(
                    self.preference_manager.sidebar_show_recent(),
                    self.recent_availability.get(),
                ) {
                    self.append_recent_place();
                }
            }
            _ => self.append_standard_place(place),
        }
    }

    fn standard_place_visible(&self, id: &str) -> bool {
        sidebar_standard_place_visible(&self.preference_manager, id)
    }

    fn append_standard_place(self: &Rc<Self>, place: &'static str) {
        if !self.standard_place_visible(place) {
            return;
        }
        if let Some((icon, name, directory)) = standard_place(place)
            && let Some(path) = glib::user_special_dir(directory)
                .filter(|path| should_show_standard_place(place, path, &home_directory()))
        {
            let row = self.append_reorderable_place(
                place,
                icon,
                &crate::i18n::tr(name),
                Location::local(path),
            );
            if let Some(step) = standard_place_chord_key(place) {
                self.add_place_keycap(&row, step);
            }
        }
    }

    fn append_pinned_places(self: &Rc<Self>) {
        let pinned = self
            .pinned_places
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (location, _))| !is_standard_place_location(location))
            .filter(|(_, (location, _))| !self.local_only || location.native_path().is_some())
            .map(|(index, (location, name))| (index, location.clone(), name.clone()))
            .collect::<Vec<_>>();
        let empty = pinned.is_empty();
        let before_section = self.widget.last_child();
        if self.widget.first_child().is_some() {
            self.append_separator();
        }
        let section = self.append_pinned_section(pinned.len());
        let drag_cache = Rc::new(pinned_drop::FolderDragCache::new(self.local_only));
        let mut previous = None;
        for (ordinal, (index, location, name)) in pinned.into_iter().enumerate() {
            pinned_drop::append_gap(self, previous, Some(location.clone()), &drag_cache);
            previous = Some(location.clone());
            self.visible_pins.borrow_mut().push(location.clone());
            let row = self.append_pinned_place(index, &name, location);
            if let Some(keycap) =
                pin_keycap(ordinal).and_then(|keycap| self.add_keycap(&row, &keycap))
            {
                sync_keycap(&keycap, self.keycaps_shown.get() && !self.rail.get());
            }
        }
        pinned_drop::append_gap(self, previous, None, &drag_cache);
        section.collect_items(&self.widget);
        self.pinned_section.replace(Some(section));
        self.sync_pinned_section_partition();
        let section = if empty {
            let first = match before_section {
                Some(widget) => widget.next_sibling(),
                None => self.widget.first_child(),
            };
            std::iter::successors(first, gtk::Widget::next_sibling)
                .inspect(|widget| widget.set_visible(self.file_drag_over.get()))
                .map(|widget| widget.downgrade())
                .collect()
        } else {
            Vec::new()
        };
        self.empty_pinned_section.replace(section);
    }

    /// With nothing pinned, the Pinned section is only a drop target, so it
    /// shows just while files are dragged over the sidebar.
    pub(super) fn set_file_drag_over(&self, over: bool) {
        self.file_drag_over.set(over);
        for widget in self.empty_pinned_section.borrow().iter() {
            if let Some(widget) = widget.upgrade() {
                widget.set_visible(over);
            }
        }
    }

    fn append_devices(self: &Rc<Self>) {
        let volumes = self.volume_monitor.volumes();
        let mounts = self.mounts_without_volumes(&volumes);
        let password_drives =
            orphaned_password_drives(&volumes, &self.volume_monitor.connected_drives());
        if volumes.is_empty()
            && mounts.is_empty()
            && password_drives.is_empty()
            && !device_release::any_pending()
        {
            return;
        }
        if self.widget.first_child().is_some() {
            self.append_separator();
        }
        let section = self.append_devices_section();
        let mut shown = Vec::new();
        for volume in volumes {
            if let Some(ids) = self.append_volume(volume) {
                shown.push(ids);
            }
        }
        for drive in password_drives {
            if let Some(ids) = self.append_password_drive(drive) {
                shown.push(ids);
            }
        }
        for (name, location, mount) in mounts {
            if let Some(ids) = self.append_mount(&name, location, mount) {
                shown.push(ids);
            }
        }
        for key in device_release::unmatched_pending(&shown) {
            self.append_release_ghost(&key);
        }
        let count = section.collect_rows(&self.widget);
        section.header.set_item_count(count);
        section.apply_state(self.section_collapsed(SidebarSection::Devices));
        self.devices_section.replace(Some(section));
    }

    fn mounts_without_volumes(
        &self,
        volumes: &[gio::Volume],
    ) -> Vec<(String, Location, gio::Mount)> {
        self.volume_monitor
            .mounts()
            .into_iter()
            .filter(|mount| !mount.is_shadowed())
            .filter(|mount| {
                !mount
                    .volume()
                    .is_some_and(|volume| volumes.contains(&volume))
            })
            .filter_map(|mount| {
                let name = mount.name().to_string();
                let location = location_for_file(&mount.root())?;
                if self.local_only && location.native_path().is_none() {
                    return None;
                }
                Some((name, location, mount))
            })
            .collect()
    }

    fn append_mount(
        self: &Rc<Self>,
        name: &str,
        location: Location,
        mount: gio::Mount,
    ) -> Option<device_release::DeviceIds> {
        if is_smb_location(&location) {
            self.append_smb_mount(name, location, mount);
            return None;
        }
        let ids = device_release::ids_for_mount(&mount);
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, name);
        if let Some(id) = drive_dialogs::PropertiesTarget::Mount(mount.clone()).label_id() {
            device_labels::bind_row_label(&row, &self.preference_manager, &id, name);
        }
        crate::ui::accessibility::set_description(&row, Some(&location.display_path()));
        if device_release::is_pending(&ids) {
            self.append_sidebar_row(&device_release::pending_device_shell(&row));
            return Some(ids);
        }
        self.bind_place_row(&row, location, PlaceNavigation::Validate);
        let encrypted = gio_mount_is_encrypted(&mount);
        let actions = device_row_actions(
            encrypted,
            true,
            false,
            mount.can_eject(),
            mount.can_unmount(),
        );
        let in_flight = Rc::new(Cell::new(false));
        let on_crypto = actions.encrypted.map(|_| {
            let crypto_mount = mount.clone();
            let crypto_parent = self.view.widget();
            let crypto_browser = self.browser.clone();
            let crypto_view = self.view.clone();
            let crypto_in_flight = in_flight.clone();
            Rc::new(move || {
                lock_encrypted_mount(
                    &crypto_mount,
                    &crypto_parent,
                    &crypto_browser,
                    &crypto_view,
                    &crypto_in_flight,
                );
            }) as Rc<dyn Fn()>
        });
        let on_release = actions.release.map(|action| {
            let release_mount = mount.clone();
            let release_browser = self.browser.clone();
            let release_parent = self.view.widget();
            let release_view = self.view.clone();
            let release_in_flight = in_flight.clone();
            Rc::new(move || {
                release_device_mount(
                    &release_mount,
                    action,
                    &release_parent,
                    &release_browser,
                    &release_view,
                    &release_in_flight,
                );
            }) as Rc<dyn Fn()>
        });
        self.append_device_chrome(
            &row,
            actions,
            on_crypto,
            on_release,
            None,
            Some(drive_dialogs::PropertiesTarget::Mount(mount)),
        );
        Some(ids)
    }

    fn pin_location(self: &Rc<Self>, location: Location, name: String) {
        self.pin_location_with(location, name, |_| {});
    }

    fn pin_location_with(
        self: &Rc<Self>,
        location: Location,
        name: String,
        done: impl FnOnce(bool) + 'static,
    ) {
        let expected = location.clone();
        let weak = Rc::downgrade(self);
        self.update_pinned_places(
            move |places| {
                if pin_status(places, &location) != PinStatus::Available {
                    return false;
                }
                places.push((location.clone(), name.clone()));
                true
            },
            move |_| {
                done(weak.upgrade().is_some_and(|state| {
                    pin_status(&state.pinned_places.borrow(), &expected) == PinStatus::Pinned
                }))
            },
        );
    }

    fn unpin_location(self: &Rc<Self>, location: &Location) {
        let location = location.clone();
        self.update_pinned_places(move |places| remove_pinned_place(places, &location), |_| {});
    }

    fn pin_folders_into(
        self: &Rc<Self>,
        folders: &[(Location, String)],
        (after, before): PinGap<'_>,
        done: impl FnOnce(bool) + 'static,
    ) {
        let folders = folders.to_vec();
        let (after, before) = (after.cloned(), before.cloned());
        self.update_pinned_places(
            move |places| {
                pin_gap_index(places, (after.as_ref(), before.as_ref()))
                    .is_some_and(|index| pin_locations_at(places, &folders, index))
            },
            done,
        );
    }

    fn reorder_pinned_into(
        self: &Rc<Self>,
        source: &Location,
        (after, before): PinGap<'_>,
        done: impl FnOnce(bool) + 'static,
    ) {
        let source = source.clone();
        let (after, before) = (after.cloned(), before.cloned());
        self.update_pinned_places(
            move |places| {
                let Some(source) = places.iter().position(|(location, _)| *location == source)
                else {
                    return false;
                };
                pin_gap_index(places, (after.as_ref(), before.as_ref()))
                    .is_some_and(|index| move_pinned_place(places, source, index))
            },
            done,
        );
    }

    fn update_pinned_places(
        self: &Rc<Self>,
        change: impl Fn(&mut Vec<(Location, String)>) -> bool + Send + 'static,
        done: impl FnOnce(bool) + 'static,
    ) {
        // Merge into the shared file, never this window's stale snapshot.
        let edit = move |contents: &[u8]| {
            let mut places = crate::adapters::bookmarks::parse_pinned_places(contents);
            change(&mut places)
                .then(|| crate::adapters::bookmarks::serialize_pinned_places(&places).into_bytes())
        };
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            match crate::adapters::bookmarks::update(edit).await {
                Ok(changed) => done(changed),
                Err(error) => {
                    if let Some(state) = weak.upgrade() {
                        show_error_dialog(
                            &state.view.widget(),
                            &crate::i18n::tr("Unable to update pinned folders"),
                            &crate::services::io_error_message(&error),
                        );
                    }
                    done(false);
                }
            }
        });
    }
    fn event_changes_active_place(event: &BrowserEvent) -> bool {
        matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnsRelocated { .. }
                | BrowserEvent::FocusChanged { .. }
                // A clicked place is highlighted before validation; a rejection
                // leaves the previous location open, including while a mount runs.
                | BrowserEvent::LocationNavigationRejected { .. }
        )
    }

    fn sync_active_place(&self) {
        let active = self.browser.active_location();
        let rows = self.place_rows.borrow();
        let selected = rows
            .iter()
            .position(|(location, row)| {
                active.as_ref() == Some(location) && row.has_css_class("active")
            })
            .or_else(|| {
                rows.iter()
                    .position(|(location, _)| active.as_ref() == Some(location))
            });
        for (index, (_, row)) in rows.iter().enumerate() {
            set_sidebar_row_active(row, selected == Some(index));
        }
        drop(rows);
        self.sync_pinned_section_partition();
    }

    #[cfg(test)]
    pub(super) fn places_for_test(&self) -> gtk::Box {
        self.widget.clone()
    }

    pub(super) fn focus_active_place(&self) -> bool {
        let rows = self.place_rows.borrow();
        rows.iter()
            .find(|(_, row)| row.has_css_class("active"))
            .or_else(|| rows.first())
            .is_some_and(|(_, row)| row.grab_focus())
    }

    fn apply_trash_menu_visibility(&self) {
        if let Some(rows) = self.trash_menu_rows.borrow().as_ref() {
            sync_trash_menu_rows(rows, self.trash_contents.get());
        }
    }

    fn set_trash_contents(&self, contents: TrashContents) {
        if self.trash_contents.get() == contents {
            return;
        }
        self.trash_contents.set(contents);
        self.apply_trash_menu_visibility();
    }

    fn refresh_trash_contents(self: &Rc<Self>) {
        if self.local_only {
            return;
        }
        // A probe already in flight may have read Trash before this change landed, so queue
        // another pass instead of trusting the result it is about to return.
        if self.trash_probe_running.get() {
            self.trash_probe_pending.set(true);
            return;
        }
        self.trash_probe_running.set(true);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let probe = trash_has_items(&gio::File::for_uri("trash:///")).await;
            if let Err(error) = &probe {
                tracing::warn!(
                    error_domain = ?error.domain(),
                    error_code = error.code(),
                    "unable to read trash contents"
                );
            }
            if let Some(state) = weak.upgrade() {
                state.trash_probe_running.set(false);
                state.set_trash_contents(trash_contents_from_probe(probe));
                if state.trash_probe_pending.replace(false) {
                    state.refresh_trash_contents();
                }
            }
        });
    }

    /// Keeps the context menu current when another application changes `trash:///`.
    fn watch_trash(self: &Rc<Self>) {
        if self.trash_monitor.borrow().is_some() {
            return;
        }
        let (changed, live) = (Rc::downgrade(self), Rc::downgrade(self));
        let monitor = crate::adapters::watch(
            &Location::uri("trash:///"),
            crate::adapters::Watched::Folder,
            move |_, _| {
                if let Some(state) = changed.upgrade() {
                    state.refresh_trash_contents();
                }
            },
            move || {
                if let Some(state) = live.upgrade() {
                    state.refresh_trash_contents();
                }
            },
        );
        *self.trash_monitor.borrow_mut() = Some(monitor);
    }

    fn append_recent_place(self: &Rc<Self>) {
        let location = Location::uri("recent:///");
        let row = sidebar_button(crate::assets::icons::CLOCK, &crate::i18n::tr("Recent"));
        crate::ui::accessibility::set_description(&row, Some("recent:///"));
        self.add_place_keycap(&row, super::keys::bindings::GO_RECENT);
        self.bind_place_row(&row, location.clone(), PlaceNavigation::Direct);
        self.attach_place_context_menu(&row, location, |state| {
            state.preference_manager.set_sidebar_show_recent(false);
        });
        self.append_sidebar_row(&row);
        self.make_place_reorderable(&row, "recent");
    }

    fn append_trash_place(self: &Rc<Self>) {
        let location = Location::uri("trash:///");
        let row = sidebar_button(crate::assets::icons::TRASH, &crate::i18n::tr("Trash"));
        crate::ui::accessibility::set_description(&row, Some("trash:///"));
        self.add_place_keycap(&row, super::keys::bindings::GO_TRASH);
        self.bind_place_row(&row, location, PlaceNavigation::Direct);

        let menu = super::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        let unpin = sidebar_context_option(crate::assets::icons::PIN, "Unpin", false);
        menu.append(&unpin);
        let empty = sidebar_context_option(crate::assets::icons::TRASH, "Empty Trash…", true);
        empty.add_css_class("danger");
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        menu.append(&properties);
        menu.append(&separator);
        menu.append(&empty);
        *self.trash_menu_rows.borrow_mut() = Some(TrashMenuRows {
            separator,
            empty: empty.clone(),
        });
        self.apply_trash_menu_visibility();
        self.watch_trash();
        self.refresh_trash_contents();
        self.view.set_trash_button(row.clone());
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);
        sidebar::append_open_in_tab(
            &menu,
            &popover,
            &row,
            &self.view,
            &Location::uri("trash:///"),
        );
        let properties_popover = popover.downgrade();
        let properties_view = self.view.downgrade();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            if let Some(view) = properties_view.upgrade() {
                view.show_location_properties(&Location::uri("trash:///"));
            }
        });
        let unpin_popover = popover.downgrade();
        let weak_state = Rc::downgrade(self);
        unpin.connect_clicked(move |_| {
            if let Some(popover) = unpin_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_state.upgrade() {
                state.preference_manager.set_sidebar_show_trash(false);
            }
        });
        let empty_popover = popover.downgrade();
        let empty_view = self.view.downgrade();
        empty.connect_clicked(move |_| {
            if let Some(popover) = empty_popover.upgrade() {
                popover.popdown();
            }
            if let Some(view) = empty_view.upgrade() {
                view.confirm_empty_trash();
            }
        });
        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        let weak_state = Rc::downgrade(self);
        popover.connect_show(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.refresh_trash_contents();
            }
        });
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
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
        row.add_controller(context);
        self.append_sidebar_row(&row);
        self.make_place_reorderable(&row, "trash");
    }

    fn make_reorderable(
        self: &Rc<Self>,
        row: &gtk::Button,
        payload: String,
        on_drop: impl Fn(&Rc<Self>, &str, bool) -> bool + 'static,
    ) {
        row.add_css_class("reorderable");
        row.set_cursor_from_name(Some("pointer"));

        let pinned = parse_pinned_drag_source(&payload);
        let folder = pinned
            .and_then(|index| self.pinned_places.borrow().get(index).cloned())
            .or_else(|| {
                self.place_rows
                    .borrow()
                    .iter()
                    .find(|(_, candidate)| candidate == row)
                    .map(|(location, _)| (location.clone(), location.display_path()))
            });
        if let Some(folder) = folder {
            sidebar::install_drag(
                row,
                folder,
                payload.clone(),
                pinned.is_some(),
                self.drop_feedback.clone(),
            );
        }

        let on_drop: SidebarReorder = Rc::new(on_drop);
        self.install_reorder_target(row, payload.clone(), None, on_drop.clone());
        if parse_pinned_drag_source(&payload).is_none() {
            for after in [false, true] {
                if let Some(gap) = self.insert_reorder_gap(row.upcast_ref(), after) {
                    self.install_reorder_target(
                        &gap,
                        payload.clone(),
                        Some(after),
                        on_drop.clone(),
                    );
                }
            }
        }
    }

    fn insert_reorder_gap(&self, row: &gtk::Widget, after: bool) -> Option<gtk::Box> {
        let neighbour = if after {
            row.next_sibling()
        } else {
            row.prev_sibling()
        };
        if neighbour
            .as_ref()
            .is_some_and(sidebar_drop_feedback::is_gap)
        {
            return None;
        }
        let gap = sidebar_drop_feedback::row_gap(None);
        let previous = if after { Some(row.clone()) } else { neighbour };
        self.widget.insert_child_after(&gap, previous.as_ref());
        Some(gap)
    }

    fn install_reorder_target(
        self: &Rc<Self>,
        widget: &impl IsA<gtk::Widget>,
        payload: String,
        fixed_after: Option<bool>,
        on_drop: SidebarReorder,
    ) {
        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        drop.set_preload(true);
        drop.connect_accept(|_, offered| {
            accepts_sidebar_reorder_payload(
                offered.formats().contains_type(String::static_type()),
                offered
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type()),
            )
        });
        let hover_y = Rc::new(Cell::new(0.0));
        let enter_y = hover_y.clone();
        let enter_payload = payload.clone();
        let enter_feedback = self.drop_feedback.clone();
        drop.connect_enter(move |target, _, y| {
            enter_y.set(y);
            if let Some(row) = target.widget() {
                enter_feedback.enter(&row);
            }
            sidebar_reorder_feedback(target, &enter_payload, y, &enter_feedback)
        });
        let motion_y = hover_y.clone();
        let motion_payload = payload.clone();
        let motion_feedback = self.drop_feedback.clone();
        drop.connect_motion(move |target, _, y| {
            motion_y.set(y);
            sidebar_reorder_feedback(target, &motion_payload, y, &motion_feedback)
        });
        let value_payload = payload.clone();
        let value_feedback = self.drop_feedback.clone();
        drop.connect_value_notify(move |target| {
            if let Some(offered) = target.current_drop()
                && target
                    .widget()
                    .is_some_and(|row| value_feedback.is_current(&row))
            {
                let action = sidebar_reorder_feedback(
                    target,
                    &value_payload,
                    hover_y.get(),
                    &value_feedback,
                );
                offered.status(action, action);
            }
        });
        let leave_feedback = self.drop_feedback.clone();
        drop.connect_leave(move |target| {
            if let Some(row) = target.widget() {
                leave_feedback.leave(&row);
            }
        });
        let weak_state = Rc::downgrade(self);
        let drop_feedback = self.drop_feedback.clone();
        drop.connect_drop(move |target, value, _, y| {
            let Some(row) = target.widget() else {
                return false;
            };
            drop_feedback.leave(&row);
            let Ok(source) = value.get::<String>() else {
                return false;
            };
            if source == payload && fixed_after.is_some() {
                return true;
            }
            if !sidebar_reorder_matches(&source, &payload) {
                return false;
            }
            let after = fixed_after.unwrap_or_else(|| y >= f64::from(row.height()) / 2.0);
            if let Some(state) = weak_state.upgrade() {
                return on_drop(&state, &source, after);
            }
            false
        });
        widget.add_controller(drop);
    }

    fn append_reorderable_place(
        self: &Rc<Self>,
        id: &'static str,
        icon: &str,
        name: &str,
        location: Location,
    ) -> gtk::Button {
        let row = sidebar_button(icon, name);
        customize_sidebar_folder_icon(&row, &location, icon);
        crate::ui::accessibility::set_description(&row, Some(&location.display_path()));
        self.bind_place_row(&row, location.clone(), PlaceNavigation::Direct);
        self.attach_place_context_menu(&row, location, move |state| {
            let manager = &state.preference_manager;
            match id {
                "desktop" => manager.set_sidebar_show_desktop(false),
                "documents" => manager.set_sidebar_show_documents(false),
                "downloads" => manager.set_sidebar_show_downloads(false),
                "music" => manager.set_sidebar_show_music(false),
                "pictures" => manager.set_sidebar_show_pictures(false),
                "videos" => manager.set_sidebar_show_videos(false),
                _ => {}
            }
        });

        self.append_sidebar_row(&row);
        self.make_place_reorderable(&row, id);
        row
    }

    fn make_place_reorderable(self: &Rc<Self>, row: &gtk::Button, id: &'static str) {
        self.make_reorderable(row, id.to_owned(), move |state, source, after| {
            if source.starts_with(PINNED_DRAG_PREFIX) {
                return false;
            }
            state.reorder_place(source, id, after);
            true
        });
    }

    fn reorder_place(self: &Rc<Self>, source: &str, target: &str, after: bool) {
        let changed = reorder_places(&mut self.place_order.borrow_mut(), source, target, after);
        if changed {
            let order = self
                .place_order
                .borrow()
                .iter()
                .map(|place| (*place).to_owned())
                .collect();
            self.preference_manager.set_sidebar_order(order);
            self.rebuild();
        }
    }

    fn reorder_pinned_place(self: &Rc<Self>, source: usize, target: usize, after: bool) {
        let (source, target) = {
            let places = self.pinned_places.borrow();
            match (places.get(source), places.get(target)) {
                (Some(source), Some(target)) => (source.0.clone(), target.0.clone()),
                _ => return,
            }
        };
        self.update_pinned_places(
            move |places| {
                let position =
                    |location: &Location| places.iter().position(|(pinned, _)| pinned == location);
                match (position(&source), position(&target)) {
                    (Some(source), Some(target)) => {
                        reorder_pinned_places(places, source, target, after)
                    }
                    _ => false,
                }
            },
            |_| {},
        );
    }

    fn append_separator(&self) {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("sidebar-separator");
        self.widget.append(&separator);
    }

    fn append_section_header(self: &Rc<Self>, section: SidebarSection) -> SectionHeader {
        let title = crate::i18n::tr(section.title());
        let chevron = crate::assets::primary_icon(crate::assets::icons::CHEVRON_RIGHT, 14);
        let label = gtk::Label::new(Some(&title));
        label.add_css_class("sidebar-heading");
        label.set_xalign(0.0);
        label.set_hexpand(true);
        let badge = gtk::Label::new(None);
        badge.add_css_class("sidebar-heading-count");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        content.append(&chevron);
        content.append(&label);
        content.append(&badge);
        let toggle = gtk::Button::builder()
            .child(&content)
            .has_frame(false)
            .build();
        toggle.add_css_class("sidebar-heading-toggle");
        toggle.set_cursor_from_name(Some("pointer"));
        toggle.set_visible(!self.rail.get());
        crate::ui::accessibility::set_label(&toggle, &title);
        let weak = Rc::downgrade(self);
        toggle.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let manager = &state.preference_manager;
                section.set_collapsed(manager, !section.is_collapsed(manager));
            }
        });
        self.widget.append(&toggle);
        SectionHeader {
            toggle,
            badge,
            item_count: Cell::new(0),
        }
    }

    fn append_section_revealer(&self, collapsed: bool) -> (gtk::Revealer, gtk::Box) {
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(section_transition_duration())
            .reveal_child(!collapsed)
            .child(&rows)
            .build();
        revealer.add_css_class("sidebar-section-revealer");
        self.widget.append(&revealer);
        (revealer, rows)
    }

    fn append_pinned_section(self: &Rc<Self>, item_count: usize) -> PinnedSection {
        let header = self.append_section_header(SidebarSection::Pinned);
        header.set_item_count(item_count);
        let collapsed = item_count > 0 && self.section_collapsed(SidebarSection::Pinned);
        header.apply_state(collapsed);
        let (upper_revealer, upper_box) = self.append_section_revealer(collapsed);
        let (lower_revealer, lower_box) = self.append_section_revealer(collapsed);
        PinnedSection {
            header,
            upper_revealer,
            upper_box,
            lower_revealer,
            lower_box,
            items: RefCell::new(Vec::new()),
        }
    }

    fn append_devices_section(self: &Rc<Self>) -> CollapsibleSection {
        let header = self.append_section_header(SidebarSection::Devices);
        let (revealer, rows) =
            self.append_section_revealer(self.section_collapsed(SidebarSection::Devices));
        CollapsibleSection {
            header,
            revealer,
            rows,
        }
    }

    /// The icon rail hides the section headers, so it shows every section.
    fn section_collapsed(&self, section: SidebarSection) -> bool {
        !self.rail.get() && section.is_collapsed(&self.preference_manager)
    }

    fn sync_section_states(&self) {
        if let Some(section) = self.pinned_section.borrow().as_ref() {
            let collapsed = section.header.item_count.get() > 0
                && self.section_collapsed(SidebarSection::Pinned);
            section.apply_state(collapsed);
        }
        if let Some(section) = self.devices_section.borrow().as_ref() {
            section.apply_state(self.section_collapsed(SidebarSection::Devices));
        }
    }

    /// Keeps the active pinned row between the two revealers, with the gaps
    /// and rows before it in the upper one and the rest in the lower one.
    fn sync_pinned_section_partition(&self) {
        let guard = self.pinned_section.borrow();
        let Some(section) = guard.as_ref() else {
            return;
        };
        let items = section.items.borrow();
        let active = self.browser.active_location();
        let active_index = active.and_then(|active| {
            let rows = self.place_rows.borrow();
            items.iter().position(|item| {
                rows.iter().any(|(location, row)| {
                    *location == active && row.upcast_ref::<gtk::Widget>() == item
                })
            })
        });
        let (upper, detached, lower) = match active_index {
            Some(index) => (&items[..index], items.get(index), &items[index + 1..]),
            None => (&items[..], None, &items[items.len()..]),
        };
        reparent_in_order(&section.upper_box, upper, None);
        reparent_in_order(&section.lower_box, lower, None);
        if let Some(row) = detached {
            reparent_in_order(
                &self.widget,
                std::slice::from_ref(row),
                Some(section.upper_revealer.upcast_ref()),
            );
        }
    }

    fn append_volume(self: &Rc<Self>, volume: gio::Volume) -> Option<device_release::DeviceIds> {
        let name = volume.name().to_string();
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &name);
        if let Some(id) = drive_dialogs::PropertiesTarget::Volume(volume.clone()).label_id() {
            device_labels::bind_row_label(&row, &self.preference_manager, &id, &name);
        }
        crate::ui::accessibility::set_description(&row, Some(&name));
        let mounted_location = volume
            .get_mount()
            .as_ref()
            .and_then(|mount| location_for_file(&mount.root()));
        if let Some(location) = mounted_location.as_ref() {
            if self.local_only && location.native_path().is_none() {
                return None;
            }
        } else if self.local_only
            && (gio_volume_unix_device(&volume).is_none()
                || volume
                    .activation_root()
                    .is_some_and(|root| root.path().is_none()))
        {
            return None;
        }
        let ids = device_release::ids_for_volume(&volume);
        if device_release::is_pending(&ids) {
            self.append_sidebar_row(&device_release::pending_device_shell(&row));
            return Some(ids);
        }
        if let Some(location) = mounted_location {
            sidebar::install_open_in_tab(&row, &self.view, &location, &self.drop_feedback);
            self.place_rows
                .borrow_mut()
                .push((location.clone(), row.clone()));
            install_sidebar_file_drop(self, &row, location);
        }
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let clicked_volume = volume.clone();
        let mount_view = self.view.clone();
        row.connect_clicked(move |_| {
            let volume = clicked_volume.clone();
            select_sidebar_row(&sidebar, &selected_row);
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if let Some(mount) = volume.get_mount() {
                navigate_to_gio_file(&browser, &mount.root());
                return;
            }

            mount_view.mount_volume(volume);
        });
        let (mount_can_eject, mount_can_unmount) = volume
            .get_mount()
            .map(|mount| (mount.can_eject(), mount.can_unmount()))
            .unwrap_or((false, false));
        let encrypted = gio_volume_is_encrypted(&volume);
        let mounted = !gio_volume_is_locked(&volume);
        let actions = device_row_actions(
            encrypted,
            mounted,
            volume.can_eject(),
            mount_can_eject,
            mount_can_unmount,
        );
        let in_flight = Rc::new(Cell::new(false));
        let on_release = actions.release.map(|action| {
            let release_volume = volume.clone();
            let release_browser = self.browser.clone();
            let release_parent = self.view.widget();
            let release_view = self.view.clone();
            let release_in_flight = in_flight.clone();
            Rc::new(move || {
                release_device_volume(
                    &release_volume,
                    action,
                    &release_parent,
                    &release_browser,
                    &release_view,
                    &release_in_flight,
                );
            }) as Rc<dyn Fn()>
        });
        let on_crypto = actions.encrypted.map(|action| {
            let crypto_volume = volume.clone();
            let crypto_view = self.view.clone();
            let crypto_parent = self.view.widget();
            let crypto_browser = self.browser.clone();
            let crypto_in_flight = in_flight.clone();
            Rc::new(move || match action {
                EncryptedMediaAction::Unlock => crypto_view.unlock_volume(crypto_volume.clone()),
                EncryptedMediaAction::Lock => lock_encrypted_volume(
                    &crypto_volume,
                    &crypto_parent,
                    &crypto_browser,
                    &crypto_view,
                    &crypto_in_flight,
                ),
            }) as Rc<dyn Fn()>
        });
        let on_mount = {
            let mount_view = self.view.clone();
            let mount_volume = volume.clone();
            Rc::new(move || mount_view.mount_volume(mount_volume.clone())) as Rc<dyn Fn()>
        };
        self.append_device_chrome(
            &row,
            actions,
            on_crypto,
            on_release,
            Some(on_mount),
            Some(drive_dialogs::PropertiesTarget::Volume(volume)),
        );
        Some(ids)
    }

    fn append_release_ghost(&self, key: &device_release::ReleaseKey) {
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &key.display_name);
        self.append_sidebar_row(&device_release::pending_device_shell(&row));
    }

    fn append_password_drive(&self, drive: gio::Drive) -> Option<device_release::DeviceIds> {
        let ids = device_release::ids_for_drive(&drive);
        let name = drive.name().to_string();
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &name);
        crate::ui::accessibility::set_description(&row, Some(&name));
        if device_release::is_pending(&ids) {
            self.append_sidebar_row(&device_release::pending_device_shell(&row));
            return Some(ids);
        }
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let clicked_drive = drive.clone();
        let mount_view = self.view.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            mount_view.start_password_drive(clicked_drive.clone(), true);
        });
        let actions = device_row_actions(true, false, drive.can_eject(), false, false);
        let on_crypto = Some({
            let drive = drive.clone();
            let view = self.view.clone();
            Rc::new(move || view.start_password_drive(drive.clone(), true)) as Rc<dyn Fn()>
        });
        let on_release = actions.release.map(|action| {
            let release_drive = drive.clone();
            let release_parent = self.view.widget();
            let release_view = self.view.clone();
            let release_in_flight = Rc::new(Cell::new(false));
            Rc::new(move || {
                eject_password_drive(
                    &release_drive,
                    action,
                    &release_parent,
                    &release_view,
                    &release_in_flight,
                );
            }) as Rc<dyn Fn()>
        });
        self.append_device_chrome(&row, actions, on_crypto, on_release, None, None);
        Some(ids)
    }

    fn append_smb_mount(self: &Rc<Self>, name: &str, location: Location, mount: gio::Mount) {
        let properties_location = location.clone();
        let row = self.append_place(crate::assets::icons::NETWORK, name, location);
        let menu = super::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        let disconnect = sidebar_context_option(crate::assets::icons::UNPLUG, "Disconnect", true);
        disconnect.add_css_class("danger");
        menu.append(&properties);
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu.append(&disconnect);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);
        sidebar::append_open_in_tab(&menu, &popover, &row, &self.view, &properties_location);

        let properties_popover = popover.downgrade();
        let properties_view = self.view.downgrade();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            if let Some(view) = properties_view.upgrade() {
                view.show_location_properties(&properties_location);
            }
        });

        let disconnect_popover = popover.downgrade();
        let parent = self.view.widget();
        disconnect.connect_clicked(move |_| {
            if let Some(popover) = disconnect_popover.upgrade() {
                popover.popdown();
            }
            let window = parent.root().and_downcast::<gtk::Window>();
            let operation = gtk::MountOperation::new(window.as_ref());
            let mount = mount.clone();
            let error_parent = parent.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Err(error) = mount
                    .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                    .await
                    && !error.matches(gio::IOErrorEnum::Cancelled)
                {
                    show_error_dialog(
                        &error_parent,
                        &crate::i18n::tr("Unable to disconnect"),
                        &error.to_string(),
                    );
                }
            });
        });

        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
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
        row.add_controller(context);
    }

    fn append_pinned_place(
        self: &Rc<Self>,
        index: usize,
        name: &str,
        location: Location,
    ) -> gtk::Button {
        let row = self.append_place(crate::assets::icons::FOLDER, name, location.clone());
        row.add_css_class("sidebar-pinned-row");
        self.make_pinned_row_reorderable(&row, index);
        let unpinned_location = location.clone();
        self.attach_place_context_menu(&row, location, move |state| {
            state.unpin_location(&unpinned_location);
        });
        row
    }

    /// Keycaps name the second key of the **g** place chord while it is armed.
    /// They stand in for the row icon so long place names are not truncated further.
    fn add_keycap(&self, row: &gtk::Button, key: &str) -> Option<gtk::Label> {
        let content = row.child().and_downcast::<gtk::Box>()?;
        let keycap = gtk::Label::new(Some(key));
        keycap.add_css_class("sidebar-keycap");
        keycap.set_valign(gtk::Align::Center);
        content.insert_child_after(&keycap, content.first_child().as_ref());
        self.keycaps.borrow_mut().push(keycap.clone());
        Some(keycap)
    }

    fn add_place_keycap(&self, row: &gtk::Button, step: super::keys::Command) {
        if let Some(keycap) = self.add_keycap(row, "") {
            super::shortcut_reference::follow_keys(&keycap, move |keycap| {
                keycap.set_text(&place_keycap(step));
            });
            sync_keycap(&keycap, self.keycaps_shown.get() && !self.rail.get());
        }
    }

    pub(in crate::ui) fn show_place_keycaps(&self, shown: bool) {
        self.keycaps_shown.set(shown);
        self.sync_keycaps();
    }

    fn sync_keycaps(&self) {
        let visible = self.keycaps_shown.get() && !self.rail.get();
        for keycap in self.keycaps.borrow().iter() {
            sync_keycap(keycap, visible);
        }
    }

    /// Pinned places in sidebar display order, excluding rows the sidebar hides.
    pub(in crate::ui) fn visible_pins(&self) -> Vec<Location> {
        self.visible_pins.borrow().clone()
    }

    fn attach_place_context_menu(
        self: &Rc<Self>,
        row: &gtk::Button,
        location: Location,
        on_unpin: impl Fn(&Rc<Self>) + 'static,
    ) {
        self.attach_place_context_menu_with(row, location, on_unpin, None);
    }

    /// Like [`Self::attach_place_context_menu`], with `extra` above the standard items.
    fn attach_place_context_menu_with(
        self: &Rc<Self>,
        row: &gtk::Button,
        location: Location,
        on_unpin: impl Fn(&Rc<Self>) + 'static,
        extra: Option<PlaceMenuExtra>,
    ) {
        let menu = super::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let extra_item = extra.map(|extra| {
            let button = sidebar_context_option(extra.icon, extra.label, false);
            menu.append(&button);
            (button, extra.action)
        });
        let unpin = sidebar_context_option(crate::assets::icons::PIN, "Unpin", false);
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        menu.append(&unpin);
        let customize = location.native_path().map(|path| {
            let button = sidebar_context_option(crate::assets::icons::PALETTE, "Customize…", false);
            menu.append(&button);
            (button, path.to_path_buf())
        });
        menu.append(&properties);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(row);
        sidebar::append_open_in_tab(&menu, &popover, row, &self.view, &location);

        if let Some((button, action)) = extra_item {
            let popover = popover.downgrade();
            let weak_state = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                if let Some(state) = weak_state.upgrade() {
                    action(&state);
                }
            });
        }

        if let Some((customize, path)) = customize {
            let weak_popover = popover.downgrade();
            let weak_row = row.downgrade();
            customize.connect_clicked(move |_| {
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
                if let Some(row) = weak_row.upgrade() {
                    super::browser::show_customize_modal(
                        &row,
                        path.clone(),
                        true,
                        crate::assets::icons::FOLDER,
                    );
                }
            });
        }

        let weak_state = Rc::downgrade(self);
        let unpin_popover = popover.downgrade();
        unpin.connect_clicked(move |_| {
            if let Some(popover) = unpin_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_state.upgrade() {
                on_unpin(&state);
            }
        });
        let properties_view = self.view.downgrade();
        let properties_location = location;
        let properties_popover = popover.downgrade();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            if let Some(view) = properties_view.upgrade() {
                view.show_location_properties(&properties_location);
            }
        });
        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
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
        row.add_controller(context);
    }

    fn make_pinned_row_reorderable(self: &Rc<Self>, row: &gtk::Button, index: usize) {
        self.make_reorderable(
            row,
            format!("{PINNED_DRAG_PREFIX}{index}"),
            move |state, source, after| {
                let Some(source) = parse_pinned_drag_source(source) else {
                    return false;
                };
                state.reorder_pinned_place(source, index, after);
                true
            },
        );
    }

    fn append_sidebar_row(&self, row: &impl IsA<gtk::Widget>) {
        if self.widget.last_child().is_some_and(|previous| {
            previous.has_css_class("sidebar-row")
                || previous.has_css_class("sidebar-device")
                || previous.has_css_class("sidebar-heading")
        }) {
            self.widget.append(&sidebar_drop_feedback::row_gap(None));
        }
        self.widget.append(row);
    }

    fn append_place(&self, icon: &str, name: &str, location: Location) -> gtk::Button {
        let row = self.append_device_place(icon, name, location.clone(), None);
        customize_sidebar_folder_icon(&row, &location, icon);
        row
    }

    fn append_device_place(
        &self,
        icon: &str,
        name: &str,
        location: Location,
        release: Option<(MediaRelease, Rc<dyn Fn()>)>,
    ) -> gtk::Button {
        let row = sidebar_button(icon, name);
        crate::ui::accessibility::set_description(&row, Some(&location.display_path()));
        self.bind_place_row(&row, location, PlaceNavigation::Validate);
        match release {
            Some((action, on_release)) => {
                let eject = sidebar_eject_button(action, {
                    let on_release = on_release.clone();
                    move || on_release()
                });
                attach_device_actions_menu(
                    &row,
                    &self.view,
                    DeviceRowActions {
                        encrypted: None,
                        release: Some(action),
                    },
                    None,
                    Some(on_release),
                    None,
                    None,
                );
                self.append_sidebar_row(&sidebar_device_row(&row, None, Some(&eject)));
            }
            None => {
                self.append_sidebar_row(&row);
            }
        }
        row
    }

    fn append_device_chrome(
        &self,
        row: &gtk::Button,
        actions: DeviceRowActions,
        on_crypto: Option<Rc<dyn Fn()>>,
        on_release: Option<Rc<dyn Fn()>>,
        on_mount: Option<Rc<dyn Fn()>>,
        properties: Option<drive_dialogs::PropertiesTarget>,
    ) {
        let lock = actions
            .encrypted
            .zip(on_crypto.clone())
            .map(|(action, on_crypto)| sidebar_lock_button(action, move || on_crypto()));
        let eject = actions
            .release
            .zip(on_release.clone())
            .map(|(action, on_release)| sidebar_eject_button(action, move || on_release()));
        if actions.encrypted.is_some() || actions.release.is_some() || properties.is_some() {
            attach_device_actions_menu(
                row, &self.view, actions, on_crypto, on_release, on_mount, properties,
            );
            self.append_sidebar_row(&sidebar_device_row(row, lock.as_ref(), eject.as_ref()));
        } else {
            self.append_sidebar_row(row);
        }
    }
}

fn accepts_sidebar_reorder_payload(has_string: bool, has_file_list: bool) -> bool {
    has_string && !has_file_list
}

fn sidebar_reorder_matches(source: &str, target: &str) -> bool {
    source != target
        && if parse_pinned_drag_source(target).is_some() {
            parse_pinned_drag_source(source).is_some()
        } else {
            STANDARD_PLACE_IDS.contains(&source) && STANDARD_PLACE_IDS.contains(&target)
        }
}

fn sidebar_reorder_feedback(
    target: &gtk::DropTarget,
    payload: &str,
    y: f64,
    feedback: &sidebar_drop_feedback::SidebarDropFeedback,
) -> gtk::gdk::DragAction {
    let Some(row) = target.widget() else {
        return gtk::gdk::DragAction::empty();
    };
    if !feedback.is_current(&row) {
        return gtk::gdk::DragAction::empty();
    }
    let Some(value) = target.value() else {
        return gtk::gdk::DragAction::MOVE;
    };
    if !value.get::<String>().is_ok_and(|source| {
        sidebar_reorder_matches(&source, payload)
            || (source == payload && sidebar_drop_feedback::is_gap(&row))
    }) {
        return gtk::gdk::DragAction::empty();
    }
    feedback.reorder(&row, y >= f64::from(row.height()) / 2.0);
    gtk::gdk::DragAction::MOVE
}

fn sidebar_accepts_file_drop(location: &Location) -> bool {
    location.native_path().is_some()
}

fn install_sidebar_file_drop(
    state: &SidebarState,
    row: &impl IsA<gtk::Widget>,
    destination: Location,
) {
    if destination == Location::uri("trash:///") {
        install_sidebar_trash_drop(state, row);
        return;
    }
    if !sidebar_accepts_file_drop(&destination) {
        return;
    }
    row.add_css_class("file-drop-zone");
    let PreparedFileDrop {
        target: drop,
        state: drop_state,
    } = prepare_file_drop_target({
        let destination = destination.clone();
        move || Some(destination.clone())
    });
    drop.set_propagation_phase(gtk::PropagationPhase::Capture);
    let spring_navigate: Rc<dyn Fn(Location)> = {
        let view = state.view.downgrade();
        let row = row.downgrade();
        Rc::new(move |location| {
            let Some(view) = view.upgrade() else {
                return;
            };
            view.browser().navigate_location(location, false);
            if let Some(row) = row.upgrade() {
                row.grab_focus();
            }
        })
    };
    let state_for_enter = drop_state.clone();
    let navigate_for_enter = spring_navigate.clone();
    let enter_feedback = state.drop_feedback.clone();
    drop.connect_enter(move |target, _, _| {
        let Some(row) = target.widget() else {
            return gtk::gdk::DragAction::empty();
        };
        let cancel_state = state_for_enter.clone();
        enter_feedback.enter_folder(&row, move || cancel_state.cancel_spring_load_navigation());
        let action = file_drop_action(target, &state_for_enter);
        let hovered = file_drag_hover_target(&state_for_enter, target).is_some();
        enter_feedback.folder(&row, hovered);
        arm_spring_load_navigation(&state_for_enter, target, &navigate_for_enter);
        action
    });
    let state_for_motion = drop_state.clone();
    let navigate_for_motion = spring_navigate.clone();
    let motion_feedback = state.drop_feedback.clone();
    drop.connect_motion(move |target, _, _| {
        let Some(row) = target
            .widget()
            .filter(|row| motion_feedback.is_current(row))
        else {
            return gtk::gdk::DragAction::empty();
        };
        let action = file_drop_action(target, &state_for_motion);
        let hovered = file_drag_hover_target(&state_for_motion, target).is_some();
        motion_feedback.folder(&row, hovered);
        arm_spring_load_navigation(&state_for_motion, target, &navigate_for_motion);
        action
    });
    let state_for_value = drop_state.clone();
    let navigate_for_value = spring_navigate.clone();
    let value_feedback = state.drop_feedback.clone();
    drop.connect_value_notify(move |target| {
        if target.current_drop().is_none() {
            return;
        }
        let Some(row) = target.widget().filter(|row| value_feedback.is_current(row)) else {
            return;
        };
        let hovered = file_drag_hover_target(&state_for_value, target).is_some();
        value_feedback.folder(&row, hovered);
        arm_spring_load_navigation(&state_for_value, target, &navigate_for_value);
    });
    let leave_feedback = state.drop_feedback.clone();
    drop.connect_leave(move |target| {
        if let Some(row) = target.widget() {
            leave_feedback.leave(&row);
        }
    });
    let handler: DropHandler = {
        let view = state.view.downgrade();
        Rc::new(move |destination, request| {
            if let Some(view) = view.upgrade() {
                view.commit_drop(destination, request);
            }
        })
    };
    let handler_for_drop = handler.clone();
    let state_for_drop = drop_state.clone();
    let drop_feedback = state.drop_feedback.clone();
    drop.connect_drop(move |target, value, _, _| {
        if let Some(row) = target.widget() {
            drop_feedback.leave(&row);
        }
        let handler = handler_for_drop.clone();
        commit_dropped_value(
            target,
            value,
            destination.clone(),
            &state_for_drop,
            move |destination, request| handler(destination, request),
        )
    });
    attach_drop_target(row, &drop, &drop_state, handler);
}

fn trash_file_drop_action(target: &gtk::DropTarget) -> gtk::gdk::DragAction {
    if target.value().as_ref().is_some_and(|value| {
        locations_from_file_list_value(value)
            .is_none_or(|sources| !BrowserView::can_trash_file_drop(&sources))
    }) {
        gtk::gdk::DragAction::empty()
    } else {
        gtk::gdk::DragAction::MOVE
    }
}

fn install_sidebar_trash_drop(state: &SidebarState, row: &impl IsA<gtk::Widget>) {
    row.add_css_class("file-drop-zone");
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::MOVE,
    );
    drop.set_preload(true);
    drop.set_propagation_phase(gtk::PropagationPhase::Capture);
    let enter_feedback = state.drop_feedback.clone();
    drop.connect_enter(move |target, _, _| {
        if let Some(row) = target.widget() {
            enter_feedback.enter(&row);
        }
        sidebar_trash_feedback(target, &enter_feedback)
    });
    let motion_feedback = state.drop_feedback.clone();
    drop.connect_motion(move |target, _, _| sidebar_trash_feedback(target, &motion_feedback));
    let value_feedback = state.drop_feedback.clone();
    drop.connect_value_notify(move |target| {
        if let Some(offered) = target.current_drop()
            && target
                .widget()
                .is_some_and(|row| value_feedback.is_current(&row))
        {
            offered.status(
                target.actions(),
                sidebar_trash_feedback(target, &value_feedback),
            );
        }
    });
    let leave_feedback = state.drop_feedback.clone();
    drop.connect_leave(move |target| {
        if let Some(row) = target.widget() {
            leave_feedback.leave(&row);
        }
    });
    let view = state.view.downgrade();
    let drop_feedback = state.drop_feedback.clone();
    drop.connect_drop(move |target, value, _, _| {
        if let Some(row) = target.widget() {
            drop_feedback.leave(&row);
        }
        view.upgrade().is_some_and(|view| {
            locations_from_file_list_value(value)
                .is_some_and(|sources| view.trash_file_drop(sources))
        })
    });
    row.add_controller(drop);
}

fn sidebar_trash_feedback(
    target: &gtk::DropTarget,
    feedback: &sidebar_drop_feedback::SidebarDropFeedback,
) -> gtk::gdk::DragAction {
    let Some(row) = target.widget().filter(|row| feedback.is_current(row)) else {
        return gtk::gdk::DragAction::empty();
    };
    let action = trash_file_drop_action(target);
    feedback.folder(&row, !action.is_empty());
    action
}

/// Moves `widgets` into `container` in order, right after `anchor` or at the
/// start, without dropping keyboard focus from a row that changes parent.
fn reparent_in_order(container: &gtk::Box, widgets: &[gtk::Widget], anchor: Option<&gtk::Widget>) {
    let mut previous = anchor.cloned();
    for widget in widgets {
        if widget.parent().as_ref() == Some(container.upcast_ref()) {
            if widget.prev_sibling() != previous {
                container.reorder_child_after(widget, previous.as_ref());
            }
        } else {
            let focused = widget
                .root()
                .and_then(|root| gtk::prelude::RootExt::focus(&root))
                .is_some_and(|focus| focus == *widget || focus.is_ancestor(widget));
            if let Some(parent) = widget.parent().and_downcast::<gtk::Box>() {
                parent.remove(widget);
            }
            container.insert_child_after(widget, previous.as_ref());
            if focused {
                widget.grab_focus();
            }
        }
        previous = Some(widget.clone());
    }
}

fn select_sidebar_row(sidebar: &gtk::Box, selected: &gtk::Button) {
    deselect_sidebar_rows(sidebar.upcast_ref());
    set_sidebar_row_active(selected, true);
}

fn deselect_sidebar_rows(container: &gtk::Widget) {
    let mut child = container.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(revealer) = widget.clone().downcast::<gtk::Revealer>() {
            if let Some(inner) = revealer.child() {
                deselect_sidebar_rows(&inner);
            }
        } else if let Some(row) = sidebar_row_button(&widget) {
            set_sidebar_row_active(&row, false);
        }
    }
}

fn set_sidebar_row_active(row: &gtk::Button, active: bool) {
    if active {
        row.add_css_class("active");
    } else {
        row.remove_css_class("active");
    }
    if let Some(shell) = sidebar_device_shell(row) {
        if active {
            shell.add_css_class("active");
        } else {
            shell.remove_css_class("active");
        }
    }
}

fn sidebar_device_shell(row: &gtk::Button) -> Option<gtk::Box> {
    row.parent()
        .and_then(|parent| parent.downcast::<gtk::Box>().ok())
        .filter(|parent| parent.has_css_class("sidebar-device"))
}

/// Resolves the navigable row button for a sidebar child. Device rows wrap
/// their button with an eject sibling, so look one level down when the child
/// itself is a container.
fn sidebar_row_button(widget: &gtk::Widget) -> Option<gtk::Button> {
    widget
        .clone()
        .downcast::<gtk::Button>()
        .ok()
        .filter(|button| !button.has_css_class("sidebar-heading-toggle"))
        .or_else(|| {
            widget
                .first_child()
                .and_then(|child| child.downcast::<gtk::Button>().ok())
        })
}

fn reorder_places(order: &mut Vec<&'static str>, source: &str, target: &str, after: bool) -> bool {
    if source == target {
        return false;
    }
    let Some(source_index) = order.iter().position(|place| *place == source) else {
        return false;
    };
    let source = order.remove(source_index);
    let Some(target_index) = order.iter().position(|place| *place == target) else {
        order.insert(source_index, source);
        return false;
    };
    order.insert(target_index + usize::from(after), source);
    true
}

fn reorder_pinned_places(
    places: &mut Vec<(Location, String)>,
    source: usize,
    target: usize,
    after: bool,
) -> bool {
    if source == target || source >= places.len() || target >= places.len() {
        return false;
    }
    let place = places.remove(source);
    // `source` is a pre-removal index, but `target` must remap onto the shrunken
    // vector: any target that sat after `source` slides left by one. Since
    // `destination == source` is checked in post-removal coordinates, a match
    // means re-inserting into the original slot — a no-op worth reporting.
    let target = target - usize::from(target > source);
    let destination = (target + usize::from(after)).min(places.len());
    if destination == source {
        places.insert(source, place);
        return false;
    }
    places.insert(destination, place);
    true
}

fn parse_pinned_drag_source(source: &str) -> Option<usize> {
    source.strip_prefix(PINNED_DRAG_PREFIX)?.parse().ok()
}

fn pin_status(places: &[(Location, String)], location: &Location) -> PinStatus {
    if is_standard_place_location(location) {
        PinStatus::Unavailable
    } else if places.iter().any(|(pinned, _)| pinned == location) {
        PinStatus::Pinned
    } else {
        PinStatus::Available
    }
}

fn remove_pinned_place(places: &mut Vec<(Location, String)>, location: &Location) -> bool {
    let original_len = places.len();
    places.retain(|(pinned, _)| pinned != location);
    places.len() != original_len
}

/// The `(after, before)` pins around an insertion gap when the sidebar was built.
type PinGap<'a> = (Option<&'a Location>, Option<&'a Location>);

/// Resolves a gap against the latest bookmarks; `None` when both of a gap's
/// neighbours were unpinned elsewhere, so a drop cannot land somewhere unrelated.
fn pin_gap_index(places: &[(Location, String)], (after, before): PinGap<'_>) -> Option<usize> {
    let position = |anchor: &Location| places.iter().position(|(location, _)| location == anchor);
    match (before.and_then(position), after.and_then(position)) {
        (Some(index), _) => Some(index),
        (None, Some(index)) => Some(index + 1),
        (None, None) if before.is_none() => Some(places.len()),
        (None, None) => None,
    }
}

/// `index` is an insertion point in the list before `source` is removed.
fn move_pinned_place(places: &mut Vec<(Location, String)>, source: usize, index: usize) -> bool {
    if source >= places.len() || index > places.len() || index == source || index == source + 1 {
        return false;
    }
    let place = places.remove(source);
    places.insert(index - usize::from(index > source), place);
    true
}

fn pin_locations_at(
    places: &mut Vec<(Location, String)>,
    sources: &[(Location, String)],
    target_index: usize,
) -> bool {
    let mut seen = places
        .iter()
        .map(|(location, _)| location.clone())
        .collect::<std::collections::HashSet<_>>();
    let additions = sources
        .iter()
        .filter(|(location, _)| {
            !is_standard_place_location(location) && seen.insert(location.clone())
        })
        .cloned()
        .collect::<Vec<_>>();
    if additions.is_empty() {
        return false;
    }
    let index = target_index.min(places.len());
    places.splice(index..index, additions);
    true
}

fn is_smb_location(location: &Location) -> bool {
    location.uri_value().is_some_and(|uri| {
        uri.get(..4)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("smb:"))
    })
}

/// Safe-removal action for a sidebar device row. Eject is preferred whenever
/// the drive reports it (USB sticks, optical discs); plain unmount covers
/// mounts without ejectable media. `None` means fixed internal storage with
/// no actionable release.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MediaRelease {
    EjectVolume,
    EjectMount,
    UnmountMount,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EncryptedMediaAction {
    Unlock,
    Lock,
}

impl EncryptedMediaAction {
    fn label(self) -> &'static str {
        match self {
            Self::Unlock => "Unlock",
            Self::Lock => "Lock",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Unlock => crate::assets::icons::LOCK,
            Self::Lock => crate::assets::icons::LOCK_OPEN,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeviceRowActions {
    encrypted: Option<EncryptedMediaAction>,
    release: Option<MediaRelease>,
}

fn device_row_actions(
    encrypted: bool,
    mounted: bool,
    can_eject_volume: bool,
    mount_can_eject: bool,
    mount_can_unmount: bool,
) -> DeviceRowActions {
    DeviceRowActions {
        encrypted: encrypted.then_some(if mounted {
            EncryptedMediaAction::Lock
        } else {
            EncryptedMediaAction::Unlock
        }),
        release: volume_release_action(can_eject_volume, mount_can_eject, mount_can_unmount),
    }
}

fn volume_release_action(
    can_eject_volume: bool,
    mount_can_eject: bool,
    mount_can_unmount: bool,
) -> Option<MediaRelease> {
    if can_eject_volume {
        Some(MediaRelease::EjectVolume)
    } else if mount_can_eject {
        Some(MediaRelease::EjectMount)
    } else if mount_can_unmount {
        Some(MediaRelease::UnmountMount)
    } else {
        None
    }
}

fn media_release_label(action: MediaRelease) -> &'static str {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => "Eject",
        MediaRelease::UnmountMount => "Unmount",
    }
}

fn media_release_error_title(action: MediaRelease) -> &'static str {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => "Unable to eject device",
        MediaRelease::UnmountMount => "Unable to unmount device",
    }
}

fn release_kind(action: MediaRelease) -> device_release::ReleaseKind {
    match action {
        MediaRelease::UnmountMount => device_release::ReleaseKind::Unmount,
        MediaRelease::EjectVolume | MediaRelease::EjectMount => device_release::ReleaseKind::Eject,
    }
}

fn release_unplug(
    action: &MediaRelease,
    mount: Option<&gio::Mount>,
    volume: Option<&gio::Volume>,
) -> bool {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => true,
        MediaRelease::UnmountMount => {
            mount.is_some_and(mount_can_unplug) || volume.is_some_and(volume_can_unplug)
        }
    }
}

fn navigate_home_if_within(browser: &Rc<Browser>, root: &gio::File) {
    let Some(device) = location_for_file(root) else {
        return;
    };
    let active = browser.active_location();
    if active
        .as_ref()
        .is_some_and(|active| active == &device || active.is_within(&device))
    {
        browser.navigate(Location::local(home_directory()));
    }
}

fn begin_media_release(in_flight: &Cell<bool>) -> bool {
    !in_flight.replace(true)
}

fn release_device_volume(
    volume: &gio::Volume,
    action: MediaRelease,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    if !begin_media_release(in_flight) {
        return;
    }
    let mount = volume.get_mount();
    if matches!(
        action,
        MediaRelease::EjectMount | MediaRelease::UnmountMount
    ) && mount.is_none()
    {
        in_flight.set(false);
        return;
    }
    let key = device_release::key_for_volume(volume);
    let sync_root = mount.as_ref().and_then(|mount| mount.root().path());
    let away_root = mount.as_ref().map(gio::Mount::root);
    let unplug = release_unplug(&action, mount.as_ref(), Some(volume));
    let volume = volume.clone();
    let mount = mount.clone();
    let in_flight_for_settle = in_flight.clone();
    let started = device_release::start_release(
        parent,
        Some(browser.clone()),
        Some(view),
        key,
        release_kind(action),
        unplug,
        sync_root,
        away_root,
        move || in_flight_for_settle.set(false),
        move |operation| async move {
            match action {
                MediaRelease::EjectVolume => {
                    volume
                        .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                        .await
                }
                MediaRelease::EjectMount => {
                    let Some(mount) = mount else {
                        return Err(glib::Error::new(
                            gio::IOErrorEnum::Failed,
                            &crate::i18n::tr("The volume is not mounted."),
                        ));
                    };
                    mount
                        .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                        .await
                }
                MediaRelease::UnmountMount => {
                    let Some(mount) = mount else {
                        return Err(glib::Error::new(
                            gio::IOErrorEnum::Failed,
                            &crate::i18n::tr("The volume is not mounted."),
                        ));
                    };
                    mount
                        .unmount_with_operation_future(
                            gio::MountUnmountFlags::NONE,
                            Some(&operation),
                        )
                        .await
                }
            }
        },
    );
    if !started {
        in_flight.set(false);
    }
}

fn release_device_mount(
    mount: &gio::Mount,
    action: MediaRelease,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    if !begin_media_release(in_flight) {
        return;
    }
    let key = device_release::key_for_mount(mount);
    let sync_root = mount.root().path();
    let away_root = mount.root();
    let unplug = release_unplug(&action, Some(mount), None);
    let mount = mount.clone();
    let in_flight_for_settle = in_flight.clone();
    let started = device_release::start_release(
        parent,
        Some(browser.clone()),
        Some(view),
        key,
        release_kind(action),
        unplug,
        sync_root,
        Some(away_root),
        move || in_flight_for_settle.set(false),
        move |operation| async move {
            match action {
                MediaRelease::EjectVolume | MediaRelease::EjectMount => {
                    mount
                        .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                        .await
                }
                MediaRelease::UnmountMount => {
                    mount
                        .unmount_with_operation_future(
                            gio::MountUnmountFlags::NONE,
                            Some(&operation),
                        )
                        .await
                }
            }
        },
    );
    if !started {
        in_flight.set(false);
    }
}

fn lock_encrypted_volume(
    volume: &gio::Volume,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    let luks_uuid = crypto_password_uuid_for_volume(volume);
    let target = lock_target(
        volume.drive(),
        gio_volume_unix_device(volume).map(Into::into),
        luks_uuid.clone(),
        device_release::key_for_volume(volume),
    );
    request_encrypted_lock_showing(
        parent,
        &volume.name(),
        luks_uuid,
        volume.get_mount(),
        target,
        browser,
        Some(view),
        in_flight,
    );
}

fn lock_encrypted_mount(
    mount: &gio::Mount,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    let luks_uuid = crypto_password_uuid_for_mount(mount);
    let unix_device = mount
        .volume()
        .and_then(|volume| gio_volume_unix_device(&volume))
        .map(String::from)
        .or_else(|| {
            mount
                .root()
                .path()
                .and_then(|root| drive_ops::block_device_for_path(&root))
                .map(|device| device.to_string_lossy().into_owned())
        });
    let target = lock_target(
        mount.drive(),
        unix_device,
        luks_uuid.clone(),
        device_release::key_for_mount(mount),
    );
    request_encrypted_lock_showing(
        parent,
        &mount.name(),
        luks_uuid,
        Some(mount.clone()),
        target,
        browser,
        Some(view),
        in_flight,
    );
}

enum LockTarget {
    /// GVfs locks removable encrypted drives by stopping them.
    StopDrive(gio::Drive),
    /// Internal disks have no stoppable drive, so UDisks locks the container.
    Container {
        unix_device: Option<String>,
        luks_uuid: Option<String>,
        key: device_release::ReleaseKey,
    },
}

fn lock_target(
    drive: Option<gio::Drive>,
    unix_device: Option<String>,
    luks_uuid: Option<String>,
    key: device_release::ReleaseKey,
) -> Option<LockTarget> {
    if let Some(drive) = password_stop_drive(drive) {
        return Some(LockTarget::StopDrive(drive));
    }
    (unix_device.is_some() || luks_uuid.is_some()).then_some(LockTarget::Container {
        unix_device,
        luks_uuid,
        key,
    })
}

pub(crate) fn crypto_password_uuid_for_volume(volume: &gio::Volume) -> Option<String> {
    let unix = gio_volume_unix_device(volume);
    let hint = unix
        .as_deref()
        .map(devices::block_crypto_hint)
        .unwrap_or_default();
    devices::crypto_password_uuid(
        volume.uuid().as_deref(),
        unix.as_deref(),
        hint.as_ref(),
        gio_volume_is_locked(volume),
    )
}

fn crypto_password_uuid_for_mount(mount: &gio::Mount) -> Option<String> {
    if let Some(volume) = mount.volume() {
        return crypto_password_uuid_for_volume(&volume);
    }
    let unix = mount
        .drive()
        .and_then(|drive| drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()));
    let hint = unix
        .as_deref()
        .map(devices::block_crypto_hint)
        .unwrap_or_default();
    devices::crypto_password_uuid(None, unix.as_deref(), hint.as_ref(), false)
}

#[expect(
    clippy::too_many_arguments,
    reason = "lock confirmation needs the volume, its mount, and the window that shows progress"
)]
fn request_encrypted_lock_showing(
    parent: &gtk::Widget,
    volume_name: &str,
    luks_uuid: Option<String>,
    mount: Option<gio::Mount>,
    target: Option<LockTarget>,
    browser: &Rc<Browser>,
    view: Option<&BrowserView>,
    in_flight: &Rc<Cell<bool>>,
) {
    if mount.is_none() && target.is_none() {
        show_error_dialog(
            parent,
            &crate::i18n::tr("Unable to lock device"),
            &crate::i18n::tr(
                "This volume has no supported lock operation while unmounted. Mount it in Strata, then try Lock again.",
            ),
        );
        return;
    }
    if !begin_media_release(in_flight) {
        return;
    }
    let parent = parent.clone();
    let volume_name = volume_name.to_owned();
    let browser = browser.clone();
    let view = view.cloned();
    let in_flight = in_flight.clone();
    glib::MainContext::default().spawn_local(async move {
        let cached = if let Some(uuid) = luks_uuid.clone() {
            gio::spawn_blocking(move || volume_password::volume_password_is_cached(&uuid))
                .await
                .unwrap_or(false)
        } else {
            false
        };
        let parent_for_lock = parent.clone();
        let parent_for_forget = parent.clone();
        let browser_for_lock = browser.clone();
        let in_flight_for_lock = in_flight.clone();
        let in_flight_for_forget = in_flight.clone();
        let start_lock = move || {
            lock_cleartext_then_close(
                mount,
                target,
                &parent_for_lock,
                &browser_for_lock,
                view,
                &in_flight_for_lock,
            );
        };
        continue_encrypted_lock(
            &parent,
            &volume_name,
            cached,
            move || {
                glib::MainContext::default().spawn_local(async move {
                    if cached && let Some(uuid) = luks_uuid {
                        let forget = gio::spawn_blocking(move || {
                            volume_password::forget_cached_volume_password(&uuid)
                        })
                        .await;
                        match forget {
                            Ok(Ok(())) => start_lock(),
                            Ok(Err(error)) => {
                                show_error_dialog(
                                    &parent_for_forget,
                                    &crate::i18n::tr("Couldn't forget the saved password"),
                                    &error.user_message(),
                                );
                                in_flight_for_forget.set(false);
                            }
                            Err(_) => {
                                show_error_dialog(
                                    &parent_for_forget,
                                    &crate::i18n::tr("Couldn't forget the saved password"),
                                    &crate::i18n::tr("The saved password could not be deleted."),
                                );
                                in_flight_for_forget.set(false);
                            }
                        }
                    } else {
                        start_lock();
                    }
                });
            },
            move || in_flight.set(false),
        );
    });
}

fn continue_encrypted_lock(
    parent: &gtk::Widget,
    volume_name: &str,
    password_cached: bool,
    on_lock: impl FnOnce() + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    if password_cached {
        confirm_forget_cached_password(parent, volume_name, on_lock, on_cancel);
    } else {
        on_lock();
    }
}

fn confirm_forget_cached_password(
    parent: &gtk::Widget,
    volume_name: &str,
    on_confirm: impl FnOnce() + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        on_cancel();
        return;
    };

    let layout = message_dialog_layout(
        crate::assets::icons::KEY,
        &crate::i18n::tr("Forget saved password?"),
        volume_name,
        &crate::i18n::tr("Forget and lock"),
        ModalTone::Danger,
    );
    layout.body.append(&message_dialog_description(&crate::i18n::tr(
        "This volume’s password is saved. Locking will forget it, and you’ll need to enter it again to unlock.",
    )));
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let confirm = layout.confirm;

    let layer = modal_layer(
        &content,
        &window_overlay,
        blurred_root.clone(),
        Some(Rc::new(|| true)),
    );
    window_overlay.add_overlay(&layer);

    let dismissed = Rc::new(Cell::new(false));
    let on_confirm = Rc::new(RefCell::new(Some(on_confirm)));
    let on_cancel = Rc::new(RefCell::new(Some(on_cancel)));
    let dismiss = Rc::new({
        let layer = layer.clone();
        let overlay = window_overlay.clone();
        let root = blurred_root.clone();
        let dismissed = dismissed.clone();
        move || {
            if dismissed.replace(true) {
                return false;
            }
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
            true
        }
    });

    let cancel_dismiss = dismiss.clone();
    let cancel_handler = on_cancel.clone();
    cancel.connect_clicked(move |_| {
        if cancel_dismiss()
            && let Some(on_cancel) = cancel_handler.borrow_mut().take()
        {
            on_cancel();
        }
    });
    let close_dismiss = dismiss.clone();
    let close_handler = on_cancel.clone();
    close.connect_clicked(move |_| {
        if close_dismiss()
            && let Some(on_cancel) = close_handler.borrow_mut().take()
        {
            on_cancel();
        }
    });
    let confirm_dismiss = dismiss.clone();
    let confirm_handler = on_confirm;
    confirm.connect_clicked(move |_| {
        if confirm_dismiss()
            && let Some(on_confirm) = confirm_handler.borrow_mut().take()
        {
            on_confirm();
        }
    });
    let escape_dismiss = dismiss;
    let escape_handler = on_cancel;
    close_on_escape(&layer, move || {
        if escape_dismiss()
            && let Some(on_cancel) = escape_handler.borrow_mut().take()
        {
            on_cancel();
        }
    });
    focus_button(&confirm);
}

fn password_stop_drive(drive: Option<gio::Drive>) -> Option<gio::Drive> {
    drive.filter(|drive| {
        drive.start_stop_type() == gio::DriveStartStopType::Password && drive.can_stop()
    })
}

fn lock_cleartext_then_close(
    mount: Option<gio::Mount>,
    target: Option<LockTarget>,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: Option<BrowserView>,
    in_flight: &Rc<Cell<bool>>,
) {
    let key = match (mount.as_ref(), target.as_ref()) {
        (Some(mount), _) => device_release::key_for_mount(mount),
        (None, Some(LockTarget::StopDrive(drive))) => device_release::key_for_drive(drive),
        (None, Some(LockTarget::Container { key, .. })) => key.clone(),
        (None, None) => device_release::ReleaseKey {
            unix_device: None,
            uuid: None,
            mount_root: None,
            display_name: String::new(),
        },
    };
    let sync_root = mount.as_ref().and_then(|mount| mount.root().path());
    let away_root = mount.as_ref().map(gio::Mount::root);
    let in_flight_for_settle = in_flight.clone();
    let parent_for_stop = parent.clone();
    let key_for_stop = key.clone();
    let started = device_release::start_release(
        parent,
        Some(browser.clone()),
        view.as_ref(),
        key,
        device_release::ReleaseKind::Lock,
        false,
        sync_root,
        away_root,
        move || in_flight_for_settle.set(false),
        move |unmount_operation| async move {
            let unmount_result = match mount {
                Some(mount) => {
                    mount
                        .unmount_with_operation_future(
                            gio::MountUnmountFlags::NONE,
                            Some(&unmount_operation),
                        )
                        .await
                }
                None => Ok(()),
            };
            match unmount_result {
                Ok(()) => match target {
                    Some(LockTarget::StopDrive(drive)) => {
                        let stop_operation =
                            device_release::mount_operation(&parent_for_stop, &key_for_stop);
                        drive
                            .stop_future(gio::MountUnmountFlags::NONE, Some(&stop_operation))
                            .await
                    }
                    Some(LockTarget::Container {
                        unix_device,
                        luks_uuid,
                        ..
                    }) => {
                        drive_ops::lock_encrypted_device_on_system_bus(unix_device, luks_uuid).await
                    }
                    None => Ok(()),
                },
                Err(error) => Err(error),
            }
        },
    );
    if !started {
        in_flight.set(false);
    }
}

fn eject_password_drive(
    drive: &gio::Drive,
    action: MediaRelease,
    parent: &gtk::Widget,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    if !matches!(action, MediaRelease::EjectVolume | MediaRelease::EjectMount) {
        return;
    }
    if !begin_media_release(in_flight) {
        return;
    }
    let key = device_release::key_for_drive(drive);
    let unplug = release_unplug(&action, None, None);
    let drive = drive.clone();
    let in_flight_for_settle = in_flight.clone();
    let started = device_release::start_release(
        parent,
        None,
        Some(view),
        key,
        release_kind(action),
        unplug,
        None,
        None,
        move || in_flight_for_settle.set(false),
        move |operation| async move {
            drive
                .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                .await
        },
    );
    if !started {
        in_flight.set(false);
    }
}

async fn trash_has_items(root: &gio::File) -> Result<bool, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            gio::FILE_ATTRIBUTE_STANDARD_NAME,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    Ok(!enumerator
        .next_files_future(1, glib::Priority::DEFAULT)
        .await?
        .is_empty())
}

/// Extra context-menu action placed above the standard place items.
struct PlaceMenuExtra {
    icon: &'static str,
    /// English message key, localized when the menu item is built.
    label: &'static str,
    action: fn(&Rc<SidebarState>),
}

fn sidebar_context_option(icon: &str, label: &str, danger: bool) -> gtk::Button {
    let translated = crate::i18n::tr(label);
    let label = translated.as_str();
    let button = super::accessibility::menu_item_button();
    super::accessibility::describe_menu_item(&button, label, "");
    button.add_css_class("item-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = if danger {
        crate::assets::danger_icon(icon, 15)
    } else {
        crate::assets::primary_icon(icon, 15)
    };
    icon.add_css_class("item-context-icon");
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&icon);
    row.append(&title);
    button.set_child(Some(&row));
    button
}

fn customize_sidebar_folder_icon(row: &gtk::Button, location: &Location, icon: &str) {
    if let Some(path) = location.native_path()
        && let Some(image) = row
            .child()
            .and_then(|content| content.first_child())
            .and_downcast::<gtk::Image>()
    {
        super::thumbnail::show_customized_folder_image(&image, path, icon, image.pixel_size());
    }
}

fn sync_keycap(keycap: &gtk::Label, visible: bool) {
    let visible = visible && !keycap.text().is_empty();
    if let Some(icon) = keycap.prev_sibling() {
        if visible && icon.is_visible() {
            // Matching the icon's width keeps the place name from shifting.
            keycap.set_width_request(icon.width());
        }
        icon.set_visible(!visible);
    }
    keycap.set_visible(visible);
}

fn sidebar_button(icon: &str, name: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let image = crate::assets::primary_icon(icon, 17);
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&image);
    content.append(&label);

    let row = gtk::Button::builder()
        .child(&content)
        .halign(gtk::Align::Fill)
        .build();
    row.add_css_class("sidebar-row");
    row.set_cursor_from_name(Some("pointer"));
    row.set_has_frame(false);
    row
}

fn sidebar_eject_button(action: MediaRelease, on_release: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::new();
    super::accessibility::set_tooltip_and_label(
        &button,
        &crate::i18n::tr(media_release_label(action)),
    );
    button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::EJECT,
        14,
    )));
    button.add_css_class("sidebar-eject");
    button.add_css_class("sidebar-device-action");
    button.set_cursor_from_name(Some("pointer"));
    button.set_has_frame(false);
    button.set_hexpand(false);
    button.set_valign(gtk::Align::Center);
    button.set_width_request(24);
    button.connect_clicked(move |_| on_release());
    button
}

fn sidebar_lock_button(action: EncryptedMediaAction, on_click: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::new();
    super::accessibility::set_tooltip_and_label(&button, &crate::i18n::tr(action.label()));
    button.set_child(Some(&crate::assets::primary_icon(action.icon(), 14)));
    button.add_css_class("sidebar-eject");
    button.add_css_class("sidebar-device-action");
    button.set_cursor_from_name(Some("pointer"));
    button.set_has_frame(false);
    button.set_hexpand(false);
    button.set_valign(gtk::Align::Center);
    button.set_width_request(24);
    button.connect_clicked(move |_| on_click());
    button
}

fn sidebar_device_row(
    row: &gtk::Button,
    lock: Option<&gtk::Button>,
    eject: Option<&gtk::Button>,
) -> gtk::Box {
    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shell.add_css_class("sidebar-device");
    shell.set_hexpand(true);
    row.set_hexpand(true);
    row.set_halign(gtk::Align::Fill);
    row.connect_has_focus_notify(|row| {
        let Some(shell) = sidebar_device_shell(row) else {
            return;
        };
        if row.has_focus() {
            shell.add_css_class("focused");
        } else {
            shell.remove_css_class("focused");
        }
    });
    shell.append(row);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("sidebar-device-actions");
    actions.set_hexpand(false);
    actions.set_halign(gtk::Align::End);
    actions.set_valign(gtk::Align::Center);
    if let Some(lock) = lock {
        lock.set_hexpand(false);
        actions.append(lock);
    }
    if let Some(eject) = eject {
        eject.set_hexpand(false);
        actions.append(eject);
    }
    if actions.first_child().is_some() {
        shell.append(&actions);
    }
    shell
}

fn attach_device_actions_menu(
    row: &gtk::Button,
    view: &BrowserView,
    actions: DeviceRowActions,
    on_crypto: Option<Rc<dyn Fn()>>,
    on_release: Option<Rc<dyn Fn()>>,
    on_mount: Option<Rc<dyn Fn()>>,
    properties: Option<drive_dialogs::PropertiesTarget>,
) {
    let volume = properties
        .as_ref()
        .and_then(drive_dialogs::PropertiesTarget::volume);
    if actions.encrypted.is_none() && actions.release.is_none() && properties.is_none() {
        return;
    }
    let menu = super::accessibility::menu_box();
    menu.add_css_class("folder-context-menu");
    let popover = gtk::Popover::builder()
        .child(&menu)
        .autohide(true)
        .has_arrow(false)
        .build();
    popover.add_css_class("folder-context-popover");
    popover.set_parent(row);
    let mounted_location = properties
        .as_ref()
        .and_then(|target| match target {
            drive_dialogs::PropertiesTarget::Volume(volume) => volume.get_mount(),
            drive_dialogs::PropertiesTarget::Mount(mount) => Some(mount.clone()),
        })
        .and_then(|mount| location_for_file(&mount.root()));
    if let Some(location) = mounted_location {
        sidebar::append_open_in_tab(&menu, &popover, row, view, &location);
    }
    if let (Some(action), Some(on_crypto)) = (actions.encrypted, on_crypto) {
        let option = sidebar_context_option(action.icon(), action.label(), false);
        menu.append(&option);
        let crypto_popover = popover.downgrade();
        option.connect_clicked(move |_| {
            if let Some(popover) = crypto_popover.upgrade() {
                popover.popdown();
            }
            on_crypto();
        });
    }
    if let (Some(action), Some(on_release)) = (actions.release, on_release.clone()) {
        let option = sidebar_context_option(
            crate::assets::icons::EJECT,
            media_release_label(action),
            false,
        );
        menu.append(&option);
        let release_popover = popover.downgrade();
        option.connect_clicked(move |_| {
            if let Some(popover) = release_popover.upgrade() {
                popover.popdown();
            }
            on_release();
        });
    }
    if drive_ops::show_mount(volume.as_ref())
        && let Some(on_mount) = on_mount
    {
        let option = sidebar_context_option(crate::assets::icons::HARD_DRIVE, "Mount", false);
        menu.append(&option);
        let mount_popover = popover.downgrade();
        option.connect_clicked(move |_| {
            if let Some(popover) = mount_popover.upgrade() {
                popover.popdown();
            }
            on_mount();
        });
    }
    if drive_ops::is_eligible(volume.as_ref())
        && let Some(volume) = &volume
    {
        let format_volume = volume.clone();
        let option = sidebar_context_option(crate::assets::icons::SHREDDER, "Format…", true);
        menu.append(&option);
        let format_popover = popover.downgrade();
        let format_view = view.clone();
        let parent = row.clone().upcast::<gtk::Widget>();
        option.connect_clicked(move |_| {
            drive_dialogs::open_from_sidebar(
                &format_view,
                format_popover.upgrade().as_ref(),
                || {
                    drive_dialogs::show_format_dialog(&parent, &format_volume);
                },
            );
        });
    }
    if let Some(target) = properties
        .as_ref()
        .filter(|target| target.label_id().is_some())
        .cloned()
    {
        let option = sidebar_context_option(crate::assets::icons::PENCIL, "Set label…", false);
        menu.append(&option);
        let label_popover = popover.downgrade();
        let label_view = view.clone();
        let parent = row.clone().upcast::<gtk::Widget>();
        option.connect_clicked(move |_| {
            drive_dialogs::open_from_sidebar(&label_view, label_popover.upgrade().as_ref(), || {
                drive_dialogs::show_label_dialog(&parent, &target);
            });
        });
    }
    if let Some(properties_target) = properties {
        let on_release = on_release.clone();
        let option = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        menu.append(&option);
        let properties_popover = popover.downgrade();
        let properties_view = view.clone();
        let parent = row.clone().upcast::<gtk::Widget>();
        option.connect_clicked(move |_| {
            drive_dialogs::open_from_sidebar(
                &properties_view,
                properties_popover.upgrade().as_ref(),
                || {
                    drive_dialogs::show_drive_properties(
                        &parent,
                        &properties_target,
                        on_release.clone(),
                    );
                },
            );
        });
    }
    let context = gtk::GestureClick::new();
    context.set_button(3);
    let weak_popover = popover.downgrade();
    context.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
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
    row.add_controller(context);
}

#[cfg(test)]
mod tests;
