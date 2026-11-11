// SPDX-License-Identifier: MIT

//! How settings are stored: string forms of sorts and backends, and the ranges values are kept within.

use super::*;

pub fn normalized_volume(volume: f64) -> f64 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

pub const SEARCH_TIME_LIMIT_CHOICES: [(&str, u32); 6] = [
    ("10 s", 10),
    ("30 s", 30),
    ("1 min", 60),
    ("5 min", 300),
    ("15 min", 900),
    ("No limit", 0),
];

pub const SEARCH_ENTRY_LIMIT_CHOICES: [(&str, u32); 5] = [
    ("200,000", 200_000),
    ("500,000", 500_000),
    ("1,000,000", 1_000_000),
    ("5,000,000", 5_000_000),
    ("No limit", 0),
];

/// A blank family keeps the bundled default rather than naming no font.
pub fn normalized_font_family(family: Option<String>) -> Option<String> {
    family
        .map(|family| family.trim().to_owned())
        .filter(|family| !family.is_empty())
}

pub const MAX_PREVIEW_VIDEO_START: u32 = 99;

pub fn normalized_video_start(percent: u32) -> u32 {
    percent.min(MAX_PREVIEW_VIDEO_START)
}

/// Unlisted limits round up to the next choice; 0 and anything past the largest
/// choice mean no limit.
pub fn normalized_search_limit(value: u32, choices: &[(&str, u32)]) -> u32 {
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

pub fn sort_preferences(preferences: &Preferences) -> ViewPreferences {
    let sorting = SortKey::from_stored(&preferences.sort_key)
        .zip(SortDirection::from_stored(&preferences.sort_direction))
        .unwrap_or((SortKey::Name, SortDirection::Ascending));
    ViewPreferences {
        show_hidden: preferences.show_hidden,
        folders_first: preferences.folders_first,
        sort_key: sorting.0,
        sort_direction: sorting.1,
        name_comparison: crate::model::NameComparison {
            natural: preferences.natural_sort,
            case_sensitive: preferences.case_sensitive_sort,
        },
    }
}

pub fn store_sort(
    preferences: &mut Preferences,
    (sort_key, sort_direction, comparison): crate::model::SortOptions,
) -> bool {
    let Some(stored_key) = sort_key.stored_name() else {
        return false;
    };
    preferences.sort_key = stored_key.to_owned();
    preferences.sort_direction = sort_direction.stored_name().to_owned();
    preferences.natural_sort = comparison.natural;
    preferences.case_sensitive_sort = comparison.case_sensitive;
    true
}

pub fn configured_video_preview_backend(preferences: &Preferences) -> MediaPreviewBackend {
    match preferences.video_preview_backend.as_str() {
        "vaapi" => MediaPreviewBackend::VaApi,
        "vulkan" => MediaPreviewBackend::Vulkan,
        _ => MediaPreviewBackend::Automatic,
    }
}

pub fn configured_hardware_acceleration(
    preferences: &Preferences,
    polaris_available: bool,
) -> bool {
    preferences
        .hardware_accelerated_video_previews
        .unwrap_or(!polaris_available)
}

impl Preferences {
    /// Brings loaded values back within the ranges and column sets the interface accepts.
    pub fn normalize_loaded(&mut self) {
        self.preview_volume = normalized_volume(self.preview_volume);
        self.search_time_limit =
            normalized_search_limit(self.search_time_limit, &SEARCH_TIME_LIMIT_CHOICES);
        self.search_entry_limit =
            normalized_search_limit(self.search_entry_limit, &SEARCH_ENTRY_LIMIT_CHOICES);
        self.preview_video_start = normalized_video_start(self.preview_video_start);
        self.thumbnail_workers = self
            .thumbnail_workers
            .clamp(1, crate::sandbox::browser::MAX_WORKERS);
        self.icons_thumbnail_size = self
            .icons_thumbnail_size
            .clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        self.chooser_column_width = self.chooser_column_width.filter(|width| *width > 0);
        self.browser_column_width = self.browser_column_width.filter(|width| *width > 0);
        self.chooser_list_columns = self.chooser_list_columns.filter(ListColumns::is_valid);
        self.browser_list_columns = self.browser_list_columns.filter(ListColumns::is_valid);
        self.list_column_order = sanitize_list_column_order(&self.list_column_order);
        self.list_hidden_columns = sanitize_list_hidden_columns(&self.list_hidden_columns);
        self.interface_font = normalized_font_family(self.interface_font.take());
    }
}
