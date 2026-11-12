// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_save_keeps_the_settings_another_process_wrote() {
    gtk_test(
        "ui::preferences::tests::preferences::saving::a_save_keeps_the_settings_another_process_wrote",
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
        "ui::preferences::tests::preferences::saving::unreadable_preferences_are_preserved_while_live_changes_still_apply",
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
        "ui::preferences::tests::preferences::saving::changes_apply_at_once_and_reach_the_file_on_the_settings_writer",
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
        "ui::preferences::tests::preferences::saving::missing_settings_allow_first_run_saves",
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
        "ui::preferences::tests::preferences::saving::symlinked_settings_save_through_to_the_target",
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
        "ui::preferences::tests::preferences::saving::failed_saves_show_one_notice_per_failure_streak",
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
fn each_view_mode_is_saved_and_restored_at_startup() {
    gtk_test(
        "ui::preferences::tests::preferences::saving::each_view_mode_is_saved_and_restored_at_startup",
        || {
            seed_saved_preferences_for_test();
            for mode in [
                BrowserMode::Compact,
                BrowserMode::Icons,
                BrowserMode::Columns,
                BrowserMode::List,
            ] {
                PreferenceManager::load().set_browser_mode(mode);
                crate::storage::writer::settle();
                assert_eq!(PreferenceManager::load().browser_mode(), mode);
            }
        },
    );
}
