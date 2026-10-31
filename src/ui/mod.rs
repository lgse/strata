// SPDX-License-Identifier: MIT

mod accessibility;
mod actions;
mod blur;
mod browser;
mod browser_modes;
mod chooser;
mod close_guard;
mod collection_edit;
mod collection_interaction;
mod controls;
mod decoration;
mod desktop_integration;
mod document_media;
mod document_view;
mod enter_target;
mod entry_list_model;
mod file_providers;
mod focus_navigation;
mod folder_picker;
mod frame;
mod icons_cell;
mod inline_search;
mod input_ownership;
mod jobs;
mod keys;
mod loading_skeleton;
mod marquee;
mod media;
mod missing_tools;
mod modal;
mod motion;
mod notice;
mod open_with;
mod pointer;
mod portal_preferences;
pub(crate) mod preferences;
mod presence;
mod preview;
mod progress_dock;
mod raw_details;
mod recent_apps;
mod resize_feedback;
mod scrolling;
mod search;
mod search_session;
mod settings;
mod shortcut_footer;
mod shortcut_reference;
mod sidebar_toggle;
mod table_view;
mod tabs_session;
mod tenxer_mode;
mod terminal;
mod theme;
mod thumbnail;
pub(crate) mod thumbnail_cache;
mod top_bar_navigation;
mod udiskie_preferences;
mod virtual_preview;
mod window;

#[cfg(feature = "qa")]
pub(crate) use accessibility::qa as accessibility_qa;
pub(crate) use chooser::{cancel_chooser, present_chooser};
#[cfg(feature = "qa")]
pub(crate) use enter_target::CSS_CLASS as ENTER_TARGET_CLASS;
pub(crate) use settings::schedule_rollback_cleanup;
pub(crate) use window::default_save_folder;
#[cfg(test)]
pub(in crate::ui) use window::offer_removable_destinations_for_test;
pub(in crate::ui) use window::{
    RemovableDestination, RemovableWatch, removable_destination_root, removable_destinations,
    send_to_recent_folders, watch_removable_destinations,
};
pub use window::{UnlockTarget, present, present_open, present_reveal, present_unlock};
#[cfg(feature = "qa")]
pub(crate) use {inline_search::SEARCH_RESULTS_LABEL, preview::PREVIEW_LABEL};

pub(crate) fn prepare_portal_ui() {
    let _theme = theme::ThemeManager::shared();
}

pub(crate) fn read_install_facts() {
    let _startup = crate::util::ui_thread::allow(crate::util::ui_thread::Exception::Startup);
    crate::services::InstallSource::detect();
    open_with::uses_portal();
}
