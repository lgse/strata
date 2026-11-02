// SPDX-License-Identifier: MIT

//! The sidebar: place order and visibility, and section state.

use super::*;

impl PreferenceManager {
    pub fn sidebar_order(&self) -> Vec<String> {
        self.preferences.borrow().sidebar_order.clone()
    }

    pub fn set_sidebar_order(&self, order: Vec<String>) {
        self.preferences.borrow_mut().sidebar_order = order;
        self.save_preferences();
    }

    pub fn sidebar_show_home(&self) -> bool {
        self.preferences.borrow().sidebar_show_home
    }

    pub fn set_sidebar_show_home(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_home = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_trash(&self) -> bool {
        self.preferences.borrow().sidebar_show_trash
    }

    pub fn set_sidebar_show_trash(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_trash = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_network(&self) -> bool {
        self.preferences.borrow().sidebar_show_network
    }

    pub fn set_sidebar_show_network(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_network = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_recent(&self) -> bool {
        self.preferences.borrow().sidebar_show_recent
    }

    pub fn set_sidebar_show_recent(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_recent = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_desktop(&self) -> bool {
        self.preferences.borrow().sidebar_show_desktop
    }

    pub fn set_sidebar_show_desktop(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_desktop = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_documents(&self) -> bool {
        self.preferences.borrow().sidebar_show_documents
    }

    pub fn set_sidebar_show_documents(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_documents = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_downloads(&self) -> bool {
        self.preferences.borrow().sidebar_show_downloads
    }

    pub fn set_sidebar_show_downloads(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_downloads = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_music(&self) -> bool {
        self.preferences.borrow().sidebar_show_music
    }

    pub fn set_sidebar_show_music(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_music = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_pictures(&self) -> bool {
        self.preferences.borrow().sidebar_show_pictures
    }

    pub fn set_sidebar_show_pictures(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_pictures = visible;
        self.save_preferences();
    }

    pub fn sidebar_show_videos(&self) -> bool {
        self.preferences.borrow().sidebar_show_videos
    }

    pub fn set_sidebar_show_videos(&self, visible: bool) {
        self.preferences.borrow_mut().sidebar_show_videos = visible;
        self.save_preferences();
    }

    pub fn sidebar_expanded(&self) -> bool {
        self.preferences.borrow().sidebar_expanded
    }

    pub fn set_sidebar_expanded(&self, expanded: bool) {
        self.preferences.borrow_mut().sidebar_expanded = expanded;
        self.save_preferences();
    }

    pub fn sidebar_pinned_collapsed(&self) -> bool {
        self.preferences.borrow().sidebar_pinned_collapsed
    }

    pub fn set_sidebar_pinned_collapsed(&self, collapsed: bool) {
        self.preferences.borrow_mut().sidebar_pinned_collapsed = collapsed;
        self.save_preferences();
    }

    pub fn sidebar_devices_collapsed(&self) -> bool {
        self.preferences.borrow().sidebar_devices_collapsed
    }

    pub fn set_sidebar_devices_collapsed(&self, collapsed: bool) {
        self.preferences.borrow_mut().sidebar_devices_collapsed = collapsed;
        self.save_preferences();
    }

    pub fn sidebar_compact(&self) -> bool {
        self.preferences.borrow().sidebar_compact
    }

    pub fn set_sidebar_compact(&self, compact: bool) {
        self.preferences.borrow_mut().sidebar_compact = compact;
        self.save_preferences();
    }

    pub fn sidebar_places_visibility(&self) -> [bool; 10] {
        let preferences = self.preferences.borrow();
        [
            preferences.sidebar_show_home,
            preferences.sidebar_show_trash,
            preferences.sidebar_show_network,
            preferences.sidebar_show_recent,
            preferences.sidebar_show_desktop,
            preferences.sidebar_show_documents,
            preferences.sidebar_show_downloads,
            preferences.sidebar_show_music,
            preferences.sidebar_show_pictures,
            preferences.sidebar_show_videos,
        ]
    }
}
