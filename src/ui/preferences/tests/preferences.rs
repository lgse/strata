// SPDX-License-Identifier: MIT

use std::{cell::RefCell, path::Path, rc::Rc};

use super::super::*;
use crate::{
    model::{
        EntryKind, FileEntry, Location, MetadataValue, SortDirection, SortKey, ViewPreferences,
    },
    test_support::gtk_test,
    ui::{
        browser_modes::{BrowserDensity, BrowserMode, ClickCount},
        preferences::fixtures::{
            non_default_preferences, seed_omarchy_for_test, seed_saved_preferences_for_test,
        },
        theme::ThemeManager,
    },
};

#[test]
fn recent_sort_is_not_stored_as_an_ordinary_folder_default() {
    gtk_test(
        "ui::preferences::tests::preferences::recent_sort_is_not_stored_as_an_ordinary_folder_default",
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
    saved.remove("open_folder_after_drop");
    saved.remove("date_format");
    let restored: Preferences = saved.try_into().expect("backward-compatible preferences");
    assert_eq!(
        restored,
        Preferences {
            filter_include_subfolders: true,
            open_folder_after_drop: false,
            date_format: "relative".into(),
            ..non_default_preferences()
        }
    );
}

#[test]
fn a_malformed_preference_does_not_discard_the_others() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.insert("show_hidden".into(), "yes".into());
    assert!(saved.clone().try_into::<Preferences>().is_err());

    assert_eq!(
        salvage_preferences(saved),
        Preferences {
            show_hidden: false,
            ..non_default_preferences()
        }
    );
}

#[test]
fn a_missing_required_preference_does_not_discard_the_others() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.remove("theme");
    assert!(saved.clone().try_into::<Preferences>().is_err());

    assert_eq!(
        salvage_preferences(saved),
        Preferences {
            theme: Preferences::default().theme,
            ..non_default_preferences()
        }
    );
}

fn assert_recovered_preferences_survive_save(
    corrupt: impl FnOnce(&mut toml::Table),
    mut expected: Preferences,
) {
    seed_saved_preferences_for_test();
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    corrupt(&mut saved);
    let malformed = toml::to_string(&saved).expect("syntactically valid TOML");
    fs::write(settings_path(), &malformed).expect("persist malformed preferences");

    let manager = PreferenceManager::shared();
    assert_eq!(*manager.preferences.borrow(), expected);
    assert_eq!(
        fs::read_to_string(settings_path()).expect("unchanged settings file"),
        malformed
    );
    expected.folder_peeking = true;
    manager.set_folder_peeking(true);

    let persisted: Preferences =
        toml::from_str(&fs::read_to_string(settings_path()).expect("saved file"))
            .expect("save repairs invalid preferences");
    assert_eq!(persisted, expected);
    assert_eq!(read_preferences().expect("saved preferences"), expected);
}

#[test]
fn unreadable_preferences_are_preserved_while_live_changes_still_apply() {
    gtk_test(
        "ui::preferences::tests::preferences::unreadable_preferences_are_preserved_while_live_changes_still_apply",
        || {
            seed_saved_preferences_for_test();
            let valid = fs::read(settings_path()).expect("saved fixture");
            for suffix in [b"\nthis is not valid toml [".as_slice(), b"\xff"] {
                let mut broken = valid.clone();
                broken.extend_from_slice(suffix);
                fs::write(settings_path(), &broken).expect("broken settings");
                let manager = PreferenceManager::load();
                let anchors = [
                    gtk::Box::new(gtk::Orientation::Vertical, 0),
                    gtk::Box::new(gtk::Orientation::Vertical, 0),
                ];
                let observations = anchors.each_ref().map(|anchor| {
                    let values = Rc::new(RefCell::new(Vec::new()));
                    let observed = values.clone();
                    manager.bind_preference(
                        anchor,
                        PreferenceManager::folder_peeking,
                        move |_, value| {
                            observed.borrow_mut().push(value);
                        },
                    );
                    values
                });
                manager.set_folder_peeking(false);
                manager.set_folder_peeking(false);
                for values in observations {
                    assert_eq!(*values.borrow(), [true, false]);
                }
                assert_eq!(
                    fs::read(settings_path()).expect("preserved settings"),
                    broken
                );
                fs::write(settings_path(), &valid).expect("repair settings");
                manager.set_folder_peeking(true);
                assert_eq!(
                    fs::read(settings_path()).expect("repair left untouched"),
                    valid
                );
                drop(manager);
            }
            let manager = PreferenceManager::load();
            assert_eq!(*manager.preferences.borrow(), non_default_preferences());
            manager.set_folder_peeking(true);
            assert!(
                read_preferences()
                    .expect("saving resumes after reload")
                    .folder_peeking
            );
        },
    );
}

