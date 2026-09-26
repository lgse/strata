// SPDX-License-Identifier: MIT

mod accessibility;
mod actions;
mod blur;
mod browser;
mod browser_modes;
mod chooser;
mod collection_edit;
mod collection_interaction;
mod controls;
mod desktop_integration;
mod document_media;
mod document_view;
mod entry_list_model;
mod focus_navigation;
mod frame;
mod icons_cell;
mod inline_search;
mod input_ownership;
mod jobs;
mod loading_skeleton;
mod marquee;
mod media;
mod modal;
mod motion;
mod open_with;
mod pointer;
mod portal_preferences;
pub(crate) mod preferences;
mod preview;
mod raw_details;
mod scrolling;
mod search;
mod search_session;
mod settings;
mod shortcut_footer;
mod table_view;
mod terminal;
mod theme;
mod thumbnail;
pub(crate) mod thumbnail_cache;
mod top_bar_navigation;
mod udiskie_preferences;
mod virtual_preview;
mod window;

pub(crate) use chooser::{cancel_chooser, present_chooser};
pub(crate) use window::default_save_folder;
pub(in crate::ui) use window::{
    RemovableDestination, removable_destinations, resolve_removable_destination,
};
pub use window::{UnlockTarget, present, present_open, present_reveal, present_unlock};

pub(crate) fn prepare_portal_ui() {
    let _theme = theme::ThemeManager::shared();
    window::load_styles();
}
