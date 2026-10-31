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

mod bindings;
mod corner_roundness;
#[cfg(test)]
pub(in crate::ui) mod fixtures;
mod folder_view_settings;
mod folder_views;
mod item_customizations;
mod keymap;
mod save_notice;
mod text_size;
mod trashed;
pub(in crate::ui) use bindings::notify_live;
pub use corner_roundness::CornerRoundness;
pub(crate) use folder_view_settings::flush_pending_folder_views;
use keymap::KeymapOverrides;
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
    #[serde(default = "default_enabled")]
    check_for_updates: bool,
    #[serde(default)]
    preview_muted: bool,
    #[serde(default = "default_full_volume")]
    preview_volume: f64,
    #[serde(default)]
    preview_text_wrap: bool,
    #[serde(default)]
    unit_system: UnitSystem,
    #[serde(default)]
    preview_autoplay: bool,
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListColumnId {
    Name,
    Mode,
    Size,
    #[serde(rename = "type")]
    Type,
    Modified,
}

impl ListColumnId {
    pub const ALL: [Self; 5] = [
        Self::Name,
        Self::Mode,
        Self::Size,
        Self::Type,
        Self::Modified,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Mode => 1,
            Self::Size => 2,
            Self::Type => 3,
            Self::Modified => 4,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Mode => "Mode",
            Self::Size => "Size",
            Self::Type => "Type",
            Self::Modified => "Modified",
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Name => "list-col-name",
            Self::Mode => "list-col-mode",
            Self::Size => "list-col-size",
            Self::Type => "list-col-type",
            Self::Modified => "list-col-modified",
        }
    }
}

pub fn default_list_column_order() -> Vec<ListColumnId> {
    ListColumnId::ALL.to_vec()
}

fn is_default_list_column_order(order: &[ListColumnId]) -> bool {
    order == ListColumnId::ALL
}

pub(crate) fn sanitize_list_column_order(order: &[ListColumnId]) -> Vec<ListColumnId> {
    let mut sanitized = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for col in order {
        if seen.insert(*col) {
            sanitized.push(*col);
        }
    }
    for col in ListColumnId::ALL {
        if seen.insert(col) {
            sanitized.push(col);
        }
    }
    sanitized
}

pub(crate) fn sanitize_list_hidden_columns(hidden: &[ListColumnId]) -> Vec<ListColumnId> {
    let mut sanitized = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for col in hidden {
        if *col != ListColumnId::Name && seen.insert(*col) {
            sanitized.push(*col);
        }
    }
    sanitized
}

/// Unscaled pixels; an unset `name` lets Name absorb the remaining space.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListColumns {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<i32>,
    pub mode: i32,
    pub size: i32,
    #[serde(rename = "type")]
    pub kind: i32,
    pub modified: i32,
}

impl ListColumns {
    fn is_valid(&self) -> bool {
        self.name.is_none_or(|width| width > 0)
            && [self.mode, self.size, self.kind, self.modified]
                .iter()
                .all(|width| *width > 0)
    }
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            mode: "theme".to_owned(),
            theme: "tokyo-night".to_owned(),
            legacy_omarchy_variant: None,
            omarchy_variants: HashMap::new(),
            folder_peeking: false,
            single_click_previews: true,
            columns_mirror_selection: true,
            columns_show_sizes: true,
            render_documents_by_default: true,
            hardware_accelerated_video_previews: None,
            video_preview_backend: default_video_preview_backend(),
            search_open_files_directly: false,
            search_exclusions: Vec::new(),
            search_time_limit: default_search_time_limit(),
            search_entry_limit: default_search_entry_limit(),
            type_to_search: true,
            arrow_navigation_scoped: false,
            tenxer_mode: false,
            filter_include_subfolders: true,
            show_keybinding_hints: true,
            window_show_close: true,
            window_show_minimize: false,
            window_show_maximize: false,
            reduce_motion: false,
            element_glow: true,
            file_icon_colors: FileIconColors::default(),
            browser_mode: default_browser_mode(),
            browser_density: default_browser_density(),
            browser_density_custom_px: default_density_custom_px(),
            frame_density: default_frame_density(),
            frame_density_custom_px: default_density_custom_px(),
            group_by_type: false,
            columns_file_clicks: default_file_clicks(),
            columns_folder_clicks: default_folder_clicks(),
            icons_file_clicks: default_file_clicks(),
            icons_folder_clicks: default_double_clicks(),
            list_file_clicks: default_file_clicks(),
            list_folder_clicks: default_double_clicks(),
            sidebar_order: default_sidebar_order(),
            sidebar_show_home: true,
            sidebar_show_trash: true,
            sidebar_show_network: true,
            sidebar_show_recent: true,
            sidebar_show_desktop: true,
            sidebar_show_documents: true,
            sidebar_show_downloads: true,
            sidebar_show_music: true,
            sidebar_show_pictures: true,
            sidebar_show_videos: true,
            sidebar_expanded: true,
            sidebar_compact: false,
            show_hidden: false,
            text_size: TextSize::default(),
            corner_roundness: CornerRoundness::default(),
            interface_font: None,
            interface_renderer: InterfaceRenderer::default(),
            language: crate::i18n::Language::default(),
            folders_first: true,
            sort_key: default_sort_key(),
            sort_direction: default_sort_direction(),
            check_for_updates: true,
            preview_muted: false,
            preview_volume: default_full_volume(),
            preview_text_wrap: false,
            unit_system: UnitSystem::Metric,
            preview_autoplay: false,
            watch_network_folders: true,
            thumbnail_workers: crate::sandbox::browser::default_worker_limit(),
            show_thumbnails: true,
            stl_thumbnails: false,
            icons_thumbnail_size: default_icons_thumbnail_size(),
            chooser_column_width: None,
            browser_column_width: None,
            cross_volume_drop_strategy: default_cross_volume_drop_strategy(),
            open_folder_after_drop: false,
            date_format: default_date_format(),
            release_channel: default_release_channel(),
            default_directory: None,
            restore_tabs: true,
            remember_folder_views: true,
            folder_colors: HashMap::new(),
            custom_icons: HashMap::new(),
            send_to_recent_destinations: HashMap::new(),
            device_labels: HashMap::new(),
            chooser_list_columns: None,
            browser_list_columns: None,
            list_column_order: default_list_column_order(),
            list_hidden_columns: Vec::new(),
            keymap: KeymapOverrides::default(),
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_release_channel() -> String {
    "stable".to_owned()
}

fn default_browser_mode() -> String {
    "columns".to_owned()
}

fn browser_mode_from_stored(value: &str) -> super::browser_modes::BrowserMode {
    match value {
        "icons" | "grid" => super::browser_modes::BrowserMode::Icons,
        "list" | "explorer" => super::browser_modes::BrowserMode::List,
        _ => super::browser_modes::BrowserMode::Columns,
    }
}

fn stored_browser_mode(mode: super::browser_modes::BrowserMode) -> &'static str {
    match mode {
        super::browser_modes::BrowserMode::Columns => "columns",
        super::browser_modes::BrowserMode::Icons => "icons",
        super::browser_modes::BrowserMode::List => "list",
    }
}

