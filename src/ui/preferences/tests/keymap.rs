// SPDX-License-Identifier: MIT

use gtk::gdk::Key;

use super::super::*;
use crate::{
    test_support::gtk_test,
    ui::keys::{
        Binding, Map,
        bindings::{COPY, CUSTOM_ACTION_1, CUSTOM_ACTION_10, CUT, GO_HOME, RENAME},
        registry,
    },
};

fn write_settings(contents: &str) {
    let path = settings_path();
    fs::create_dir_all(path.parent().expect("settings directory")).expect("settings directory");
    fs::write(path, contents).expect("seed settings");
}

fn saved_settings() -> String {
    crate::storage::writer::settle();
    fs::read_to_string(settings_path()).expect("saved settings")
}

#[test]
fn unknown_ids_and_bad_accelerators_round_trip() {
    gtk_test(
        "ui::preferences::tests::keymap::unknown_ids_and_bad_accelerators_round_trip",
        || {
            write_settings(concat!(
                "folder_peeking = false\n",
                "\n",
                "[keymap.default]\n",
                "copy = [\"<Bogus>c\"]\n",
                "cut = \"<Control>x\"\n",
                "future_cmd = [\"x\"]\n",
            ));
            let manager = PreferenceManager::shared();
            let keymap = manager.keymap();
            assert_eq!(
                keymap.bindings(Map::Default, COPY),
                COPY.defaults(Map::Default)
            );
            assert_eq!(
                keymap.bindings(Map::Default, CUT),
                CUT.defaults(Map::Default)
            );
            manager.set_folder_peeking(true);
            let saved = saved_settings();
            for line in [
                "copy = [\"<Bogus>c\"]",
                "cut = \"<Control>x\"",
                "future_cmd = [\"x\"]",
            ] {
                assert!(saved.contains(line), "{line} in {saved}");
            }
            assert!(saved.contains("folder_peeking = true"), "{saved}");
        },
    );
}

#[test]
fn a_malformed_map_keeps_the_other_maps_entries() {
    gtk_test(
        "ui::preferences::tests::keymap::a_malformed_map_keeps_the_other_maps_entries",
        || {
            write_settings("[keymap]\ndefault = \"x\"\n\n[keymap.tenxer]\nrename = [\"e\"]\n");
            let manager = PreferenceManager::shared();
            assert_eq!(
                manager.keymap().bindings(Map::Tenxer, RENAME),
                [Binding::plain(Key::e)]
            );
            manager.set_folder_peeking(!manager.folder_peeking());
            let saved = saved_settings();
            assert!(
                saved.contains("[keymap.tenxer]\nrename = [\"e\"]"),
                "{saved}"
            );
        },
    );
}

#[test]
fn files_without_overrides_save_no_keymap() {
    let saved = toml::to_string_pretty(&Preferences::default()).expect("default preferences");
    assert!(!saved.contains("keymap"), "{saved}");
    let mut preferences = Preferences::default();
    preferences.keymap.tenxer = [fixtures::keymap_entry(("go.home", &["<Shift>h"][..]))]
        .into_iter()
        .collect();
    let saved = toml::to_string_pretty(&preferences).expect("one override");
    assert!(saved.contains("[keymap.tenxer]"), "{saved}");
    assert!(!saved.contains("[keymap.default]"), "{saved}");
    assert_eq!(parse_preferences(&saved).expect("reload"), preferences);
}

#[test]
fn tenxer_override_does_not_touch_default_map() {
    gtk_test(
        "ui::preferences::tests::keymap::tenxer_override_does_not_touch_default_map",
        || {
            let manager = PreferenceManager::shared();
            manager.set_bindings(Map::Tenxer, GO_HOME, vec![Binding::shift(Key::h)]);
            let keymap = manager.keymap();
            assert_eq!(
                keymap.bindings(Map::Tenxer, GO_HOME),
                [Binding::shift(Key::h)]
            );
            let mut checked = 0;
            for command in registry() {
                assert_eq!(
                    keymap.bindings(Map::Default, command),
                    command.defaults(Map::Default),
                    "{}",
                    command.id()
                );
                checked += 1;
            }
            assert!(checked > 0);
            let saved = saved_settings();
            assert!(saved.contains("[keymap.tenxer]"), "{saved}");
            assert!(saved.contains("\"go.home\" = [\"<Shift>h\"]"), "{saved}");
            assert!(!saved.contains("[keymap.default]"), "{saved}");
        },
    );
}

