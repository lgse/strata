// SPDX-License-Identifier: MIT

//! The saved settings, `settings.toml`, without an interface: what the file holds and its
//! defaults, how values are stored and kept within range, and how the file is read, salvaged
//! and merged into on a save.
//!
//! The GTK application's `PreferenceManager` (`src/ui/preferences.rs`) owns a [`Preferences`]
//! and a [`Saving`], and adds what needs its toolkit: the setters, change notifications,
//! widget bindings and the notices about saving. The Rune interface holds one in `strata-app`'s `settings::Saved`,
//! which saves the few values that interface changes. `docs/preferences.md` describes each
//! setting.
//!
//! [`folder_views`] is the model of `folder-views.toml`, the sort and icon size a folder
//! remembers, and [`folder_views_path`], [`load_folder_views`] and [`save_folder_views`] find,
//! read and save that file. [`FolderViewStore`] holds the values a process has and the
//! bookkeeping of writing them; when a write is scheduled is each interface's.
//!
//! [`Saving`] is the bookkeeping of a save: an interface changes its [`Preferences`], commits,
//! and awaits the save the commit hands back.

use std::{
    collections::{BTreeSet, HashMap},
    fs, io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::{
    model::{SortDirection, SortKey, ViewPreferences},
    sandbox::MediaPreviewBackend,
    services::{CrossVolumeDropStrategy, SearchLimits},
};

mod corner_roundness;
mod defaults;
mod folder_view_file;
pub mod folder_view_policy;
mod folder_view_store;
pub mod folder_views;
mod keymap;
mod list_columns;
mod persistence;
mod saving;
mod stored;
mod text_size;

pub use corner_roundness::CornerRoundness;
pub use defaults::default_sidebar_order;
use defaults::{
    default_browser_density, default_browser_mode, default_cross_volume_drop_strategy,
    default_date_format, default_density_custom_px, default_double_clicks, default_enabled,
    default_file_clicks, default_folder_clicks, default_frame_density, default_full_volume,
    default_icons_thumbnail_size, default_release_channel, default_search_entry_limit,
    default_search_time_limit, default_sort_direction, default_sort_key,
    default_video_preview_backend,
};
pub use folder_view_file::{
    FolderViewsSave, folder_views_path, load_folder_views, save_folder_views,
};
pub use folder_view_store::{
    FolderViewStore, PendingWrite, SAVE_DELAY as FOLDER_VIEWS_SAVE_DELAY, SavedWrite,
    USE_SAVE_DELAY as FOLDER_VIEWS_USE_SAVE_DELAY, UnreadableAtStartup, Written,
};
pub use keymap::KeymapOverrides;
use list_columns::is_default_list_column_order;
pub use list_columns::{
    ListColumnId, ListColumns, default_list_column_order, sanitize_list_column_order,
    sanitize_list_hidden_columns,
};
#[cfg(any(test, feature = "test-support"))]
pub use persistence::read_preferences;
pub use persistence::{
    RETIRED_SETTINGS, SettingKey, lock_saves, merged_settings, parse_preferences,
    read_loaded_preferences, read_state_file, record_changes, salvage_preferences, settings_path,
};
pub use saving::{Commit, FinishedSave, PendingSave, SaveOutcome, SaveProblem, Saving, load};
pub use stored::{
    MAX_PREVIEW_VIDEO_START, SEARCH_ENTRY_LIMIT_CHOICES, SEARCH_TIME_LIMIT_CHOICES,
    configured_hardware_acceleration, configured_video_preview_backend, normalized_font_family,
    normalized_search_limit, normalized_video_start, normalized_volume, sort_preferences,
    store_sort,
};
pub use text_size::TextSize;

/// The range of the Icons view's thumbnail size, in pixels.
pub const MIN_ICONS_THUMBNAIL_SIZE: i32 = 32;
pub const MAX_ICONS_THUMBNAIL_SIZE: i32 = 256;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceRenderer {
    Cairo,
    #[default]
    System,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OmarchyVariant {
    #[serde(alias = "original")]
    Lighter,
    #[default]
    Darker,
    #[serde(alias = "high_contrast")]
    Contrast,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileIconColors {
    #[default]
    Brand,
    Theme,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnitSystem {
    #[default]
    Metric,
    Imperial,
}

/// What `settings.toml` holds, with each value as it is stored.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Preferences {
    pub mode: String,
    pub theme: String,
    /// Pre-per-theme global choice, migrated to the active Omarchy theme.
    #[serde(default, rename = "omarchy_variant", skip_serializing)]
    pub legacy_omarchy_variant: Option<OmarchyVariant>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub omarchy_variants: HashMap<String, OmarchyVariant>,
    #[serde(default)]
    pub folder_peeking: bool,
    #[serde(default = "default_enabled")]
    pub single_click_previews: bool,
    #[serde(default = "default_enabled")]
    pub columns_mirror_selection: bool,
    #[serde(default = "default_enabled")]
    pub columns_show_sizes: bool,
    #[serde(default = "default_enabled")]
    pub render_documents_by_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_accelerated_video_previews: Option<bool>,
    #[serde(default = "default_video_preview_backend")]
    pub video_preview_backend: String,
    #[serde(default)]
    pub search_open_files_directly: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_exclusions: Vec<String>,
    #[serde(default = "default_search_time_limit")]
    pub search_time_limit: u32,
    #[serde(default = "default_search_entry_limit")]
    pub search_entry_limit: u32,
    #[serde(default = "default_enabled")]
    pub type_to_search: bool,
    #[serde(default)]
    pub arrow_navigation_scoped: bool,
    #[serde(default)]
    pub tenxer_mode: bool,
    #[serde(default = "default_enabled")]
    pub filter_include_subfolders: bool,
    #[serde(default = "default_enabled")]
    pub show_keybinding_hints: bool,
    #[serde(default = "default_enabled")]
    pub window_show_close: bool,
    #[serde(default)]
    pub window_show_minimize: bool,
    #[serde(default)]
    pub window_show_maximize: bool,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default = "default_enabled")]
    pub element_glow: bool,
    #[serde(default)]
    pub file_icon_colors: FileIconColors,
    #[serde(default = "default_browser_mode")]
    pub browser_mode: String,
    #[serde(default = "default_browser_density")]
    pub browser_density: String,
    #[serde(default = "default_density_custom_px")]
    pub browser_density_custom_px: u8,
    #[serde(default = "default_frame_density")]
    pub frame_density: String,
    #[serde(default = "default_density_custom_px")]
    pub frame_density_custom_px: u8,
    #[serde(default)]
    pub group_by_type: bool,
    #[serde(default = "default_file_clicks", rename = "list_file_clicks")]
    pub columns_file_clicks: u8,
    #[serde(default = "default_folder_clicks", rename = "list_folder_clicks")]
    pub columns_folder_clicks: u8,
    #[serde(default = "default_file_clicks", rename = "grid_file_clicks")]
    pub icons_file_clicks: u8,
    #[serde(default = "default_double_clicks", rename = "grid_folder_clicks")]
    pub icons_folder_clicks: u8,
    #[serde(default = "default_file_clicks", rename = "explorer_file_clicks")]
    pub list_file_clicks: u8,
    #[serde(default = "default_double_clicks", rename = "explorer_folder_clicks")]
    pub list_folder_clicks: u8,
    #[serde(default = "default_sidebar_order")]
    pub sidebar_order: Vec<String>,
    #[serde(default = "default_enabled")]
    pub sidebar_show_home: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_trash: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_network: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_recent: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_desktop: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_documents: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_downloads: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_music: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_pictures: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_show_videos: bool,
    #[serde(default = "default_enabled")]
    pub sidebar_expanded: bool,
    #[serde(default)]
    pub sidebar_pinned_collapsed: bool,
    #[serde(default)]
    pub sidebar_devices_collapsed: bool,
    #[serde(default)]
    pub sidebar_compact: bool,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default)]
    pub text_size: TextSize,
    #[serde(default)]
    pub corner_roundness: CornerRoundness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_font: Option<String>,
    #[serde(default)]
    pub interface_renderer: InterfaceRenderer,
    #[serde(default)]
    pub language: crate::i18n::Language,
    #[serde(default = "default_enabled")]
    pub folders_first: bool,
    #[serde(default = "default_sort_key")]
    pub sort_key: String,
    #[serde(default = "default_sort_direction")]
    pub sort_direction: String,
    #[serde(default = "defaults::default_natural_sort")]
    pub natural_sort: bool,
    #[serde(default)]
    pub case_sensitive_sort: bool,
    #[serde(default = "default_enabled")]
    pub check_for_updates: bool,
    #[serde(default)]
    pub preview_muted: bool,
    #[serde(default = "default_full_volume")]
    pub preview_volume: f64,
    #[serde(default = "default_enabled")]
    pub preview_text_wrap: bool,
    #[serde(default)]
    pub unit_system: UnitSystem,
    #[serde(default)]
    pub preview_autoplay: bool,
    #[serde(default)]
    pub preview_video_start: u32,
    #[serde(default = "default_enabled")]
    pub watch_network_folders: bool,
    #[serde(default = "crate::sandbox::browser::default_worker_limit")]
    pub thumbnail_workers: usize,
    #[serde(default = "default_enabled")]
    pub show_thumbnails: bool,
    #[serde(default)]
    pub stl_thumbnails: bool,
    #[serde(default = "default_icons_thumbnail_size")]
    pub icons_thumbnail_size: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chooser_column_width: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_column_width: Option<i32>,
    #[serde(default = "default_cross_volume_drop_strategy")]
    pub cross_volume_drop_strategy: String,
    #[serde(default)]
    pub open_folder_after_drop: bool,
    #[serde(default = "default_date_format")]
    pub date_format: String,
    #[serde(default = "default_release_channel")]
    pub release_channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_directory: Option<PathBuf>,
    #[serde(default = "default_enabled")]
    pub restore_tabs: bool,
    #[serde(default = "default_enabled")]
    pub remember_folder_views: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub folder_colors: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_icons: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub send_to_recent_destinations: HashMap<String, Vec<PathBuf>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub device_labels: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chooser_list_columns: Option<ListColumns>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_list_columns: Option<ListColumns>,
    #[serde(
        default = "default_list_column_order",
        skip_serializing_if = "is_default_list_column_order"
    )]
    pub list_column_order: Vec<ListColumnId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list_hidden_columns: Vec<ListColumnId>,
    #[serde(default, skip_serializing_if = "KeymapOverrides::is_empty")]
    pub keymap: KeymapOverrides,
}