fn default_video_preview_backend() -> String {
    "automatic".to_owned()
}

fn default_browser_density() -> String {
    "compact".to_owned()
}

fn default_frame_density() -> String {
    "compact".to_owned()
}

/// Only read when the matching density is `"custom"`.
fn default_density_custom_px() -> u8 {
    0
}

fn stored_density<D: DensityScale>(mode: &str, custom_px: u8) -> D {
    match mode {
        "airy" => D::from_px(D::AIRY_PX),
        "custom" => D::from_px(custom_px),
        _ => D::from_px(D::COMPACT_PX),
    }
}

fn store_density<D: DensityScale>(density: D, mode: &mut String, custom_px: &mut u8) {
    let px = density.px();
    *mode = if px == D::COMPACT_PX {
        "compact"
    } else if px == D::AIRY_PX {
        "airy"
    } else {
        *custom_px = px;
        "custom"
    }
    .to_owned();
}

fn default_file_clicks() -> u8 {
    2
}

fn default_folder_clicks() -> u8 {
    1
}

fn default_double_clicks() -> u8 {
    2
}

fn default_sidebar_order() -> Vec<String> {
    vec![
        "home".to_owned(),
        "trash".to_owned(),
        "network".to_owned(),
        "recent".to_owned(),
        "desktop".to_owned(),
        "documents".to_owned(),
        "downloads".to_owned(),
        "music".to_owned(),
        "pictures".to_owned(),
        "videos".to_owned(),
    ]
}

fn default_search_time_limit() -> u32 {
    SearchLimits::default()
        .time_budget
        .and_then(|budget| u32::try_from(budget.as_secs()).ok())
        .unwrap_or(0)
}

fn default_search_entry_limit() -> u32 {
    SearchLimits::default()
        .max_entries
        .and_then(|entries| u32::try_from(entries).ok())
        .unwrap_or(0)
}

fn default_sort_key() -> String {
    "name".to_owned()
}

fn default_sort_direction() -> String {
    "ascending".to_owned()
}

fn default_full_volume() -> f64 {
    1.0
}

fn default_icons_thumbnail_size() -> i32 {
    64
}

fn default_date_format() -> String {
    crate::util::DateFormat::default().as_str().to_owned()
}

fn default_cross_volume_drop_strategy() -> String {
    CrossVolumeDropStrategy::Ask.as_str().to_owned()
}

fn normalized_volume(volume: f64) -> f64 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

pub(in crate::ui) const SEARCH_TIME_LIMIT_CHOICES: [(&str, u32); 6] = [
    ("10 s", 10),
    ("30 s", 30),
    ("1 min", 60),
    ("5 min", 300),
    ("15 min", 900),
    ("No limit", 0),
];

pub(in crate::ui) const SEARCH_ENTRY_LIMIT_CHOICES: [(&str, u32); 5] = [
    ("200,000", 200_000),
    ("500,000", 500_000),
    ("1,000,000", 1_000_000),
    ("5,000,000", 5_000_000),
    ("No limit", 0),
];

