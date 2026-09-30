// SPDX-License-Identifier: MIT

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::*;
use crate::{
    model::{SortDirection, SortKey},
    ui::tenxer_mode::{Chord, Prompt},
};

fn enable_tenxer(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_filter_include_subfolders(false);
    preferences.set_tenxer_mode(true);
    pump(50);
    fixture.view.browser().clear_active_selection();
}

fn plain(fixture: &KeyboardFixture, key: Key) {
    assert!(fixture.press(key, ModifierType::empty()), "{key:?}");
}

fn shifted(fixture: &KeyboardFixture, key: Key) {
    assert!(
        fixture.press(key, ModifierType::SHIFT_MASK),
        "Shift+{key:?}"
    );
}

fn feedback(fixture: &KeyboardFixture) -> String {
    let text = fixture.shortcuts.feedback_text();
    fixture.shortcuts.dismiss_feedback();
    text
}

fn contents(path: &Path) -> String {
    std::fs::read_to_string(path).expect("fixture contents")
}

fn names_at(browser: &crate::app::Browser, depth: usize) -> Vec<String> {
    let count = browser
        .column_snapshot(depth)
        .map_or(0, |column| column.count);
    browser
        .with_entries(depth, 0..count, |entries| {
            entries
                .iter()
                .map(|entry| entry.display_name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn refresh(fixture: &KeyboardFixture, name: &str) {
    plain(fixture, Key::F5);
    let browser = fixture.view.browser();
    // The reload streams its rows; commands must not race the rest of them.
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
            && names_at(&browser, 0).iter().any(|entry| entry == name)
    });
    pump(100);
    wait_loaded(&browser, 0);
    focus_files(fixture);
}

fn open_rename(fixture: &KeyboardFixture, key: Key) {
    focus_files(fixture);
    plain(fixture, key);
    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Rename));
    assert_eq!(
        fixture.shortcuts.prompt_label().as_deref(),
        Some("rename \u{203a}")
    );
}

fn submit_rename(fixture: &KeyboardFixture, text: &str) {
    open_rename(fixture, Key::r);
    fixture.shortcuts.prompt().set_text(text);
    plain(fixture, Key::Return);
}

fn open_empty_folder(fixture: &KeyboardFixture) {
    let empty = fixture._directory.path().join("empty");
    std::fs::create_dir(&empty).expect("empty folder");
    let browser = fixture.view.browser();
    browser.navigate(Location::local(&empty));
    wait_until(|| browser.active_location() == Some(Location::local(&empty)));
    wait_loaded(&browser, 0);
    focus_files(fixture);
}

