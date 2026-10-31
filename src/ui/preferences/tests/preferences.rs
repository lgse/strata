// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use sourceview5::prelude::BufferExt as _;

use super::super::*;
use crate::{
    model::{
        EntryKind, FileEntry, FolderSort, Location, MetadataValue, SortDirection, SortKey,
        ViewPreferences,
    },
    services::SearchLimits,
    test_support::gtk_test,
    ui::{
        browser_modes::{BrowserDensity, BrowserMode, ClickCount, DensityScale, FrameDensity},
        keys::{Binding, Map, bindings},
        preferences::fixtures::{
            non_default_preferences, seed_omarchy_colors_for_test, seed_omarchy_for_test,
            seed_saved_preferences_for_test,
        },
        theme::ThemeManager,
    },
};

#[test]
fn saved_language_applies_before_settings_and_changes_only_after_restart() {
    gtk_test(
        "ui::preferences::tests::preferences::saved_language_applies_before_settings_and_changes_only_after_restart",
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
            ..non_default_preferences()
        }
    );
}

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
        "ui::preferences::tests::preferences::send_to_history_is_device_scoped_deduplicated_capped_and_persistent",
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

fn assert_recovered_preferences_survive_save(
    corrupt: impl FnOnce(&mut toml::Table),
    mut expected: Preferences,
) {
    seed_saved_preferences_for_test();
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    corrupt(&mut saved);
    let malformed_keys: Vec<String> = saved.keys().cloned().collect();
    let malformed = toml::to_string(&saved).expect("syntactically valid TOML");
    fs::write(settings_path(), &malformed).expect("persist malformed preferences");

    let manager = PreferenceManager::shared();
    assert_eq!(*manager.preferences.borrow(), expected);
    crate::storage::writer::settle();
    assert_eq!(
        fs::read_to_string(settings_path()).expect("unchanged settings file"),
        malformed
    );
    expected.folder_peeking = false;
    manager.set_folder_peeking(false);
    crate::storage::writer::settle();

    let saved = fs::read_to_string(settings_path()).expect("saved file");
    let persisted: Preferences = toml::from_str(&saved).expect("save repairs invalid preferences");
    assert_eq!(persisted, expected);
    let saved_keys: BTreeSet<String> = toml::from_str::<toml::Table>(&saved)
        .expect("saved TOML")
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let every_key = toml::Table::try_from(non_default_preferences()).expect("every preference");
    let kept_keys: BTreeSet<String> = toml::Table::try_from(&expected)
        .expect("known preferences")
        .into_iter()
        .map(|(key, _)| key)
        .chain(malformed_keys.into_iter().filter(|key| {
            !every_key.contains_key(key) && !RETIRED_SETTINGS.contains(&key.as_str())
        }))
        .collect();
    assert_eq!(
        saved_keys, kept_keys,
        "a save keeps the valid and unknown preferences"
    );
    crate::storage::writer::settle();
    assert_eq!(read_preferences().expect("saved preferences"), expected);
}

#[test]
fn a_save_keeps_the_settings_another_process_wrote() {
    gtk_test(
        "ui::preferences::tests::preferences::a_save_keeps_the_settings_another_process_wrote",
        || {
            seed_saved_preferences_for_test();
            let read = || {
                toml::from_str::<toml::Table>(
                    &fs::read_to_string(settings_path()).expect("saved file"),
                )
                .expect("saved table")
            };
            let write = |table: &toml::Table| {
                fs::write(settings_path(), toml::to_string(table).expect("settings"))
                    .expect("other process saves");
            };
            let mut repairable = read();
            repairable.insert("search_time_limit".into(), toml::Value::Integer(45));
            write(&repairable);
            let manager = PreferenceManager::load();
            let (peeking, tenxer) = {
                let preferences = manager.preferences.borrow();
                (preferences.folder_peeking, preferences.tenxer_mode)
            };

            let mut other = read();
            other.insert("tenxer_mode".into(), toml::Value::Boolean(!tenxer));
            other.insert(
                "list_hidden_columns".into(),
                toml::Value::Array(vec!["size".into()]),
            );
            other.insert("future_setting".into(), toml::Value::String("kept".into()));
            other.insert("auto_refresh_interval".into(), toml::Value::Integer(600));
            other["device_labels"]
                .as_table_mut()
                .expect("device labels")
                .insert("volume:other".into(), "Other drive".into());
            write(&other);

            let saved = || {
                crate::storage::writer::settle();
                read()
            };
            manager.set_folder_peeking(!peeking);
            manager.set_device_label("volume:this", "This drive");
            let table = saved();
            assert_eq!(table["folder_peeking"].as_bool(), Some(!peeking));
            assert_eq!(
                table["search_time_limit"].as_integer(),
                Some(60),
                "a repair made at startup is saved"
            );
            assert_eq!(table["tenxer_mode"].as_bool(), Some(!tenxer));
            assert_eq!(table["list_hidden_columns"], other["list_hidden_columns"]);
            assert_eq!(table["future_setting"].as_str(), Some("kept"));
            assert!(!table.contains_key("auto_refresh_interval"));
            let labels = table["device_labels"].as_table().expect("device labels");
            assert_eq!(labels["volume:other"].as_str(), Some("Other drive"));
            assert_eq!(labels["volume:this"].as_str(), Some("This drive"));

            manager.set_folder_peeking(peeking);
            let table = saved();
            assert_eq!(table["folder_peeking"].as_bool(), Some(peeking));
            assert_eq!(table["tenxer_mode"].as_bool(), Some(!tenxer));
            assert_eq!(table["future_setting"].as_str(), Some("kept"));

            let broken = "this is not valid toml [";
            fs::write(settings_path(), broken).expect("broken by another process");
            manager.set_folder_peeking(!peeking);
            crate::storage::writer::settle();
            assert_eq!(
                fs::read_to_string(settings_path()).expect("broken settings"),
                broken,
                "a file that cannot be parsed is not overwritten"
            );
        },
    );
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
                let window = save_notice_window(&manager);
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
                assert!(save_notices(&window).is_empty(), "nothing changed yet");
                manager.set_folder_peeking(true);
                manager.set_folder_peeking(true);
                for values in observations {
                    assert_eq!(*values.borrow(), [false, true]);
                }
                let notices = save_notices(&window);
                assert_eq!(notices.len(), 1, "{notices:?}");
                assert_notice(
                    &notices[0],
                    "Settings file can't be read",
                    &format!(
                        "Strata couldn't read “{}” when it started:",
                        settings_path().display()
                    ),
                );
                crate::storage::writer::settle();
                assert_eq!(
                    fs::read(settings_path()).expect("preserved settings"),
                    broken
                );
                fs::write(settings_path(), &valid).expect("repair settings");
                manager.set_folder_peeking(false);
                crate::storage::writer::settle();
                assert_eq!(
                    fs::read(settings_path()).expect("repair left untouched"),
                    valid
                );
                assert_eq!(save_notices(&window).len(), 1, "one notice per session");
                window.destroy();
                drop(manager);
            }
            let manager = PreferenceManager::load();
            let window = save_notice_window(&manager);
            assert_eq!(*manager.preferences.borrow(), non_default_preferences());
            manager.set_folder_peeking(false);
            assert!(save_notices(&window).is_empty());
            crate::storage::writer::settle();
            assert!(
                !read_preferences()
                    .expect("saving resumes after reload")
                    .folder_peeking
            );
        },
    );
}

