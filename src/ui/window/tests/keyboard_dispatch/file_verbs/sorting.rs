// SPDX-License-Identifier: MIT

use super::*;

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
    browser.column_preferences(depth).expect("column").sort()
}

fn open_sorting_folder(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    refresh(fixture, "sorting");
    move_to_named(fixture, &browser, "sorting");
    plain(fixture, Key::l);
    wait_until(|| browser.active_depth() == Some(1));
    wait_loaded(&browser, 1);
    focus_files(fixture);
    move_to_named(fixture, &browser, "gamma.md");
}

fn sort_chord_lists_its_keys(fixture: &KeyboardFixture) {
    plain(fixture, Key::comma);
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
            ("x", "Extension"),
            ("b", "Created"),
            ("u", "Unsorted"),
            ("n", "Natural"),
            ("l", "Alphabetical"),
            ("c", "Case-sensitive names"),
            ("f", "Folders first"),
            ("Shift", "Reverse"),
        ]
        .map(|(key, action)| (key.to_owned(), action.to_owned()))
    );
    plain(fixture, Key::Escape);
}

fn saved_sort(folder: &Location) -> (SortKey, SortDirection) {
    match PreferenceManager::shared().resolve_folder_sort(folder, false) {
        crate::model::FolderSort::Saved(sort_key, sort_direction, _) => (sort_key, sort_direction),
        _ => PreferenceManager::shared().default_sort(),
    }
}

struct SortStep {
    key: Key,
    shift: bool,
    sorting: (SortKey, SortDirection),
    order: [&'static str; 3],
}

const SORT_STEPS: [SortStep; 6] = [
    SortStep {
        key: Key::s,
        shift: false,
        sorting: (SortKey::Size, SortDirection::Ascending),
        order: ["beta.txt", "gamma.md", "alpha.zip"],
    },
    SortStep {
        key: Key::S,
        shift: true,
        sorting: (SortKey::Size, SortDirection::Descending),
        order: ["alpha.zip", "gamma.md", "beta.txt"],
    },
    SortStep {
        key: Key::m,
        shift: false,
        sorting: (SortKey::Modified, SortDirection::Ascending),
        order: ["alpha.zip", "beta.txt", "gamma.md"],
    },
    SortStep {
        key: Key::M,
        shift: true,
        sorting: (SortKey::Modified, SortDirection::Descending),
        order: ["gamma.md", "beta.txt", "alpha.zip"],
    },
    SortStep {
        key: Key::A,
        shift: true,
        sorting: (SortKey::Name, SortDirection::Descending),
        order: ["gamma.md", "beta.txt", "alpha.zip"],
    },
    SortStep {
        key: Key::a,
        shift: false,
        sorting: (SortKey::Name, SortDirection::Ascending),
        order: ["alpha.zip", "beta.txt", "gamma.md"],
    },
];

/// Each sort applies to the focused pane only and is remembered for its folder,
/// leaving the default, the cursor and the other columns alone.
fn sorts_are_remembered_for_the_folder(fixture: &KeyboardFixture, outer: (SortKey, SortDirection)) {
    let browser = fixture.view.browser();
    PreferenceManager::shared().set_remember_folder_views(true);
    let folder = browser.location_at(1).expect("sorted folder");
    let default = PreferenceManager::shared().default_sort();
    for SortStep {
        key,
        shift,
        sorting,
        order,
    } in SORT_STEPS
    {
        sort(fixture, key, shift);
        wait_until(|| sorting_of(&browser, 1) == sorting && names_at(&browser, 1) == order);
        wait_until(|| fixture.view.item_view_has_focus());
        assert_eq!(
            saved_sort(&folder),
            sorting,
            "{key:?} is remembered for the folder"
        );
        assert_eq!(
            (
                PreferenceManager::shared().default_sort(),
                focused_name(&browser),
                fixture.shortcuts.open_prompt_kind(),
                sorting_of(&browser, 0),
            ),
            (default, "gamma.md".to_owned(), None, outer),
            "{key:?}: (leaves the default, keeps the cursor, closes no prompt, \
             other columns keep their order)"
        );
    }
}

fn unknown_sort_keys_keep_the_order(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    for key in [Key::q, Key::t] {
        sort(fixture, key, false);
        assert_eq!(feedback(fixture), "Unknown chord", "{key:?}");
        assert_eq!(fixture.shortcuts.armed_chord(), None);
        pump(100);
        assert_eq!(sorting_of(&browser, 1).0, SortKey::Type, "{key:?}");
    }
    assert_eq!(
        focused_name(&browser),
        "gamma.md",
        "n does not repeat a find"
    );
}

#[test]
fn tenxer_sort_chord_sorts_the_focused_pane_and_remembers_it_for_the_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::sorting::tenxer_sort_chord_sorts_the_focused_pane_and_remembers_it_for_the_folder",
        || {
            let fixture = KeyboardFixture::new();
            seed_sorting(fixture._directory.path());
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            open_sorting_folder(&fixture);
            let outer = sorting_of(&browser, 0);

            sort_chord_lists_its_keys(&fixture);
            sorts_are_remembered_for_the_folder(&fixture, outer);
            sort(&fixture, Key::e, false);
            wait_until(|| sorting_of(&browser, 1).0 == SortKey::Type);
            wait_until(|| fixture.view.item_view_has_focus());
            unknown_sort_keys_keep_the_order(&fixture);
        },
    );
}