fn button_labeled(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && button.is_visible()
        && rendered_name(widget, label)
    {
        return Some(button.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = button_labeled(&widget, label) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn tenxer_footer_rename_changes_only_the_focused_item_on_enter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_footer_rename_changes_only_the_focused_item_on_enter",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            move_to_named(&fixture, &browser, "c.txt");
            plain(&fixture, Key::space);
            move_to_named(&fixture, &browser, "a.txt");
            assert_eq!(fill_names(&browser), ["c.txt"]);

            for key in [Key::r, Key::F2] {
                open_rename(&fixture, key);
                assert_eq!(fixture.shortcuts.prompt_text(), "a.txt", "{key:?}");
                assert_eq!(
                    fixture.shortcuts.prompt().selection_bounds(),
                    Some((0, 1)),
                    "the stem is selected"
                );
                plain(&fixture, Key::Down);
                assert_eq!(focused_name(&browser), "a.txt", "the target stays put");
                plain(&fixture, Key::Escape);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                assert!(fixture.view.item_view_has_focus());
            }
            assert!(directory.join("a.txt").is_file());

            submit_rename(&fixture, "a.txt");
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            submit_rename(&fixture, "renamed.txt");
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            wait_until(|| directory.join("renamed.txt").is_file());
            assert!(!directory.join("a.txt").exists());
            assert_eq!(contents(&directory.join("renamed.txt")), "preview");
            assert!(directory.join("c.txt").is_file(), "the fill is not renamed");
            wait_until(|| focused_name(&browser) == "renamed.txt");

            for (text, hint) in [
                ("", "Enter a name"),
                ("  ", "Enter a name"),
                ("..", "That name is reserved"),
                ("nested/name", "Names cannot contain /"),
                ("b.txt", "\u{201c}b.txt\u{201d} already exists"),
            ] {
                submit_rename(&fixture, text);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Rename));
                assert_eq!(fixture.shortcuts.prompt_hint().as_deref(), Some(hint));
                assert_eq!(
                    fixture.shortcuts.prompt_text(),
                    text,
                    "the name can be fixed"
                );
                plain(&fixture, Key::Escape);
            }
            assert_eq!(contents(&directory.join("b.txt")), "preview");
            assert!(directory.join("renamed.txt").is_file());

            let exact = " spaced 日本語 ✓ ";
            submit_rename(&fixture, exact);
            wait_until(|| directory.join(exact).is_file());
            assert_eq!(contents(&directory.join(exact)), "preview");

            std::fs::create_dir(directory.join("folder")).expect("folder");
            std::fs::write(directory.join("folder/inside.txt"), "kept").expect("child");
            refresh(&fixture, "folder");
            move_to_named(&fixture, &browser, "folder");
            open_rename(&fixture, Key::F2);
            assert_eq!(
                fixture.shortcuts.prompt().selection_bounds(),
                Some((0, 6)),
                "a folder's whole name is selected"
            );
            fixture.shortcuts.prompt().set_text("folder.d");
            plain(&fixture, Key::Return);
            wait_until(|| directory.join("folder.d/inside.txt").is_file());
            assert_eq!(contents(&directory.join("folder.d/inside.txt")), "kept");

            move_to_named(&fixture, &browser, "b.txt");
            submit_rename(&fixture, &"x".repeat(300));
            wait_until(|| modal_visible(&fixture.overlay));
            assert!(click_class(&fixture.overlay, "action-dialog-close"));
            wait_until(|| !modal_visible(&fixture.overlay));
            assert_eq!(contents(&directory.join("b.txt")), "preview");

            let writable = std::fs::metadata(&directory)
                .expect("fixture")
                .permissions();
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o555))
                .expect("read-only fixture");
            // Root ignores directory permissions.
            if std::fs::write(directory.join("probe"), "").is_err() {
                focus_files(&fixture);
                submit_rename(&fixture, "denied.txt");
                wait_until(|| modal_visible(&fixture.overlay));
                assert!(click_class(&fixture.overlay, "action-dialog-close"));
                wait_until(|| !modal_visible(&fixture.overlay));
                assert!(directory.join("b.txt").is_file());
                assert!(!directory.join("denied.txt").exists());
            }
            std::fs::set_permissions(&directory, writable).expect("restore fixture");
        },
    );
}