#[test]
fn changes_apply_at_once_and_reach_the_file_on_the_settings_writer() {
    gtk_test(
        "ui::preferences::tests::preferences::changes_apply_at_once_and_reach_the_file_on_the_settings_writer",
        || {
            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let anchors = [
                gtk::Box::new(gtk::Orientation::Vertical, 0),
                gtk::Box::new(gtk::Orientation::Vertical, 0),
            ];
            let values = Rc::new(RefCell::new(Vec::new()));
            for anchor in &anchors {
                let observed = values.clone();
                manager.bind_preference(
                    anchor,
                    PreferenceManager::folder_peeking,
                    move |_, value| {
                        observed.borrow_mut().push(value);
                    },
                );
            }
            let held = crate::storage::writer::hold();
            manager.set_folder_peeking(false);
            assert_eq!(*values.borrow(), [true, true, false, false]);
            assert!(
                read_preferences()
                    .expect("saved preferences")
                    .folder_peeking,
                "the file waits for the settings writer"
            );
            drop(held);
            crate::storage::writer::settle();
            assert!(
                !read_preferences()
                    .expect("saved preferences")
                    .folder_peeking
            );

            let held = crate::storage::writer::hold();
            manager.set_folder_peeking(true);
            let releasing = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                drop(held);
            });
            crate::storage::writer::flush_at_exit();
            assert!(
                read_preferences()
                    .expect("saved preferences")
                    .folder_peeking,
                "quitting waits for queued saves"
            );
            releasing.join().expect("released writer");
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
            assert!(!manager.folder_peeking());
            manager.set_folder_peeking(true);
            crate::storage::writer::settle();
            assert!(read_preferences().expect("first run save").folder_peeking);
            crate::storage::writer::settle();
            assert!(PreferenceManager::load().folder_peeking());
            manager.set_folder_peeking(false);
            crate::storage::writer::settle();
            assert!(!PreferenceManager::load().folder_peeking());
        },
    );
}

#[test]
fn symlinked_settings_save_through_to_the_target() {
    gtk_test(
        "ui::preferences::tests::preferences::symlinked_settings_save_through_to_the_target",
        || {
            seed_saved_preferences_for_test();
            let config_home = std::env::var_os("XDG_CONFIG_HOME").expect("isolated config home");
            let target = Path::new(&config_home)
                .parent()
                .expect("sandbox root")
                .join("dotfiles/settings.toml");
            fs::create_dir_all(target.parent().expect("dotfiles directory"))
                .expect("dotfiles directory");
            fs::rename(settings_path(), &target).expect("move settings into dotfiles");
            std::os::unix::fs::symlink(&target, settings_path()).expect("dotfiles link");

            let manager = PreferenceManager::load();
            assert!(manager.folder_peeking());
            manager.set_folder_peeking(false);
            crate::storage::writer::settle();

            assert!(
                !manager.persistence_dirty.get(),
                "the save through the link must not stay pending"
            );
            assert!(
                fs::symlink_metadata(settings_path())
                    .expect("settings link")
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(
                fs::read_link(settings_path()).expect("settings link"),
                target
            );
            let saved: Preferences =
                toml::from_str(&fs::read_to_string(&target).expect("dotfiles settings"))
                    .expect("dotfiles settings parse");
            assert!(!saved.folder_peeking);
            drop(manager);
            crate::storage::writer::settle();
            assert!(!PreferenceManager::load().folder_peeking());
        },
    );
}

#[test]
fn failed_saves_show_one_notice_per_failure_streak() {
    gtk_test(
        "ui::preferences::tests::preferences::failed_saves_show_one_notice_per_failure_streak",
        || {
            let manager = PreferenceManager::load();
            let window = save_notice_window(&manager);
            fs::create_dir_all(settings_path()).expect("block the settings file with a directory");

            let other = gtk::Window::new();
            other.present();
            wait_for(|| !window.is_active(), "another window to take focus");
            manager.set_folder_peeking(true);
            crate::storage::writer::settle();
            assert!(
                save_notices(&window).is_empty(),
                "a change while no browser window is active only logs"
            );
            other.destroy();
            window.present();
            wait_for(|| window.is_active(), "the notice window to become active");
            manager.set_folder_peeking(true);
            window.set_visible(false);
            crate::storage::writer::settle();
            assert!(
                save_notices(&window).is_empty(),
                "a window hidden before the notice opens shows nothing"
            );
            window.present();
            wait_for(|| window.is_active(), "the notice window to return");

            manager.set_folder_peeking(true);
            manager.set_type_to_search(!manager.type_to_search());
            crate::storage::writer::settle();
            let notices = save_notices(&window);
            assert_eq!(
                notices.len(),
                1,
                "one notice per failure streak: {notices:?}"
            );
            assert_notice(
                &notices[0],
                "Settings can't be saved",
                &format!(
                    "Strata couldn't write “{path}”: The destination “{path}” is not a regular file.",
                    path = settings_path().display()
                ),
            );

            fs::remove_dir(settings_path()).expect("repair the settings file");
            manager.set_folder_peeking(true);
            crate::storage::writer::settle();
            assert!(read_preferences().expect("retried save").folder_peeking);
            assert_eq!(save_notices(&window).len(), 1);

            fs::remove_file(settings_path()).expect("saved settings file");
            fs::create_dir(settings_path()).expect("block the settings file again");
            manager.set_folder_peeking(false);
            crate::storage::writer::settle();
            assert_eq!(
                save_notices(&window).len(),
                2,
                "a failure after a successful save starts a new streak"
            );
            window.destroy();
        },
    );
}

#[derive(Debug)]
struct SaveNoticeText {
    title: String,
    summary: String,
    detail: String,
}

fn save_notice_window(manager: &PreferenceManager) -> gtk::Window {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&gtk::Button::with_label("Origin")));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    wait_for(|| window.is_active(), "the notice window to become active");
    manager.register_save_notice_window(&window);
    window
}

fn wait_for(condition: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "waiting for {what}");
        glib::MainContext::default().iteration(false);
    }
}

fn save_notices(window: &gtk::Window) -> Vec<SaveNoticeText> {
    while glib::MainContext::default().iteration(false) {}
    fn descendants(widget: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
        found.push(widget.clone());
        let mut child = widget.first_child();
        while let Some(current) = child {
            descendants(&current, found);
            child = current.next_sibling();
        }
    }
    let mut widgets = Vec::new();
    descendants(window.upcast_ref(), &mut widgets);
    widgets
        .iter()
        .filter(|widget| widget.has_css_class("app-modal-layer"))
        .map(|layer| {
            let mut inner = Vec::new();
            descendants(layer, &mut inner);
            let text = |class: &str| {
                inner
                    .iter()
                    .find(|widget| widget.has_css_class(class))
                    .and_then(|widget| {
                        widget
                            .downcast_ref::<gtk::Label>()
                            .map(|label| label.text())
                    })
                    .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
                    .unwrap_or_default()
            };
            SaveNoticeText {
                title: text("action-dialog-title"),
                summary: text("action-dialog-subtitle"),
                detail: text("action-dialog-description"),
            }
        })
        .collect()
}

