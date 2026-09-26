// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{glib, prelude::*};
use serde::{Deserialize, Serialize};

use crate::{
    model::{FolderColorValue, SortDirection, SortKey, ViewPreferences},
    sandbox::MediaPreviewBackend,
    services::{Channel, CrossVolumeDropStrategy},
};

use super::icons_cell::{MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE};

mod bindings;
#[cfg(test)]
pub(in crate::ui) mod fixtures;
mod text_size;
pub(in crate::ui) use bindings::notify_live;
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub(in crate::ui) struct Preferences {
    mode: String,
    theme: String,
    #[serde(default = "default_enabled")]
    folder_peeking: bool,
    #[serde(default = "default_enabled")]
    single_click_previews: bool,
    #[serde(default = "default_enabled")]
    columns_mirror_selection: bool,
    #[serde(default = "default_enabled")]
    render_documents_by_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hardware_accelerated_video_previews: Option<bool>,
    #[serde(default = "default_video_preview_backend")]
    video_preview_backend: String,
    #[serde(default)]
    search_open_files_directly: bool,
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
    #[serde(default)]
    reduce_motion: bool,
    #[serde(default = "default_enabled")]
    element_glow: bool,
    #[serde(default = "default_browser_mode")]
    browser_mode: String,
    #[serde(default = "default_browser_density")]
    browser_density: String,
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
    sidebar_show_pictures: bool,
    #[serde(default = "default_enabled")]
    sidebar_show_videos: bool,
    #[serde(default)]
    show_hidden: bool,
    #[serde(default)]
    text_size: TextSize,
    #[serde(default)]
    interface_renderer: InterfaceRenderer,
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
    preview_autoplay: bool,
    #[serde(default)]
    auto_refresh_interval: u32,
    #[serde(default = "crate::sandbox::browser::default_worker_limit")]
    thumbnail_workers: usize,
    #[serde(default = "default_icons_thumbnail_size")]
    icons_thumbnail_size: i32,
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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    folder_colors: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    custom_icons: HashMap<String, String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            mode: "theme".to_owned(),
            theme: "tokyo-night".to_owned(),
            folder_peeking: true,
            single_click_previews: true,
            columns_mirror_selection: true,
            render_documents_by_default: true,
            hardware_accelerated_video_previews: None,
            video_preview_backend: default_video_preview_backend(),
            search_open_files_directly: false,
            type_to_search: true,
            arrow_navigation_scoped: false,
            tenxer_mode: false,
            filter_include_subfolders: true,
            show_keybinding_hints: true,
            reduce_motion: false,
            element_glow: true,
            browser_mode: default_browser_mode(),
            browser_density: default_browser_density(),
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
            sidebar_show_pictures: true,
            sidebar_show_videos: true,
            show_hidden: false,
            text_size: TextSize::default(),
            interface_renderer: InterfaceRenderer::default(),
            folders_first: true,
            sort_key: default_sort_key(),
            sort_direction: default_sort_direction(),
            check_for_updates: true,
            preview_muted: false,
            preview_volume: default_full_volume(),
            preview_text_wrap: false,
            preview_autoplay: false,
            auto_refresh_interval: 0,
            thumbnail_workers: crate::sandbox::browser::default_worker_limit(),
            icons_thumbnail_size: default_icons_thumbnail_size(),
            cross_volume_drop_strategy: default_cross_volume_drop_strategy(),
            open_folder_after_drop: false,
            date_format: default_date_format(),
            release_channel: default_release_channel(),
            default_directory: None,
            folder_colors: HashMap::new(),
            custom_icons: HashMap::new(),
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
        "pictures".to_owned(),
        "videos".to_owned(),
    ]
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

pub struct PreferenceManager {
    preferences: RefCell<Preferences>,
    startup_interface_renderer: InterfaceRenderer,
    changes: bindings::PreferenceChanges,
    persistence_dirty: Cell<bool>,
    persistence_enabled: bool,
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