#[test]
fn tenxer_footer_rename_leaves_no_stale_target() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_footer_rename_leaves_no_stale_target",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            let names = directory_names(&directory);
            let pending = |fixture: &KeyboardFixture| {
                move_to_named(fixture, &browser, "a.txt");
                open_rename(fixture, Key::r);
                fixture.shortcuts.prompt().set_text("stale.txt");
            };

            pending(&fixture);
            assert!(press_file_row(&fixture.view.widget(), "b.txt"));
            focus_files(&fixture);
            wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
            assert_eq!(focused_name(&browser), "b.txt", "the click keeps its row");

            pending(&fixture);
            fixture.shortcuts.open_prompt(Prompt::Find);
            assert!(fixture.shortcuts.prompt_text().is_empty());
            fixture.shortcuts.prompt().set_text("c");
            plain(&fixture, Key::Return);
            assert_eq!(focused_name(&browser), "c.txt");

            pending(&fixture);
            PreferenceManager::shared().set_tenxer_mode(false);
            pump(50);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            pump(200);
            assert_eq!(directory_names(&directory), names, "nothing was renamed");

            move_to_named(&fixture, &browser, "a.txt");
            assert!(fixture.press(Key::F2, ModifierType::empty()));
            assert!(
                fixture.view.rename_is_active(),
                "the default map keeps inline rename"
            );
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!fixture.view.rename_is_active());

            PreferenceManager::shared().set_tenxer_mode(true);
            pump(50);
            super::footer_prompt::commit_filter(&fixture, "b");
            wait_until(|| fixture.view.filter_result_names() == ["b.txt"]);
            open_rename(&fixture, Key::r);
            assert_eq!(
                fixture.shortcuts.prompt_text(),
                "b.txt",
                "the focused result"
            );
            plain(&fixture, Key::Escape);
            plain(&fixture, Key::Escape);

            open_empty_folder(&fixture);
            for key in [Key::r, Key::F2] {
                plain(&fixture, key);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                assert_eq!(feedback(&fixture), "Nothing to rename");
            }
        },
    );
}

fn set_age(path: &Path, seconds_ago: u64) {
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("aged file");
    file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago))
        .expect("modification time");
}

fn seed_sorting(directory: &Path) -> PathBuf {
    let sorting = directory.join("sorting");
    std::fs::create_dir(&sorting).expect("sorting folder");
    for (name, bytes, age) in [
        ("alpha.zip", 3, 300),
        ("beta.txt", 1, 200),
        ("gamma.md", 2, 100),
    ] {
        std::fs::write(sorting.join(name), vec![b'x'; bytes]).expect("sorted file");
        set_age(&sorting.join(name), age);
    }
    sorting
}

fn sort(fixture: &KeyboardFixture, key: Key, shift: bool) {
    plain(fixture, Key::comma);
    if shift {
        shifted(fixture, key);
    } else {
        plain(fixture, key);
    }
}

fn sorting_of(browser: &crate::app::Browser, depth: usize) -> (SortKey, SortDirection) {
    let preferences = browser.column_preferences(depth).expect("column");
    (preferences.sort_key, preferences.sort_direction)
}

