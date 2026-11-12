// SPDX-License-Identifier: MIT

//! Defaults for settings a file omits.

use super::*;

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
            compact_full_names: false,
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
            sidebar_pinned_collapsed: false,
            sidebar_devices_collapsed: false,
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
            natural_sort: true,
            case_sensitive_sort: false,
            check_for_updates: true,
            preview_muted: false,
            preview_volume: default_full_volume(),
            preview_text_wrap: true,
            unit_system: UnitSystem::Metric,
            preview_autoplay: false,
            preview_video_start: 0,
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

pub(super) fn default_enabled() -> bool {
    true
}

pub(super) fn default_release_channel() -> String {
    "stable".to_owned()
}

pub(super) fn default_browser_mode() -> String {
    "columns".to_owned()
}

pub(super) fn default_video_preview_backend() -> String {
    "automatic".to_owned()
}

pub(super) fn default_browser_density() -> String {
    "compact".to_owned()
}

pub(super) fn default_frame_density() -> String {
    "compact".to_owned()
}

/// Only read when the matching density is `"custom"`.
pub(super) fn default_density_custom_px() -> u8 {
    0
}

pub(super) fn default_file_clicks() -> u8 {
    2
}

pub(super) fn default_folder_clicks() -> u8 {
    1
}

pub(super) fn default_double_clicks() -> u8 {
    2
}

pub(super) fn default_sidebar_order() -> Vec<String> {
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

pub(super) fn default_search_time_limit() -> u32 {
    SearchLimits::default()
        .time_budget
        .and_then(|budget| u32::try_from(budget.as_secs()).ok())
        .unwrap_or(0)
}

pub(super) fn default_search_entry_limit() -> u32 {
    SearchLimits::default()
        .max_entries
        .and_then(|entries| u32::try_from(entries).ok())
        .unwrap_or(0)
}

pub(super) fn default_sort_key() -> String {
    "name".to_owned()
}

pub(super) fn default_sort_direction() -> String {
    "ascending".to_owned()
}

pub(super) fn default_full_volume() -> f64 {
    1.0
}

pub(super) fn default_icons_thumbnail_size() -> i32 {
    64
}

pub(super) fn default_date_format() -> String {
    crate::util::DateFormat::default().as_str().to_owned()
}

pub(super) fn default_cross_volume_drop_strategy() -> String {
    CrossVolumeDropStrategy::Ask.as_str().to_owned()
}

pub(super) fn default_natural_sort() -> bool {
    true
}
