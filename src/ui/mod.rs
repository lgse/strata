// SPDX-License-Identifier: MIT

mod accessibility;
mod blur;
mod browser;
mod browser_modes;
mod chooser;
mod controls;
mod desktop_integration;
mod document_media;
mod document_view;
mod entry_list_model;
mod focus_navigation;
mod icons_cell;
mod inline_search;
mod input_ownership;
mod loading_skeleton;
mod marquee;
mod media;
mod modal;
mod motion;
mod open_with;
mod pointer;
mod portal_preferences;
mod preview;
mod scrolling;
mod search;
mod settings;
mod shortcut_footer;
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
pub use window::{UnlockTarget, present, present_open, present_reveal, present_unlock};

pub(crate) fn prepare_portal_ui() {
    let _theme = theme::ThemeManager::shared();
    window::load_styles();
}