#[test]
fn tenxer_sort_chord_sorts_the_focused_pane_and_saves_the_default() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_sort_chord_sorts_the_focused_pane_and_saves_the_default",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            seed_sorting(&directory);
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            refresh(&fixture, "sorting");
            move_to_named(&fixture, &browser, "sorting");
            plain(&fixture, Key::l);
            wait_until(|| browser.active_depth() == Some(1));
            wait_loaded(&browser, 1);
            focus_files(&fixture);
            move_to_named(&fixture, &browser, "gamma.md");
            let outer = sorting_of(&browser, 0);

            plain(&fixture, Key::comma);
            assert_eq!(fixture.shortcuts.armed_chord(), Some(Chord::Sort));
            assert_eq!(fixture.shortcuts.chord().text(), ",-");
            wait_until(|| fixture.shortcuts.chord_options().is_some());
            assert_eq!(
                fixture.shortcuts.chord_options().expect("options"),
                [
                    ("a", "Name"),
                    ("m", "Modified"),
                    ("s", "Size"),
                    ("e", "Type"),
                    ("Shift", "Reverse"),
                ]
                .map(|(key, action)| (key.to_owned(), action.to_owned()))
            );
            plain(&fixture, Key::Escape);

            let saved = || {
                let saved = PreferenceManager::shared().sort_preferences();
                (saved.sort_key, saved.sort_direction)
            };
            for (key, shift, sorting, order) in [
                (
                    Key::s,
                    false,
                    (SortKey::Size, SortDirection::Ascending),
                    ["beta.txt", "gamma.md", "alpha.zip"],
                ),
                (
                    Key::S,
                    true,
                    (SortKey::Size, SortDirection::Descending),
                    ["alpha.zip", "gamma.md", "beta.txt"],
                ),
                (
                    Key::m,
                    false,
                    (SortKey::Modified, SortDirection::Ascending),
                    ["alpha.zip", "beta.txt", "gamma.md"],
                ),
                (
                    Key::M,
                    true,
                    (SortKey::Modified, SortDirection::Descending),
                    ["gamma.md", "beta.txt", "alpha.zip"],
                ),
                (
                    Key::A,
                    true,
                    (SortKey::Name, SortDirection::Descending),
                    ["gamma.md", "beta.txt", "alpha.zip"],
                ),
                (
                    Key::a,
                    false,
                    (SortKey::Name, SortDirection::Ascending),
                    ["alpha.zip", "beta.txt", "gamma.md"],
                ),
            ] {
                sort(&fixture, key, shift);
                wait_until(|| sorting_of(&browser, 1) == sorting && names_at(&browser, 1) == order);
                wait_until(|| fixture.view.item_view_has_focus());
                assert_eq!(saved(), sorting, "{key:?} saves the default");
                assert_eq!(
                    focused_name(&browser),
                    "gamma.md",
                    "{key:?} keeps the cursor"
                );
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None, "{key:?}");
                assert_eq!(
                    sorting_of(&browser, 0),
                    outer,
                    "other columns keep their order"
                );
            }
            sort(&fixture, Key::e, false);
            wait_until(|| sorting_of(&browser, 1).0 == SortKey::Type);
            wait_until(|| fixture.view.item_view_has_focus());

            for key in [Key::n, Key::t] {
                sort(&fixture, key, false);
                assert_eq!(feedback(&fixture), "Unknown chord", "{key:?}");
                assert_eq!(fixture.shortcuts.armed_chord(), None);
                pump(100);
                assert_eq!(sorting_of(&browser, 1).0, SortKey::Type, "{key:?}");
            }
            assert_eq!(
                focused_name(&browser),
                "gamma.md",
                "n does not repeat a find"
            );
        },
    );
}

#[test]
fn tenxer_list_headings_follow_keyboard_sorting() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_list_headings_follow_keyboard_sorting",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let sorting = seed_sorting(&directory);
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            browser.navigate(Location::local(&sorting));
            wait_until(|| browser.active_location() == Some(Location::local(&sorting)));
            wait_loaded(&browser, 0);
            fixture.view.set_view_mode(BrowserMode::List);
            pump(100);
            focus_files(&fixture);

            fixture
                .view
                .sort_focused_pane(SortKey::Modified, SortDirection::Ascending);
            fixture
                .view
                .sort_focused_pane(SortKey::Name, SortDirection::Ascending);
            wait_until(|| sorting_of(&browser, 0) == (SortKey::Name, SortDirection::Ascending));
            wait_until(|| names_at(&browser, 0) == ["alpha.zip", "beta.txt", "gamma.md"]);
            pump(150);
            wait_until(|| {
                gtk::prelude::RootExt::focus(&fixture.window).is_some_and(|focused| {
                    !focused.is::<gtk::Stack>() && !focused.is::<gtk::ListView>()
                })
            });
            focus_files(&fixture);
            sort(&fixture, Key::s, false);
            wait_until(|| sorting_of(&browser, 0) == (SortKey::Size, SortDirection::Ascending));
            let heading = widget_with_class(&fixture.view.widget(), "list-headings")
                .and_then(|headings| button_labeled(&headings, "Size"))
                .expect("Size heading");
            heading.emit_clicked();
            wait_until(|| sorting_of(&browser, 0) == (SortKey::Size, SortDirection::Descending));
            wait_until(|| names_at(&browser, 0) == ["alpha.zip", "gamma.md", "beta.txt"]);
        },
    );
}