#[test]
fn missing_settings_allow_first_run_saves() {
    gtk_test(
        "ui::preferences::tests::preferences::missing_settings_allow_first_run_saves",
        || {
            assert!(!settings_path().exists());
            let manager = PreferenceManager::load();
            manager.set_folder_peeking(false);
            assert!(!read_preferences().expect("first run save").folder_peeking);
        },
    );
}

#[test]
fn malformed_preferences_survive_startup_and_an_unrelated_save() {
    gtk_test(
        "ui::preferences::tests::preferences::malformed_preferences_survive_startup_and_an_unrelated_save",
        || {
            assert_recovered_preferences_survive_save(
                |saved| {
                    saved.insert("show_hidden".into(), "yes".into());
                },
                Preferences {
                    show_hidden: false,
                    ..non_default_preferences()
                },
            );
        },
    );
}

#[test]
fn missing_required_preferences_survive_startup_and_an_unrelated_save() {
    gtk_test(
        "ui::preferences::tests::preferences::missing_required_preferences_survive_startup_and_an_unrelated_save",
        || {
            assert_recovered_preferences_survive_save(
                |saved| {
                    saved.remove("theme");
                },
                Preferences {
                    theme: Preferences::default().theme,
                    ..non_default_preferences()
                },
            );
        },
    );
}

#[test]
fn multiple_invalid_preferences_do_not_block_later_valid_entries() {
    gtk_test(
        "ui::preferences::tests::preferences::multiple_invalid_preferences_do_not_block_later_valid_entries",
        || {
            assert_recovered_preferences_survive_save(
                |saved| {
                    saved.insert("auto_refresh_interval".into(), (-1).into());
                    saved.insert("hardware_accelerated_video_previews".into(), "no".into());
                    saved.insert("show_hidden".into(), "yes".into());
                    saved.insert("future_preference".into(), true.into());
                },
                Preferences {
                    auto_refresh_interval: 0,
                    hardware_accelerated_video_previews: None,
                    show_hidden: false,
                    ..non_default_preferences()
                },
            );
        },
    );
}

