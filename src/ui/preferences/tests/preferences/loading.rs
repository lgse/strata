// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn saved_language_applies_before_settings_and_changes_only_after_restart() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::saved_language_applies_before_settings_and_changes_only_after_restart",
        || {
            let mut preferences = non_default_preferences();
            preferences.language = crate::i18n::Language::French;
            fs::create_dir_all(settings_path().parent().expect("settings directory"))
                .expect("create settings directory");
            fs::write(
                settings_path(),
                toml::to_string(&preferences).expect("serialize settings"),
            )
            .expect("save fixture");
            let manager = PreferenceManager::load();
            assert_eq!(crate::i18n::tr("Language"), "Langue");
            assert!(!manager.language_restart_required());
            manager.set_language(crate::i18n::Language::Japanese);
            assert!(manager.language_restart_required());
            assert_eq!(crate::i18n::tr("Language"), "Langue");
            crate::storage::writer::settle();
            assert_eq!(
                read_preferences().expect("read saved language").language,
                crate::i18n::Language::Japanese
            );
            manager.set_language(crate::i18n::Language::French);
            assert!(!manager.language_restart_required());
            manager.set_language(crate::i18n::Language::Japanese);
            crate::storage::writer::settle();
            let restarted = PreferenceManager::load();
            assert!(!restarted.language_restart_required());
            assert_eq!(&*rust_i18n::locale(), "ja");
        },
    );
}

#[test]
fn recent_sort_is_not_stored_as_an_ordinary_folder_default() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::recent_sort_is_not_stored_as_an_ordinary_folder_default",
        || {
            seed_saved_preferences_for_test();
            let manager = PreferenceManager::load();
            let saved = manager.preferences.borrow().clone();

            manager.set_sort_preferences(ViewPreferences {
                sort_key: SortKey::Recency,
                sort_direction: SortDirection::Descending,
                ..ViewPreferences::default()
            });

            assert_eq!(*manager.preferences.borrow(), saved);
            crate::storage::writer::settle();
            let persisted: Preferences =
                toml::from_str(&fs::read_to_string(settings_path()).expect("saved preferences"))
                    .expect("persisted preferences");
            assert_eq!(persisted, saved);
        },
    );
}

#[test]
fn older_preferences_keep_backward_compatible_behavior_defaults() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.remove("filter_include_subfolders");
    saved.remove("search_time_limit");
    saved.remove("search_entry_limit");
    saved.remove("open_folder_after_drop");
    saved.remove("date_format");
    saved.remove("send_to_recent_destinations");
    saved.remove("tenxer_mode");
    saved.remove("omarchy_variants");
    saved.remove("file_icon_colors");
    saved.remove("folder_peeking");
    saved.remove("columns_show_sizes");
    saved.remove("language");
    saved.remove("preview_video_start");
    saved.remove("preview_text_wrap");
    let restored: Preferences = saved.try_into().expect("backward-compatible preferences");
    assert_eq!(
        restored,
        Preferences {
            language: crate::i18n::Language::Auto,
            filter_include_subfolders: true,
            search_time_limit: 10,
            search_entry_limit: 200_000,
            open_folder_after_drop: false,
            date_format: "relative".into(),
            send_to_recent_destinations: HashMap::new(),
            tenxer_mode: false,
            omarchy_variants: HashMap::new(),
            file_icon_colors: FileIconColors::Brand,
            folder_peeking: false,
            columns_show_sizes: true,
            preview_video_start: 0,
            preview_text_wrap: true,
            ..non_default_preferences()
        }
    );
}

#[test]
fn blank_interface_fonts_load_as_the_default_font() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::blank_interface_fonts_load_as_the_default_font",
        || {
            for (stored, expected) in [
                ("", None),
                ("   ", None),
                ("  Fixture Sans ", Some("Fixture Sans")),
            ] {
                assert_recovered_preferences_survive_save(
                    |saved| {
                        saved.insert("interface_font".into(), stored.into());
                    },
                    Preferences {
                        interface_font: expected.map(str::to_owned),
                        ..non_default_preferences()
                    },
                );
            }
        },
    );
}