fn reachable_names(fixture: &KeyboardFixture) -> Vec<String> {
    pump(200);
    focus_files(fixture);
    let browser = fixture.view.browser();
    plain(fixture, Key::Home);
    let mut names = vec![focused_name(&browser)];
    loop {
        plain(fixture, Key::j);
        let name = focused_name(&browser);
        if names.last() == Some(&name) {
            return names;
        }
        names.push(name);
    }
}

#[test]
fn tenxer_dot_toggles_hidden_files_in_every_window_and_filter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_dot_toggles_hidden_files_in_every_window_and_filter",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            std::fs::write(directory.join(".secret.txt"), "hidden").expect("hidden file");
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            refresh(&fixture, ".secret.txt");
            let other = browser_for_window();
            other.browser().navigate(Location::local(&directory));
            wait_loaded(&other.browser(), 0);
            let shows_hidden = |browser: &crate::app::Browser| {
                browser.preferences().show_hidden
                    && browser
                        .column_preferences(0)
                        .is_some_and(|column| column.show_hidden)
            };
            if shows_hidden(&browser) {
                plain(&fixture, Key::period);
                wait_until(|| !shows_hidden(&browser));
            }

            plain(&fixture, Key::period);
            assert!(PreferenceManager::shared().sort_preferences().show_hidden);
            wait_until(|| shows_hidden(&browser) && shows_hidden(&other.browser()));
            assert!(reachable_names(&fixture).contains(&".secret.txt".to_owned()));
            plain(&fixture, Key::period);
            wait_until(|| !shows_hidden(&browser) && !shows_hidden(&other.browser()));
            assert!(!PreferenceManager::shared().sort_preferences().show_hidden);

            focus_files(&fixture);
            super::footer_prompt::commit_filter(&fixture, "secret");
            wait_until(|| fixture.shortcuts.filter_mark().as_deref() == Some("filter: secret"));
            pump(200);
            assert!(fixture.view.filter_result_names().is_empty());
            assert!(fixture.press(Key::h, ModifierType::CONTROL_MASK));
            wait_until(|| fixture.view.filter_result_names() == [".secret.txt"]);
            assert!(fixture.press(Key::period, ModifierType::CONTROL_MASK));
            wait_until(|| fixture.view.filter_result_names().is_empty());
            plain(&fixture, Key::Escape);
            wait_until(|| fixture.shortcuts.filter_mark().is_none());
            assert!(
                !shows_hidden(&browser),
                "clearing the query reveals nothing"
            );
            assert_eq!(reachable_names(&fixture), ["a.txt", "b.txt", "c.txt"]);
        },
    );
}

/// Writes a recorder application for `mime_types` into the test's private
/// data directory and associates it with each of them.
fn chooser_sections(overlay: &gtk::Overlay) -> Vec<(String, Vec<String>)> {
    let Some(list) = widget_with_class(overlay.upcast_ref(), "open-with-list") else {
        return Vec::new();
    };
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut row = list.first_child();
    while let Some(widget) = row {
        let labels = label_texts(&widget);
        if widget.has_css_class("open-with-heading-row") {
            sections.push((labels.join(""), Vec::new()));
        } else if let (Some(section), Some(name)) = (sections.last_mut(), labels.first()) {
            section.1.push(name.clone());
        }
        row = widget.next_sibling();
    }
    sections
}

fn label_texts(widget: &gtk::Widget) -> Vec<String> {
    let mut texts = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        texts.push(label.text().to_string());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        texts.extend(label_texts(&widget));
        child = widget.next_sibling();
    }
    texts
}

fn received(output: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(output)
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .collect()
}