fn assert_notice(notice: &SaveNoticeText, title: &str, detail_start: &str) {
    let compact = |text: &str| text.split_whitespace().collect::<String>();
    assert_eq!(notice.title, title);
    assert_eq!(notice.summary, "Changes last only until Strata closes");
    assert!(
        compact(&notice.detail).starts_with(&compact(detail_start)),
        "{notice:?}"
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

#[test]
fn unlisted_search_limits_round_up_on_load_and_set() {
    gtk_test(
        "ui::preferences::tests::preferences::unlisted_search_limits_round_up_on_load_and_set",
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
            assert!(manager.folder_peeking());
            assert!(!manager.single_click_previews());
            assert!(!manager.columns_mirror_selection());
            assert!(!manager.columns_show_sizes());
            assert!(!manager.hardware_accelerated_video_previews());
            assert_eq!(manager.video_preview_backend(), MediaPreviewBackend::Vulkan);
            assert_eq!(
                manager.media_preview_backend(),
                MediaPreviewBackend::Software
            );
            assert!(manager.search_open_files_directly());
            assert_eq!(
                manager.search_exclusions(),
                vec![".venv", "/fixture/custom_excluded"]
            );
            assert_eq!(manager.search_time_limit(), 300);
            assert_eq!(manager.search_entry_limit(), 1_000_000);
            assert_eq!(
                manager.search_limits(),
                SearchLimits {
                    time_budget: Some(Duration::from_secs(300)),
                    max_entries: Some(1_000_000),
                }
            );
            assert!(!manager.type_to_search());
            assert!(manager.arrow_navigation_scoped());
            assert!(manager.tenxer_mode());
            assert!(!manager.filter_include_subfolders());
            assert!(!manager.show_keybinding_hints());
            assert!(!manager.window_show_close());
            assert!(manager.window_show_minimize());
            assert!(manager.window_show_maximize());
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
                            style.lookup_color("strata_glow").expect("glow color"),
                            style.lookup_color("strata_accent").expect("accent color"),
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
            let display = gtk::gdk::Display::default().expect("test display");
            let user_css = gtk::CssProvider::new();
            user_css.load_from_string(
                "@define-color theme_bg #ff00ff; @define-color theme_accent #00ff00;",
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &user_css,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
            for theme in ["nord", "azure-glow", "nord"] {
                themes.select_theme(theme);
                let tokens = themes.current_tokens().expect("active theme");
                for window in &windows {
                    #[expect(
                        deprecated,
                        reason = "GTK has no replacement API for resolving named CSS colors"
                    )]
                    let style = window.style_context();
                    #[expect(
                        deprecated,
                        reason = "GTK has no replacement API for resolving named CSS colors"
                    )]
                    for (name, expected) in [
                        ("strata_bg", tokens.background.as_str()),
                        ("strata_accent", tokens.accent.as_str()),
                        ("theme_bg", "#ff00ff"),
                        ("theme_accent", "#00ff00"),
                    ] {
                        assert_eq!(
                            style.lookup_color(name).expect("named color"),
                            gtk::gdk::RGBA::parse(expected).expect("token color"),
                            "{theme}: {name}",
                        );
                    }
                }
            }
            gtk::style_context_remove_provider_for_display(&display, &user_css);
            for window in windows {
                window.close();
            }
            assert!(!crate::ui::motion::animations_enabled());
            assert_eq!(manager.browser_mode(), BrowserMode::List);
            assert_eq!(manager.browser_density(), BrowserDensity::Airy);
            assert_eq!(manager.frame_density(), FrameDensity::Custom(6));
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
            assert!(!manager.sidebar_show_music());
            assert!(!manager.sidebar_show_pictures());
            assert!(!manager.sidebar_show_videos());
            assert!(!manager.sidebar_expanded());
            assert!(manager.sidebar_compact());
            assert_eq!(
                manager.sidebar_places_visibility(),
                [
                    false, false, false, false, false, false, false, false, false, false
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
            assert!(!manager.remember_folder_views());
            assert!(!manager.checks_for_updates());
            assert_eq!(manager.release_channel(), Channel::Nightly);
            assert!(manager.preview_muted());
            assert_eq!(manager.preview_volume(), 0.35);
            assert!(manager.preview_text_wrap());
            assert_eq!(manager.unit_system(), UnitSystem::Imperial);
            assert!(manager.preview_autoplay());
            assert!(!manager.watch_network_folders());
            assert_eq!(manager.thumbnail_workers(), 6);
            assert!(!manager.show_thumbnails());
            assert!(manager.stl_thumbnails());
            assert_eq!(
                manager.list_column_order(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Size,
                    ListColumnId::Type,
                    ListColumnId::Modified,
                    ListColumnId::Mode,
                ]
            );
            assert_eq!(manager.list_hidden_columns(), vec![ListColumnId::Mode]);
            assert_eq!(
                manager.cross_volume_drop_strategy(),
                CrossVolumeDropStrategy::Move
            );
            assert_eq!(manager.date_format(), crate::util::DateFormat::Iso8601);
            assert_eq!(
                manager.device_label("volume:fixture-kingston").as_deref(),
                Some("Research drive")
            );
            assert_eq!(
                manager.send_to_recent_destinations("volume:fixture-kingston"),
                [PathBuf::from("Academia/2026"), PathBuf::from("Teaching")]
            );
            assert_eq!(
                manager.send_to_recent_destinations("volume:fixture-sandisk"),
                [PathBuf::from("Backup")]
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
            let keymap = manager.keymap();
            assert_eq!(
                keymap.bindings(Map::Default, bindings::SETTINGS),
                [
                    Binding::ctrl(gtk::gdk::Key::comma),
                    Binding::ctrl_alt(gtk::gdk::Key::F12)
                ]
            );
            assert_eq!(
                keymap.bindings(Map::Tenxer, bindings::SIDEBAR_FOCUS),
                [
                    Binding::ctrl_shift(gtk::gdk::Key::b),
                    Binding::ctrl_alt(gtk::gdk::Key::F11)
                ]
            );
            assert_eq!(
                keymap.bindings(Map::Tenxer, bindings::SETTINGS),
                bindings::SETTINGS.defaults(Map::Tenxer)
            );
        },
    );
}

#[test]
fn legacy_omarchy_variant_migrates_to_the_active_theme() {
    gtk_test(
        "ui::preferences::tests::preferences::legacy_omarchy_variant_migrates_to_the_active_theme",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let mut saved =
                toml::Table::try_from(non_default_preferences()).expect("saved preferences");
            saved.remove("omarchy_variants");
            saved.insert("mode".into(), "omarchy".into());
            saved.insert("omarchy_variant".into(), "high_contrast".into());
            fs::write(
                settings_path(),
                toml::to_string(&saved).expect("legacy settings"),
            )
            .expect("persist legacy variant");
            let themes = ThemeManager::shared();
            let preferences = PreferenceManager::shared();
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Contrast);
            let migrated = themes.appearance_tokens();
            preferences.set_folder_peeking(false);
            crate::storage::writer::settle();
            let saved = fs::read_to_string(settings_path()).expect("migrated settings");
            let saved: toml::Table = toml::from_str(&saved).expect("migrated settings table");
            assert!(!saved.contains_key("omarchy_variant"));
            assert_eq!(
                read_preferences()
                    .expect("migrated variants")
                    .omarchy_variants,
                HashMap::from([("fixture".into(), OmarchyVariant::Contrast)])
            );
            preferences.set_omarchy_variant(OmarchyVariant::Darker);
            assert_ne!(themes.appearance_tokens(), migrated);
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
            let preferences = PreferenceManager::shared();
            let windows = [gtk::Window::new(), gtk::Window::new()];
            let buffer = gtk::TextBuffer::new(None);
            buffer.create_tag(Some("document-accent"), &[]);
            crate::ui::theme::register_document_buffer(&buffer);
            let source = crate::ui::theme::new_source_buffer();
            let startup = manager.appearance_tokens();
            for variant in [
                OmarchyVariant::Darker,
                OmarchyVariant::Lighter,
                OmarchyVariant::Contrast,
                OmarchyVariant::Darker,
            ] {
                preferences.set_omarchy_variant(variant);
                crate::storage::writer::settle();
                let tokens = manager.appearance_tokens();
                assert_eq!(tokens == startup, variant == OmarchyVariant::Lighter);
                for window in &windows {
                    assert_theme_colors(window, &tokens);
                    let rebuilt_view = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    window.set_child(Some(&rebuilt_view));
                    assert_theme_colors(&rebuilt_view, &tokens);
                }
                let rebuilt_buffer = gtk::TextBuffer::new(None);
                rebuilt_buffer.create_tag(Some("document-accent"), &[]);
                crate::ui::theme::register_document_buffer(&rebuilt_buffer);
                for buffer in [&buffer, &rebuilt_buffer] {
                    assert_eq!(
                        buffer
                            .tag_table()
                            .lookup("document-accent")
                            .expect("document accent tag")
                            .foreground_rgba(),
                        Some(gtk::gdk::RGBA::parse(&tokens.accent).expect("valid accent"))
                    );
                }
                assert_eq!(
                    source
                        .style_scheme()
                        .expect("active source scheme")
                        .style("text")
                        .expect("source text style")
                        .background()
                        .as_deref(),
                    Some(tokens.surface.as_str())
                );
                crate::storage::writer::settle();
                assert_eq!(
                    read_preferences()
                        .expect("saved variant")
                        .omarchy_variants
                        .get("fixture")
                        .copied()
                        .unwrap_or_default(),
                    variant
                );
            }
            let before_change = manager.appearance_tokens();
            fs::write(
                crate::ui::theme::omarchy_state_dir().join("theme/colors.toml"),
                "background = '#201a12'\nforeground = '#ffeedd'\naccent = '#eebb99'\n",
            )
            .expect("update Omarchy colors");
            wait_for_theme(|| {
                manager.appearance_tokens() != before_change && {
                    #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
                    let accent = windows[0]
                        .style_context()
                        .lookup_color("strata_accent")
                        .expect("applied accent");
                    accent
                        == gtk::gdk::RGBA::parse(&manager.appearance_tokens().accent)
                            .expect("valid updated accent")
                }
            });
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Darker);
            for window in &windows {
                assert_theme_colors(window, &manager.appearance_tokens());
            }
            manager.set_follow_omarchy(false);
            let builtin = manager.appearance_tokens();
            preferences.set_omarchy_variant(OmarchyVariant::Contrast);
            assert_eq!(manager.appearance_tokens(), builtin);
            for window in &windows {
                assert_theme_colors(window, &builtin);
            }
            manager.set_follow_omarchy(true);
            assert_ne!(manager.appearance_tokens(), builtin);
            assert_eq!(*values.borrow(), [true, false, true]);
            for window in windows {
                window.close();
            }
            crate::storage::writer::settle();
            assert_eq!(
                read_preferences().expect("saved theme mode").mode,
                "omarchy"
            );
        },
    );
}

