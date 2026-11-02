// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::*;

/// Typing into the **s** prompt searches live without touching the saved scope;
/// Enter applies the hits and the footer follows the cursor through them.
fn search_prompt_live_then_commit(
    fixture: &KeyboardFixture,
    preferences: &PreferenceManager,
    mode: BrowserMode,
) {
    type_search(fixture, "report");
    assert_eq!(fixture.shortcuts.prompt_label().as_deref(), Some("search:"));
    wait_results(fixture, &ALL_REPORTS);
    assert!(
        fixture.shortcuts.prompt_has_focus(),
        "{mode:?}: typing searches live and stays in the prompt"
    );
    assert!(
        !preferences.filter_include_subfolders(),
        "{mode:?}: the saved scope is untouched"
    );
    assert!(fixture.press(Key::Return, ModifierType::empty()));
    wait_until(|| {
        fixture.view.item_view_has_focus() && fixture.view.selected_search_result().is_some()
    });
    wait_until(|| {
        fixture.shortcuts.filter_mark().as_deref() == Some("search: report")
            && fixture.shortcuts.count_text().0 == "4 items"
    });
    for _ in ALL_REPORTS {
        let name = fixture
            .view
            .selected_search_result()
            .expect("cursor hit")
            .display_name;
        let path = hit_path(&name);
        wait_until(|| fixture.shortcuts.current_hit() == Some((path.clone(), path.clone())));
        assert!(fixture.press(Key::j, ModifierType::empty()));
    }
    assert!(
        in_fixture_folder(fixture),
        "{mode:?}: applying the search opens nothing"
    );
    wait_highlights(fixture, &ALL_REPORTS);
}

/// Find searches the visible hits; Ctrl+R leaves search hits alone.
fn find_within_hits_and_ctrl_r(fixture: &KeyboardFixture, mode: BrowserMode) {
    type_and_submit(fixture, Key::slash, "deep-report");
    assert_eq!(
        fixture
            .view
            .selected_search_result()
            .expect("find focuses a visible hit")
            .display_name,
        "deep-report.txt",
        "{mode:?}: find searches visible hits, not the hidden directory"
    );
    wait_highlights(fixture, &["deep-report.txt"]);
    assert!(fixture.press(Key::Escape, ModifierType::empty()));
    wait_highlights(fixture, &ALL_REPORTS);

    let selected = selected_result_names(fixture);
    assert!(fixture.press(Key::r, ModifierType::CONTROL_MASK));
    pump(20);
    assert_eq!(
        selected_result_names(fixture),
        selected,
        "{mode:?}: Ctrl+R is inactive for search hits"
    );
}

fn s_prefills_the_showing_search(fixture: &KeyboardFixture, mode: BrowserMode) {
    assert!(fixture.press(Key::s, ModifierType::empty()));
    assert_eq!(
        fixture.shortcuts.prompt().text(),
        "report",
        "{mode:?}: s pre-fills the showing search"
    );
}

/// Escape in the prompt keeps the hits; Escape in the listing, an emptied
/// prompt, and a narrowed query all restore the filter underneath.
fn escape_variants_restore_the_filter(fixture: &KeyboardFixture, mode: BrowserMode) {
    let none = ModifierType::empty();
    s_prefills_the_showing_search(fixture, mode);
    assert!(fixture.press(Key::Escape, none));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    wait_until(|| fixture.view.item_view_has_focus());
    assert_eq!(
        result_names(fixture),
        ALL_REPORTS,
        "{mode:?}: prompt Esc keeps hits"
    );
    assert!(fixture.view.listing_search_active());

    assert!(fixture.press(Key::Escape, none));
    wait_filter_restored(fixture, "gamma", &["gamma-report.md"]);
    assert!(fixture.view.item_view_has_focus(), "{mode:?}");

    type_search(fixture, "report");
    wait_results(fixture, &ALL_REPORTS);
    fixture.shortcuts.prompt().set_text("");
    assert!(fixture.press(Key::Escape, none));
    wait_filter_restored(fixture, "gamma", &["gamma-report.md"]);

    type_search(fixture, "deep reports");
    wait_results(fixture, &["deep-report.txt"]);
    wait_highlights(fixture, &["deep-report.txt"]);
    assert!(fixture.press(Key::Escape, none));
    wait_until(|| fixture.view.item_view_has_focus());
    assert!(fixture.press(Key::Escape, none));
    wait_filter_restored(fixture, "gamma", &["gamma-report.md"]);
}