#[test]
fn fresh_preferences_select_tokyo_night_before_settings_opens() {
    gtk_test(
        "ui::preferences::tests::preferences::fresh_preferences_select_tokyo_night_before_settings_opens",
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
        "ui::preferences::tests::preferences::fresh_preferences_still_follow_available_omarchy_theme",
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
fn every_saved_preference_loads_before_any_settings_page_exists() {
    gtk_test(
        "ui::preferences::tests::preferences::every_saved_preference_loads_before_any_settings_page_exists",
        || {
            seed_saved_preferences_for_test();
            let themes = ThemeManager::shared();
            let manager = PreferenceManager::shared();
            assert_eq!(*manager.preferences.borrow(), non_default_preferences());
            assert!(!themes.follows_omarchy());
            assert_eq!(themes.selected_id(), "nord");
            assert!(!manager.folder_peeking());
            assert!(!manager.single_click_previews());
            assert!(!manager.columns_mirror_selection());
            assert!(!manager.hardware_accelerated_video_previews());
            assert_eq!(manager.video_preview_backend(), MediaPreviewBackend::Vulkan);
            assert_eq!(
                manager.media_preview_backend(),
                MediaPreviewBackend::Software
            );
            assert!(manager.search_open_files_directly());
            assert!(!manager.type_to_search());
            assert!(manager.arrow_navigation_scoped());
            assert!(!manager.filter_include_subfolders());
            assert!(!manager.show_keybinding_hints());
            assert!(manager.reduce_motion());
            assert!(!manager.element_glow());
            let windows = [gtk::Window::new(), gtk::Window::new()];
            for enabled in [false, true, false] {
                manager.set_element_glow(enabled);
                for window in &windows {
                    let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    window.set_child(Some(&surface));
                    #[expect(
                        deprecated,
                        reason = "GTK has no replacement API for resolving named CSS colors"
                    )]
                    let (glow, accent) = {
                        let style = surface.style_context();
                        (
                            style.lookup_color("theme_glow").expect("glow color"),
                            style.lookup_color("theme_accent").expect("accent color"),
                        )
                    };
                    if enabled {
                        assert_eq!(glow, accent);
                    } else {
                        assert_eq!(glow.alpha(), 0.0);
                        assert!(accent.alpha() > 0.0);
                    }
                }
            }
            for window in windows {
                window.close();
            }
            assert!(!crate::ui::motion::animations_enabled());
            assert_eq!(manager.browser_mode(), BrowserMode::List);
            assert_eq!(manager.browser_density(), BrowserDensity::Airy);
            assert!(manager.group_by_type());
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let activation = manager.click_activation(mode);
                assert_eq!(activation.files, ClickCount::One);
                assert_eq!(
                    activation.folders,
                    if mode == BrowserMode::Columns {
                        ClickCount::Two
                    } else {
                        ClickCount::One
                    }
                );
            }
            assert_eq!(
                manager.sidebar_order(),
                non_default_preferences().sidebar_order
            );
            assert!(!manager.sidebar_show_home());
            assert!(!manager.sidebar_show_trash());
            assert!(!manager.sidebar_show_network());
            assert!(!manager.sidebar_show_recent());
            assert!(!manager.sidebar_show_desktop());
            assert!(!manager.sidebar_show_documents());
            assert!(!manager.sidebar_show_downloads());
            assert!(!manager.sidebar_show_pictures());
            assert!(!manager.sidebar_show_videos());
            assert_eq!(
                manager.sidebar_places_visibility(),
                [
                    false, false, false, false, false, false, false, false, false
                ]
            );
            assert_eq!(manager.text_size(), TextSize::new(24));
            assert_eq!(
                manager.sort_preferences(),
                ViewPreferences {
                    show_hidden: true,
                    folders_first: false,
                    sort_key: SortKey::Size,
                    sort_direction: SortDirection::Descending,
                }
            );
            assert!(!manager.checks_for_updates());
            assert_eq!(manager.release_channel(), Channel::Nightly);
            assert!(manager.preview_muted());
            assert_eq!(manager.preview_volume(), 0.35);
            assert!(manager.preview_text_wrap());
            assert!(manager.preview_autoplay());
            assert_eq!(manager.auto_refresh_interval(), 600);
            assert_eq!(manager.thumbnail_workers(), 6);
            assert_eq!(
                manager.cross_volume_drop_strategy(),
                CrossVolumeDropStrategy::Move
            );
            assert_eq!(manager.date_format(), crate::util::DateFormat::Iso8601);
            assert_eq!(
                manager.default_directory(),
                Some(std::path::PathBuf::from("/fixture/default"))
            );
            assert_eq!(
                manager.folder_color(Path::new("/fixture/folder")),
                FolderColorValue::parse("red")
            );
            assert_eq!(
                manager.custom_icon(Path::new("/fixture/folder")).as_deref(),
                Some(crate::assets::icons::HOME)
            );
        },
    );
}

#[test]
fn saved_omarchy_mode_loads_and_changes_through_the_same_binding() {
    gtk_test(
        "ui::preferences::tests::preferences::saved_omarchy_mode_loads_and_changes_through_the_same_binding",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let mut preferences = non_default_preferences();
            preferences.mode = "omarchy".into();
            fs::write(
                settings_path(),
                toml::to_string(&preferences).expect("Omarchy preference fixture"),
            )
            .expect("persist Omarchy mode");
            let manager = ThemeManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_theme_preference(
                &anchor,
                ThemeManager::follows_omarchy,
                move |_, value| observed.borrow_mut().push(value),
            );
            assert_eq!(*values.borrow(), [true]);
            manager.set_follow_omarchy(false);
            manager.set_follow_omarchy(true);
            assert_eq!(*values.borrow(), [true, false, true]);
            assert_eq!(
                read_preferences().expect("saved theme mode").mode,
                "omarchy"
            );
        },
    );
}

