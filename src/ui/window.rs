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
mod sidebar_sections;
mod sidebar_width;
mod unlock_argument;
mod volume_password;
mod sidebar_trash;
mod sidebar_view;
mod sidebar_rail;
mod sidebar_places;
mod sidebar_pins;
mod sidebar_reorder;
mod sidebar_rows;
mod sidebar_context_menu;
mod sidebar_file_drop;
mod sidebar_devices;
mod device_actions;
mod device_lock;
mod device_menu;

pub(super) use appearance_menu::build_appearance_menu;
pub(crate) use gio_devices::gio_volume_is_encrypted;
pub(super) use gio_devices::gio_volume_unix_device;
use gio_devices::{gio_mount_is_encrypted, gio_volume_is_locked, orphaned_password_drives};
pub(super) use modal_layers::{foreign_modal_visible, install_modal_focus_trap, visible_modal_layer};
use modal_layers::topmost_modal_layer;
use sidebar_sections::{CollapsibleSection, PinnedSection, SectionHeader, SidebarSection};
use sidebar_trash::{TrashMenuRows, TrashContents, TrashMenuVisibility, trash_menu_visibility, sync_trash_menu_rows, trash_contents_from_probe, event_changes_trash_contents, trash_has_items, trash_file_drop_action, install_sidebar_trash_drop, sidebar_trash_feedback};
pub(super) use sidebar_view::{SidebarView};
use sidebar_view::{unparent_sidebar_popovers};
use sidebar_rail::{sync_sidebar_button, paned_separator};
use sidebar_places::{RecentAvailability, should_show_recent_place};
use sidebar_pins::{pin_status, remove_pinned_place, PinGap, pin_gap_index, move_pinned_place, pin_locations_at};
use sidebar_reorder::{PINNED_DRAG_PREFIX, SidebarReorder, accepts_sidebar_reorder_payload, sidebar_reorder_matches, sidebar_reorder_feedback, reorder_places, reorder_pinned_places, parse_pinned_drag_source};
use sidebar_rows::{sync_keycap, reparent_in_order, select_sidebar_row, deselect_sidebar_rows, set_sidebar_row_active, sidebar_row_button, sidebar_button, customize_sidebar_folder_icon};
use sidebar_context_menu::{PlaceMenuExtra, sidebar_context_option};
use sidebar_file_drop::{sidebar_accepts_file_drop, install_sidebar_file_drop};
use sidebar_devices::{sidebar_device_shell, is_smb_location, sidebar_device_row, sidebar_eject_button, sidebar_lock_button};
use device_actions::{MediaRelease, EncryptedMediaAction, DeviceRowActions, device_row_actions, volume_release_action, media_release_label, media_release_error_title, release_kind, release_unplug, navigate_home_if_within, begin_media_release, release_device_volume, release_device_mount};
pub(crate) use device_lock::{crypto_password_uuid_for_volume};
use device_lock::{lock_encrypted_volume, lock_encrypted_mount, LockTarget, lock_target, crypto_password_uuid_for_mount, request_encrypted_lock_showing, continue_encrypted_lock, confirm_forget_cached_password, password_stop_drive, lock_cleartext_then_close, eject_password_drive};
use device_menu::{attach_device_actions_menu};
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
