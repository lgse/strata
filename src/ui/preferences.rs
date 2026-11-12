// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
    fs, io,
    path::{Component, Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{glib, prelude::*};
use serde::{Deserialize, Serialize};

use crate::{
    model::{FolderColorValue, SortDirection, SortKey, ViewPreferences},
    sandbox::MediaPreviewBackend,
    services::{Channel, CrossVolumeDropStrategy, SearchLimits},
    storage::{
        file_stamp,
        writer::{self, Policy},
    },
};

use super::browser_modes::DensityScale;
use super::icons_cell::{MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE};

pub(in crate::ui) const SEND_TO_RECENT_DESTINATIONS_LIMIT: usize = 3;

mod appearance_settings;
mod bindings;
mod corner_roundness;
mod defaults;
mod desktop_scale;
mod file_settings;
#[cfg(test)]
pub(in crate::ui) mod fixtures;
mod folder_view_settings;
mod folder_views;
mod input_settings;
mod item_customizations;
mod keymap;
mod layout_settings;
mod list_columns;
mod persistence;
mod preview_settings;
mod save_notice;
mod sidebar_settings;
mod stored;
mod text_size;
mod trashed;
pub(in crate::ui) use bindings::notify_live;
pub use corner_roundness::CornerRoundness;
use defaults::{
    default_browser_density, default_browser_mode, default_cross_volume_drop_strategy,
    default_date_format, default_density_custom_px, default_double_clicks, default_enabled,
    default_file_clicks, default_folder_clicks, default_frame_density, default_full_volume,
    default_icons_thumbnail_size, default_release_channel, default_search_entry_limit,
    default_search_time_limit, default_sidebar_order, default_sort_direction, default_sort_key,
    default_video_preview_backend,
};
#[cfg(test)]
use desktop_scale::text_scale_factor_from_xft_dpi;
pub(in crate::ui) use desktop_scale::{desktop_text_scale_factor, snapped_root_font_px};
pub(crate) use folder_view_settings::flush_pending_folder_views;
use keymap::KeymapOverrides;
use list_columns::is_default_list_column_order;
pub use list_columns::{ListColumnId, ListColumns, default_list_column_order};
pub(crate) use list_columns::{sanitize_list_column_order, sanitize_list_hidden_columns};
#[cfg(test)]
use persistence::read_preferences;
#[cfg(test)]
use persistence::{RETIRED_SETTINGS, salvage_preferences};
use persistence::{
    SettingKey, parse_preferences, read_loaded_preferences, read_state_file, record_changes,
    settings_path,
};
pub(in crate::ui) use stored::{
    MAX_PREVIEW_VIDEO_START, SEARCH_ENTRY_LIMIT_CHOICES, SEARCH_TIME_LIMIT_CHOICES,
};
use stored::{
    browser_mode_from_stored, configured_hardware_acceleration, configured_video_preview_backend,
    normalized_search_limit, normalized_video_start, normalized_volume, sort_preferences,
    store_density, store_sort, stored_browser_mode, stored_density,
};
pub use text_size::TextSize;

thread_local! {
    static SHARED_MANAGER: RefCell<std::rc::Weak<PreferenceManager>> = const { RefCell::new(std::rc::Weak::new()) };
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InterfaceRenderer {
    Cairo,
    #[default]
    System,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OmarchyVariant {
    #[serde(alias = "original")]
    Lighter,
    #[default]
    Darker,
    #[serde(alias = "high_contrast")]
    Contrast,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FileIconColors {
    #[default]
    Brand,
    Theme,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnitSystem {
    #[default]
    Metric,
    Imperial,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub(in crate::ui) struct Preferences {
    mode: String,
    theme: String,
    /// Pre-per-theme global choice, migrated to the active Omarchy theme.
    #[serde(default, rename = "omarchy_variant", skip_serializing)]
    legacy_omarchy_variant: Option<OmarchyVariant>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    omarchy_variants: HashMap<String, OmarchyVariant>,
    #[serde(default)]
    folder_peeking: bool,
    #[serde(default = "default_enabled")]
    single_click_previews: bool,
    #[serde(default = "default_enabled")]
    columns_mirror_selection: bool,
    #[serde(default = "default_enabled")]
    columns_show_sizes: bool,
    #[serde(default = "default_enabled")]
    render_documents_by_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hardware_accelerated_video_previews: Option<bool>,
    #[serde(default = "default_video_preview_backend")]
    video_preview_backend: String,
    #[serde(default)]
    search_open_files_directly: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    search_exclusions: Vec<String>,
    #[serde(default = "default_search_time_limit")]
    search_time_limit: u32,
    #[serde(default = "default_search_entry_limit")]
    search_entry_limit: u32,
    #[serde(default = "default_enabled")]
    type_to_search: bool,
    #[serde(default)]
    arrow_navigation_scoped: bool,
    #[serde(default)]
    tenxer_mode: bool,
    #[serde(default = "default_enabled")]
    filter_include_subfolders: bool,
    #[serde(default = "default_enabled")]
    show_keybinding_hints: bool,
    #[serde(default = "default_enabled")]
    window_show_close: bool,
    #[serde(default)]
    window_show_minimize: bool,
    #[serde(default)]
    window_show_maximize: bool,
    #[serde(default)]
    reduce_motion: bool,
    #[serde(default = "default_enabled")]
    element_glow: bool,
    #[serde(default)]
    file_icon_colors: FileIconColors,
    #[serde(default = "default_browser_mode")]
    browser_mode: String,
    #[serde(default = "default_browser_density")]
    browser_density: String,
    #[serde(default = "default_density_custom_px")]
    browser_density_custom_px: u8,
    #[serde(default = "default_frame_density")]
    frame_density: String,
    #[serde(default = "default_density_custom_px")]
    frame_density_custom_px: u8,
    #[serde(default)]
    group_by_type: bool,
    #[serde(default)]
    compact_full_names: bool,
    #[serde(default = "default_file_clicks", rename = "list_file_clicks")]
    columns_file_clicks: u8,
    #[serde(default = "default_folder_clicks", rename = "list_folder_clicks")]
    columns_folder_clicks: u8,
    #[serde(default = "default_file_clicks", rename = "grid_file_clicks")]
    icons_file_clicks: u8,
    #[serde(default = "default_double_clicks", rename = "grid_folder_clicks")]
    icons_folder_clicks: u8,
    #[serde(default = "default_file_clicks", rename = "explorer_file_clicks")]
    list_file_clicks: u8,
    #[serde(default = "default_double_clicks", rename = "explorer_folder_clicks")]
    list_folder_clicks: u8,
    #[serde(default = "default_sidebar_order")]
    sidebar_order: Vec<String>,
    #[serde(default = "default_enabled")]
    sidebar_show_home: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_trash: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_network: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_recent: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_desktop: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_documents: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_downloads: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_music: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_pictures: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_videos: bool,
    #[serde(default = "default_enabled")]
    sidebar_expanded: bool,
    #[serde(default)]
    sidebar_pinned_collapsed: bool,
    #[serde(default)]
    sidebar_devices_collapsed: bool,
    #[serde(default)]
    sidebar_compact: bool,
    #[serde(default)]
    show_hidden: bool,
    #[serde(default)]
    text_size: TextSize,
    #[serde(default)]
    corner_roundness: CornerRoundness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interface_font: Option<String>,
    #[serde(default)]
    interface_renderer: InterfaceRenderer,
    #[serde(default)]
    language: crate::i18n::Language,
    #[serde(default = "default_enabled")]
    folders_first: bool,
    #[serde(default = "default_sort_key")]
    sort_key: String,
    #[serde(default = "default_sort_direction")]
    sort_direction: String,
    #[serde(default = "defaults::default_natural_sort")]
    natural_sort: bool,
    #[serde(default)]
    case_sensitive_sort: bool,
    #[serde(default = "default_enabled")]
    check_for_updates: bool,
    #[serde(default)]
    preview_muted: bool,
    #[serde(default = "default_full_volume")]
    preview_volume: f64,
    #[serde(default = "default_enabled")]
    preview_text_wrap: bool,
    #[serde(default)]
    unit_system: UnitSystem,
    #[serde(default)]
    preview_autoplay: bool,
    #[serde(default)]
    preview_video_start: u32,
    #[serde(default = "default_enabled")]
    watch_network_folders: bool,
    #[serde(default = "crate::sandbox::browser::default_worker_limit")]
    thumbnail_workers: usize,
    #[serde(default = "default_enabled")]
    show_thumbnails: bool,
    #[serde(default)]
    stl_thumbnails: bool,
    #[serde(default = "default_icons_thumbnail_size")]
    icons_thumbnail_size: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chooser_column_width: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser_column_width: Option<i32>,
    #[serde(default = "default_cross_volume_drop_strategy")]
    cross_volume_drop_strategy: String,
    #[serde(default)]
    open_folder_after_drop: bool,
    #[serde(default = "default_date_format")]
    date_format: String,
    #[serde(default = "default_release_channel")]
    release_channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_directory: Option<PathBuf>,
    #[serde(default = "default_enabled")]
    restore_tabs: bool,
    #[serde(default = "default_enabled")]
    remember_folder_views: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    folder_colors: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    custom_icons: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    send_to_recent_destinations: HashMap<String, Vec<PathBuf>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    device_labels: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chooser_list_columns: Option<ListColumns>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser_list_columns: Option<ListColumns>,
    #[serde(
        default = "default_list_column_order",
        skip_serializing_if = "is_default_list_column_order"
    )]
    list_column_order: Vec<ListColumnId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    list_hidden_columns: Vec<ListColumnId>,
    #[serde(default, skip_serializing_if = "KeymapOverrides::is_empty")]
    keymap: KeymapOverrides,
}

pub(in crate::ui) fn is_valid_send_to_relative_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_)))
        && components.all(|component| matches!(component, Component::Normal(_)))
}

