// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib};

use crate::{app::Browser, model::Location};

use super::browser::{BrowserView, WeakBrowserView, show_error_dialog};

mod appearance_menu;
mod bookmarks;
mod composition;
mod density_slider;
mod device_actions;
mod device_labels;
mod device_lock;
mod device_menu;
mod device_order;
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
mod sidebar_context_menu;
mod sidebar_devices;
mod sidebar_drop_feedback;
mod sidebar_file_drop;
mod sidebar_pins;
mod sidebar_places;
mod sidebar_popover;
mod sidebar_rail;
mod sidebar_reorder;
mod sidebar_rows;
mod sidebar_sections;
mod sidebar_trash;
mod sidebar_view;
mod sidebar_width;
mod unlock_argument;
mod volume_password;

pub(super) use appearance_menu::build_appearance_menu;
use device_actions::{MediaRelease, media_release_error_title};
pub(crate) use device_lock::crypto_password_uuid_for_volume;
#[cfg(test)]
pub(super) use devices::offer_removable_destinations_for_test;
pub(super) use devices::{
    RemovableDestination, RemovableWatch, removable_destination_root, removable_destinations,
    send_to_recent_folders, watch_removable_destinations,
};
pub(crate) use gio_devices::gio_volume_is_encrypted;
pub(super) use gio_devices::gio_volume_unix_device;
use gio_devices::orphaned_password_drives;
use key_routing::{
    MouseHistoryAction, SinglePaneArrow, TypeToSearch, mouse_history_action,
    sidebar_focus_direction, single_pane_arrow_action,
};
pub(super) use key_routing::{
    TypeToSearchQuery, apply_browser_mode, browser_mode_for_digit, chooser_keys,
    is_browser_navigation_key, type_to_search_query,
};
pub(in crate::ui) use keyboard::{ChooserKeys, ChooserPolicy, edit_focused_text};
pub(super) use launch::present_target;
use launch::{
    bind_update_notice_preferences, browser_for_window, open_checked, open_default_folder,
    schedule_after_first_paint,
};
pub(crate) use launch::{default_save_folder, home_directory};
pub use launch::{present, present_reveal};
use modal_layers::topmost_modal_layer;
pub(super) use modal_layers::{
    foreign_modal_visible, install_modal_focus_trap, visible_modal_layer,
};
pub use open_argument::present_open;
use places::resolve_place_order;
pub(super) use sidebar::build_sidebar;
use sidebar_context_menu::{PlaceMenuExtra, sidebar_context_option};
use sidebar_devices::sidebar_device_row;
use sidebar_file_drop::install_sidebar_file_drop;
use sidebar_pins::pin_status;
use sidebar_places::RecentAvailability;
use sidebar_rows::select_sidebar_row;
use sidebar_sections::{CollapsibleSection, PinnedSection};
use sidebar_trash::{TrashContents, TrashMenuRows, event_changes_trash_contents};
pub(super) use sidebar_view::SidebarView;
use sidebar_width::animate_sidebar;
pub(super) use sidebar_width::{
    MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH, bind_sidebar_text_size, preferred_sidebar_width,
    sidebar_rail_width,
};
pub(in crate::ui) use sidebar_width::{RailPolicy, SidebarFit, sidebar_fits_without_preview};
pub use unlock_argument::{UnlockTarget, present_unlock};

// Test modules reach these through `super::`.
#[cfg(all(test, feature = "qa"))]
use sidebar_rail::sync_sidebar_button;
#[cfg(test)]
use {
    super::{browser::PinStatus, browser_modes::BrowserMode, preferences::PreferenceManager},
    device_actions::DeviceRowActions,
    device_menu::attach_device_actions_menu,
    gtk::prelude::*,
    sidebar_devices::sidebar_eject_button,
    sidebar_pins::{move_pinned_place, pin_gap_index, pin_locations_at},
    sidebar_reorder::sidebar_reorder_matches,
    sidebar_rows::sidebar_button,
    std::time::{Duration, Instant},
};

type RailListener = Rc<dyn Fn(bool)>;

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

#[cfg(test)]
mod tests;