    fn load() -> Rc<Self> {
        let loaded = read_preferences();
        let persistence_enabled = loaded.is_ok();
        let mut preferences = loaded.unwrap_or_else(|error| {
            tracing::warn!(%error, path = %settings_path().display(),
                "unable to load settings; using temporary defaults without saving; fix the file and restart Strata");
            Preferences::default()
        });
        preferences.preview_volume = normalized_volume(preferences.preview_volume);
        preferences.thumbnail_workers = preferences
            .thumbnail_workers
            .clamp(1, crate::sandbox::browser::MAX_WORKERS);
        preferences.icons_thumbnail_size = preferences
            .icons_thumbnail_size
            .clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        super::motion::set_reduce_motion(preferences.reduce_motion);
        crate::util::set_date_format(crate::util::DateFormat::parse(&preferences.date_format));

        Rc::new(Self {
            startup_interface_renderer: preferences.interface_renderer,
            changes: bindings::PreferenceChanges::new(preferences.clone()),
            persistence_dirty: Cell::new(false),
            persistence_enabled,
            preferences: RefCell::new(preferences),
        })
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

    /// Registers a process-lifetime callback invoked after preference changes are
    /// published. Observers run before widget bindings.
    pub(in crate::ui) fn observe(&self, observer: Rc<dyn Fn()>) {
        self.changes.observe(observer);
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
        self.arrow_navigation_scoped() && !self.tenxer_mode()
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

    pub fn thumbnail_workers(&self) -> usize {
        self.preferences.borrow().thumbnail_workers
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

    pub fn auto_refresh_interval(&self) -> u32 {
        self.preferences.borrow().auto_refresh_interval
    }

    pub fn set_auto_refresh_interval(&self, secs: u32) {
        self.preferences.borrow_mut().auto_refresh_interval = secs;
        self.save_preferences();
    }

    pub fn default_directory(&self) -> Option<PathBuf> {
        self.preferences.borrow().default_directory.clone()
    }

    pub fn set_default_directory(&self, path: Option<PathBuf>) {
        self.preferences.borrow_mut().default_directory = path;
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
        match self.preferences.borrow().browser_density.as_str() {
            "airy" => super::browser_modes::BrowserDensity::Airy,
            _ => super::browser_modes::BrowserDensity::Compact,
        }
    }

    pub fn set_browser_density(&self, density: super::browser_modes::BrowserDensity) {
        self.preferences.borrow_mut().browser_density = match density {
            super::browser_modes::BrowserDensity::Compact => "compact",
            super::browser_modes::BrowserDensity::Airy => "airy",
        }
        .to_owned();
        self.save_preferences();
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

    pub fn sidebar_places_visibility(&self) -> [bool; 9] {
        let preferences = self.preferences.borrow();
        [
            preferences.sidebar_show_home,
            preferences.sidebar_show_trash,
            preferences.sidebar_show_network,
            preferences.sidebar_show_recent,
            preferences.sidebar_show_desktop,
            preferences.sidebar_show_documents,
            preferences.sidebar_show_downloads,
            preferences.sidebar_show_pictures,
            preferences.sidebar_show_videos,
        ]
    }

    pub fn sort_preferences(&self) -> ViewPreferences {
        sort_preferences(&self.preferences.borrow())
    }

    pub fn set_sort_preferences(&self, preferences: ViewPreferences) {
        if preferences.sort_key == SortKey::Recency {
            return;
        }
        let mut stored = self.preferences.borrow_mut();
        stored.show_hidden = preferences.show_hidden;
        stored.folders_first = preferences.folders_first;
        let sort_key = match preferences.sort_key {
            SortKey::DeviceOrder => None,
            SortKey::Recency => None,
            SortKey::Name => Some("name"),
            SortKey::Size => Some("size"),
            SortKey::Modified => Some("modified"),
            SortKey::Type => Some("type"),
        };
        if let Some(sort_key) = sort_key {
            stored.sort_key = sort_key.to_owned();
            stored.sort_direction = match preferences.sort_direction {
                SortDirection::Ascending => "ascending",
                SortDirection::Descending => "descending",
            }
            .to_owned();
        }
        drop(stored);
        self.save_preferences();
    }

    fn save_preferences(&self) {
        let changed = self.changes.record(&self.preferences.borrow());
        if !changed && !self.persistence_dirty.get() {
            return;
        }
        if !self.persistence_enabled {
            if changed {
                self.changes.notify(self);
            }
            return;
        }
        self.persistence_dirty.set(true);
        let path = settings_path();
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let value =
                toml::to_string_pretty(&*self.preferences.borrow()).map_err(io::Error::other)?;
            crate::storage::atomic_write(&path, value.as_bytes())
        })();
        match result {
            Ok(()) => self.persistence_dirty.set(false),
            Err(error) => tracing::warn!(%error, "unable to save preference"),
        }
        if changed {
            self.changes.notify(self);
        }
    }
}

fn read_preferences() -> io::Result<Preferences> {
    let contents = match fs::read_to_string(settings_path()) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Preferences::default()),
        Err(error) => return Err(error),
    };
    let table: toml::Table = toml::from_str(&contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    match table.clone().try_into() {
        Ok(preferences) => Ok(preferences),
        Err(error) => {
            tracing::warn!(%error, "settings file has invalid entries; keeping the valid ones");
            Ok(salvage_preferences(table))
        }
    }
}

/// Rebuilds preferences from every entry that deserializes on its own, so one
/// malformed value does not reset the rest (and, on the next save, overwrite
/// them with defaults).
fn salvage_preferences(saved: toml::Table) -> Preferences {
    let Ok(mut merged) = toml::Table::try_from(Preferences::default()) else {
        return Preferences::default();
    };
    for (key, value) in saved {
        let previous = merged.insert(key.clone(), value);
        if merged.clone().try_into::<Preferences>().is_err() {
            match previous {
                Some(previous) => {
                    merged.insert(key, previous);
                }
                None => {
                    merged.remove(&key);
                }
            }
        }
    }
    merged.try_into().unwrap_or_default()
}

fn sort_preferences(preferences: &Preferences) -> ViewPreferences {
    let sorting = match (
        preferences.sort_key.as_str(),
        preferences.sort_direction.as_str(),
    ) {
        ("name", "ascending") => Some((SortKey::Name, SortDirection::Ascending)),
        ("name", "descending") => Some((SortKey::Name, SortDirection::Descending)),
        ("size", "ascending") => Some((SortKey::Size, SortDirection::Ascending)),
        ("size", "descending") => Some((SortKey::Size, SortDirection::Descending)),
        ("modified", "ascending") => Some((SortKey::Modified, SortDirection::Ascending)),
        ("modified", "descending") => Some((SortKey::Modified, SortDirection::Descending)),
        ("type", "ascending") => Some((SortKey::Type, SortDirection::Ascending)),
        ("type", "descending") => Some((SortKey::Type, SortDirection::Descending)),
        _ => None,
    }
    .unwrap_or((SortKey::Name, SortDirection::Ascending));
    ViewPreferences {
        show_hidden: preferences.show_hidden,
        folders_first: preferences.folders_first,
        sort_key: sorting.0,
        sort_direction: sorting.1,
    }
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