pub struct PreferenceManager {
    preferences: RefCell<Preferences>,
    startup_interface_renderer: InterfaceRenderer,
    startup_locale: &'static str,
    changes: bindings::PreferenceChanges,
    persistence_dirty: Cell<bool>,
    saves_requested: Cell<u64>,
    /// The settings as of the last load or queued save, to see what a save changes.
    last_queued: RefCell<toml::Table>,
    /// Changed or repaired since a save last succeeded; a save writes only these.
    dirty_keys: RefCell<BTreeSet<SettingKey>>,
    this: std::rc::Weak<Self>,
    /// Why saving is off for this session: the file existed but could not be read.
    load_failure: Option<io::Error>,
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
        writer::note(&settings_path(), file_stamp(&settings_path()));
        let (mut preferences, load_failure, salvaged) = read_loaded_preferences();
        let parsed = preferences.clone();
        preferences.normalize_loaded();
        super::motion::set_reduce_motion(preferences.reduce_motion);
        crate::util::set_date_format(crate::util::DateFormat::parse(&preferences.date_format));
        let saved_base = toml::Table::try_from(&preferences).unwrap_or_default();
        let mut repaired: BTreeSet<SettingKey> =
            salvaged.into_iter().map(|key| (key, None)).collect();
        record_changes(
            &mut repaired,
            &toml::Table::try_from(&parsed).unwrap_or_default(),
            &saved_base,
        );

        let startup_locale = preferences.language.locale();
        rust_i18n::set_locale(startup_locale);
        let manager = Rc::new_cyclic(|this| Self {
            startup_locale,
            startup_interface_renderer: preferences.interface_renderer,
            changes: bindings::PreferenceChanges::new(preferences.clone()),
            persistence_dirty: Cell::new(false),
            saves_requested: Cell::new(0),
            last_queued: RefCell::new(saved_base),
            dirty_keys: RefCell::new(repaired),
            this: this.clone(),
            load_failure,
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
        if let Ok(normalized) = toml::Table::try_from(&*preferences) {
            let mut last = self.last_queued.borrow_mut();
            record_changes(&mut self.dirty_keys.borrow_mut(), &last, &normalized);
            *last = normalized;
        }
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