fn assert_theme_colors(widget: &impl IsA<gtk::Widget>, tokens: &crate::ui::theme::ThemeTokens) {
    for (name, expected) in [
        ("strata_bg", &tokens.background),
        ("strata_surface", &tokens.surface),
        ("strata_accent", &tokens.accent),
        ("strata_text", &tokens.text),
    ] {
        #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
        let actual = widget
            .style_context()
            .lookup_color(name)
            .expect("applied theme color");
        assert_eq!(
            actual,
            gtk::gdk::RGBA::parse(expected).expect("valid token"),
            "{name}"
        );
    }
}

#[test]
fn omarchy_colors_in_gtk_only_hex_forms_apply_to_css_and_icons() {
    gtk_test(
        "ui::preferences::tests::preferences::omarchy_colors_in_gtk_only_hex_forms_apply_to_css_and_icons",
        || {
            seed_omarchy_colors_for_test(
                "background = '#112233'\nforeground = '#ddeeff'\naccent = '#000aaafff'\nselection = '#1111222233338888'\n",
            );
            let themes = ThemeManager::shared();
            assert!(themes.follows_omarchy());
            let tokens = themes.appearance_tokens();
            assert_eq!(tokens.accent, "#00aaff");
            assert_eq!(tokens.highlight, "#11223388");
            let window = gtk::Window::new();
            for (name, expected) in [
                ("strata_accent", "#00aaff"),
                ("strata_highlight", "#11223388"),
            ] {
                #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
                let applied = window.style_context().lookup_color(name);
                assert_eq!(
                    applied,
                    Some(gtk::gdk::RGBA::parse(expected).expect("canonical color")),
                    "{name}"
                );
            }
            assert_eq!(crate::assets::primary_icon_color(), "#00aaff");
            window.close();
        },
    );
}