#[test]
fn tenxer_sort_chord_toggles_folders_first_and_keeps_the_sort() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::sorting::tenxer_sort_chord_toggles_folders_first_and_keeps_the_sort",
        || {
            const FOLDERS_FIRST: [&str; 4] = ["directory", "gamma.md", "beta.txt", "alpha.zip"];
            const MIXED: [&str; 4] = ["gamma.md", "directory", "beta.txt", "alpha.zip"];
            let fixture = KeyboardFixture::new();
            let sorting = seed_sorting(fixture._directory.path());
            std::fs::create_dir(sorting.join("directory")).expect("nested folder");
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            browser.set_folders_first(0, true);
            open_sorting_folder(&fixture);
            sort(&fixture, Key::A, true);
            wait_until(|| names_at(&browser, 1) == FOLDERS_FIRST);
            wait_until(|| fixture.view.item_view_has_focus());

            for (folders_first, order) in [(false, MIXED), (true, FOLDERS_FIRST)] {
                sort(&fixture, Key::f, false);
                wait_until(|| names_at(&browser, 1) == order);
                wait_until(|| fixture.view.item_view_has_focus());
                assert_eq!(
                    (
                        browser.preferences().folders_first,
                        sorting_of(&browser, 1),
                        focused_name(&browser),
                    ),
                    (
                        folders_first,
                        (SortKey::Name, SortDirection::Descending),
                        "gamma.md".to_owned(),
                    )
                );
            }

            sort(&fixture, Key::F, true);
            assert_eq!(feedback(&fixture), "Unknown chord");
            assert!(browser.preferences().folders_first);
        },
    );
}

fn button_labeled(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    let labelled = |button: &&gtk::Button| button.is_visible() && rendered_name(widget, label);
    if let Some(button) = widget.downcast_ref::<gtk::Button>().filter(labelled) {
        return Some(button.clone());
    }
    widget_children(widget).find_map(|child| button_labeled(&child, label))
}

fn focus_left_the_list_container(fixture: &KeyboardFixture) -> bool {
    gtk::prelude::RootExt::focus(&fixture.window)
        .is_some_and(|focused| !focused.is::<gtk::Stack>() && !focused.is::<gtk::ListView>())
}

#[test]
fn tenxer_list_headings_follow_keyboard_sorting() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::sorting::tenxer_list_headings_follow_keyboard_sorting",
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
            wait_until(|| focus_left_the_list_container(&fixture));
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

fn shows_hidden(browser: &crate::app::Browser) -> bool {
    browser.preferences().show_hidden
        && browser
            .column_preferences(0)
            .is_some_and(|column| column.show_hidden)
}

/// `.` toggles hidden files for every window and saves the preference.
fn dot_toggles_hidden_files_everywhere(fixture: &KeyboardFixture, other: &crate::app::Browser) {
    let browser = fixture.view.browser();
    plain(fixture, Key::period);
    assert!(PreferenceManager::shared().sort_preferences().show_hidden);
    wait_until(|| shows_hidden(&browser) && shows_hidden(other));
    assert!(reachable_names(fixture).contains(&".secret.txt".to_owned()));
    plain(fixture, Key::period);
    wait_until(|| !shows_hidden(&browser) && !shows_hidden(other));
    assert!(!PreferenceManager::shared().sort_preferences().show_hidden);
}

/// Inside a filter, Ctrl+H and Ctrl+. reveal and hide hidden hits without
/// changing the saved preference.
fn filter_hidden_toggles_stay_in_the_filter(fixture: &KeyboardFixture) {
    focus_files(fixture);
    super::super::footer_prompt::commit_filter(fixture, "secret");
    wait_until(|| fixture.shortcuts.filter_mark().as_deref() == Some("filter: secret"));
    pump(200);
    assert!(fixture.view.filter_result_names().is_empty());
    assert!(fixture.press(Key::h, ModifierType::CONTROL_MASK));
    wait_until(|| fixture.view.filter_result_names() == [".secret.txt"]);
    assert!(fixture.press(Key::period, ModifierType::CONTROL_MASK));
    wait_until(|| fixture.view.filter_result_names().is_empty());
    plain(fixture, Key::Escape);
    wait_until(|| fixture.shortcuts.filter_mark().is_none());
    assert!(
        !shows_hidden(&fixture.view.browser()),
        "clearing the query reveals nothing"
    );
    assert_eq!(reachable_names(fixture), ["a.txt", "b.txt", "c.txt"]);
}

#[test]
fn tenxer_dot_toggles_hidden_files_in_every_window_and_filter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::sorting::tenxer_dot_toggles_hidden_files_in_every_window_and_filter",
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
            if shows_hidden(&browser) {
                plain(&fixture, Key::period);
                wait_until(|| !shows_hidden(&browser));
            }

            dot_toggles_hidden_files_everywhere(&fixture, &other.browser());
            filter_hidden_toggles_stay_in_the_filter(&fixture);
        },
    );
}
