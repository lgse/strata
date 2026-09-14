// SPDX-License-Identifier: MIT

use super::super::*;
use crate::{
    test_support::gtk_test,
    ui::browser_modes::{BrowserDensity, BrowserMode, ClickCount},
};

fn non_default_preferences() -> Preferences {
    // Deliberately exhaustive: adding a stored preference requires extending this fixture.
    Preferences {
        mode: "theme".into(),
        theme: "nord".into(),
        folder_peeking: false,
        single_click_previews: false,
        hardware_accelerated_video_previews: Some(false),
        video_preview_backend: "vulkan".into(),
        search_open_files_directly: true,
        type_to_search: false,
        arrow_navigation_scoped: true,
        filter_include_subfolders: false,
        show_keybinding_hints: false,
        reduce_motion: true,
        element_glow: false,
        browser_mode: "list".into(),
        browser_density: "airy".into(),
        group_by_type: true,
        columns_file_clicks: 1,
        columns_folder_clicks: 2,
        icons_file_clicks: 1,
        icons_folder_clicks: 1,
        list_file_clicks: 1,
        list_folder_clicks: 1,
        sidebar_order: vec![
            "videos".into(),
            "pictures".into(),
            "downloads".into(),
            "documents".into(),
            "desktop".into(),
        ],
        show_hidden: true,
        text_size: TextSize::new(24),
        folders_first: false,
        sort_key: "size".into(),
        sort_direction: "descending".into(),
        check_for_updates: false,
        preview_muted: true,
        preview_volume: 0.35,
        preview_text_wrap: true,
        auto_refresh_interval: 600,
        cross_volume_drop_strategy: CrossVolumeDropStrategy::Move.as_str().into(),
        open_folder_after_drop: true,
        release_channel: "nightly".into(),
        default_directory: Some("/fixture/default".into()),
        folder_colors: HashMap::from([("/fixture/folder".into(), "red".into())]),
        custom_icons: HashMap::from([(
            "/fixture/folder".into(),
            crate::assets::icons::HOME.into(),
        )]),
        smart_folders: Some(vec![SmartFolderDef {
            id: "smart-fixture".into(),
            name: "Fixture Search".into(),
            query: "pdf".into(),
            rules: vec![crate::model::SmartQueryRule::Kind(
                crate::model::FileCategory::Document,
            )],
            roots: vec![PathBuf::from("/tmp")],
            show_hidden: true,
        }]),
    }
}

impl ThemeManager {
    pub(in crate::ui) fn seed_omarchy_for_test() {
        let state = omarchy_state_dir();
        fs::create_dir_all(state.join("theme")).expect("isolated Omarchy theme directory");
        fs::write(state.join("theme.name"), "fixture").expect("isolated Omarchy name");
        fs::write(
            state.join("theme/colors.toml"),
            "background = '#112233'\nforeground = '#ddeeff'\naccent = '#445566'\n",
        )
        .expect("isolated Omarchy colors");
    }

    pub(in crate::ui) fn seed_saved_preferences_for_test() {
        let path = settings_path();
        fs::create_dir_all(path.parent().expect("settings parent"))
            .expect("isolated preferences directory");
        fs::write(
            path,
            toml::to_string(&non_default_preferences()).expect("serialize complete fixture"),
        )
        .expect("persist complete fixture");
    }
}

