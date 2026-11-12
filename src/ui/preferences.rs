// SPDX-License-Identifier: MIT

// The tests and fixtures under this module import through it.
use std::{
    cell::{Cell, RefCell},
    path::{Component, Path, PathBuf},
    rc::Rc,
    time::Duration,
};
#[cfg(test)]
use std::{collections::HashMap, fs};

use gtk::{glib, prelude::*};
// The schema, its defaults and stored forms, and the reading of the file live in
// strata-desktop, where the Rune interface reads them too. This module keeps what needs GTK.
use strata_desktop::settings::folder_views;
#[cfg(test)]
use strata_desktop::settings::read_preferences;
pub use strata_desktop::settings::{
    CornerRoundness, ListColumnId, ListColumns, TextSize, default_list_column_order,
};
pub(crate) use strata_desktop::settings::{
    FileIconColors, InterfaceRenderer, OmarchyVariant, UnitSystem, sanitize_list_column_order,
    sanitize_list_hidden_columns,
};
use strata_desktop::settings::{FinishedSave, PendingSave, SaveOutcome, SaveProblem, Saving};
use strata_desktop::settings::{
    KeymapOverrides, MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE,
    configured_hardware_acceleration, configured_video_preview_backend, lock_saves,
    normalized_font_family, normalized_search_limit, normalized_video_start, normalized_volume,
    parse_preferences, read_state_file, settings_path, sort_preferences, store_sort,
};
pub(in crate::ui) use strata_desktop::settings::{
    MAX_PREVIEW_VIDEO_START, Preferences, SEARCH_ENTRY_LIMIT_CHOICES, SEARCH_TIME_LIMIT_CHOICES,
};
#[cfg(test)]
use strata_desktop::settings::{RETIRED_SETTINGS, default_sidebar_order, salvage_preferences};

use crate::{
    model::{FolderColorValue, SortKey, ViewPreferences},
    sandbox::MediaPreviewBackend,
    services::{Channel, CrossVolumeDropStrategy, SearchLimits},
    storage::file_stamp,
};

use super::browser_modes::DensityScale;

pub(in crate::ui) const SEND_TO_RECENT_DESTINATIONS_LIMIT: usize = 3;

mod appearance_settings;
mod bindings;
mod desktop_scale;
mod file_settings;
#[cfg(test)]
pub(in crate::ui) mod fixtures;
mod folder_view_settings;
mod input_settings;
mod item_customizations;
mod keymap;
mod layout_settings;
mod persistence;
mod preview_settings;
mod save_notice;
mod sidebar_settings;
mod stored;
mod text_size;
mod trashed;
pub(in crate::ui) use bindings::notify_live;
#[cfg(test)]
use desktop_scale::text_scale_factor_from_xft_dpi;
pub(in crate::ui) use desktop_scale::{desktop_text_scale_factor, snapped_root_font_px};
pub(crate) use folder_view_settings::flush_pending_folder_views;
use stored::{browser_mode_from_stored, store_density, stored_browser_mode, stored_density};
pub(in crate::ui) use text_size::TextSizeShortcut;

thread_local! {
    static SHARED_MANAGER: RefCell<std::rc::Weak<PreferenceManager>> = const { RefCell::new(std::rc::Weak::new()) };
}

pub(in crate::ui) fn is_valid_send_to_relative_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_)))
        && components.all(|component| matches!(component, Component::Normal(_)))
}

pub struct PreferenceManager {
    preferences: RefCell<Preferences>,
    changes: bindings::PreferenceChanges,
    /// What a save writes and whether one is owed; shared with the Rune interface.
    saving: RefCell<Saving>,
    this: std::rc::Weak<Self>,
    save_notices: save_notice::SaveNotices,
    folder_views: std::cell::OnceCell<folder_view_settings::FolderViewStore>,
    folder_sorts_revision: Cell<u64>,
    folder_save_notices: save_notice::SaveNotices,
    item_customizations: item_customizations::ItemCustomizationState,
    keymap: RefCell<Option<Rc<crate::ui::keys::Keymap>>>,
    keymap_revision: Cell<u64>,
    active_omarchy_theme: RefCell<Option<String>>,
}