/// Enter with no hits opens nothing; **h** dismisses the hits instead of
/// leaving the folder.
fn miss_and_h_dismiss_the_hits(fixture: &KeyboardFixture, mode: BrowserMode) {
    let none = ModifierType::empty();
    commit_search(fixture, "zzz");
    wait_until(|| {
        fixture.shortcuts.filter_mark().as_deref() == Some("search: zzz")
            && fixture.shortcuts.count_text().0 == "0 items"
    });
    assert!(fixture.press(Key::Return, none));
    pump(100);
    assert!(
        in_fixture_folder(fixture),
        "{mode:?}: Enter with no hits opens no hidden item"
    );
    assert!(fixture.press(Key::Escape, none));
    wait_filter_restored(fixture, "gamma", &["gamma-report.md"]);

    if mode != BrowserMode::Icons {
        commit_search(fixture, "report");
        wait_results(fixture, &ALL_REPORTS);
        assert!(fixture.press(Key::h, none));
        wait_filter_restored(fixture, "gamma", &["gamma-report.md"]);
        assert!(
            in_fixture_folder(fixture),
            "{mode:?}: h dismisses the hits rather than leaving the folder"
        );
    }
}

fn enter_opens_the_hit_and_ends_the_search(fixture: &KeyboardFixture, mode: BrowserMode) {
    let browser = fixture.view.browser();
    commit_search(fixture, "reports");
    wait_until(|| {
        fixture
            .view
            .selected_search_result()
            .is_some_and(|entry| entry.display_name == "reports")
    });
    assert!(fixture.press(Key::Return, ModifierType::empty()));
    wait_until(|| location_ends_with(browser.active_location(), "reports"));
    wait_loaded(&browser, 0);
    assert!(!fixture.view.listing_search_active(), "{mode:?}");
    browser.back();
    wait_until(|| in_fixture_folder(fixture));
    wait_loaded(&browser, 0);
    pump(100);
    assert!(
        !fixture.view.listing_search_active(),
        "{mode:?}: navigation ends the search"
    );
    assert_ne!(
        fixture.shortcuts.filter_mark().as_deref(),
        Some("search: reports")
    );
    fixture.view.clear_listing_filter();
}

#[test]
fn tenxer_search_covers_the_current_tree_and_restores_the_filter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::search::tenxer_search_covers_the_current_tree_and_restores_the_filter",
        || {
            let (fixture, preferences) = tenxer_filter_fixture();
            let browser = fixture.view.browser();

            for mode in ALL_MODES {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                // A mistaken activation of the hidden cursor would open this folder.
                select_named(&fixture, "reports");
                commit_filter(&fixture, "gamma");
                wait_results(&fixture, &["gamma-report.md"]);

                search_prompt_live_then_commit(&fixture, &preferences, mode);
                find_within_hits_and_ctrl_r(&fixture, mode);
                escape_variants_restore_the_filter(&fixture, mode);
                miss_and_h_dismiss_the_hits(&fixture, mode);
                enter_opens_the_hit_and_ends_the_search(&fixture, mode);
            }
        },
    );
}

#[test]
fn tenxer_search_caps_hits_and_retires_stale_queries() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::search::tenxer_search_caps_hits_and_retires_stale_queries",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let nested = fixture._directory.path().join("many/deeper");
            std::fs::create_dir_all(&nested).expect("nested folder");
            for index in 0..130 {
                std::fs::write(nested.join(format!("hit-{index:03}.txt")), b"hit")
                    .expect("hit file");
            }
            let browser = fixture.view.browser();
            fixture.view.refresh();
            wait_loaded(&browser, 0);
            let preferences = enable_tenxer(&fixture);
            preferences.set_filter_include_subfolders(false);
            let none = ModifierType::empty();

            for mode in [BrowserMode::Columns, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                focus_files(&fixture);
                commit_search(&fixture, "hit-");
                wait_until(|| fixture.view.filter_result_names().len() == 100);
                wait_until(|| fixture.shortcuts.count_text().0 == "100 items");
                pump(200);
                assert_eq!(fixture.view.filter_result_names().len(), 100, "{mode:?}");

                // A replaced query, including a slower recursive one, never
                // overwrites the newer query's hits.
                type_search(&fixture, "hit-");
                fixture.shortcuts.prompt().set_text("gamma");
                wait_results(&fixture, &["gamma-report.md"]);
                pump(300);
                assert_eq!(result_names(&fixture), ["gamma-report.md"], "{mode:?}");

                fixture.shortcuts.prompt().set_text("hit-");
                fixture.shortcuts.prompt().set_text("");
                assert!(fixture.press(Key::Escape, none));
                wait_until(|| fixture.view.selected_search_results().is_none());
                pump(300);
                assert!(!fixture.view.listing_search_active(), "{mode:?}");
                assert!(fixture.view.selected_search_results().is_none(), "{mode:?}");
                assert_eq!(fixture.shortcuts.filter_mark(), None, "{mode:?}");
            }
        },
    );
}