#[test]
fn unlisted_search_limits_round_up_on_load_and_set() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::unlisted_search_limits_round_up_on_load_and_set",
        || {
            for (stored_secs, expected_secs, stored_entries, expected_entries) in [
                (0, 0, 0, 0),
                (1, 10, 1, 200_000),
                (11, 30, 200_001, 500_000),
                (61, 300, 500_001, 1_000_000),
                (301, 900, 1_000_001, 5_000_000),
                (901, 0, 5_000_001, 0),
            ] {
                assert_recovered_preferences_survive_save(
                    |saved| {
                        saved.insert("search_time_limit".into(), i64::from(stored_secs).into());
                        saved.insert(
                            "search_entry_limit".into(),
                            i64::from(stored_entries).into(),
                        );
                    },
                    Preferences {
                        search_time_limit: expected_secs,
                        search_entry_limit: expected_entries,
                        ..non_default_preferences()
                    },
                );
            }

            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            for (requested, expected) in [(45, 60), (u32::MAX, 0), (300, 300), (10, 10)] {
                manager.set_search_time_limit(requested);
                assert_eq!(manager.search_time_limit(), expected);
                crate::storage::writer::settle();
                assert_eq!(
                    read_preferences()
                        .expect("saved preferences")
                        .search_time_limit,
                    expected
                );
            }
            for (requested, expected) in [
                (300_000, 500_000),
                (u32::MAX, 0),
                (200_000, 200_000),
                (1, 200_000),
            ] {
                manager.set_search_entry_limit(requested);
                assert_eq!(manager.search_entry_limit(), expected);
                crate::storage::writer::settle();
                assert_eq!(
                    read_preferences()
                        .expect("saved preferences")
                        .search_entry_limit,
                    expected
                );
            }
            manager.set_search_time_limit(0);
            manager.set_search_entry_limit(0);
            assert_eq!(
                manager.search_limits(),
                SearchLimits {
                    time_budget: None,
                    max_entries: None,
                }
            );
        },
    );
}

#[test]
fn video_starts_past_the_maximum_clamp_on_load_and_set() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::video_starts_past_the_maximum_clamp_on_load_and_set",
        || {
            for stored in [100, 5_000] {
                assert_recovered_preferences_survive_save(
                    |saved| {
                        saved.insert("preview_video_start".into(), i64::from(stored).into());
                    },
                    Preferences {
                        preview_video_start: 99,
                        ..non_default_preferences()
                    },
                );
            }

            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            manager.set_preview_video_start(u32::MAX);
            assert_eq!(manager.preview_video_start(), 99);
            crate::storage::writer::settle();
            assert_eq!(
                read_preferences()
                    .expect("saved preferences")
                    .preview_video_start,
                99
            );
        },
    );
}

#[test]
fn fresh_preferences_select_tokyo_night_before_settings_opens() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::fresh_preferences_select_tokyo_night_before_settings_opens",
        || {
            assert!(!settings_path().exists());
            let themes = ThemeManager::shared();
            let manager = PreferenceManager::shared();
            assert_eq!(themes.selected_id(), "tokyo-night");
            assert!(manager.filter_include_subfolders());
            assert!(!themes.follows_omarchy());
            assert_eq!(
                themes.current_tokens().expect("selected theme").name,
                "Tokyo Night"
            );
            assert!(!settings_path().exists());
        },
    );
}

#[test]
fn fresh_preferences_still_follow_available_omarchy_theme() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::fresh_preferences_still_follow_available_omarchy_theme",
        || {
            seed_omarchy_for_test();
            let themes = ThemeManager::shared();
            assert!(themes.follows_omarchy());
            assert_eq!(themes.selected_id(), "tokyo-night");
            themes.set_follow_omarchy(false);
            assert_eq!(
                themes.current_tokens().expect("selected theme").name,
                "Tokyo Night"
            );
        },
    );
}