impl PreferenceManager {
    pub fn shared() -> Rc<Self> {
        SHARED_MANAGER.with(|shared| {
            if let Some(manager) = shared.borrow().upgrade() {
                return manager;
            }
            let manager = Self::load();
            shared.replace(Rc::downgrade(&manager));
            manager
        })
    }

    /// The shared manager while something holds it, without loading settings.
    pub(crate) fn existing() -> Option<Rc<Self>> {
        SHARED_MANAGER.with(|shared| shared.borrow().upgrade())
    }

    fn load() -> Rc<Self> {
        let (preferences, saving) = strata_desktop::settings::load();
        super::motion::set_reduce_motion(preferences.reduce_motion);
        crate::util::set_date_format(crate::util::DateFormat::parse(&preferences.date_format));
        rust_i18n::set_locale(saving.startup_locale());
        let manager = Rc::new_cyclic(|this| Self {
            changes: bindings::PreferenceChanges::new(preferences.clone()),
            saving: RefCell::new(saving),
            this: this.clone(),
            save_notices: save_notice::SaveNotices::default(),
            folder_views: std::cell::OnceCell::new(),
            folder_sorts_revision: Cell::new(0),
            folder_save_notices: save_notice::SaveNotices::for_folder_views(),
            item_customizations: Default::default(),
            keymap: RefCell::new(None),
            keymap_revision: Cell::new(0),
            active_omarchy_theme: RefCell::new(None),
            preferences: RefCell::new(preferences),
        });
        manager.folder_view_store();
        manager
    }

    /// Repairs loaded values whose validity depends on data the preference store
    /// does not own (theme catalog membership and Omarchy availability). The
    /// caller runs this once during startup, before appearance changes apply.
    pub(in crate::ui) fn normalize_loaded_theme_selection(
        &self,
        theme_is_known: impl Fn(&str) -> bool,
        system_available: bool,
        omarchy_available: bool,
    ) {
        let mut preferences = self.preferences.borrow_mut();
        if !theme_is_known(&preferences.theme) {
            preferences.theme = "azure-glow".to_owned();
        }
        let mode_available = match preferences.mode.as_str() {
            "theme" => true,
            "system" => system_available,
            "omarchy" => omarchy_available,
            _ => false,
        };
        if !mode_available {
            preferences.mode = "theme".to_owned();
        } else if !settings_path().is_file() && omarchy_available {
            preferences.mode = "omarchy".to_owned();
        }
        self.saving.borrow_mut().rebase(&preferences);
        self.changes.reset_baseline(&preferences);
    }

    /// Applies the current value immediately, then only changes to that value.
    /// Capture weak references to owned widgets/state; the anchor owns the binding's lifetime.
    pub(crate) fn bind_preference<T: PartialEq + Clone + 'static>(
        &self,
        anchor: &impl IsA<gtk::Widget>,
        read: impl Fn(&Self) -> T + 'static,
        apply: impl Fn(&gtk::Widget, T) + 'static,
    ) {
        self.changes.bind(self, anchor, read, apply);
    }

    /// A closed window can outlive its close while its own closures still
    /// reference it, so its bindings end here rather than on destroy.
    pub(in crate::ui) fn release_bindings_within(&self, root: &impl IsA<gtk::Widget>) {
        self.changes.release_within(root.as_ref());
    }

    #[cfg(test)]
    pub(in crate::ui) fn listener_count(&self) -> usize {
        self.changes.listener_count()
    }

    /// Registers a process-lifetime callback invoked after preference changes are
    /// published. Observers run before widget bindings.
    pub(in crate::ui) fn observe(&self, observer: Rc<dyn Fn()>) {
        self.changes.observe(observer);
    }

    /// Lets `window` show the notice when changes cannot be saved. Windows that
    /// never register, such as the portal chooser, leave failures to the log.
    pub(in crate::ui) fn register_save_notice_window(&self, window: &gtk::Window) {
        self.save_notices.register(window);
        self.folder_save_notices.register(window);
    }

    /// Republishes current preferences without a stored change, for runtime state
    /// that bindings derive from preferences (such as theme availability).
    pub(in crate::ui) fn notify_changes(&self) {
        self.changes.notify(self);
    }
}

pub(in crate::ui) fn config_directory() -> PathBuf {
    crate::storage::config_directory()
}

#[cfg(test)]
mod tests;