fn rebuild_keeps_search(first: &KeyboardFixture, mode: BrowserMode) {
    first.view.set_view_mode(mode);
    wait_results(first, &ALL_REPORTS);
    assert!(first.view.listing_search_active(), "{mode:?}");
    wait_until(|| first.shortcuts.filter_mark().as_deref() == Some("search: report"));
    assert_eq!(revealed_filter_funnels(&first.view.widget()), 0, "{mode:?}");

    let browser = first.view.browser();
    browser.reload_active();
    wait_loaded(&browser, 0);
    pump(200);
    wait_results(first, &ALL_REPORTS);
    assert!(first.view.listing_search_active(), "{mode:?} after reload");
}

#[test]
fn tenxer_search_survives_view_rebuilds_and_ends_with_the_mode() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::search::tenxer_search_survives_view_rebuilds_and_ends_with_the_mode",
        || {
            let first = KeyboardFixture::new();
            let second = KeyboardFixture::new();
            let preferences = enable_tenxer(&first);
            second.shortcuts.bind_preferences(&preferences);
            preferences.set_filter_include_subfolders(false);
            for fixture in [&first, &second] {
                seed_filter_tree(fixture);
                focus_files(fixture);
                commit_filter(fixture, "gamma");
                commit_search(fixture, "report");
                wait_results(fixture, &ALL_REPORTS);
            }

            preferences.set_filter_include_subfolders(true);
            preferences.set_filter_include_subfolders(false);
            pump(200);
            wait_results(&second, &ALL_REPORTS);

            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                rebuild_keeps_search(&first, mode);
            }
            focus_files(&first);
            assert!(first.press(Key::Escape, ModifierType::empty()));
            wait_filter_restored(&first, "gamma", &["gamma-report.md"]);
            commit_search(&first, "report");
            wait_results(&first, &ALL_REPORTS);

            preferences.set_tenxer_mode(false);
            pump(50);
            for fixture in [&first, &second] {
                wait_until(|| fixture.view.selected_search_results().is_none());
                assert!(!fixture.view.listing_search_active());
                assert_eq!(fixture.view.listing_filter(), None);
                wait_until(|| fixture.shortcuts.filter_mark().is_none());
            }
            assert!(!preferences.filter_include_subfolders());
        },
    );
}

#[derive(Debug, Clone, Copy)]
enum RenamedFolder {
    /// The folder the search runs in.
    Searched(BrowserMode),
    /// A hit folder opened beside the search, holding focus when it is renamed.
    OpenedHit,
}

impl RenamedFolder {
    fn kept(self) -> bool {
        matches!(self, RenamedFolder::OpenedHit)
    }

    /// The view mode, the folder searched, and the folder renamed from outside.
    fn prepare(self, root: &Path) -> (BrowserMode, PathBuf, PathBuf) {
        match self {
            RenamedFolder::Searched(mode) => {
                let folder = root.join(format!("{mode:?}-open"));
                std::fs::create_dir(&folder).expect("open folder");
                std::fs::write(folder.join("gamma-report.txt"), b"filter").expect("fixture file");
                (mode, folder.clone(), folder)
            }
            RenamedFolder::OpenedHit => (
                BrowserMode::Columns,
                root.to_path_buf(),
                root.join("reports"),
            ),
        }
    }
}

fn search_in_folder(fixture: &KeyboardFixture, mode: BrowserMode, searched: &Path) {
    let browser = fixture.view.browser();
    fixture.view.set_view_mode(mode);
    browser.navigate(Location::local(searched));
    wait_until(|| browser.location_at(0) == Some(Location::local(searched)));
    wait_loaded(&browser, 0);
    focus_files(fixture);
    commit_filter(fixture, "gamma");
    commit_search(fixture, "report");
}