#[test]
fn tenxer_open_with_offers_shared_handlers_for_the_fill() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_open_with_offers_shared_handlers_for_the_fill",
        || {
            let output = glib::user_data_dir().join("received");
            // GIO indexes applications on first use.
            recorder_app(
                "strata-shared",
                "Shared Viewer",
                "text/plain;image/png;",
                &output,
            );
            recorder_app("strata-text", "Text Only", "text/plain;", &output);
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            std::fs::write(directory.join("picture.png"), b"\x89PNG\r\n\x1a\n").expect("png");
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            refresh(&fixture, "picture.png");

            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::space);
            move_to_named(&fixture, &browser, "picture.png");
            plain(&fixture, Key::space);
            move_to_named(&fixture, &browser, "b.txt");
            shifted(&fixture, Key::O);
            assert!(
                !modal_visible(&fixture.overlay),
                "the lookup does not block"
            );
            wait_until(|| modal_visible(&fixture.overlay));
            let sections = chooser_sections(&fixture.overlay);
            assert_eq!(sections[0].0, "Recommended Applications");
            assert!(
                sections[0].1.contains(&"Shared Viewer".to_owned()),
                "{sections:?}"
            );
            assert!(
                !sections[0].1.contains(&"Text Only".to_owned()),
                "{sections:?}"
            );
            assert!(
                sections
                    .iter()
                    .any(|(title, apps)| title == "Other Applications"
                        && apps.contains(&"Text Only".to_owned())),
                "{sections:?}"
            );
            assert!(click_class(&fixture.overlay, "action-dialog-close"));
            wait_until(|| !modal_visible(&fixture.overlay));
            pump(200);
            assert!(!output.exists(), "cancelling launches nothing");

            focus_files(&fixture);
            shifted(&fixture, Key::O);
            wait_until(|| modal_visible(&fixture.overlay));
            assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
            wait_until(|| received(&output).len() == 2);
            let mut opened = received(&output);
            opened.sort();
            assert_eq!(
                opened,
                [directory.join("a.txt"), directory.join("picture.png")]
            );
        },
    );
}

#[test]
fn tenxer_open_with_drops_a_lookup_overtaken_by_a_newer_interaction() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_open_with_drops_a_lookup_overtaken_by_a_newer_interaction",
        || {
            recorder_app(
                "strata-text",
                "Text Only",
                "text/plain;",
                &glib::user_data_dir().join("out"),
            );
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            std::fs::create_dir(directory.join("folder")).expect("folder");
            std::os::unix::fs::symlink(directory.join("missing"), directory.join("link"))
                .expect("dangling link");
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            refresh(&fixture, "link");
            let settle = |fixture: &KeyboardFixture| {
                pump(300);
                assert!(!modal_visible(&fixture.overlay));
            };

            move_to_named(&fixture, &browser, "a.txt");
            shifted(&fixture, Key::O);
            plain(&fixture, Key::j);
            settle(&fixture);

            move_to_named(&fixture, &browser, "a.txt");
            shifted(&fixture, Key::O);
            browser.select(0, 1);
            settle(&fixture);

            move_to_named(&fixture, &browser, "a.txt");
            shifted(&fixture, Key::O);
            browser.navigate(Location::local(directory.join("folder")));
            settle(&fixture);
            browser.navigate(Location::local(&directory));
            wait_loaded(&browser, 0);
            focus_files(&fixture);

            move_to_named(&fixture, &browser, "a.txt");
            shifted(&fixture, Key::O);
            PreferenceManager::shared().set_tenxer_mode(false);
            settle(&fixture);
            PreferenceManager::shared().set_tenxer_mode(true);
            pump(50);

            move_to_named(&fixture, &browser, "a.txt");
            shifted(&fixture, Key::O);
            fixture.sidebar.state.focus_active_place();
            settle(&fixture);

            move_to_named(&fixture, &browser, "link");
            shifted(&fixture, Key::O);
            wait_until(|| !fixture.shortcuts.feedback_text().is_empty());
            assert_eq!(
                feedback(&fixture),
                "Broken symbolic links cannot be opened with an application"
            );
            assert!(!modal_visible(&fixture.overlay));

            open_empty_folder(&fixture);
            shifted(&fixture, Key::O);
            assert_eq!(feedback(&fixture), "Nothing to open");
        },
    );
}

