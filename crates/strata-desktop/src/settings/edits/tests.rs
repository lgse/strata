// SPDX-License-Identifier: MIT

use std::time::Duration;

use crate::{sandbox::MediaPreviewBackend, services::SearchLimits};

use super::{
    MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, OmarchyVariant, Preferences,
    ThemeAvailability,
};

#[test]
fn theme_selection_repair_falls_back_or_adopts_omarchy_on_first_run() {
    let mut prefs = Preferences {
        mode: "system".to_owned(),
        theme: "unknown-id".to_owned(),
        ..Preferences::default()
    };
    prefs.normalize_theme_selection(
        |id| id == "azure-glow",
        ThemeAvailability {
            system: false,
            omarchy: true,
            settings_file_exists: true,
        },
    );
    assert_eq!(prefs.theme, "azure-glow");
    assert_eq!(prefs.mode, "theme");

    let mut fresh = Preferences::default();
    fresh.normalize_theme_selection(
        |_| true,
        ThemeAvailability {
            system: false,
            omarchy: true,
            settings_file_exists: false,
        },
    );
    assert_eq!(fresh.mode, "omarchy");
}

#[test]
fn omarchy_variants_migrate_contrast_only_and_drop_default() {
    let mut prefs = Preferences {
        legacy_omarchy_variant: Some(OmarchyVariant::Contrast),
        ..Preferences::default()
    };
    let active = prefs.migrate_active_omarchy_theme(" tokyo-night ");
    assert_eq!(active, "tokyo-night");
    assert_eq!(
        prefs.omarchy_variant(Some(&active)),
        OmarchyVariant::Contrast
    );
    assert!(prefs.legacy_omarchy_variant.is_none());

    assert!(!prefs.set_omarchy_variant(&active, OmarchyVariant::Contrast));
    assert!(prefs.set_omarchy_variant(&active, OmarchyVariant::Darker));
    assert!(!prefs.omarchy_variants.contains_key(&active));

    let mut legacy_lighter = Preferences {
        legacy_omarchy_variant: Some(OmarchyVariant::Lighter),
        ..Preferences::default()
    };
    legacy_lighter.migrate_active_omarchy_theme("nord");
    assert_eq!(
        legacy_lighter.omarchy_variant(Some("nord")),
        OmarchyVariant::Darker
    );
}

#[test]
fn video_backend_search_limits_and_adopted_defaults() {
    let mut prefs = Preferences::default();
    prefs.set_hardware_accelerated_video_previews(false);
    assert_eq!(prefs.hardware_accelerated_video_previews, Some(false));

    assert!(prefs.set_video_preview_backend(MediaPreviewBackend::VaApi));
    assert_eq!(prefs.video_preview_backend, "vaapi");
    assert!(!prefs.set_video_preview_backend(MediaPreviewBackend::Software));
    assert_eq!(prefs.video_preview_backend, "vaapi");

    prefs.search_time_limit = 0;
    prefs.search_entry_limit = 50_000;
    assert_eq!(
        prefs.search_limits(),
        SearchLimits {
            time_budget: None,
            max_entries: Some(50_000),
        }
    );
    prefs.search_time_limit = 15;
    assert_eq!(
        prefs.search_limits().time_budget,
        Some(Duration::from_secs(15))
    );

    let mut other = prefs.clone();
    assert!(!prefs.adopt_defaults(other.clone()));
    other.sort_key = "size".to_owned();
    other.icons_thumbnail_size = MAX_ICONS_THUMBNAIL_SIZE + 50;
    assert!(prefs.adopt_defaults(other));
    assert_eq!(prefs.sort_key, "size");
    assert_eq!(prefs.icons_thumbnail_size, MAX_ICONS_THUMBNAIL_SIZE);

    let mut tiny = prefs.clone();
    tiny.icons_thumbnail_size = MIN_ICONS_THUMBNAIL_SIZE - 10;
    assert!(prefs.adopt_defaults(tiny));
    assert_eq!(prefs.icons_thumbnail_size, MIN_ICONS_THUMBNAIL_SIZE);
}