#[test]
fn invalid_omarchy_palette_while_running_keeps_the_applied_palette() {
    gtk_test(
        "ui::preferences::tests::preferences::invalid_omarchy_palette_while_running_keeps_the_applied_palette",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let mut preferences = non_default_preferences();
            preferences.mode = "omarchy".into();
            preferences.omarchy_variants.clear();
            fs::write(
                settings_path(),
                toml::to_string(&preferences).expect("Omarchy preference fixture"),
            )
            .expect("persist Omarchy mode");
            let themes = ThemeManager::shared();
            assert_eq!(crate::assets::primary_icon_color(), "#445566");
            assert_eq!(themes.appearance_tokens().accent, "#445566");

            seed_omarchy_colors_for_test(
                "background = '#112233'\nforeground = '#ddeeff'\naccent = '0x7aa2f7'\n",
            );
            themes.refresh_omarchy_state();
            crate::storage::writer::settle();
            assert!(themes.follows_omarchy());
            assert!(themes.is_omarchy_available());
            assert_eq!(crate::assets::primary_icon_color(), "#445566");
            assert_eq!(themes.appearance_tokens().accent, "#445566");
            let buffer = gtk::TextBuffer::new(None);
            buffer.create_tag(Some("document-accent"), &[]);
            crate::ui::theme::register_document_buffer(&buffer);
            assert_eq!(
                buffer
                    .tag_table()
                    .lookup("document-accent")
                    .expect("document accent tag")
                    .foreground_rgba(),
                Some(gtk::gdk::RGBA::parse("#445566").expect("accent"))
            );
            let mut preview = themes.starter_tokens();
            preview.accent = "#13579b".to_owned();
            themes.preview(&preview).expect("valid preview");
            assert_eq!(themes.appearance_tokens().accent, "#445566");
            themes.cancel_preview();
            themes.set_follow_omarchy(false);
            themes.set_follow_omarchy(true);
            assert!(themes.follows_omarchy());
            assert_eq!(
                themes.appearance_tokens(),
                themes.starter_tokens(),
                "a built-in theme applied since then replaces the remembered Omarchy palette"
            );

            fs::remove_file(crate::ui::theme::omarchy_state_dir().join("theme.name"))
                .expect("remove Omarchy theme name");
            let held = crate::storage::writer::hold();
            themes.refresh_omarchy_state();
            for _ in 0..20 {
                glib::MainContext::default().iteration(false);
            }
            assert!(
                themes.follows_omarchy(),
                "Omarchy files are read again on the settings writer"
            );
            drop(held);
            crate::storage::writer::settle();
            assert!(!themes.follows_omarchy());
            crate::storage::writer::settle();
            assert_eq!(read_preferences().expect("saved theme mode").mode, "theme");
        },
    );
}