fn write_action(id: &str, name: &str, menu: &str, extensions: &str, extra: &str, output: &Path) {
    let directory = crate::storage::config_directory().join("actions").join(id);
    std::fs::create_dir_all(&directory).expect("action folder");
    std::fs::write(
        directory.join("action.toml"),
        format!(
            "schema_version = 1\nid = \"{id}\"\nname = \"{name}\"\nmenu = \"{menu}\"\n{extra}\n[when]\nextensions = [{extensions}]\n\n[run]\nruntime = \"command\"\nprogram = \"/bin/sh\"\nargs = [\"-c\", \"printf '%s\\\\n' {id} \\\"$@\\\" >> '{}'\", \"sh\", \"{{paths}}\"]\n",
            output.display()
        ),
    )
    .expect("action manifest");
}

fn run_jobs(duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        crate::ui::jobs::shared().pump();
        pump(20);
    }
}

fn wait_invocations(output: &Path, lines: usize) -> Vec<String> {
    wait_until(|| {
        crate::ui::jobs::shared().pump();
        invocations(output).len() >= lines
    });
    invocations(output)
}

fn invocations(output: &Path) -> Vec<String> {
    std::fs::read_to_string(output)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn tenxer_numbered_actions_run_the_listed_match_on_the_current_targets() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_numbered_actions_run_the_listed_match_on_the_current_targets",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let output = fixture._history_directory.path().join("invocations");
            write_action("top-b", "Top B", "top", "\"txt\"", "", &output);
            write_action("top-a", "Top A", "top", "\"txt\"", "", &output);
            for index in 1..=9 {
                write_action(
                    &format!("sub-{index}"),
                    &format!("Sub {index}"),
                    "submenu",
                    "\"txt\"",
                    "",
                    &output,
                );
            }
            write_action(
                "disabled",
                "Aaa disabled",
                "top",
                "\"txt\"",
                "enabled = false",
                &output,
            );
            write_action("images", "Aaa images", "top", "\"png\"", "", &output);
            write_action(
                "confirming",
                "Aaa confirm",
                "submenu",
                "\"md\"",
                "",
                &output,
            );
            let confirming =
                crate::storage::config_directory().join("actions/confirming/action.toml");
            let manifest = std::fs::read_to_string(&confirming)
                .expect("manifest")
                .replace(
                    "runtime = \"command\"",
                    "runtime = \"command\"\nconfirm = true",
                );
            std::fs::write(&confirming, manifest).expect("confirming action");
            crate::ui::actions::shared().reload();
            std::fs::write(directory.join("notes.md"), "notes").expect("markdown");
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            refresh(&fixture, "notes.md");

            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::semicolon);
            assert_eq!(fixture.shortcuts.armed_chord(), Some(Chord::Action));
            assert_eq!(fixture.shortcuts.chord().text(), ";-");
            wait_until(|| fixture.shortcuts.chord_options().is_some());
            let mut listed = vec![("1", "Top A"), ("2", "Top B")];
            let subs: Vec<String> = (1..=8).map(|index| format!("Sub {index}")).collect();
            let keys = ["3", "4", "5", "6", "7", "8", "9", "0"];
            listed.extend(keys.iter().copied().zip(subs.iter().map(String::as_str)));
            listed.extend(LETTERED_ACTIONS);
            assert_eq!(
                fixture.shortcuts.chord_options().expect("options"),
                listed
                    .iter()
                    .map(|(key, name)| ((*key).to_owned(), (*name).to_owned()))
                    .collect::<Vec<_>>(),
                "top-level first, disabled and non-matching omitted, eleventh dropped"
            );
            plain(&fixture, Key::_1);
            assert_eq!(
                wait_invocations(&output, 2),
                ["top-a", directory.join("a.txt").to_string_lossy().as_ref()]
            );
            std::fs::remove_file(&output).expect("reset recorder");

            focus_files(&fixture);
            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::space);
            plain(&fixture, Key::space);
            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::_0);
            assert_eq!(
                wait_invocations(&output, 3),
                [
                    "sub-8",
                    directory.join("a.txt").to_string_lossy().as_ref(),
                    directory.join("b.txt").to_string_lossy().as_ref(),
                ],
                "the filled selection"
            );
            std::fs::remove_file(&output).expect("reset recorder");

            focus_files(&fixture);
            browser.clear_active_selection();
            move_to_named(&fixture, &browser, "notes.md");
            plain(&fixture, Key::semicolon);
            wait_until(|| fixture.shortcuts.chord_options().is_some());
            assert_eq!(
                fixture.shortcuts.chord_options().expect("options"),
                owned_rows(&[&[("1", "Aaa confirm")], LETTERED_ACTIONS].concat())
            );
            plain(&fixture, Key::_2);
            assert_eq!(feedback(&fixture), "No action 2");
            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::Escape);
            assert_eq!(fixture.shortcuts.armed_chord(), None);
            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::x);
            assert_eq!(feedback(&fixture), "Unknown chord");

            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::_1);
            wait_until(|| modal_visible(&fixture.overlay));
            assert!(click_class(&fixture.overlay, "action-dialog-close"));
            wait_until(|| !modal_visible(&fixture.overlay));
            run_jobs(Duration::from_millis(300));
            assert!(
                invocations(&output).is_empty(),
                "a cancelled confirmation runs nothing"
            );
            wait_until(|| fixture.view.item_view_has_focus());

            focus_files(&fixture);
            move_to_named(&fixture, &browser, "notes.md");
            plain(&fixture, Key::semicolon);
            browser.select(0, 0);
            plain(&fixture, Key::_1);
            assert_eq!(feedback(&fixture), "Selection changed");
            run_jobs(Duration::from_millis(300));
            assert!(invocations(&output).is_empty(), "stale targets run nothing");
            assert!(!modal_visible(&fixture.overlay));

            let unlisted = fixture._history_directory.path().join("unlisted");
            browser.clear_active_selection();
            move_to_named(&fixture, &browser, "notes.md");
            plain(&fixture, Key::semicolon);
            write_action("aa-new", "Aa new", "submenu", "\"md\"", "", &unlisted);
            crate::ui::actions::shared().reload();
            plain(&fixture, Key::_1);
            assert_eq!(feedback(&fixture), "Actions changed");
            run_jobs(Duration::from_millis(300));
            assert!(!unlisted.exists() && !modal_visible(&fixture.overlay));

            open_empty_folder(&fixture);
            plain(&fixture, Key::semicolon);
            wait_until(|| fixture.shortcuts.chord_options().is_some());
            assert_eq!(
                fixture.shortcuts.chord_options().expect("options"),
                owned_rows(&[&[("1\u{2013}0", "No matching actions")], LETTERED_ACTIONS].concat())
            );
            plain(&fixture, Key::_1);
            assert_eq!(feedback(&fixture), "No action 1");
        },
    );
}

const LETTERED_ACTIONS: &[(&str, &str)] = &[
    ("t", "Open terminal here"),
    ("c", "Compress\u{2026}"),
    ("e", "Extract here"),
    ("E", "Extract to\u{2026}"),
];

fn owned_rows(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(key, name)| ((*key).to_owned(), (*name).to_owned()))
        .collect()
}

#[test]
fn tenxer_action_chord_terminal_refuses_a_non_local_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::tenxer_action_chord_terminal_refuses_a_non_local_folder",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            browser.navigate(Location::uri("trash:///"));
            wait_until(|| browser.active_location() == Some(Location::uri("trash:///")));
            wait_loaded(&browser, 0);
            focus_files(&fixture);

            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::t);
            assert_eq!(feedback(&fixture), "Can\u{2019}t open a terminal here");
            assert!(!modal_visible(&fixture.overlay), "no error dialog");
            assert_eq!(fixture.shortcuts.armed_chord(), None);
        },
    );
}