#[test]
fn all_preference_setters_publish_and_persist_without_duplicate_notifications() {
    gtk_test(
        "ui::preferences::tests::preferences::all_preference_setters_publish_and_persist_without_duplicate_notifications",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let manager = PreferenceManager::shared();
            let themes = ThemeManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let observations = Rc::new(RefCell::new(Vec::new()));
            let observed = observations.clone();
            manager.bind_preference(
                &anchor,
                |manager| manager.preferences.borrow().clone(),
                move |_, value| observed.borrow_mut().push(value),
            );
            let preference_setters: &[fn(&PreferenceManager)] = &[
                |m| m.set_folder_peeking(true),
                |m| m.set_single_click_previews(true),
                |m| m.set_columns_mirror_selection(true),
                |m| m.set_render_documents_by_default(true),
                |m| m.set_hardware_accelerated_video_previews(true),
                |m| m.set_video_preview_backend(MediaPreviewBackend::VaApi),
                |m| m.set_search_open_files_directly(false),
                |m| m.set_type_to_search(true),
                |m| m.set_arrow_navigation_scoped(false),
                |m| m.set_filter_include_subfolders(true),
                |m| m.set_show_keybinding_hints(true),
                |m| m.set_reduce_motion(false),
                |m| m.set_element_glow(true),
                |m| m.set_browser_mode(BrowserMode::Icons),
                |m| m.set_browser_density(BrowserDensity::Compact),
                |m| m.set_group_by_type(false),
                |m| {
                    m.set_click_activation(
                        BrowserMode::Columns,
                        crate::ui::browser_modes::ClickActivation::default_for(
                            BrowserMode::Columns,
                        ),
                    )
                },
                |m| {
                    m.set_click_activation(
                        BrowserMode::Icons,
                        crate::ui::browser_modes::ClickActivation::default_for(BrowserMode::Icons),
                    )
                },
                |m| {
                    m.set_click_activation(
                        BrowserMode::List,
                        crate::ui::browser_modes::ClickActivation::default_for(BrowserMode::List),
                    )
                },
                |m| m.set_sidebar_order(default_sidebar_order()),
                |m| m.set_sidebar_show_home(true),
                |m| m.set_sidebar_show_trash(true),
                |m| m.set_sidebar_show_network(true),
                |m| m.set_sidebar_show_recent(true),
                |m| m.set_sidebar_show_desktop(true),
                |m| m.set_sidebar_show_documents(true),
                |m| m.set_sidebar_show_downloads(true),
                |m| m.set_sidebar_show_pictures(true),
                |m| m.set_sidebar_show_videos(true),
                |m| m.set_sort_preferences(ViewPreferences::default()),
                |m| m.set_text_size(TextSize::new(11)),
                |m| m.set_interface_renderer(InterfaceRenderer::System),
                |m| m.set_checks_for_updates(true),
                |m| m.set_release_channel(Channel::Stable),
                |m| m.set_preview_muted(false),
                |m| m.set_preview_volume(0.8),
                |m| m.set_preview_text_wrap(false),
                |m| m.set_preview_autoplay(false),
                |m| m.set_auto_refresh_interval(60),
                |m| m.set_thumbnail_workers(3),
                |m| m.set_icons_thumbnail_size(96),
                |m| m.set_cross_volume_drop_strategy(CrossVolumeDropStrategy::Copy),
                |m| m.set_date_format(crate::util::DateFormat::Long),
                |m| m.set_default_directory(None),
                |m| m.set_open_folder_after_drop(false),
                |m| m.set_folder_color(Path::new("/fixture/folder"), None),
                |m| m.set_custom_icon(Path::new("/fixture/folder"), None),
            ];
            let theme_setters: &[fn(&ThemeManager)] = &[
                |m| m.set_follow_omarchy(true),
                |m| m.select_theme("azure-glow"),
            ];
            let mut previous = toml::Table::try_from(&*manager.preferences.borrow())
                .expect("preference inventory");
            let all_keys = previous
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let mut changed_keys = std::collections::BTreeSet::new();
            let mut published = 1;
            let mut assert_setter = |published: &mut usize| {
                let expected = manager.preferences.borrow().clone();
                let current =
                    toml::Table::try_from(&expected).expect("changed preference inventory");
                for key in previous.keys().chain(current.keys()) {
                    if previous.get(key) != current.get(key) {
                        changed_keys.insert(key.clone());
                    }
                }
                previous = current;
                assert_eq!(
                    observations.borrow().len(),
                    *published,
                    "setter publishes exactly once"
                );
                assert_eq!(observations.borrow().last(), Some(&expected));
                assert_eq!(read_preferences().expect("saved preferences"), expected);
            };
            for setter in preference_setters {
                setter(&manager);
                published += 1;
                assert_setter(&mut published);
                setter(&manager);
                assert_eq!(
                    observations.borrow().len(),
                    published,
                    "preference setter ignores redundant values"
                );
            }
            for setter in theme_setters {
                setter(&themes);
                published += 1;
                assert_setter(&mut published);
                setter(&themes);
                assert_eq!(
                    observations.borrow().len(),
                    published,
                    "theme setter ignores redundant values"
                );
            }
            assert_eq!(
                changed_keys, all_keys,
                "every stored field needs setter and notification coverage"
            );
        },
    );
}

#[test]
fn saved_date_format_renders_before_settings_and_updates_bound_labels() {
    gtk_test(
        "ui::preferences::tests::preferences::saved_date_format_renders_before_settings_and_updates_bound_labels",
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
                mode: MetadataValue::Known(0o100644),
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
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