#[test]
fn custom_action_slots_load_and_save_their_quoted_ids() {
    gtk_test(
        "ui::preferences::tests::keymap::custom_action_slots_load_and_save_their_quoted_ids",
        || {
            write_settings("[keymap.default]\n\"custom_action.10\" = [\"<Control><Alt>0\"]\n");
            let manager = PreferenceManager::shared();
            assert_eq!(
                manager.keymap().bindings(Map::Default, CUSTOM_ACTION_10),
                [Binding::ctrl_alt(Key::_0)]
            );
            manager.set_bindings(Map::Default, CUSTOM_ACTION_1, vec![Binding::alt(Key::_1)]);
            let saved = saved_settings();
            for line in [
                "\"custom_action.1\" = [\"<Alt>1\"]",
                "\"custom_action.10\" = [\"<Control><Alt>0\"]",
            ] {
                assert!(saved.contains(line), "{line} in {saved}");
            }
        },
    );
}

#[test]
fn keymap_setters_save_only_overrides_and_publish_changes() {
    gtk_test(
        "ui::preferences::tests::keymap::keymap_setters_save_only_overrides_and_publish_changes",
        || {
            write_settings("[keymap.tenxer]\nfuture_cmd = [\"x\"]\nrename = [\"e\"]\n");
            let manager = PreferenceManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let revisions = Rc::new(RefCell::new(Vec::new()));
            let seen = revisions.clone();
            manager.bind_preference(
                &anchor,
                PreferenceManager::keymap_revision,
                move |_, revision| {
                    seen.borrow_mut().push(revision);
                },
            );
            let ctrl_e = Binding::ctrl(Key::e);
            manager.set_bindings(Map::Default, RENAME, vec![ctrl_e, ctrl_e]);
            assert_eq!(manager.keymap().bindings(Map::Default, RENAME), [ctrl_e]);
            assert!(saved_settings().contains("rename = [\"<Control>e\"]"));
            manager.set_bindings(Map::Default, RENAME, vec![ctrl_e]);
            assert_eq!(
                revisions.borrow().len(),
                2,
                "an unchanged entry publishes nothing"
            );
            manager.set_bindings(Map::Default, RENAME, RENAME.defaults(Map::Default).to_vec());
            assert!(
                !manager
                    .preferences
                    .borrow()
                    .keymap
                    .default
                    .contains_key("rename")
            );
            manager.set_bindings(Map::Default, RENAME, vec![ctrl_e]);
            manager.reset_binding(Map::Default, RENAME);
            assert!(manager.preferences.borrow().keymap.default.is_empty());
            manager.reset_map(Map::Tenxer);
            assert_eq!(
                manager.keymap().bindings(Map::Tenxer, RENAME),
                RENAME.defaults(Map::Tenxer)
            );
            let saved = saved_settings();
            assert!(saved.contains("future_cmd = [\"x\"]"), "{saved}");
            assert!(!saved.contains("rename"), "{saved}");
            assert_eq!(revisions.borrow().len(), 6, "{:?}", revisions.borrow());
        },
    );
}

#[test]
fn conflicting_keys_in_the_file_are_logged_once_with_its_path() {
    gtk_test(
        "ui::preferences::tests::keymap::conflicting_keys_in_the_file_are_logged_once_with_its_path",
        || {
            write_settings("[keymap.default]\nrename = [\"F2\", \"<Control>c\"]\n");
            let manager = PreferenceManager::shared();
            let output = crate::test_support::capture_logs(|| {
                manager.keymap();
                manager.keymap();
            });
            let line = crate::test_support::captured_event(
                &output,
                "keymap entry has a key that never fires there or stops another key working",
            );
            assert!(
                line.contains(&settings_path().display().to_string()),
                "{line}"
            );
            assert!(
                line.contains("rename") && line.contains("<Control>c") && line.contains("copy"),
                "{line}"
            );
            assert_eq!(
                output.matches("keymap entry has a key").count(),
                1,
                "{output}"
            );
            assert_eq!(
                manager.keymap().bindings(Map::Default, RENAME),
                [Binding::plain(Key::F2), Binding::ctrl(Key::c)]
            );
        },
    );
}

#[test]
fn uppercase_letters_without_shift_are_logged() {
    gtk_test(
        "ui::preferences::tests::keymap::uppercase_letters_without_shift_are_logged",
        || {
            write_settings(
                "[keymap.default]\nrename = [\"<Control>E\"]\n\n[keymap.tenxer]\nrename = [\"<Shift>R\"]\n",
            );
            let manager = PreferenceManager::shared();
            let output = crate::test_support::capture_logs(|| {
                manager.keymap();
            });
            let warning = "keymap accelerator names an uppercase letter without <Shift>";
            let line = crate::test_support::captured_event(&output, warning);
            assert!(line.contains("<Control>E"), "{line}");
            assert_eq!(output.matches(warning).count(), 1, "{output}");
            assert_eq!(
                manager.keymap().bindings(Map::Default, RENAME),
                [Binding::ctrl(Key::e)]
            );
        },
    );
}
