// SPDX-License-Identifier: MIT

mod bookmarks;
mod devices;
mod keyboard_dispatch;
mod keyboard_policy;
mod preferences;
mod sidebar_policy;
mod trash;
mod type_to_search;

use std::{cell::Cell, path::Path, rc::Rc};

use gtk::glib;

use crate::{
    app::BrowserEvent,
    model::Location,
    services::{BuildKind, ReleaseMetadata},
    test_support::gtk_test,
    ui::theme::ThemeManager,
};

use super::{
    BrowserMode, DEFAULT_ACCELS, EncryptedMediaAction, MediaRelease, MouseHistoryAction, PinStatus,
    SIDEBAR_WIDTH, STANDARD_PLACE_IDS, SidebarState, SinglePaneArrow, TrashContents,
    TrashMenuVisibility, TypeToSearchQuery, accepts_sidebar_reorder_payload, begin_media_release,
    browser_for_window, browser_mode_for_digit, build_sidebar, confirm_forget_cached_password,
    continue_encrypted_lock, device_row_actions, event_changes_trash_contents, gio_icon_names,
    home_directory, is_browser_navigation_key, is_context_menu_shortcut, is_open_terminal_shortcut,
    is_native_editing_shortcut, is_refresh_shortcut, is_rename_shortcut, is_sidebar_focus_shortcut, is_smb_location,
    is_standard_place_location, is_toggle_hidden_shortcut, is_undo_shortcut, jump_direction,
    load_pinned_places, media_release_label, mouse_history_action, page_direction,
    parse_pinned_drag_source, parse_pinned_places, pin_status, pinned_places_path,
    remove_pinned_place, reorder_pinned_places, reorder_places, request_encrypted_lock,
    resolve_place_order, select_sidebar_row, serialize_pinned_places, should_show_standard_place,
    sidebar_accepts_file_drop, sidebar_button, sidebar_device_row, sidebar_eject_button,
    sidebar_focus_direction, sidebar_lock_button, sidebar_update_label, single_pane_arrow_action,
    standard_place, trash_contents_from_probe, trash_has_entries, trash_menu_visibility,
    type_to_search_query, vim_focus_direction, volume_release_action,
};
