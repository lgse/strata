// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Key, Mods};
use serde_json::Value;
use strata_core::i18n::tr;

use super::{ADD_KEY, BROWSE_KEY, EMPTY_KEY, ERROR_KEY, INPUT_KEY, validate_exclusion_input};
use crate::{
    commands::Holds,
    host::offscreen::Offscreen,
    launch,
    settings_dialog::{
        SettingsDialog,
        tests::{click, press},
    },
    shell,
    slots::Part,
    test_support::{
        PATIENCE, hosted,
        view::{self, idle},
    },
};

const MODULE: &str = "settings_dialog::exclusions::tests";

fn find_by_key<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

fn type_text(host: &mut Offscreen, value: &str) {
    for ch in value.chars() {
        press(host, Key::Text(ch.to_string()), Mods::default());
    }
}

#[test]
fn validation_rejects_invalid_entries_and_equivalent_duplicates() {
    let current = vec!["cache".to_owned(), "~/Secret".to_owned()];
    for invalid in [
        "",
        " ",
        "/",
        "//",
        "~",
        "~/",
        "~/.",
        "/./",
        ".",
        "..",
        "project/build",
        "~user/dir",
        "/var/../log",
        "bad\0name",
    ] {
        assert!(
            validate_exclusion_input(invalid, &current).is_err(),
            "{invalid:?}"
        );
    }
    let home_secret = glib::home_dir()
        .join("Secret")
        .to_string_lossy()
        .into_owned();
    for duplicate in [" CACHE/ ", "~/Secret/./", &home_secret] {
        assert_eq!(
            validate_exclusion_input(duplicate, &current),
            Err("This exclusion has already been added.")
        );
    }
    for valid in ["[build]", r"foo\bar", "/var/log"] {
        assert_eq!(
            validate_exclusion_input(valid, &current),
            Ok(valid.to_owned())
        );
    }
    assert_eq!(
        validate_exclusion_input("~/secret", &current),
        Ok(glib::home_dir()
            .join("secret")
            .to_string_lossy()
            .into_owned())
    );
}

#[test]
fn exclusions_editor_adds_validates_browses_removes_and_filters_in_search() {
    hosted(
        MODULE,
        "exclusions_editor_adds_validates_browses_removes_and_filters_in_search",
        |mut host| {
            click(&mut host, "header-settings");
            let tree = host.tree();
            assert!(
                find_by_key(&tree, EMPTY_KEY).is_some(),
                "empty message renders when no custom exclusions exist"
            );

            // Focus input, verify modal field hold, and add a folder name via Enter.
            click(&mut host, INPUT_KEY);
            assert_eq!(
                SettingsDialog::keyboard(host.app()),
                Some(Holds::Modal { field: true })
            );
            type_text(&mut host, "private-build");
            press(&mut host, Key::Enter, Mods::default());
            assert_eq!(
                shell::preferences(host.app()).search_exclusions,
                vec!["private-build".to_owned()]
            );
            assert!(host.app().settings_dialog.exclusions.input.text.is_empty());
            let tree = host.tree();
            assert!(find_by_key(&tree, EMPTY_KEY).is_none());
            assert!(find_by_key(&tree, "settings-search-exclusion-row-0").is_some());

            // Case-insensitive duplicate shows inline error; editing clears it.
            click(&mut host, INPUT_KEY);
            type_text(&mut host, "PRIVATE-BUILD");
            click(&mut host, ADD_KEY);
            let tree = host.tree();
            let err = find_by_key(&tree, ERROR_KEY).expect("duplicate error shown");
            assert_eq!(
                view::text(err, "name"),
                tr("This exclusion has already been added.")
            );
            click(&mut host, INPUT_KEY);
            press(&mut host, Key::Backspace, Mods::default());
            assert!(
                find_by_key(&host.tree(), ERROR_KEY).is_none(),
                "editing input clears inline error"
            );

            // Browse… populates abbreviated ~/ path and focuses the input; Add saves it.
            let picked = launch::home_directory().join("Datasets/large");
            let picked_clone = picked.clone();
            host.app_mut().settings_dialog.folder_picker =
                Some(Rc::new(move |_| Ok(Some(picked_clone.clone()))));
            click(&mut host, BROWSE_KEY);
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().settings_dialog.exclusions.input.text == "~/Datasets/large"
                }),
                "Browse populates ~/ abbreviated path"
            );
            idle(&mut host);
            assert!(
                host.app().settings_dialog.exclusions.focused(),
                "Browse focuses input"
            );

            click(&mut host, ADD_KEY);
            let expected_dir = glib::home_dir()
                .join("Datasets/large")
                .to_string_lossy()
                .into_owned();
            assert_eq!(
                shell::preferences(host.app()).search_exclusions,
                vec!["private-build".to_owned(), expected_dir]
            );

            // Remove both exclusions live; empty state returns.
            click(&mut host, "settings-search-exclusion-remove-0");
            assert_eq!(shell::preferences(host.app()).search_exclusions.len(), 1);
            click(&mut host, "settings-search-exclusion-remove-0");
            assert!(shell::preferences(host.app()).search_exclusions.is_empty());
            assert!(find_by_key(&host.tree(), EMPTY_KEY).is_some());

            // Settings search for "exclusions" keeps the exclusions editor visible.
            click(&mut host, "settings-search-field");
            type_text(&mut host, "exclusions");
            host.runtime().frame(0.2);
            idle(&mut host);
            assert!(find_by_key(&host.tree(), INPUT_KEY).is_some());

            // Closing Settings clears draft input and any inline error.
            click(&mut host, INPUT_KEY);
            type_text(&mut host, "..");
            click(&mut host, ADD_KEY);
            assert!(find_by_key(&host.tree(), ERROR_KEY).is_some());
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().settings_dialog.is_open());
            assert!(host.app().settings_dialog.exclusions.input.text.is_empty());
            assert!(host.app().settings_dialog.exclusions.error.is_none());
        },
    );
}