#[test]
fn density_sliders_save_snap_points_as_presets_and_other_values_as_custom() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::density_sliders_save_snap_points_as_presets_and_other_values_as_custom",
        || {
            let manager = PreferenceManager::shared();
            assert_eq!(manager.browser_density(), BrowserDensity::Compact);
            assert_eq!(manager.frame_density(), FrameDensity::Compact);

            manager.set_browser_density(BrowserDensity::from_px(11));
            manager.set_frame_density(FrameDensity::from_px(4));
            {
                let saved = manager.preferences.borrow();
                assert_eq!(saved.browser_density, "custom");
                assert_eq!(saved.browser_density_custom_px, 11);
                assert_eq!(saved.frame_density, "airy");
            }
            assert_eq!(manager.browser_density(), BrowserDensity::Custom(11));
            assert_eq!(manager.frame_density(), FrameDensity::Airy);

            manager.set_browser_density(BrowserDensity::Custom(4));
            assert_eq!(manager.preferences.borrow().browser_density, "compact");
            assert_eq!(manager.browser_density(), BrowserDensity::Compact);
            assert_eq!(
                manager.frame_density(),
                FrameDensity::Airy,
                "content changes leave the frame alone"
            );

            manager.set_frame_density(FrameDensity::Custom(200));
            assert_eq!(manager.frame_density(), FrameDensity::Custom(12));
        },
    );
}

#[test]
fn saved_date_format_renders_before_settings_and_updates_bound_labels() {
    gtk_test(
        "ui::preferences::tests::preferences::loading::saved_date_format_renders_before_settings_and_updates_bound_labels",
        || {
            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let seconds = glib::DateTime::now_local().expect("local time").to_unix() - 120;
            let entry = FileEntry {
                location: Location::local("/fixture/recent.txt"),
                native_name: "recent.txt".into(),
                display_name: "recent.txt".into(),
                thumbnail_path: None,
                kind: EntryKind::File,
                size: MetadataValue::Known(4),
                modified_unix_seconds: MetadataValue::Known(seconds),
                created_unix_seconds: MetadataValue::Unknown,
                mode: MetadataValue::Known(0o100644),
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
                listable: MetadataValue::Unknown,
                recent_uri: None,
            };
            let absolute = |pattern: &str| {
                glib::DateTime::from_unix_local(seconds)
                    .expect("modified date")
                    .format(pattern)
                    .expect("format")
                    .to_string()
            };
            let windows = [gtk::Window::new(), gtk::Window::new()];
            let labels: Vec<gtk::Label> = windows
                .iter()
                .map(|window| {
                    let label = gtk::Label::new(None);
                    window.set_child(Some(&label));
                    crate::util::set_modified_date(&label, Some(&entry), "—");
                    label
                })
                .collect();
            for label in &labels {
                assert_eq!(label.label(), absolute("%Y-%m-%d %H:%M"));
            }
            manager.set_date_format(crate::util::DateFormat::Long);
            for label in &labels {
                assert_eq!(label.label(), absolute("%B %-d, %Y, %H:%M"));
            }
            crate::util::set_modified_date(&labels[0], None, "unknown");
            manager.set_date_format(crate::util::DateFormat::Relative);
            assert_eq!(labels[0].label(), "unknown");
            let text = labels[1].label();
            assert!(text == "Just now" || text.ends_with(" ago"), "{text}");
            let mut rebound = entry.clone();
            rebound.modified_unix_seconds = MetadataValue::Known(seconds - 86400);
            crate::util::set_modified_date(&labels[0], Some(&rebound), "—");
            manager.set_date_format(crate::util::DateFormat::Iso8601);
            let expected = glib::DateTime::from_unix_local(seconds - 86400)
                .expect("rebound date")
                .format("%Y-%m-%d %H:%M")
                .expect("format");
            assert_eq!(labels[0].label(), expected);
            assert_eq!(labels[1].label(), absolute("%Y-%m-%d %H:%M"));
            let rebuilt = gtk::Label::new(None);
            windows[1].set_child(Some(&rebuilt));
            crate::util::set_modified_date(&rebuilt, Some(&entry), "—");
            assert_eq!(rebuilt.label(), absolute("%Y-%m-%d %H:%M"));
            manager.set_date_format(crate::util::DateFormat::Long);
            assert_eq!(rebuilt.label(), absolute("%B %-d, %Y, %H:%M"));
            for window in windows {
                window.close();
            }
        },
    );
}