#[test]
fn older_preferences_keep_backward_compatible_behavior_defaults() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.remove("filter_include_subfolders");
    saved.remove("open_folder_after_drop");
    let restored: Preferences = saved.try_into().expect("backward-compatible preferences");
    assert_eq!(
        restored,
        Preferences {
            filter_include_subfolders: true,
            open_folder_after_drop: false,
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
    ThemeManager::seed_saved_preferences_for_test();
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    corrupt(&mut saved);
    let malformed = toml::to_string(&saved).expect("syntactically valid TOML");
    fs::write(settings_path(), &malformed).expect("persist malformed preferences");

    let manager = ThemeManager::shared();
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
        "ui::theme::tests::preferences::unreadable_preferences_are_preserved_while_live_changes_still_apply",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let valid = fs::read(settings_path()).expect("saved fixture");
            for suffix in [b"\nthis is not valid toml [".as_slice(), b"\xff"] {
                let mut broken = valid.clone();
                broken.extend_from_slice(suffix);
                fs::write(settings_path(), &broken).expect("broken settings");
                let manager = ThemeManager::load();
                let anchors = [
                    gtk::Box::new(gtk::Orientation::Vertical, 0),
                    gtk::Box::new(gtk::Orientation::Vertical, 0),
                ];
                let observations = anchors.each_ref().map(|anchor| {
                    let values = Rc::new(RefCell::new(Vec::new()));
                    let observed = values.clone();
                    manager.bind_preference(
                        anchor,
                        ThemeManager::folder_peeking,
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
            let manager = ThemeManager::load();
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
        "ui::theme::tests::preferences::missing_settings_allow_first_run_saves",
        || {
            assert!(!settings_path().exists());
            let manager = ThemeManager::load();
            manager.set_folder_peeking(false);
            assert!(!read_preferences().expect("first run save").folder_peeking);
        },
    );
}

#[test]
fn malformed_preferences_survive_startup_and_an_unrelated_save() {
    gtk_test(
        "ui::theme::tests::preferences::malformed_preferences_survive_startup_and_an_unrelated_save",
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
        "ui::theme::tests::preferences::missing_required_preferences_survive_startup_and_an_unrelated_save",
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
        "ui::theme::tests::preferences::multiple_invalid_preferences_do_not_block_later_valid_entries",
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
        "ui::theme::tests::preferences::fresh_preferences_select_tokyo_night_before_settings_opens",
        || {
            assert!(!settings_path().exists());
            let manager = ThemeManager::shared();
            assert_eq!(manager.selected_id(), "tokyo-night");
            assert!(manager.filter_include_subfolders());
            assert!(!manager.follows_omarchy());
            assert_eq!(
                manager.current_tokens().expect("selected theme").name,
                "Tokyo Night"
            );
            assert!(!settings_path().exists());
        },
    );
}

#[test]
fn fresh_preferences_still_follow_available_omarchy_theme() {
    gtk_test(
        "ui::theme::tests::preferences::fresh_preferences_still_follow_available_omarchy_theme",
        || {
            ThemeManager::seed_omarchy_for_test();
            let manager = ThemeManager::shared();
            assert!(manager.follows_omarchy());
            assert_eq!(manager.selected_id(), "tokyo-night");
            manager.set_follow_omarchy(false);
            assert_eq!(
                manager.current_tokens().expect("selected theme").name,
                "Tokyo Night"
            );
        },
    );
}

#[test]
fn every_saved_preference_loads_before_any_settings_page_exists() {
    gtk_test(
        "ui::theme::tests::preferences::every_saved_preference_loads_before_any_settings_page_exists",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            assert_eq!(*manager.preferences.borrow(), non_default_preferences());
            assert!(!manager.follows_omarchy());
            assert_eq!(manager.selected_id(), "nord");
            assert!(!manager.folder_peeking());
            assert!(!manager.single_click_previews());
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
            assert_eq!(manager.auto_refresh_interval(), 600);
            assert_eq!(
                manager.cross_volume_drop_strategy(),
                CrossVolumeDropStrategy::Move
            );
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
            assert_eq!(
                manager.smart_folders(),
                non_default_preferences()
                    .smart_folders
                    .expect("custom smart folders")
            );
        },
    );
}

#[test]
fn saved_omarchy_mode_loads_and_changes_through_the_same_binding() {
    gtk_test(
        "ui::theme::tests::preferences::saved_omarchy_mode_loads_and_changes_through_the_same_binding",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            ThemeManager::seed_omarchy_for_test();
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
            manager.bind_preference(&anchor, ThemeManager::follows_omarchy, move |_, value| {
                observed.borrow_mut().push(value)
            });
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
        "ui::theme::tests::preferences::all_preference_setters_publish_and_persist_without_duplicate_notifications",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            ThemeManager::seed_omarchy_for_test();
            let manager = ThemeManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let observations = Rc::new(RefCell::new(Vec::new()));
            let observed = observations.clone();
            manager.bind_preference(
                &anchor,
                |manager| manager.preferences.borrow().clone(),
                move |_, value| observed.borrow_mut().push(value),
            );
            let setters: &[fn(&ThemeManager)] = &[
                |m| m.set_folder_peeking(true),
                |m| m.set_single_click_previews(true),
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
                |m| m.set_sort_preferences(ViewPreferences::default()),
                |m| m.set_text_size(TextSize::new(11)),
                |m| m.set_checks_for_updates(true),
                |m| m.set_release_channel(Channel::Stable),
                |m| m.set_preview_muted(false),
                |m| m.set_preview_volume(0.8),
                |m| m.set_preview_text_wrap(false),
                |m| m.set_auto_refresh_interval(60),
                |m| m.set_cross_volume_drop_strategy(CrossVolumeDropStrategy::Copy),
                |m| m.set_default_directory(None),
                |m| m.set_open_folder_after_drop(false),
                |m| m.set_folder_color(Path::new("/fixture/folder"), None),
                |m| m.set_custom_icon(Path::new("/fixture/folder"), None),
                |m| m.set_follow_omarchy(true),
                |m| m.select_theme("azure-glow"),
                |m| m.remove_smart_folder("smart-fixture"),
            ];
            let mut previous = toml::Table::try_from(&*manager.preferences.borrow())
                .expect("preference inventory");
            let all_keys = previous
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let mut changed_keys = std::collections::BTreeSet::new();
            for (index, setter) in setters.iter().enumerate() {
                setter(&manager);
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
                    index + 2,
                    "setter {index} publishes exactly once"
                );
                assert_eq!(observations.borrow().last(), Some(&expected));
                assert_eq!(read_preferences().expect("saved preferences"), expected);
                setter(&manager);
                assert_eq!(
                    observations.borrow().len(),
                    index + 2,
                    "setter {index} ignores redundant values"
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
fn smart_folder_definitions_round_trip_through_preferences() {
    gtk_test("smart_folder_round_trip", || {
        let definition = SmartFolderDef {
            id: "smart-round-trip".into(),
            name: "Round Trip".into(),
            query: "invoice".into(),
            rules: vec![crate::model::SmartQueryRule::Kind(
                crate::model::FileCategory::Document,
            )],
            roots: vec![PathBuf::from("/home"), PathBuf::from("/tmp")],
            show_hidden: false,
        };
        let manager = ThemeManager::shared();
        manager.add_smart_folder(definition.clone());
        assert_eq!(manager.smart_folder("smart-round-trip"), Some(definition));
        manager.rename_smart_folder("smart-round-trip", "Renamed Invoices");
        assert_eq!(
            manager.smart_folder("smart-round-trip").map(|f| f.name),
            Some("Renamed Invoices".to_string())
        );
        manager.remove_smart_folder("smart-round-trip");
        assert_eq!(manager.smart_folder("smart-round-trip"), None);
    });
}

#[test]
fn removing_every_default_smart_folder_does_not_restore_them() {
    gtk_test("remove_default_smart_folders", || {
        let manager = ThemeManager::shared();
        let defaults = manager.smart_folders();
        assert!(!defaults.is_empty());
        for folder in defaults {
            manager.remove_smart_folder(&folder.id);
        }
        assert!(manager.smart_folders().is_empty());
        let saved = read_preferences().expect("saved preferences");
        assert_eq!(saved.smart_folders, Some(Vec::new()));
    });
}

#[test]
fn adding_smart_folder_with_existing_id_replaces_it() {
    gtk_test("smart_folder_replace", || {
        let manager = ThemeManager::shared();
        let original = SmartFolderDef {
            id: "smart-replace".into(),
            name: "Original".into(),
            query: "old".into(),
            rules: vec![],
            roots: vec![],
            show_hidden: false,
        };
        manager.add_smart_folder(original);
        let updated = SmartFolderDef {
            id: "smart-replace".into(),
            name: "Updated".into(),
            query: "new".into(),
            rules: vec![crate::model::SmartQueryRule::Kind(
                crate::model::FileCategory::Image,
            )],
            roots: vec![],
            show_hidden: true,
        };
        manager.add_smart_folder(updated.clone());
        assert_eq!(manager.smart_folder("smart-replace"), Some(updated));
        manager.remove_smart_folder("smart-replace");
    });
}