/// Unlisted limits round up to the next choice; 0 and anything past the largest
/// choice mean no limit.
fn normalized_search_limit(value: u32, choices: &[(&str, u32)]) -> u32 {
    if value == 0 {
        return 0;
    }
    choices
        .iter()
        .map(|(_, limit)| *limit)
        .filter(|limit| *limit != 0)
        .find(|limit| *limit >= value)
        .unwrap_or(0)
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
        let (mut preferences, load_failure, salvaged) = match read_preferences_checked() {
            Ok((preferences, salvaged)) => (preferences, None, salvaged),
            Err(error) => {
                tracing::warn!(%error, path = %settings_path().display(),
                    "unable to load settings; using temporary defaults without saving; fix the file and restart Strata");
                (Preferences::default(), Some(error), BTreeSet::new())
            }
        };
        let parsed = preferences.clone();
        preferences.preview_volume = normalized_volume(preferences.preview_volume);
        preferences.search_time_limit =
            normalized_search_limit(preferences.search_time_limit, &SEARCH_TIME_LIMIT_CHOICES);
        preferences.search_entry_limit =
            normalized_search_limit(preferences.search_entry_limit, &SEARCH_ENTRY_LIMIT_CHOICES);
        preferences.thumbnail_workers = preferences
            .thumbnail_workers
            .clamp(1, crate::sandbox::browser::MAX_WORKERS);
        preferences.icons_thumbnail_size = preferences
            .icons_thumbnail_size
            .clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        preferences.chooser_column_width =
            preferences.chooser_column_width.filter(|width| *width > 0);
        preferences.browser_column_width =
            preferences.browser_column_width.filter(|width| *width > 0);
        preferences.chooser_list_columns = preferences
            .chooser_list_columns
            .filter(ListColumns::is_valid);
        preferences.browser_list_columns = preferences
            .browser_list_columns
            .filter(ListColumns::is_valid);
        preferences.list_column_order = sanitize_list_column_order(&preferences.list_column_order);
        preferences.list_hidden_columns =
            sanitize_list_hidden_columns(&preferences.list_hidden_columns);
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
        omarchy_available: bool,
    ) {
        let mut preferences = self.preferences.borrow_mut();
        if !theme_is_known(&preferences.theme) {
            preferences.theme = "azure-glow".to_owned();
        }
        if preferences.mode == "omarchy" && !omarchy_available {
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

    pub(in crate::ui) fn theme_mode(&self) -> String {
        self.preferences.borrow().mode.clone()
    }

    pub(in crate::ui) fn selected_theme_id(&self) -> String {
        self.preferences.borrow().theme.clone()
    }

    /// Writes the theme selection as one change so mode and theme publish together.
    pub(in crate::ui) fn set_theme_selection(&self, mode: &str, theme: Option<&str>) {
        {
            let mut preferences = self.preferences.borrow_mut();
            preferences.mode = mode.to_owned();
            if let Some(theme) = theme {
                preferences.theme = theme.to_owned();
            }
        }
        self.save_preferences();
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

    pub fn folder_peeking(&self) -> bool {
        self.preferences.borrow().folder_peeking
    }

    pub fn set_folder_peeking(&self, enabled: bool) {
        self.preferences.borrow_mut().folder_peeking = enabled;
        self.save_preferences();
    }

    pub fn folder_color(&self, path: &Path) -> Option<FolderColorValue> {
        let preferences = self.preferences.borrow();
        if preferences.folder_colors.is_empty() {
            return None;
        }
        let key = path.to_string_lossy();
        let color_name = preferences.folder_colors.get(key.as_ref())?;
        FolderColorValue::parse(color_name)
    }

    pub fn set_folder_color(&self, path: &Path, color: Option<FolderColorValue>) {
        self.set_folder_colors(&[path.to_path_buf()], color);
    }

    pub fn set_folder_colors(&self, paths: &[PathBuf], color: Option<FolderColorValue>) {
        if paths.is_empty() {
            return;
        }
        {
            let mut preferences = self.preferences.borrow_mut();
            for path in paths {
                let key = path.to_string_lossy().into_owned();
                if let Some(color) = &color {
                    preferences
                        .folder_colors
                        .insert(key, color.to_preference_string());
                } else {
                    preferences.folder_colors.remove(&key);
                }
            }
        }
        self.save_preferences();
        super::thumbnail::refresh_customized_icons(paths);
    }

    pub fn custom_icon(&self, path: &Path) -> Option<String> {
        let preferences = self.preferences.borrow();
        if preferences.custom_icons.is_empty() {
            return None;
        }
        let key = path.to_string_lossy();
        preferences
            .custom_icons
            .get(key.as_ref())
            .filter(|name| crate::assets::icons::is_customization_choice(name))
            .cloned()
    }

    pub fn set_custom_icon(&self, path: &Path, icon_name: Option<&str>) {
        {
            let mut preferences = self.preferences.borrow_mut();
            let key = path.to_string_lossy().into_owned();
            if let Some(name) =
                icon_name.filter(|name| crate::assets::icons::is_customization_choice(name))
            {
                preferences.custom_icons.insert(key, name.to_owned());
            } else {
                preferences.custom_icons.remove(&key);
            }
        }
        self.save_preferences();
        super::thumbnail::refresh_customized_icons(&[path.to_path_buf()]);
    }

    pub fn clear_item_customization(&self, path: &Path) {
        {
            let mut preferences = self.preferences.borrow_mut();
            let key = path.to_string_lossy();
            preferences.folder_colors.remove(key.as_ref());
            preferences.custom_icons.remove(key.as_ref());
        }
        self.save_preferences();
        super::thumbnail::refresh_customized_icons(&[path.to_path_buf()]);
    }

    pub fn single_click_previews(&self) -> bool {
        self.preferences.borrow().single_click_previews
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.preferences.borrow_mut().single_click_previews = enabled;
        self.save_preferences();
    }

    pub fn columns_show_sizes(&self) -> bool {
        self.preferences.borrow().columns_show_sizes
    }

    pub fn set_columns_show_sizes(&self, enabled: bool) {
        self.preferences.borrow_mut().columns_show_sizes = enabled;
        self.save_preferences();
    }

    pub fn columns_mirror_selection(&self) -> bool {
        self.preferences.borrow().columns_mirror_selection
    }

    pub fn set_columns_mirror_selection(&self, enabled: bool) {
        self.preferences.borrow_mut().columns_mirror_selection = enabled;
        self.save_preferences();
    }

    pub fn render_documents_by_default(&self) -> bool {
        self.preferences.borrow().render_documents_by_default
    }

    pub fn set_render_documents_by_default(&self, enabled: bool) {
        self.preferences.borrow_mut().render_documents_by_default = enabled;
        self.save_preferences();
    }

    pub fn hardware_accelerated_video_previews(&self) -> bool {
        configured_hardware_acceleration(
            &self.preferences.borrow(),
            crate::sandbox::polaris_gpu_available(),
        )
    }

    pub fn set_hardware_accelerated_video_previews(&self, enabled: bool) {
        self.preferences
            .borrow_mut()
            .hardware_accelerated_video_previews = Some(enabled);
        self.save_preferences();
    }

    pub fn video_preview_backend(&self) -> MediaPreviewBackend {
        configured_video_preview_backend(&self.preferences.borrow())
    }

    pub fn set_video_preview_backend(&self, backend: MediaPreviewBackend) {
        let backend = match backend {
            MediaPreviewBackend::Automatic => "automatic",
            MediaPreviewBackend::VaApi => "vaapi",
            MediaPreviewBackend::Vulkan => "vulkan",
            MediaPreviewBackend::Software => return,
        };
        self.preferences.borrow_mut().video_preview_backend = backend.to_owned();
        self.save_preferences();
    }

    pub(crate) fn media_preview_backend(&self) -> MediaPreviewBackend {
        if !self.hardware_accelerated_video_previews() {
            MediaPreviewBackend::Software
        } else {
            self.video_preview_backend()
        }
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

    pub fn window_show_close(&self) -> bool {
        self.preferences.borrow().window_show_close
    }

    pub fn set_window_show_close(&self, enabled: bool) {
        self.preferences.borrow_mut().window_show_close = enabled;
        self.save_preferences();
    }

    pub fn window_show_minimize(&self) -> bool {
        self.preferences.borrow().window_show_minimize
    }

    pub fn set_window_show_minimize(&self, enabled: bool) {
        self.preferences.borrow_mut().window_show_minimize = enabled;
        self.save_preferences();
    }

    pub fn window_show_maximize(&self) -> bool {
        self.preferences.borrow().window_show_maximize
    }

    pub fn set_window_show_maximize(&self, enabled: bool) {
        self.preferences.borrow_mut().window_show_maximize = enabled;
        self.save_preferences();
    }

    /// The variant remembered for the active Omarchy theme.
    pub fn omarchy_variant(&self) -> OmarchyVariant {
        let active = self.active_omarchy_theme.borrow();
        active
            .as_ref()
            .and_then(|theme| {
                self.preferences
                    .borrow()
                    .omarchy_variants
                    .get(theme)
                    .copied()
            })
            .unwrap_or_default()
    }

    pub fn set_omarchy_variant(&self, variant: OmarchyVariant) {
        let Some(theme) = self.active_omarchy_theme.borrow().clone() else {
            return;
        };
        if self.omarchy_variant() == variant {
            return;
        }
        {
            let variants = &mut self.preferences.borrow_mut().omarchy_variants;
            if variant == OmarchyVariant::default() {
                variants.remove(&theme);
            } else {
                variants.insert(theme, variant);
            }
        }
        self.save_preferences();
    }

    /// Records which Omarchy theme is current without publishing; the theme
    /// manager reapplies and notifies once its palette state is updated.
    pub(in crate::ui) fn set_active_omarchy_theme(&self, theme: &str) {
        let theme = theme.trim().to_owned();
        let mut preferences = self.preferences.borrow_mut();
        // Older versions always saved "original", so only a contrast choice was deliberate.
        if let Some(legacy) = preferences.legacy_omarchy_variant.take()
            && legacy == OmarchyVariant::Contrast
        {
            preferences
                .omarchy_variants
                .entry(theme.clone())
                .or_insert(legacy);
        }
        self.active_omarchy_theme.replace(Some(theme));
    }

    pub fn element_glow(&self) -> bool {
        self.preferences.borrow().element_glow
    }

    pub fn set_element_glow(&self, enabled: bool) {
        if self.element_glow() == enabled {
            return;
        }
        self.preferences.borrow_mut().element_glow = enabled;
        self.save_preferences();
    }

    pub fn file_icon_colors(&self) -> FileIconColors {
        self.preferences.borrow().file_icon_colors
    }

    pub fn set_file_icon_colors(&self, colors: FileIconColors) {
        if self.file_icon_colors() == colors {
            return;
        }
        self.preferences.borrow_mut().file_icon_colors = colors;
        self.save_preferences();
    }

    pub fn reduce_motion(&self) -> bool {
        self.preferences.borrow().reduce_motion
    }

    pub fn set_reduce_motion(&self, reduced: bool) {
        self.preferences.borrow_mut().reduce_motion = reduced;
        super::motion::set_reduce_motion(reduced);
        self.save_preferences();
    }

    pub fn checks_for_updates(&self) -> bool {
        self.preferences.borrow().check_for_updates
    }

    pub fn set_checks_for_updates(&self, enabled: bool) {
        self.preferences.borrow_mut().check_for_updates = enabled;
        self.save_preferences();
    }

    pub fn preview_muted(&self) -> bool {
        self.preferences.borrow().preview_muted
    }

    pub fn set_preview_muted(&self, muted: bool) {
        self.preferences.borrow_mut().preview_muted = muted;
        self.save_preferences();
    }

    pub fn preview_volume(&self) -> f64 {
        self.preferences.borrow().preview_volume
    }

    pub fn set_preview_volume(&self, volume: f64) {
        self.preferences.borrow_mut().preview_volume = normalized_volume(volume);
        self.save_preferences();
    }

    pub(crate) fn unit_system(&self) -> UnitSystem {
        self.preferences.borrow().unit_system
    }

    pub(crate) fn set_unit_system(&self, units: UnitSystem) {
        self.preferences.borrow_mut().unit_system = units;
        self.save_preferences();
    }

    pub fn preview_text_wrap(&self) -> bool {
        self.preferences.borrow().preview_text_wrap
    }

    pub fn set_preview_text_wrap(&self, wrapped: bool) {
        self.preferences.borrow_mut().preview_text_wrap = wrapped;
        self.save_preferences();
    }

    pub fn preview_autoplay(&self) -> bool {
        self.preferences.borrow().preview_autoplay
    }

    pub fn set_preview_autoplay(&self, autoplay: bool) {
        self.preferences.borrow_mut().preview_autoplay = autoplay;
        self.save_preferences();
    }

    pub fn set_preview_audio(&self, volume: f64, muted: bool) {
        self.preferences.borrow_mut().preview_muted = muted;
        if volume > 0.0 {
            self.set_preview_volume(volume);
        } else {
            self.save_preferences();
        }
    }

    pub fn show_thumbnails(&self) -> bool {
        self.preferences.borrow().show_thumbnails
    }

    pub fn set_show_thumbnails(&self, enabled: bool) {
        self.preferences.borrow_mut().show_thumbnails = enabled;
        self.save_preferences();
    }

    pub fn thumbnail_workers(&self) -> usize {
        self.preferences.borrow().thumbnail_workers
    }

    pub fn stl_thumbnails(&self) -> bool {
        self.preferences.borrow().stl_thumbnails
    }

    pub fn set_stl_thumbnails(&self, enabled: bool) {
        self.preferences.borrow_mut().stl_thumbnails = enabled;
        self.save_preferences();
    }

    pub fn set_thumbnail_workers(&self, workers: usize) {
        self.preferences.borrow_mut().thumbnail_workers =
            workers.clamp(1, crate::sandbox::browser::MAX_WORKERS);
        self.save_preferences();
    }

    pub fn icons_thumbnail_size(&self) -> i32 {
        self.preferences.borrow().icons_thumbnail_size
    }

    pub fn set_icons_thumbnail_size(&self, size: i32) {
        self.preferences.borrow_mut().icons_thumbnail_size =
            size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        self.save_preferences();
    }

    pub fn chooser_column_width(&self) -> Option<i32> {
        self.preferences.borrow().chooser_column_width
    }

    pub fn set_chooser_column_width(&self, width: Option<i32>) {
        self.preferences.borrow_mut().chooser_column_width = width.filter(|width| *width > 0);
        self.save_preferences();
    }

    pub fn browser_column_width(&self) -> Option<i32> {
        self.preferences.borrow().browser_column_width
    }

    pub fn set_browser_column_width(&self, width: Option<i32>) {
        self.preferences.borrow_mut().browser_column_width = width.filter(|width| *width > 0);
        self.save_preferences();
    }

    pub fn browser_list_columns(&self) -> Option<ListColumns> {
        self.preferences.borrow().browser_list_columns
    }

    pub fn set_browser_list_columns(&self, columns: Option<ListColumns>) {
        self.preferences.borrow_mut().browser_list_columns = columns.filter(ListColumns::is_valid);
        self.save_preferences();
    }

    pub fn chooser_list_columns(&self) -> Option<ListColumns> {
        self.preferences.borrow().chooser_list_columns
    }

    pub fn set_chooser_list_columns(&self, columns: Option<ListColumns>) {
        self.preferences.borrow_mut().chooser_list_columns = columns.filter(ListColumns::is_valid);
        self.save_preferences();
    }

    pub fn list_column_order(&self) -> Vec<ListColumnId> {
        sanitize_list_column_order(&self.preferences.borrow().list_column_order)
    }

    pub fn set_list_column_order(&self, order: Vec<ListColumnId>) {
        self.preferences.borrow_mut().list_column_order = sanitize_list_column_order(&order);
        self.save_preferences();
    }

    pub fn list_hidden_columns(&self) -> Vec<ListColumnId> {
        sanitize_list_hidden_columns(&self.preferences.borrow().list_hidden_columns)
    }

    pub fn set_list_hidden_columns(&self, hidden: Vec<ListColumnId>) {
        self.preferences.borrow_mut().list_hidden_columns = sanitize_list_hidden_columns(&hidden);
        self.save_preferences();
    }

    pub fn is_list_column_visible(&self, column: ListColumnId) -> bool {
        if column == ListColumnId::Name {
            return true;
        }
        !self.list_hidden_columns().contains(&column)
    }

    pub fn set_list_column_visible(&self, column: ListColumnId, visible: bool) {
        if column == ListColumnId::Name {
            return;
        }
        let mut hidden = self.list_hidden_columns();
        if visible {
            hidden.retain(|col| *col != column);
        } else if !hidden.contains(&column) {
            hidden.push(column);
        }
        self.set_list_hidden_columns(hidden);
    }

    pub fn move_list_column(&self, column: ListColumnId, target: ListColumnId, after: bool) {
        let mut order = self.list_column_order();
        let Some(from) = order.iter().position(|item| *item == column) else {
            return;
        };
        let Some(target) = order.iter().position(|item| *item == target) else {
            return;
        };
        let mut to = target + usize::from(after);
        if from < to {
            to -= 1;
        }
        if from != to {
            let column = order.remove(from);
            order.insert(to, column);
            self.set_list_column_order(order);
        }
    }

    pub fn reset_list_columns(&self) {
        {
            let mut prefs = self.preferences.borrow_mut();
            prefs.list_column_order = default_list_column_order();
            prefs.list_hidden_columns.clear();
        }
        self.save_preferences();
    }

    pub fn list_visible_columns(&self) -> Vec<ListColumnId> {
        let hidden = self.list_hidden_columns();
        self.list_column_order()
            .into_iter()
            .filter(|col| *col == ListColumnId::Name || !hidden.contains(col))
            .collect()
    }

    pub fn watch_network_folders(&self) -> bool {
        self.preferences.borrow().watch_network_folders
    }

    pub fn set_watch_network_folders(&self, enabled: bool) {
        self.preferences.borrow_mut().watch_network_folders = enabled;
        self.save_preferences();
    }

    pub fn default_directory(&self) -> Option<PathBuf> {
        self.preferences.borrow().default_directory.clone()
    }

    pub fn set_default_directory(&self, path: Option<PathBuf>) {
        self.preferences.borrow_mut().default_directory = path;
        self.save_preferences();
    }

    pub fn restore_tabs(&self) -> bool {
        self.preferences.borrow().restore_tabs
    }

    pub fn set_restore_tabs(&self, enabled: bool) {
        self.preferences.borrow_mut().restore_tabs = enabled;
        if !enabled {
            super::tabs_session::remove();
        }
        self.save_preferences();
    }

    pub(in crate::ui) fn device_label(&self, device_id: &str) -> Option<String> {
        self.preferences
            .borrow()
            .device_labels
            .get(device_id)
            .filter(|label| {
                !label.trim().is_empty()
                    && label.chars().count() <= 255
                    && !label.chars().any(char::is_control)
            })
            .cloned()
    }

    pub(in crate::ui) fn set_device_label(&self, device_id: &str, label: &str) {
        let label = label.trim();
        if device_id.is_empty()
            || label.chars().count() > 255
            || label.chars().any(char::is_control)
        {
            return;
        }
        {
            let mut preferences = self.preferences.borrow_mut();
            if label.is_empty() {
                preferences.device_labels.remove(device_id);
            } else {
                preferences
                    .device_labels
                    .insert(device_id.to_owned(), label.to_owned());
            }
        }
        self.save_preferences();
    }

    pub(in crate::ui) fn send_to_recent_destinations(&self, device_id: &str) -> Vec<PathBuf> {
        self.preferences
            .borrow()
            .send_to_recent_destinations
            .get(device_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(in crate::ui) fn remember_send_to_destination(
        &self,
        device_id: &str,
        relative_path: &Path,
        remove_relative_path: Option<&Path>,
    ) {
        if !is_valid_send_to_relative_path(relative_path) {
            return;
        }
        {
            let mut preferences = self.preferences.borrow_mut();
            let recent = preferences
                .send_to_recent_destinations
                .entry(device_id.to_owned())
                .or_default();
            recent.retain(|path| {
                path != relative_path && Some(path.as_path()) != remove_relative_path
            });
            recent.insert(0, relative_path.to_path_buf());
            recent.truncate(SEND_TO_RECENT_DESTINATIONS_LIMIT);
        }
        self.save_preferences();
    }

    pub fn open_folder_after_drop(&self) -> bool {
        self.preferences.borrow().open_folder_after_drop
    }

    pub fn set_open_folder_after_drop(&self, enabled: bool) {
        self.preferences.borrow_mut().open_folder_after_drop = enabled;
        self.save_preferences();
    }

    pub fn date_format(&self) -> crate::util::DateFormat {
        crate::util::DateFormat::parse(&self.preferences.borrow().date_format)
    }

    pub fn set_date_format(&self, format: crate::util::DateFormat) {
        if self.date_format() == format {
            return;
        }
        self.preferences.borrow_mut().date_format = format.as_str().to_owned();
        crate::util::set_date_format(format);
        self.save_preferences();
    }

    pub fn cross_volume_drop_strategy(&self) -> CrossVolumeDropStrategy {
        CrossVolumeDropStrategy::parse(&self.preferences.borrow().cross_volume_drop_strategy)
    }

    pub fn set_cross_volume_drop_strategy(&self, strategy: CrossVolumeDropStrategy) {
        if self.cross_volume_drop_strategy() == strategy {
            return;
        }
        self.preferences.borrow_mut().cross_volume_drop_strategy = strategy.as_str().to_owned();
        self.save_preferences();
    }

    pub fn release_channel(&self) -> Channel {
        crate::services::InstallSource::detect()
            .managed()
            .and_then(crate::services::ManagedInstall::tracked_channel)
            .unwrap_or_else(|| Channel::parse(&self.preferences.borrow().release_channel))
    }

    pub fn set_release_channel(&self, channel: Channel) {
        if crate::services::InstallSource::detect()
            .managed()
            .and_then(crate::services::ManagedInstall::tracked_channel)
            .is_some()
        {
            return;
        }
        self.preferences.borrow_mut().release_channel = channel.as_str().to_owned();
        self.save_preferences();
    }

    pub fn on_release_channel_changed(
        &self,
        anchor: &impl IsA<gtk::Widget>,
        refresh: Rc<dyn Fn()>,
    ) {
        let initial = Cell::new(true);
        self.bind_preference(anchor, Self::release_channel, move |_, _| {
            if !initial.replace(false) {
                refresh();
            }
        });
    }
    pub fn browser_mode(&self) -> super::browser_modes::BrowserMode {
        browser_mode_from_stored(&self.preferences.borrow().browser_mode)
    }

    pub fn set_browser_mode(&self, mode: super::browser_modes::BrowserMode) {
        self.preferences.borrow_mut().browser_mode = stored_browser_mode(mode).to_owned();
        self.save_preferences();
    }

    pub fn browser_density(&self) -> super::browser_modes::BrowserDensity {
        let preferences = self.preferences.borrow();
        stored_density(
            &preferences.browser_density,
            preferences.browser_density_custom_px,
        )
    }

    pub fn set_browser_density(&self, density: super::browser_modes::BrowserDensity) {
        {
            let mut preferences = self.preferences.borrow_mut();
            let preferences = &mut *preferences;
            store_density(
                density,
                &mut preferences.browser_density,
                &mut preferences.browser_density_custom_px,
            );
        }
        self.save_preferences();
    }

    pub fn frame_density(&self) -> super::browser_modes::FrameDensity {
        let preferences = self.preferences.borrow();
        stored_density(
            &preferences.frame_density,
            preferences.frame_density_custom_px,
        )
    }

    pub fn set_frame_density(&self, density: super::browser_modes::FrameDensity) {
        {
            let mut preferences = self.preferences.borrow_mut();
            let preferences = &mut *preferences;
            store_density(
                density,
                &mut preferences.frame_density,
                &mut preferences.frame_density_custom_px,
            );
        }
        self.save_preferences();
    }

    pub fn language(&self) -> crate::i18n::Language {
        self.preferences.borrow().language
    }

    pub fn set_language(&self, language: crate::i18n::Language) {
        self.preferences.borrow_mut().language = language;
        self.save_preferences();
    }

    pub fn language_restart_required(&self) -> bool {
        self.language().locale() != self.startup_locale
    }

    pub fn interface_renderer(&self) -> InterfaceRenderer {
        self.preferences.borrow().interface_renderer
    }

    pub fn set_interface_renderer(&self, renderer: InterfaceRenderer) {
        self.preferences.borrow_mut().interface_renderer = renderer;
        self.save_preferences();
    }

    pub fn interface_renderer_restart_required(&self) -> bool {
        self.interface_renderer() != self.startup_interface_renderer
    }

    pub fn text_size(&self) -> TextSize {
        self.preferences.borrow().text_size
    }

    pub fn set_text_size(&self, size: TextSize) {
        if self.text_size() == size {
            self.save_preferences();
            return;
        }
        self.preferences.borrow_mut().text_size = size;
        self.save_preferences();
    }

    pub fn corner_roundness(&self) -> CornerRoundness {
        self.preferences.borrow().corner_roundness
    }

    pub fn set_corner_roundness(&self, roundness: CornerRoundness) {
        if self.corner_roundness() == roundness {
            return;
        }
        self.preferences.borrow_mut().corner_roundness = roundness;
        self.save_preferences();
    }

    /// The chosen interface font family; `None` keeps the bundled default.
    pub fn interface_font(&self) -> Option<String> {
        self.preferences.borrow().interface_font.clone()
    }

    pub fn set_interface_font(&self, family: Option<String>) {
        let family = family
            .map(|family| family.trim().to_owned())
            .filter(|family| !family.is_empty());
        if self.interface_font() == family {
            return;
        }
        self.preferences.borrow_mut().interface_font = family;
        self.save_preferences();
    }

    pub fn interface_scale(&self) -> f64 {
        snapped_root_font_px(self.text_size().root_font_px(), desktop_text_scale_factor()) / 13.0
    }

    pub fn bind_interface_scale(
        self: &Rc<Self>,
        anchor: &impl IsA<gtk::Widget>,
        apply: impl Fn(&gtk::Widget, f64) + 'static,
    ) {
        let apply = Rc::new(apply);
        let changed = apply.clone();
        self.bind_preference(anchor, Self::interface_scale, move |widget, scale| {
            changed(widget, scale)
        });
        if let Some(settings) = gtk::Settings::default() {
            let widget = anchor.downgrade();
            let manager = Rc::downgrade(self);
            let handler = settings.connect_gtk_xft_dpi_notify(move |_| {
                if let (Some(widget), Some(manager)) = (widget.upgrade(), manager.upgrade()) {
                    apply(widget.upcast_ref(), manager.interface_scale());
                }
            });
            let handler = RefCell::new(Some(handler));
            anchor.connect_destroy(move |_| {
                if let Some(handler) = handler.borrow_mut().take() {
                    settings.disconnect(handler);
                }
            });
        }
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
        mode: super::browser_modes::BrowserMode,
    ) -> super::browser_modes::ClickActivation {
        use super::browser_modes::{BrowserMode, ClickActivation, ClickCount};

        let preferences = self.preferences.borrow();
        let (files, folders) = match mode {
            BrowserMode::Columns => (
                preferences.columns_file_clicks,
                preferences.columns_folder_clicks,
            ),
            BrowserMode::Icons => (
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
        mode: super::browser_modes::BrowserMode,
        activation: super::browser_modes::ClickActivation,
    ) {
        use super::browser_modes::BrowserMode;

        let mut preferences = self.preferences.borrow_mut();
        let files = activation.files.stored();
        let folders = activation.folders.stored();
        match mode {
            BrowserMode::Columns => {
                preferences.columns_file_clicks = files;
                preferences.columns_folder_clicks = folders;
            }
            BrowserMode::Icons => {
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

    pub fn sort_preferences(&self) -> ViewPreferences {
        sort_preferences(&self.preferences.borrow())
    }

    /// Unlike `set_default_sort`, this keeps folder sorts that equal the new
    /// default: with per-folder sorts off, they are kept but ignored.
    pub fn set_sort_preferences(&self, preferences: ViewPreferences) {
        if preferences.sort_key == SortKey::Recency {
            return;
        }
        let mut stored = self.preferences.borrow_mut();
        stored.show_hidden = preferences.show_hidden;
        stored.folders_first = preferences.folders_first;
        store_sort(&mut stored, preferences.sort());
        drop(stored);
        self.save_preferences();
    }

    fn save_preferences(&self) {
        let changed = self.changes.record(&self.preferences.borrow());
        if !changed && !self.persistence_dirty.get() {
            return;
        }
        let path = settings_path();
        if let Some(error) = &self.load_failure {
            if changed {
                self.changes.notify(self);
                self.save_notices.unreadable_at_startup(&path, error);
            }
            return;
        }
        self.persistence_dirty.set(true);
        let snapshot = toml::Table::try_from(&*self.preferences.borrow()).map_err(io::Error::other);
        if let Ok(snapshot) = &snapshot {
            self.queue_settings_save(path.clone(), snapshot.clone());
        }
        if changed {
            self.changes.notify(self);
        }
        // Reported after bindings apply, and whether or not anything changed,
        // because a retry with an unchanged value can fail too.
        if let Err(error) = snapshot {
            self.save_notices.write_failed(&path, &error);
        }
    }

    fn queue_settings_save(&self, path: PathBuf, snapshot: toml::Table) {
        let generation = self.saves_requested.get() + 1;
        self.saves_requested.set(generation);
        let keys = {
            let mut last = self.last_queued.borrow_mut();
            let mut dirty = self.dirty_keys.borrow_mut();
            record_changes(&mut dirty, &last, &snapshot);
            *last = snapshot.clone();
            // Never serialized, so a migration cannot show up as a change.
            if self.preferences.borrow().legacy_omarchy_variant.is_none() {
                dirty.insert(("omarchy_variant".to_owned(), None));
            }
            dirty.extend(RETIRED_SETTINGS.map(|key| (key.to_owned(), None)));
            dirty.clone()
        };
        let target = path.clone();
        let saved = writer::save_locked(path.clone(), Policy::Config, move || {
            merged_settings(&target, &snapshot, &keys)
        });
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = saved
                .await
                .unwrap_or_else(|_| Err(io::Error::from_raw_os_error(libc::ECANCELED)));
            if let Some(manager) = this.upgrade() {
                manager.settings_saved(generation, &path, result);
            }
        });
    }

    fn settings_saved(&self, generation: u64, path: &Path, result: io::Result<()>) {
        if generation != self.saves_requested.get() {
            return;
        }
        match result {
            Ok(()) => {
                self.persistence_dirty.set(false);
                self.dirty_keys.borrow_mut().clear();
                self.save_notices.saved();
            }
            Err(error) => self.save_notices.write_failed(path, &error),
        }
    }

    /// Whether a save not yet written, or one that failed, would replace `key` on disk.
    fn setting_unsaved(&self, key: &str) -> bool {
        self.dirty_keys
            .borrow()
            .iter()
            .any(|(dirty, _)| dirty == key)
    }
}

fn read_state_file(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn read_preferences() -> io::Result<Preferences> {
    read_preferences_checked().map(|(preferences, _)| preferences)
}

/// Also returns the keys salvage had to repair.
fn read_preferences_checked() -> io::Result<(Preferences, BTreeSet<String>)> {
    let Some(contents) = read_state_file(&settings_path())? else {
        return Ok((Preferences::default(), BTreeSet::new()));
    };
    parse_preferences_checked(&contents)
}

fn parse_preferences(contents: &str) -> io::Result<Preferences> {
    parse_preferences_checked(contents).map(|(preferences, _)| preferences)
}

fn parse_preferences_checked(contents: &str) -> io::Result<(Preferences, BTreeSet<String>)> {
    let table: toml::Table = toml::from_str(contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    match table.clone().try_into() {
        Ok(preferences) => Ok((preferences, BTreeSet::new())),
        Err(error) => {
            tracing::warn!(%error, "settings file has invalid entries; keeping the valid ones");
            Ok(salvage_preferences(table))
        }
    }
}

const SETTINGS_LOCK_WAIT: Duration = Duration::from_millis(500);

/// Dropped by every save.
const RETIRED_SETTINGS: [&str; 1] = ["auto_refresh_interval"];

/// A changed key, or one entry of a key in [`ENTRY_MERGED_SETTINGS`].
type SettingKey = (String, Option<String>);

/// Maps whose entries are merged one by one, so entries another process added survive.
const ENTRY_MERGED_SETTINGS: [&str; 5] = [
    "omarchy_variants",
    "folder_colors",
    "custom_icons",
    "send_to_recent_destinations",
    "device_labels",
];

fn record_changes(changed: &mut BTreeSet<SettingKey>, before: &toml::Table, after: &toml::Table) {
    let empty = toml::Table::new();
    for key in after.keys().chain(before.keys()) {
        let (old, new) = (before.get(key), after.get(key));
        if old == new {
            continue;
        }
        fn entries<'a>(
            value: Option<&'a toml::Value>,
            empty: &'a toml::Table,
        ) -> Option<&'a toml::Table> {
            value.map_or(Some(empty), toml::Value::as_table)
        }
        match (entries(old, &empty), entries(new, &empty)) {
            (Some(old), Some(new)) if ENTRY_MERGED_SETTINGS.contains(&key.as_str()) => {
                for entry in new.keys().chain(old.keys()) {
                    if old.get(entry) != new.get(entry) {
                        changed.insert((key.clone(), Some(entry.clone())));
                    }
                }
            }
            _ => {
                changed.insert((key.clone(), None));
            }
        }
    }
}

/// Another Strata process, such as the portal chooser, saves the same file, so a save
/// replaces only what this process changed and keeps the rest as it is on disk,
/// including keys this version does not know. A file that cannot be read or parsed
/// is left alone.
fn merged_settings(
    path: &Path,
    snapshot: &toml::Table,
    changed: &BTreeSet<SettingKey>,
) -> io::Result<(Vec<u8>, Option<fs::File>)> {
    let lock = folder_view_settings::lock_saves(path, SETTINGS_LOCK_WAIT)
        .inspect_err(|error| {
            tracing::debug!(%error, path = %path.display(), "saving settings unlocked");
        })
        .ok();
    // Anything but a file is left for the write to report.
    let on_disk = if path.is_file() {
        read_state_file(path)?
    } else {
        None
    };
    let merged = match on_disk {
        None => snapshot.clone(),
        Some(contents) => {
            let mut table: toml::Table = toml::from_str(&contents)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            for (key, entry) in changed {
                let Some(entry) = entry else {
                    match snapshot.get(key) {
                        Some(value) => table.insert(key.clone(), value.clone()),
                        None => table.remove(key),
                    };
                    continue;
                };
                let value = snapshot
                    .get(key)
                    .and_then(toml::Value::as_table)
                    .and_then(|entries| entries.get(entry));
                let slot = table
                    .entry(key.clone())
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if !slot.is_table() {
                    *slot = toml::Value::Table(toml::Table::new());
                }
                let Some(entries) = slot.as_table_mut() else {
                    continue;
                };
                match value {
                    Some(value) => entries.insert(entry.clone(), value.clone()),
                    None => entries.remove(entry),
                };
                if entries.is_empty() {
                    table.remove(key);
                }
            }
            table
        }
    };
    let contents = toml::to_string_pretty(&merged).map_err(io::Error::other)?;
    Ok((contents.into_bytes(), lock))
}

/// Rebuilds preferences from every entry that deserializes on its own, so one
/// malformed value does not reset the rest (and, on the next save, overwrite
/// them with defaults). A malformed list or table keeps its other entries.
/// Also returns the keys whose saved value was dropped or filled in.
fn salvage_preferences(saved: toml::Table) -> (Preferences, BTreeSet<String>) {
    let Ok(mut merged) = toml::Table::try_from(Preferences::default()) else {
        return (
            Preferences::default(),
            saved.into_iter().map(|(key, _)| key).collect(),
        );
    };
    for (key, value) in saved.clone() {
        let kept = {
            let accepts = |candidate: &toml::Value| {
                let mut trial = merged.clone();
                trial.insert(key.clone(), candidate.clone());
                trial.try_into::<Preferences>().is_ok()
            };
            salvage_value(value, &accepts)
        };
        if let Some(kept) = kept {
            merged.insert(key, kept);
        }
    }
    let repaired = merged
        .keys()
        .chain(saved.keys())
        .filter(|key| merged.get(*key) != saved.get(*key))
        .cloned()
        .collect();
    (merged.try_into().unwrap_or_default(), repaired)
}

/// Returns `value` if `accepts` takes it, otherwise the entries that still pass
/// after dropping the bad ones, level by level. Entries are tried alone, so
/// salvage stays linear in a collection's size, and then checked together. A
/// value of the wrong shape, or a record missing a required field, yields `None`.
fn salvage_value(
    value: toml::Value,
    accepts: &dyn Fn(&toml::Value) -> bool,
) -> Option<toml::Value> {
    if accepts(&value) {
        return Some(value);
    }
    let kept = match value {
        toml::Value::Table(table) => {
            let mut kept = toml::Table::new();
            for (key, child) in table {
                let alone = |candidate: &toml::Value| {
                    let trial = toml::Table::from_iter([(key.clone(), candidate.clone())]);
                    accepts(&toml::Value::Table(trial))
                };
                if let Some(child) = salvage_value(child, &alone) {
                    kept.insert(key, child);
                }
            }
            toml::Value::Table(kept)
        }
        toml::Value::Array(items) => {
            let mut kept = Vec::new();
            for item in items {
                let alone =
                    |candidate: &toml::Value| accepts(&toml::Value::Array(vec![candidate.clone()]));
                if let Some(item) = salvage_value(item, &alone) {
                    kept.push(item);
                }
            }
            toml::Value::Array(kept)
        }
        _ => return None,
    };
    accepts(&kept).then_some(kept)
}

fn sort_preferences(preferences: &Preferences) -> ViewPreferences {
    let sorting = SortKey::from_stored(&preferences.sort_key)
        .zip(SortDirection::from_stored(&preferences.sort_direction))
        .unwrap_or((SortKey::Name, SortDirection::Ascending));
    ViewPreferences {
        show_hidden: preferences.show_hidden,
        folders_first: preferences.folders_first,
        sort_key: sorting.0,
        sort_direction: sorting.1,
    }
}

fn store_sort(
    preferences: &mut Preferences,
    (sort_key, sort_direction): (SortKey, SortDirection),
) -> bool {
    let Some(stored_key) = sort_key.stored_name() else {
        return false;
    };
    preferences.sort_key = stored_key.to_owned();
    preferences.sort_direction = sort_direction.stored_name().to_owned();
    true
}

fn configured_video_preview_backend(preferences: &Preferences) -> MediaPreviewBackend {
    match preferences.video_preview_backend.as_str() {
        "vaapi" => MediaPreviewBackend::VaApi,
        "vulkan" => MediaPreviewBackend::Vulkan,
        _ => MediaPreviewBackend::Automatic,
    }
}

fn configured_hardware_acceleration(preferences: &Preferences, polaris_available: bool) -> bool {
    preferences
        .hardware_accelerated_video_previews
        .unwrap_or(!polaris_available)
}

const GTK_DEFAULT_DPI: f64 = 96.0;
const GTK_DPI_UNITS: f64 = 1024.0;

pub(in crate::ui) fn desktop_text_scale_factor() -> f64 {
    gtk::Settings::default()
        .map(|settings| text_scale_factor_from_xft_dpi(settings.gtk_xft_dpi()))
        .unwrap_or(1.0)
}

fn text_scale_factor_from_xft_dpi(xft_dpi: i32) -> f64 {
    if xft_dpi <= 0 {
        return 1.0;
    }
    f64::from(xft_dpi) / (GTK_DEFAULT_DPI * GTK_DPI_UNITS)
}

pub(in crate::ui) fn snapped_root_font_px(root_font_px: u32, scale_factor: f64) -> f64 {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return f64::from(root_font_px);
    }
    // CSS px bypass Xft DPI. Apply desktop text scaling here, once; GTK applies
    // the surface's monitor scale independently. Keep hinted glyph rows whole.
    (f64::from(root_font_px) * scale_factor).round()
}

pub(in crate::ui) fn config_directory() -> PathBuf {
    crate::storage::config_directory()
}

fn settings_path() -> PathBuf {
    config_directory().join("settings.toml")
}

#[cfg(test)]
mod tests;