fn wait_for_theme(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "live Omarchy refresh timed out"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
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
                |m| m.set_folder_peeking(false),
                |m| m.set_single_click_previews(true),
                |m| m.set_columns_mirror_selection(true),
                |m| m.set_columns_show_sizes(true),
                |m| m.set_render_documents_by_default(true),
                |m| m.set_hardware_accelerated_video_previews(true),
                |m| m.set_video_preview_backend(MediaPreviewBackend::VaApi),
                |m| m.set_search_open_files_directly(false),
                |m| m.set_search_exclusions(vec!["changed_exclusion".to_owned()]),
                |m| m.set_search_time_limit(0),
                |m| m.set_search_entry_limit(0),
                |m| m.set_type_to_search(true),
                |m| m.set_arrow_navigation_scoped(false),
                |m| m.set_tenxer_mode(false),
                |m| m.set_filter_include_subfolders(true),
                |m| m.set_show_keybinding_hints(true),
                |m| m.set_window_show_close(true),
                |m| m.set_window_show_minimize(false),
                |m| m.set_window_show_maximize(false),
                |m| m.set_reduce_motion(false),
                |m| m.set_element_glow(true),
                |m| m.set_omarchy_variant(OmarchyVariant::Contrast),
                |m| m.set_file_icon_colors(FileIconColors::Brand),
                |m| m.set_browser_mode(BrowserMode::Icons),
                |m| m.set_browser_density(BrowserDensity::Custom(9)),
                |m| m.set_browser_density(BrowserDensity::Compact),
                |m| m.set_frame_density(FrameDensity::Custom(5)),
                |m| m.set_frame_density(FrameDensity::Airy),
                |m| m.set_frame_density(FrameDensity::Compact),
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
                |m| m.set_sidebar_show_music(true),
                |m| m.set_sidebar_show_pictures(true),
                |m| m.set_sidebar_show_videos(true),
                |m| m.set_sidebar_expanded(true),
                |m| m.set_sidebar_compact(false),
                |m| m.set_sort_preferences(ViewPreferences::default()),
                |m| m.set_text_size(TextSize::new(11)),
                |m| m.set_corner_roundness(CornerRoundness::new(150)),
                |m| m.set_interface_font(None),
                |m| m.set_interface_renderer(InterfaceRenderer::System),
                |m| m.set_language(crate::i18n::Language::Auto),
                |m| m.set_checks_for_updates(true),
                |m| m.set_release_channel(Channel::Stable),
                |m| m.set_preview_muted(false),
                |m| m.set_preview_volume(0.8),
                |m| m.set_preview_text_wrap(false),
                |m| m.set_unit_system(UnitSystem::Metric),
                |m| m.set_preview_autoplay(false),
                |m| m.set_watch_network_folders(true),
                |m| m.set_thumbnail_workers(3),
                |m| m.set_show_thumbnails(true),
                |m| m.set_stl_thumbnails(false),
                |m| m.set_icons_thumbnail_size(96),
                |m| m.set_chooser_column_width(Some(360)),
                |m| m.set_chooser_list_columns(None),
                |m| m.set_browser_column_width(Some(400)),
                |m| m.set_browser_list_columns(None),
                |m| m.set_list_column_order(default_list_column_order()),
                |m| m.set_list_hidden_columns(Vec::new()),
                |m| m.set_cross_volume_drop_strategy(CrossVolumeDropStrategy::Copy),
                |m| m.set_date_format(crate::util::DateFormat::Long),
                |m| m.set_default_directory(None),
                |m| m.set_restore_tabs(true),
                |m| m.set_remember_folder_views(true),
                |m| m.set_device_label("volume:fixture-kingston", "Photos / 📁"),
                |m| m.set_device_label("volume:fixture-kingston", ""),
                |m| {
                    m.remember_send_to_destination(
                        "volume:fixture-kingston",
                        Path::new("Research"),
                        None,
                    )
                },
                |m| m.set_open_folder_after_drop(false),
                |m| m.set_folder_color(Path::new("/fixture/folder"), None),
                |m| m.set_custom_icon(Path::new("/fixture/folder"), None),
                |m| {
                    m.set_bindings(
                        Map::Default,
                        bindings::RENAME,
                        vec![Binding::ctrl(gtk::gdk::Key::e)],
                    )
                },
                |m| m.reset_binding(Map::Tenxer, bindings::SIDEBAR_FOCUS),
                |m| m.reset_map(Map::Default),
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
                crate::storage::writer::settle();
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
fn density_sliders_save_snap_points_as_presets_and_other_values_as_custom() {
    gtk_test(
        "ui::preferences::tests::preferences::density_sliders_save_snap_points_as_presets_and_other_values_as_custom",
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

#[derive(Clone, Copy)]
enum ItemStep {
    Relocate {
        from: &'static str,
        to: Option<&'static str>,
        merged: bool,
    },
    Forget {
        root: &'static str,
        trashed: Option<TrashMount>,
    },
    Restore(&'static str),
    RestoreRenumbered(&'static str),
    /// Puts a different folder at `path`, keeping the original alive elsewhere.
    ReplaceOnDisk(&'static str),
}

#[derive(Clone, Copy)]
enum TrashMount {
    KeepsNumbers,
    RenewsNumbers,
    RenewsNumbersWithoutBirths,
    RenewsNumbersInOneBirthTick,
}

#[test]
fn item_customizations_follow_relocations_and_removals() {
    gtk_test(
        "ui::preferences::tests::preferences::item_customizations_follow_relocations_and_removals",
        || {
            use crate::assets::icons::{FILE_CODE, PICTURES};
            use crate::model::FolderColor::{Blue, Green, Red};
            use ItemStep::{Forget, Relocate, ReplaceOnDisk, Restore, RestoreRenumbered};
            use TrashMount::{
                KeepsNumbers, RenewsNumbers, RenewsNumbersInOneBirthTick,
                RenewsNumbersWithoutBirths,
            };

            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let notifications = Rc::new(std::cell::Cell::new(0));
            let counted = notifications.clone();
            manager.observe(Rc::new(move || counted.set(counted.get() + 1)));
            let relocate = |from, to| Relocate {
                from,
                to: Some(to),
                merged: false,
            };
            let trash = |root| Forget {
                root,
                trashed: Some(KeepsNumbers),
            };
            let trash_on = |root, mount| Forget {
                root,
                trashed: Some(mount),
            };
            let customized_docs = [
                ("docs", Some(Red), Some(FILE_CODE)),
                ("docs/inner", Some(Blue), None),
                ("docs2", Some(Green), None),
            ];
            let forgotten_docs = [
                ("docs", None, None),
                ("docs/inner", None, None),
                ("docs2", Some(Green), None),
            ];
            type Customization = (
                &'static str,
                Option<crate::model::FolderColor>,
                Option<&'static str>,
            );
            type Case<'a> = (
                &'a str,
                &'a [Customization],
                &'a [ItemStep],
                &'a [Customization],
                usize,
            );
            // One coalesced color save and one folder sort change per step that changed something.
            let cases: [Case<'_>; 15] = [
                (
                    "a rename carries the root and descendants but not a look-alike sibling",
                    &customized_docs,
                    &[relocate("docs", "notes")],
                    &[
                        ("notes", Some(Red), Some(FILE_CODE)),
                        ("notes/inner", Some(Blue), None),
                        ("docs", None, None),
                        ("docs/inner", None, None),
                        ("docs2", Some(Green), None),
                    ],
                    1,
                ),
                (
                    "a move drops stale keys at and under its destination",
                    &[
                        ("docs", Some(Red), Some(FILE_CODE)),
                        ("dest/docs", Some(Green), Some(PICTURES)),
                        ("dest/docs/old", Some(Blue), None),
                    ],
                    &[relocate("docs", "dest/docs")],
                    &[
                        ("dest/docs", Some(Red), Some(FILE_CODE)),
                        ("dest/docs/old", None, None),
                        ("docs", None, None),
                    ],
                    1,
                ),
                (
                    "a merge keeps the destination root and carries descendants over it",
                    &[
                        ("docs", Some(Red), Some(FILE_CODE)),
                        ("docs/inner", Some(Blue), None),
                        ("dest/docs", Some(Green), Some(PICTURES)),
                        ("dest/docs/inner", Some(Red), None),
                    ],
                    &[Relocate {
                        from: "docs",
                        to: Some("dest/docs"),
                        merged: true,
                    }],
                    &[
                        ("dest/docs", Some(Green), Some(PICTURES)),
                        ("dest/docs/inner", Some(Blue), None),
                        ("docs", None, None),
                        ("docs/inner", None, None),
                    ],
                    1,
                ),
                (
                    "a repeated relocation is a no-op",
                    &customized_docs,
                    &[relocate("docs", "notes"), relocate("docs", "notes")],
                    &[
                        ("notes", Some(Red), Some(FILE_CODE)),
                        ("notes/inner", Some(Blue), None),
                        ("docs", None, None),
                    ],
                    1,
                ),
                (
                    "a move off local storage drops the keys",
                    &customized_docs,
                    &[Relocate {
                        from: "docs",
                        to: None,
                        merged: false,
                    }],
                    &forgotten_docs,
                    1,
                ),
                (
                    "the trashed item restored at its path gets its keys back",
                    &customized_docs,
                    &[trash("docs"), Restore("docs")],
                    &customized_docs,
                    2,
                ),
                (
                    "another item restored at the trashed path gets nothing",
                    &customized_docs,
                    &[trash("docs"), ReplaceOnDisk("docs"), Restore("docs")],
                    &forgotten_docs,
                    1,
                ),
                (
                    "a later trashed item with the same identity supersedes the kept keys",
                    &customized_docs,
                    &[trash("docs"), trash("docs"), Restore("docs")],
                    &forgotten_docs,
                    1,
                ),
                (
                    "a permanent delete keeps nothing to restore",
                    &customized_docs,
                    &[
                        Forget {
                            root: "docs",
                            trashed: None,
                        },
                        Restore("docs"),
                    ],
                    &forgotten_docs,
                    1,
                ),
                (
                    "a trashed item its mount renumbered gets its keys back by its birth time",
                    &customized_docs,
                    &[trash_on("docs", RenewsNumbers), RestoreRenumbered("docs")],
                    &customized_docs,
                    2,
                ),
                (
                    "another number where numbers are kept is another item",
                    &customized_docs,
                    &[trash_on("docs", KeepsNumbers), RestoreRenumbered("docs")],
                    &forgotten_docs,
                    1,
                ),
                (
                    "where only the number tells, the same number gets the keys back",
                    &customized_docs,
                    &[
                        trash_on("docs", RenewsNumbersWithoutBirths),
                        Restore("docs"),
                    ],
                    &customized_docs,
                    2,
                ),
                (
                    "where only the number tells, another number gets nothing",
                    &customized_docs,
                    &[
                        trash_on("docs", RenewsNumbersWithoutBirths),
                        RestoreRenumbered("docs"),
                    ],
                    &forgotten_docs,
                    1,
                ),
                (
                    "a sibling sharing the trashed folder's birth tick keeps its keys apart",
                    &customized_docs,
                    &[
                        trash_on("docs", RenewsNumbersInOneBirthTick),
                        trash_on("docs2", RenewsNumbersInOneBirthTick),
                        Restore("docs"),
                    ],
                    &[
                        ("docs", Some(Red), Some(FILE_CODE)),
                        ("docs/inner", Some(Blue), None),
                        ("docs2", None, None),
                    ],
                    3,
                ),
                (
                    "uncustomized items change nothing",
                    &[("docs2", Some(Green), None)],
                    &[relocate("docs", "notes"), trash("docs"), Restore("docs")],
                    &[("docs2", Some(Green), None)],
                    0,
                ),
            ];

            let settle = || {
                let context = glib::MainContext::default();
                while context.pending() {
                    context.iteration(false);
                }
            };
            // Folder sorts follow the same operations as colors, so each color
            // seeds a distinct sort that must end up wherever the color does.
            let sort_for = |color| match color {
                Red => FolderSort::Saved(SortKey::Type, SortDirection::Ascending),
                Blue => FolderSort::Saved(SortKey::Modified, SortDirection::Ascending),
                _ => FolderSort::Saved(SortKey::Name, SortDirection::Descending),
            };
            manager.set_remember_folder_views(true);
            for (case, seed, steps, expected, notified) in cases {
                let root = tempfile::tempdir().expect("case root");
                let path = |relative: &str| root.path().join(relative);
                let location = |relative: &str| Location::local(path(relative));
                let identity = |relative: &str, mount: TrashMount, renumbered: bool| {
                    use std::os::unix::fs::MetadataExt;
                    let metadata = fs::symlink_metadata(path(relative)).expect("identity");
                    let numbers = match mount {
                        KeepsNumbers => crate::services::InodeNumbers::Kept,
                        RenewsNumbers
                        | RenewsNumbersWithoutBirths
                        | RenewsNumbersInOneBirthTick => crate::services::InodeNumbers::Renewed,
                    };
                    let born = match mount {
                        RenewsNumbersInOneBirthTick => 1,
                        _ => metadata.ino() as i64,
                    };
                    crate::services::ItemIdentity {
                        evidence: crate::services::Evidence {
                            device: metadata.dev(),
                            inode: metadata.ino() + u64::from(renumbered),
                            kind: rustix::fs::FileType::Directory,
                            birth: (!matches!(mount, RenewsNumbersWithoutBirths))
                                .then_some((born, 0)),
                            handle: None,
                            attributes: None,
                        },
                        traits: crate::services::IdentityTraits {
                            numbers,
                            ..Default::default()
                        },
                    }
                };
                let mut trashed_on = KeepsNumbers;
                fs::create_dir_all(path("docs")).expect("docs folder");
                for (relative, color, icon) in seed {
                    fs::create_dir_all(path(relative)).expect("customized folder");
                    manager.set_folder_color(&path(relative), color.map(FolderColorValue::Preset));
                    manager.set_custom_icon(&path(relative), *icon);
                    if let Some(FolderSort::Saved(sort_key, sort_direction)) = color.map(sort_for) {
                        manager.set_folder_sort(&location(relative), sort_key, sort_direction);
                    }
                }
                settle();
                notifications.set(0);
                let folder_revision = manager.folder_sorts_revision();

                for step in steps {
                    match *step {
                        Relocate { from, to, merged } => {
                            manager.relocate_item_customizations(
                                &path(from),
                                to.map(path).as_deref(),
                                merged,
                            );
                            let to = to.map_or_else(
                                || Location::uri("sftp://example.invalid/docs"),
                                location,
                            );
                            manager.relocate_folder_views(&location(from), &to, merged);
                        }
                        Forget { root, trashed } => {
                            trashed_on = trashed.unwrap_or(trashed_on);
                            let identity = trashed.map(|mount| identity(root, mount, false));
                            manager.forget_item_customizations(&path(root), identity);
                            manager.forget_folder_views_within(&location(root), identity);
                        }
                        Restore(root) | RestoreRenumbered(root) => {
                            let renumbered = matches!(step, RestoreRenumbered(_));
                            let restored = identity(root, trashed_on, renumbered);
                            manager.restore_item_customizations(&path(root), restored);
                            manager.restore_folder_views(&location(root), Some(restored));
                        }
                        ReplaceOnDisk(relative) => {
                            fs::rename(path(relative), root.path().join("kept-alive"))
                                .expect("move the original aside");
                            fs::create_dir(path(relative)).expect("another folder");
                        }
                    }
                    settle();
                }

                let folder_changes = manager
                    .folder_sorts_revision()
                    .wrapping_sub(folder_revision);
                assert_eq!(
                    (notifications.get(), folder_changes),
                    (notified * 2, notified as u64),
                    "{case}: notifications of color and folder sort changes"
                );
                crate::storage::writer::settle();
                let saved = read_preferences().expect("saved preferences");
                for (relative, color, icon) in expected {
                    assert_eq!(
                        manager.resolve_folder_sort(&location(relative), false),
                        color.map_or(FolderSort::Default, sort_for),
                        "{case}: folder sort of {relative}"
                    );
                    let path = path(relative);
                    let key = path.to_string_lossy().into_owned();
                    assert_eq!(
                        manager.folder_color(&path),
                        color.map(FolderColorValue::Preset),
                        "{case}: color of {relative}"
                    );
                    assert_eq!(
                        manager.custom_icon(&path).as_deref(),
                        *icon,
                        "{case}: icon of {relative}"
                    );
                    assert_eq!(
                        (
                            saved.folder_colors.contains_key(&key),
                            saved.custom_icons.contains_key(&key)
                        ),
                        (color.is_some(), icon.is_some()),
                        "{case}: saved keys of {relative}"
                    );
                }
            }
        },
    );
}

#[test]
fn settings_files_are_watched_from_the_monitors_worker_without_missing_a_save() {
    gtk_test(
        "ui::preferences::tests::preferences::settings_files_are_watched_from_the_monitors_worker_without_missing_a_save",
        || {
            use crate::adapters::{FsOp, FsQuirks, Quirks};

            seed_saved_preferences_for_test();
            let folder_views = super::super::folder_view_settings::folder_views_path();
            fs::create_dir_all(folder_views.parent().expect("state directory"))
                .expect("state directory");
            fs::write(&folder_views, "").expect("folder settings file");
            let sandbox = glib::home_dir()
                .parent()
                .expect("isolated sandbox")
                .to_path_buf();
            let gate = crate::test_support::Gate::default();
            let scope = FsQuirks::install(
                &sandbox,
                Quirks {
                    before: vec![(FsOp::Watch, "settings.toml", 0, gate.racer())],
                    ..Quirks::default()
                },
            );
            let ticked = Rc::new(std::cell::Cell::new(false));
            let tick = ticked.clone();
            glib::timeout_add_local_once(Duration::from_millis(10), move || tick.set(true));

            let manager = PreferenceManager::shared();
            wait_for(
                || gate.reached() && ticked.get(),
                "the settings watch to wait on the monitors worker while the window runs",
            );
            assert_eq!(
                manager.default_sort(),
                (SortKey::Size, SortDirection::Descending)
            );
            let mut saved = non_default_preferences();
            saved.sort_key = "modified".into();
            saved.sort_direction = "ascending".into();
            fs::write(
                settings_path(),
                toml::to_string(&saved).expect("serialize another process's save"),
            )
            .expect("another process saves");
            gate.release();
            wait_for(
                || manager.default_sort() == (SortKey::Modified, SortDirection::Ascending),
                "the save made before the watch was live",
            );
            assert_eq!(scope.count(FsOp::Watch), 2);

            drop(manager);
            wait_for(
                || scope.count(FsOp::Unwatch) == 2,
                "both watches to be released on the monitors worker",
            );
        },
    );
}

#[test]
fn a_folder_that_failed_to_open_forgets_its_settings_once_a_worker_proves_it_deleted() {
    gtk_test(
        "ui::preferences::tests::preferences::a_folder_that_failed_to_open_forgets_its_settings_once_a_worker_proves_it_deleted",
        || {
            use crate::adapters::{FsOp, FsQuirks, Quirks};

            let manager = PreferenceManager::shared();
            manager.set_remember_folder_views(true);
            let root = tempfile::tempdir().expect("fixture");
            let crowded = root.path().join("crowded");
            let empty = root.path().join("empty");
            fs::create_dir_all(crowded.join("kept")).expect("fixture folder");
            fs::create_dir(&empty).expect("fixture folder");
            let saved = FolderSort::Saved(SortKey::Type, SortDirection::Ascending);
            let folders = [
                crowded.join("gone"),
                crowded.join("kept"),
                empty.join("lost"),
            ]
            .map(Location::local);
            for folder in &folders {
                manager.set_folder_sort(folder, SortKey::Type, SortDirection::Ascending);
            }
            let gate = crate::test_support::Gate::default();
            let _scope = FsQuirks::install(
                root.path(),
                Quirks {
                    before: vec![(FsOp::Stat, "gone", 0, gate.racer())],
                    ..Quirks::default()
                },
            );
            let sorts = || {
                folders
                    .each_ref()
                    .map(|folder| manager.resolve_folder_sort(folder, false))
            };

            for folder in &folders {
                manager.forget_missing_folder(folder);
            }
            wait_for(|| gate.reached(), "the deletion check to reach the seam");
            assert_eq!(sorts(), [saved; 3], "kept until the check answers");

            gate.release();
            let forgotten = [FolderSort::Default, saved, saved];
            wait_for(
                || sorts() == forgotten,
                "the deleted folder to be forgotten",
            );
            let settled = std::time::Instant::now() + Duration::from_millis(200);
            while std::time::Instant::now() < settled {
                glib::MainContext::default().iteration(false);
            }
            assert_eq!(sorts(), forgotten, "only the folder missing beside others");
        },
    );
}

#[test]
fn list_column_visibility_preserves_default_sort_and_publishes_once() {
    gtk_test(
        "ui::preferences::tests::preferences::list_column_visibility_preserves_default_sort_and_publishes_once",
        || {
            let manager = PreferenceManager::shared();
            let snapshots = Rc::new(RefCell::new(Vec::new()));
            let observed = snapshots.clone();
            manager.observe(Rc::new(move || {
                let manager = PreferenceManager::shared();
                observed
                    .borrow_mut()
                    .push((manager.default_sort(), manager.list_hidden_columns()));
            }));

            for (sort_key, column, direction) in [
                (SortKey::Size, ListColumnId::Size, SortDirection::Ascending),
                (SortKey::Type, ListColumnId::Type, SortDirection::Descending),
                (
                    SortKey::Modified,
                    ListColumnId::Modified,
                    SortDirection::Descending,
                ),
            ] {
                manager.reset_list_columns();
                manager.set_default_sort(sort_key, direction);
                for (visible, hidden) in
                    [(false, vec![column]), (true, vec![]), (false, vec![column])]
                {
                    snapshots.borrow_mut().clear();
                    manager.set_list_column_visible(column, visible);

                    assert_eq!(manager.is_list_column_visible(column), visible);
                    assert_eq!(manager.default_sort(), (sort_key, direction));
                    assert_eq!(
                        snapshots.borrow().as_slice(),
                        &[((sort_key, direction), hidden.clone())],
                        "visibility must publish once without changing the sort"
                    );
                    crate::storage::writer::settle();
                    let saved = read_preferences().expect("saved preferences");
                    assert_eq!(saved.list_hidden_columns, hidden);
                    assert_eq!(SortKey::from_stored(&saved.sort_key), Some(sort_key));
                    assert_eq!(
                        SortDirection::from_stored(&saved.sort_direction),
                        Some(direction)
                    );
                }
                snapshots.borrow_mut().clear();
                manager.reset_list_columns();
                assert_eq!(manager.default_sort(), (sort_key, direction));
                assert_eq!(
                    snapshots.borrow().as_slice(),
                    &[((sort_key, direction), vec![])],
                    "resetting columns must preserve sorting too"
                );
            }
        },
    );
}

#[test]
fn list_columns_order_and_visibility_persist_and_update() {
    gtk_test(
        "ui::preferences::tests::preferences::list_columns_order_and_visibility_persist_and_update",
        || {
            let manager = PreferenceManager::shared();

            assert_eq!(
                manager.list_column_order(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Mode,
                    ListColumnId::Size,
                    ListColumnId::Type,
                    ListColumnId::Modified,
                ]
            );
            assert!(manager.list_hidden_columns().is_empty());
            assert!(manager.is_list_column_visible(ListColumnId::Mode));

            manager.set_list_column_visible(ListColumnId::Mode, false);
            manager.set_list_column_visible(ListColumnId::Type, false);
            assert!(!manager.is_list_column_visible(ListColumnId::Mode));
            assert!(!manager.is_list_column_visible(ListColumnId::Type));
            assert!(manager.is_list_column_visible(ListColumnId::Name));
            assert!(manager.is_list_column_visible(ListColumnId::Size));
            assert_eq!(
                manager.list_visible_columns(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Size,
                    ListColumnId::Modified,
                ]
            );

            manager.set_list_column_visible(ListColumnId::Name, false);
            assert!(manager.is_list_column_visible(ListColumnId::Name));

            manager.move_list_column(ListColumnId::Modified, ListColumnId::Type, false);
            assert_eq!(
                manager.list_column_order(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Mode,
                    ListColumnId::Size,
                    ListColumnId::Modified,
                    ListColumnId::Type,
                ]
            );
            use ListColumnId::{Mode, Modified, Name, Size, Type};
            for (column, target, after, expected) in [
                (Name, Type, true, [Mode, Size, Modified, Type, Name]),
                (Name, Mode, false, [Name, Mode, Size, Modified, Type]),
                (Type, Name, false, [Type, Name, Mode, Size, Modified]),
                (Type, Modified, true, [Name, Mode, Size, Modified, Type]),
                (Size, Modified, false, [Name, Mode, Size, Modified, Type]),
                (Modified, Modified, true, [Name, Mode, Size, Modified, Type]),
            ] {
                manager.move_list_column(column, target, after);
                assert_eq!(manager.list_column_order(), expected);
            }
            crate::storage::writer::settle();

            let saved = read_preferences().expect("saved preferences");
            assert_eq!(saved.list_column_order, manager.list_column_order());
            assert_eq!(saved.list_hidden_columns, manager.list_hidden_columns());

            manager.reset_list_columns();
            assert_eq!(
                manager.list_column_order(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Mode,
                    ListColumnId::Size,
                    ListColumnId::Type,
                    ListColumnId::Modified,
                ]
            );
            assert!(manager.list_hidden_columns().is_empty());
            crate::storage::writer::settle();
            let saved_reset = read_preferences().expect("saved preferences");
            assert_eq!(saved_reset.list_column_order, manager.list_column_order());
            assert!(saved_reset.list_hidden_columns.is_empty());
        },
    );
}
