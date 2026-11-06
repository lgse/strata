// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn empty_send_to_history_is_omitted_from_saved_preferences() {
    let serialized = toml::Table::try_from(Preferences::default()).expect("default preferences");
    assert!(!serialized.contains_key("send_to_recent_destinations"));
}

#[test]
fn malformed_send_to_history_does_not_discard_other_preferences() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.insert("send_to_recent_destinations".into(), "invalid".into());
    assert!(saved.clone().try_into::<Preferences>().is_err());

    assert_eq!(
        salvage_preferences(saved).0,
        Preferences {
            send_to_recent_destinations: HashMap::new(),
            ..non_default_preferences()
        }
    );
}

#[test]
fn send_to_history_is_device_scoped_deduplicated_capped_and_persistent() {
    gtk_test(
        "ui::preferences::tests::preferences::recovery::send_to_history_is_device_scoped_deduplicated_capped_and_persistent",
        || {
            let manager = PreferenceManager::shared();
            for path in ["A", "B", "C", "B", "D"] {
                manager.remember_send_to_destination("volume:kingston", Path::new(path), None);
            }
            manager.remember_send_to_destination("volume:sandisk", Path::new("Backup"), None);
            manager.remember_send_to_destination("volume:drive-root", Path::new(""), None);

            let expected = vec![PathBuf::from("D"), PathBuf::from("B"), PathBuf::from("C")];
            assert_eq!(
                manager.send_to_recent_destinations("volume:kingston"),
                expected
            );
            assert_eq!(
                manager.send_to_recent_destinations("volume:sandisk"),
                [PathBuf::from("Backup")]
            );
            assert!(
                manager
                    .send_to_recent_destinations("volume:drive-root")
                    .is_empty()
            );
            crate::storage::writer::settle();

            let reloaded = PreferenceManager::load();
            assert_eq!(
                reloaded.send_to_recent_destinations("volume:kingston"),
                expected
            );
            assert_eq!(
                reloaded.send_to_recent_destinations("volume:sandisk"),
                [PathBuf::from("Backup")]
            );
            crate::storage::writer::settle();
            let saved: Preferences =
                toml::from_str(&fs::read_to_string(settings_path()).expect("saved preferences"))
                    .expect("persisted preferences reload");
            assert!(
                !saved
                    .send_to_recent_destinations
                    .contains_key("volume:drive-root")
            );
        },
    );
}

#[test]
fn a_malformed_preference_does_not_discard_the_others() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.insert("show_hidden".into(), "yes".into());
    saved.insert(
        "omarchy_variants".into(),
        toml::Table::from_iter([("fixture".into(), "unknown".into())]).into(),
    );
    assert!(saved.clone().try_into::<Preferences>().is_err());

    assert_eq!(
        salvage_preferences(saved).0,
        Preferences {
            show_hidden: false,
            omarchy_variants: HashMap::new(),
            ..non_default_preferences()
        }
    );
}

#[test]
fn one_malformed_entry_keeps_the_rest_of_its_collection() {
    let corrupt: toml::Table = toml::from_str(
        r#"
        search_exclusions = [".venv", 12, "/fixture/custom_excluded"]
        list_hidden_columns = ["size", "not_a_column", "mode"]
        list_column_order = ["name", "size", "bogus", "modified"]

        [omarchy_variants]
        fixture = "lighter"
        broken = "not-a-variant"

        [device_labels]
        "volume:fixture-kingston" = "Research drive"
        "volume:broken" = 5

        [send_to_recent_destinations]
        "volume:fixture-kingston" = ["Academia/2026", 7, "Teaching"]
        "volume:broken" = "not a list"
        "volume:fixture-sandisk" = ["Backup"]

        [browser_list_columns]
        mode = "wide"
        size = 80
        type = 95
        modified = 140
        "#,
    )
    .expect("fixture TOML");
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.extend(corrupt);
    assert!(saved.clone().try_into::<Preferences>().is_err());

    assert_eq!(
        salvage_preferences(saved).0,
        Preferences {
            search_exclusions: vec![".venv".into(), "/fixture/custom_excluded".into()],
            list_hidden_columns: vec![ListColumnId::Size, ListColumnId::Mode],
            list_column_order: vec![
                ListColumnId::Name,
                ListColumnId::Size,
                ListColumnId::Modified,
            ],
            omarchy_variants: HashMap::from([("fixture".into(), OmarchyVariant::Lighter)]),
            device_labels: HashMap::from([(
                "volume:fixture-kingston".into(),
                "Research drive".into()
            )]),
            send_to_recent_destinations: HashMap::from([
                (
                    "volume:fixture-kingston".into(),
                    vec![PathBuf::from("Academia/2026"), PathBuf::from("Teaching")],
                ),
                (
                    "volume:fixture-sandisk".into(),
                    vec![PathBuf::from("Backup")]
                ),
            ]),
            browser_list_columns: None,
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
        salvage_preferences(saved).0,
        Preferences {
            theme: Preferences::default().theme,
            ..non_default_preferences()
        }
    );
}

#[test]
fn malformed_preferences_survive_startup_and_an_unrelated_save() {
    gtk_test(
        "ui::preferences::tests::preferences::recovery::malformed_preferences_survive_startup_and_an_unrelated_save",
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
        "ui::preferences::tests::preferences::recovery::missing_required_preferences_survive_startup_and_an_unrelated_save",
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
        "ui::preferences::tests::preferences::recovery::multiple_invalid_preferences_do_not_block_later_valid_entries",
        || {
            assert_recovered_preferences_survive_save(
                |saved| {
                    saved.insert("watch_network_folders".into(), "no".into());
                    saved.insert("hardware_accelerated_video_previews".into(), "no".into());
                    saved.insert("show_hidden".into(), "yes".into());
                    saved.insert("future_preference".into(), true.into());
                    saved.insert("auto_refresh_interval".into(), 600.into());
                    saved.extend(
                        toml::from_str::<toml::Table>(
                            r#"
                            search_exclusions = [".venv", 12, "/fixture/custom_excluded"]

                            [omarchy_variants]
                            fixture = "lighter"
                            broken = "not-a-variant"

                            [send_to_recent_destinations]
                            "volume:fixture-kingston" = ["Academia/2026", 7, "Teaching"]
                            "volume:fixture-sandisk" = ["Backup"]
                            "#,
                        )
                        .expect("fixture TOML"),
                    );
                },
                Preferences {
                    watch_network_folders: true,
                    hardware_accelerated_video_previews: None,
                    show_hidden: false,
                    ..non_default_preferences()
                },
            );
        },
    );
}