fn open_hit_beside_the_search(fixture: &KeyboardFixture, hit: &Path) {
    let browser = fixture.view.browser();
    wait_results(fixture, &ALL_REPORTS);
    fixture.view.open_hit_column(0, Location::local(hit));
    wait_loaded(&browser, 1);
    browser.set_active_column(1);
    browser.focus_active();
    pump(50);
}

fn rename_from_outside(fixture: &KeyboardFixture, case: RenamedFolder, renamed: &Path) {
    let browser = fixture.view.browser();
    let moved = renamed.with_file_name(format!("{case:?}-moved"));
    std::fs::rename(renamed, &moved).expect("rename the open folder");
    let depth = usize::from(case.kept());
    wait_until(|| browser.location_at(depth) == Some(Location::local(&moved)));
    wait_loaded(&browser, depth);
    pump(100);
    browser.set_active_column(0);
    focus_files(fixture);
}

fn assert_search_survived(fixture: &KeyboardFixture, case: RenamedFolder) {
    let kept = case.kept();
    assert_eq!(fixture.view.listing_search_active(), kept, "{case:?}");
    assert_eq!(
        fixture.view.listing_filter().as_deref(),
        kept.then_some("gamma"),
        "{case:?}: the search keeps the filter it replaced only while it lasts"
    );
    if kept {
        assert!(fixture.view.dismiss_listing_search(), "{case:?}");
    } else {
        assert!(
            !fixture.view.dismiss_listing_search(),
            "{case:?}: the search ended with its folder"
        );
    }
}

#[test]
fn tenxer_search_ends_only_when_an_outside_rename_moves_its_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::search::tenxer_search_ends_only_when_an_outside_rename_moves_its_folder",
        || {
            let (fixture, _preferences) = tenxer_filter_fixture();
            let root = fixture._directory.path().to_path_buf();

            // The root search runs first, before the other cases add folders below it.
            for case in [
                RenamedFolder::OpenedHit,
                RenamedFolder::Searched(BrowserMode::List),
                RenamedFolder::Searched(BrowserMode::Columns),
            ] {
                let (mode, searched, renamed) = case.prepare(&root);
                search_in_folder(&fixture, mode, &searched);
                if case.kept() {
                    open_hit_beside_the_search(&fixture, &renamed);
                }
                rename_from_outside(&fixture, case, &renamed);
                assert_search_survived(&fixture, case);
                fixture.view.clear_listing_filter();
            }
        },
    );
}

/// `g f` opens the cursor hit's folder with the hit selected and ends the search.
fn go_hit_folder_reveals(
    fixture: &KeyboardFixture,
    mode: BrowserMode,
    (query, folder, hit): (&str, &str, &str),
) {
    let browser = fixture.view.browser();
    let none = ModifierType::empty();
    focus_files(fixture);
    commit_search(fixture, query);
    wait_results(fixture, &[hit]);
    wait_until(|| fixture.view.selected_search_result().is_some());
    assert!(fixture.press(Key::g, none));
    assert!(fixture.press(Key::f, none));
    wait_until(|| {
        location_ends_with(browser.active_location(), folder) && hidden_selection(fixture) == [hit]
    });
    wait_until(|| !fixture.view.listing_search_active());
    assert_eq!(fixture.view.listing_filter(), None, "{mode:?} {query}");
    wait_until(|| fixture.shortcuts.filter_mark().is_none());
    if folder == "reports" {
        browser.back();
        wait_until(|| in_fixture_folder(fixture));
        wait_loaded(&browser, 0);
    }
}

#[test]
fn tenxer_go_hit_folder_reveals_the_cursor_hit() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::search::tenxer_go_hit_folder_reveals_the_cursor_hit",
        || {
            let (fixture, _preferences) = tenxer_filter_fixture();
            let browser = fixture.view.browser();

            for mode in [BrowserMode::Icons, BrowserMode::Columns, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                for case in [
                    ("deep", "reports", "deep-report.txt"),
                    ("gamma", fixture_name(&fixture), "gamma-report.md"),
                ] {
                    go_hit_folder_reveals(&fixture, mode, case);
                }
            }
        },
    );
}
