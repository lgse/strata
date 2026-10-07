// SPDX-License-Identifier: MIT

use super::footer_prompt::{
    ALL_REPORTS, IMMEDIATE_REPORTS, commit_search, enable_tenxer, seed_filter_tree, wait_results,
};
use super::*;

fn peek_showing(fixture: &KeyboardFixture) -> bool {
    widget_with_class(fixture.window.upcast_ref(), "peek-popover")
        .is_some_and(|popover| popover.is_visible())
}

#[test]
fn leaving_tenxer_ends_peeks_and_ranges_but_keeps_fills_and_columns_in_every_window() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::mode_exit::leaving_tenxer_ends_peeks_and_ranges_but_keeps_fills_and_columns_in_every_window",
        || {
            let none = ModifierType::empty();
            let listing = KeyboardFixture::new();
            let columns = KeyboardFixture::new();
            let preferences = enable_tenxer(&listing);
            columns.shortcuts.bind_preferences(&preferences);
            for fixture in [&listing, &columns] {
                seed_filter_tree(fixture);
            }

            listing.view.set_view_mode(BrowserMode::List);
            let listed = listing.view.browser();
            wait_loaded(&listed, 0);
            move_to_named(&listing, &listed, "reports");
            assert!(listing.press(Key::i, none));
            wait_until(|| peek_showing(&listing));

            let browser = columns.view.browser();
            move_to_named(&columns, &browser, "reports");
            assert!(columns.press(Key::i, none));
            wait_loaded(&browser, 1);
            move_to_named(&columns, &browser, "a.txt");
            assert!(columns.press(Key::v, none));
            assert!(columns.press(Key::j, none));
            assert!(browser.visual_kind().is_some());
            let range = fill_names(&browser);
            assert_eq!(range.len(), 2);

            preferences.set_tenxer_mode(false);
            pump(50);
            wait_until(|| !peek_showing(&listing));
            assert_eq!(browser.visual_kind(), None);
            assert_eq!(columns.shortcuts.visual_text(), None);
            assert_eq!(
                fill_names(&browser),
                range,
                "leaving the range dropped its fill"
            );
            assert!(
                browser.column_snapshot(1).is_some(),
                "leaving closed the Miller column opened with i"
            );

            preferences.set_tenxer_mode(true);
            pump(50);
            assert_eq!(
                browser.visual_kind(),
                None,
                "re-entering restored the range"
            );
            assert!(!peek_showing(&listing), "re-entering restored the peek");
        },
    );
}

#[test]
fn default_filter_follows_the_saved_scope_after_a_search_and_mode_exit() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::mode_exit::default_filter_follows_the_saved_scope_after_a_search_and_mode_exit",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let preferences = enable_tenxer(&fixture);
            preferences.set_filter_include_subfolders(false);
            focus_files(&fixture);
            commit_search(&fixture, "report");
            wait_results(&fixture, &ALL_REPORTS);

            preferences.set_tenxer_mode(false);
            pump(50);
            focus_files(&fixture);
            assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
            let entry = gtk::prelude::RootExt::focus(&fixture.window)
                .and_then(|focus| {
                    focus
                        .clone()
                        .downcast::<gtk::Entry>()
                        .ok()
                        .or_else(|| focus.ancestor(gtk::Entry::static_type()).and_downcast())
                })
                .expect("default filter field");
            assert!(entry.text().is_empty(), "Ctrl+F reopened the search query");
            entry.set_text("report");
            wait_results(&fixture, &IMMEDIATE_REPORTS);
            pump(300);
            wait_results(&fixture, &IMMEDIATE_REPORTS);
            assert!(!preferences.filter_include_subfolders());
        },
    );
}
