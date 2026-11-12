// SPDX-License-Identifier: MIT

//! Search, filtering, keyboard navigation and click behaviour.

use super::*;

impl PreferenceManager {
    pub fn single_click_previews(&self) -> bool {
        self.preferences.borrow().single_click_previews
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.preferences.borrow_mut().single_click_previews = enabled;
        self.save_preferences();
    }

    pub fn search_open_files_directly(&self) -> bool {
        self.preferences.borrow().search_open_files_directly
    }

    pub fn set_search_open_files_directly(&self, enabled: bool) {
        self.preferences.borrow_mut().search_open_files_directly = enabled;
        self.save_preferences();
    }

    pub fn search_exclusions(&self) -> Vec<String> {
        self.preferences.borrow().search_exclusions.clone()
    }

    pub fn set_search_exclusions(&self, exclusions: Vec<String>) {
        self.preferences.borrow_mut().search_exclusions = exclusions;
        self.save_preferences();
    }

    /// Seconds one search walk may index; 0 means no limit.
    pub fn search_time_limit(&self) -> u32 {
        self.preferences.borrow().search_time_limit
    }

    pub fn set_search_time_limit(&self, secs: u32) {
        self.preferences.borrow_mut().search_time_limit =
            normalized_search_limit(secs, &SEARCH_TIME_LIMIT_CHOICES);
        self.save_preferences();
    }

    /// Files and folders one search walk may index; 0 means no limit.
    pub fn search_entry_limit(&self) -> u32 {
        self.preferences.borrow().search_entry_limit
    }

    pub fn set_search_entry_limit(&self, entries: u32) {
        self.preferences.borrow_mut().search_entry_limit =
            normalized_search_limit(entries, &SEARCH_ENTRY_LIMIT_CHOICES);
        self.save_preferences();
    }

    pub fn search_limits(&self) -> SearchLimits {
        let preferences = self.preferences.borrow();
        SearchLimits {
            time_budget: (preferences.search_time_limit != 0)
                .then(|| Duration::from_secs(u64::from(preferences.search_time_limit))),
            max_entries: (preferences.search_entry_limit != 0)
                .then(|| preferences.search_entry_limit as usize),
        }
    }

    pub fn filter_include_subfolders(&self) -> bool {
        self.preferences.borrow().filter_include_subfolders
    }

    pub fn set_filter_include_subfolders(&self, enabled: bool) {
        self.preferences.borrow_mut().filter_include_subfolders = enabled;
        self.save_preferences();
    }

    pub fn type_to_search(&self) -> bool {
        self.preferences.borrow().type_to_search
    }

    pub fn set_type_to_search(&self, enabled: bool) {
        self.preferences.borrow_mut().type_to_search = enabled;
        self.save_preferences();
    }

    pub fn arrow_navigation_scoped(&self) -> bool {
        self.preferences.borrow().arrow_navigation_scoped
    }

    pub fn set_arrow_navigation_scoped(&self, scoped: bool) {
        self.preferences.borrow_mut().arrow_navigation_scoped = scoped;
        self.save_preferences();
    }

    pub fn tenxer_mode(&self) -> bool {
        self.preferences.borrow().tenxer_mode
    }

    pub fn set_tenxer_mode(&self, enabled: bool) {
        self.preferences.borrow_mut().tenxer_mode = enabled;
        self.save_preferences();
    }

    pub fn type_to_search_active(&self) -> bool {
        self.type_to_search() && !self.tenxer_mode()
    }

    pub fn arrow_navigation_scoped_active(&self) -> bool {
        self.tenxer_mode() || self.arrow_navigation_scoped()
    }

    pub fn show_keybinding_hints(&self) -> bool {
        self.preferences.borrow().show_keybinding_hints
    }

    pub fn set_show_keybinding_hints(&self, enabled: bool) {
        self.preferences.borrow_mut().show_keybinding_hints = enabled;
        self.save_preferences();
    }

    pub fn on_keybinding_hints_changed(
        &self,
        anchor: &impl IsA<gtk::Widget>,
        refresh: impl Fn(&gtk::Widget, bool) + 'static,
    ) {
        self.bind_preference(anchor, Self::show_keybinding_hints, refresh);
    }

    pub fn group_by_type(&self) -> bool {
        self.preferences.borrow().group_by_type
    }

    pub fn set_group_by_type(&self, enabled: bool) {
        self.preferences.borrow_mut().group_by_type = enabled;
        self.save_preferences();
    }

    pub fn click_activation(
        &self,
        mode: super::super::browser_modes::BrowserMode,
    ) -> super::super::browser_modes::ClickActivation {
        use super::super::browser_modes::{BrowserMode, ClickActivation, ClickCount};

        let preferences = self.preferences.borrow();
        let (files, folders) = match mode {
            BrowserMode::Columns => (
                preferences.columns_file_clicks,
                preferences.columns_folder_clicks,
            ),
            // Compact shares the Icons click settings.
            BrowserMode::Icons | BrowserMode::Compact => (
                preferences.icons_file_clicks,
                preferences.icons_folder_clicks,
            ),
            BrowserMode::List => (preferences.list_file_clicks, preferences.list_folder_clicks),
        };
        let defaults = ClickActivation::default_for(mode);
        ClickActivation {
            files: ClickCount::from_stored(files).unwrap_or(defaults.files),
            folders: ClickCount::from_stored(folders).unwrap_or(defaults.folders),
        }
    }

    pub fn set_click_activation(
        &self,
        mode: super::super::browser_modes::BrowserMode,
        activation: super::super::browser_modes::ClickActivation,
    ) {
        use super::super::browser_modes::BrowserMode;

        let mut preferences = self.preferences.borrow_mut();
        let files = activation.files.stored();
        let folders = activation.folders.stored();
        match mode {
            BrowserMode::Columns => {
                preferences.columns_file_clicks = files;
                preferences.columns_folder_clicks = folders;
            }
            BrowserMode::Icons | BrowserMode::Compact => {
                preferences.icons_file_clicks = files;
                preferences.icons_folder_clicks = folders;
            }
            BrowserMode::List => {
                preferences.list_file_clicks = files;
                preferences.list_folder_clicks = folders;
            }
        }
        drop(preferences);
        self.save_preferences();
    }
}
