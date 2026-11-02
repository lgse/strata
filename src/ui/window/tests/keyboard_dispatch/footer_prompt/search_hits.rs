// SPDX-License-Identifier: MIT

use super::*;

/// The applied hits of one view mode, in the order the view lists them.
struct Hits<'a> {
    fixture: &'a KeyboardFixture,
    mode: BrowserMode,
    order: Vec<String>,
}

impl Hits<'_> {
    fn at(&self, position: usize) -> Option<String> {
        Some(self.order[position].clone())
    }

    fn wait_cursor(&self, position: usize) {
        wait_until(|| hit_cursor(self.fixture) == self.at(position));
    }

    fn wait_footer(&self, after: &str, position: usize) {
        wait_footer_hit(self.fixture, self.mode, after, &self.order[position]);
    }

    fn names(&self, positions: &[usize]) -> Vec<String> {
        let mut names: Vec<_> = positions.iter().map(|&at| self.order[at].clone()).collect();
        names.sort();
        names
    }

    fn press(&self, key: Key) {
        assert!(self.fixture.press(key, ModifierType::empty()));
    }

    fn wait_visual(&self, text: Option<&str>) {
        wait_until(|| self.fixture.shortcuts.visual_text().as_deref() == text);
    }
}

/// A search started from a visual range ends the hidden directory's range;
/// returns the hits and the hidden directory's selection from before them.
fn search_from_a_range(fixture: &KeyboardFixture, mode: BrowserMode) -> (Hits<'_>, Vec<String>) {
    select_named(fixture, "beta.txt");
    let hidden = hidden_selection(fixture);
    assert_eq!(hidden, ["beta.txt"], "{mode:?}");
    assert!(fixture.press(Key::v, ModifierType::empty()));
    wait_until(|| fixture.shortcuts.visual_text().as_deref() == Some("VISUAL"));
    commit_search(fixture, "report");
    wait_results(fixture, &ALL_REPORTS);
    wait_until(|| fixture.shortcuts.filter_mark().as_deref() == Some("search: report"));
    assert_eq!(
        fixture.shortcuts.visual_text(),
        None,
        "{mode:?}: hits end the hidden directory's range"
    );
    let hits = Hits {
        fixture,
        mode,
        order: fixture.view.filter_result_names(),
    };
    hits.wait_cursor(0);
    (hits, hidden)
}

/// `G` and `g g` move between the ends; Home, End, Page keys and Ctrl+U are swallowed.
fn jump_keys_are_swallowed(hits: &Hits<'_>) {
    let fixture = hits.fixture;
    let mode = hits.mode;
    assert!(fixture.press(Key::G, ModifierType::SHIFT_MASK));
    hits.wait_cursor(3);
    hits.wait_footer("G", 3);
    for key in [Key::Home, Key::Page_Up] {
        hits.press(key);
        assert!(fixture.press(Key::u, ModifierType::CONTROL_MASK));
        pump(20);
        assert_eq!(
            hit_cursor(fixture),
            hits.at(3),
            "{mode:?} {key:?} is swallowed"
        );
    }
    hits.press(Key::g);
    hits.press(Key::g);
    hits.wait_cursor(0);
    hits.wait_footer("g g", 0);
    for key in [Key::End, Key::Page_Down] {
        hits.press(key);
        pump(20);
        assert_eq!(
            hit_cursor(fixture),
            hits.at(0),
            "{mode:?} {key:?} is swallowed"
        );
    }
}

/// Space fills hits without previewing; focus outside the hits keeps the cursor's hit.
fn space_fills_hits(hits: &Hits<'_>) {
    let fixture = hits.fixture;
    let mode = hits.mode;
    hits.press(Key::space);
    hits.wait_cursor(1);
    hits.wait_footer("first Space", 1);
    assert_eq!(selected_result_names(fixture), hits.names(&[0]));
    hits.press(Key::space);
    hits.wait_cursor(2);
    hits.wait_footer("second Space (fill)", 2);
    assert_eq!(selected_result_names(fixture), hits.names(&[0, 1]));
    assert!(
        !fixture.preview.is_enabled(),
        "{mode:?}: Space never previews"
    );
    pump(20);
    assert_eq!(
        fixture.shortcuts.count_text().0,
        "4 items",
        "{mode:?}: the footer keeps the hit total"
    );
    assert!(fixture.sidebar_toggle.grab_focus(), "{mode:?}");
    pump(50);
    assert_eq!(
        fixture.shortcuts.current_hit().map(|(path, _)| path),
        Some(hit_path(&hits.order[2])),
        "{mode:?}: focus outside the hits keeps the cursor's hit"
    );
    assert!(fixture.view.focus_results_cursor(), "{mode:?}");
    hits.wait_cursor(2);
}

/// Spatial moves keep the fill wherever the grid puts the cursor.
fn icons_spatial_moves_keep_the_fill(hits: &Hits<'_>) {
    let fixture = hits.fixture;
    hits.press(Key::h);
    pump(20);
    let cursor = hit_cursor(fixture).expect("cursor");
    wait_footer_hit(fixture, hits.mode, "h over the fill", &cursor);
    assert_eq!(selected_result_names(fixture), hits.names(&[0, 1]));
    hits.press(Key::v);
    hits.wait_visual(Some("VISUAL"));
    hits.press(Key::v);
    hits.wait_visual(None);
    assert!(fixture.view.listing_search_active(), "{:?}", hits.mode);
}

fn linear_motion_keeps_the_fill(hits: &Hits<'_>) {
    hits.press(Key::j);
    hits.wait_cursor(3);
    hits.wait_footer("j over the fill", 3);
    assert_eq!(selected_result_names(hits.fixture), hits.names(&[0, 1]));
    hits.press(Key::k);
    hits.wait_cursor(2);
    hits.wait_footer("k over the fill", 2);
}

/// `v` extends the fill into a range that follows the cursor; Escape keeps the range's
/// selection as a fill.
fn v_range_grows_and_shrinks(hits: &Hits<'_>) {
    let fixture = hits.fixture;
    hits.press(Key::v);
    hits.wait_visual(Some("VISUAL"));
    assert_eq!(selected_result_names(fixture), hits.names(&[0, 1, 2]));
    hits.press(Key::j);
    hits.wait_cursor(3);
    hits.wait_footer("j in a v range", 3);
    assert_eq!(selected_result_names(fixture), hits.names(&[0, 1, 2, 3]));
    hits.press(Key::k);
    hits.wait_cursor(2);
    hits.wait_footer("k in a v range", 2);
    assert_eq!(selected_result_names(fixture), hits.names(&[0, 1, 2]));

    hits.press(Key::Escape);
    hits.wait_visual(None);
    assert!(fixture.view.listing_search_active(), "{:?}", hits.mode);
    assert_eq!(selected_result_names(fixture), hits.names(&[0, 1, 2]));
}

/// `V` unsets along the cursor's path.
fn unset_range_removes_the_cursor(hits: &Hits<'_>) {
    let fixture = hits.fixture;
    hits.press(Key::k);
    hits.wait_cursor(1);
    hits.wait_footer("k after Escape", 1);
    assert!(fixture.press(Key::V, ModifierType::SHIFT_MASK));
    hits.wait_visual(Some("UNSET"));
    assert_eq!(selected_result_names(fixture), hits.names(&[0, 2]));
    assert!(fixture.press(Key::V, ModifierType::SHIFT_MASK));
    hits.wait_visual(None);
    assert_eq!(selected_result_names(fixture), hits.names(&[0, 2]));
}

/// Extending the selection to an unselected hit and pressing Space toggles the
/// cursor's own membership.
fn space_toggles_the_cursor_hit(hits: &Hits<'_>) {
    let fixture = hits.fixture;
    let cursor = hit_cursor(fixture).expect("a cursor hit");
    let mut expected = selected_result_names(fixture);
    let extra = hits
        .order
        .iter()
        .position(|name| *name != cursor && !expected.contains(name))
        .expect("an unselected hit");
    expected.push(hits.order[extra].clone());
    match expected.iter().position(|name| *name == cursor) {
        Some(at) => {
            expected.remove(at);
        }
        None => expected.push(cursor),
    }
    expected.sort();
    assert!(fixture.view.extend_result_selection(extra as u32));
    hits.press(Key::space);
    wait_until(|| selected_result_names(fixture) == expected);
    let cursor = hit_cursor(fixture).expect("cursor");
    wait_footer_hit(fixture, hits.mode, "final Space", &cursor);
}

fn escape_keeps_the_hidden_selection(hits: &Hits<'_>, hidden: &[String]) {
    let fixture = hits.fixture;
    let mode = hits.mode;
    assert_eq!(
        hidden_selection(fixture),
        hidden,
        "{mode:?}: hits never touch the hidden directory's fill"
    );
    assert_eq!(fixture.shortcuts.count_text().0, "4 items", "{mode:?}");

    hits.press(Key::Escape);
    wait_until(|| !fixture.view.listing_search_active());
    wait_until(|| fixture.view.selected_search_results().is_none());
    assert_eq!(hidden_selection(fixture), hidden, "{mode:?}");
}

#[test]
fn tenxer_search_hits_fill_and_range_apart_from_the_hidden_directory() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::search_hits::tenxer_search_hits_fill_and_range_apart_from_the_hidden_directory",
        || {
            let (fixture, _preferences) = tenxer_filter_fixture();
            let browser = fixture.view.browser();

            for mode in ALL_MODES {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                let (hits, hidden) = search_from_a_range(&fixture, mode);

                jump_keys_are_swallowed(&hits);
                space_fills_hits(&hits);
                if mode == BrowserMode::Icons {
                    icons_spatial_moves_keep_the_fill(&hits);
                } else {
                    linear_motion_keeps_the_fill(&hits);
                    v_range_grows_and_shrinks(&hits);
                    unset_range_removes_the_cursor(&hits);
                }
                space_toggles_the_cursor_hit(&hits);
                escape_keeps_the_hidden_selection(&hits, &hidden);
            }
        },
    );
}

fn window_focus(fixture: &KeyboardFixture) -> Option<gtk::Widget> {
    gtk::prelude::RootExt::focus(&fixture.window)
}

/// `i` on a hit folder opens it beside the hits (Columns) or peeks it; the
/// search and its cursor survive.
fn i_peeks_the_hit_folder(fixture: &KeyboardFixture, mode: BrowserMode, folder: &str) {
    let browser = fixture.view.browser();
    let none = ModifierType::empty();
    cursor_to_hit(fixture, "reports");
    assert!(fixture.press(Key::i, none));
    if mode == BrowserMode::Columns {
        wait_until(|| location_ends_with(browser.location_at(1), "reports"));
    } else {
        wait_until(|| fixture.view.widget().has_css_class("peek-open"));
        assert!(fixture.press(Key::i, none));
        wait_until(|| !fixture.view.widget().has_css_class("peek-open"));
    }
    assert_eq!(
        (
            fixture.view.listing_search_active(),
            fixture.view.item_view_has_focus(),
            hit_cursor(fixture).as_deref()
        ),
        (true, true, Some("reports")),
        "{mode:?}"
    );
    assert!(location_ends_with(browser.active_location(), folder));
}

/// `i` on a hit file toggles the preview without taking ownership of focus.
fn i_previews_a_file_without_ownership(fixture: &KeyboardFixture, mode: BrowserMode) {
    let none = ModifierType::empty();
    cursor_to_hit(fixture, "alpha-report.txt");
    assert!(fixture.press(Key::i, none));
    wait_until(|| fixture.preview.is_enabled());
    assert!(
        !fixture.preview.owns_focus(window_focus(fixture).as_ref()),
        "{mode:?}: i never takes preview ownership"
    );
    assert!(fixture.view.item_view_has_focus(), "{mode:?}");
    assert!(fixture.press(Key::i, none));
    wait_until(|| !fixture.preview.is_enabled());
}

/// `l` enters the preview, `h` returns to the same hit, and a second `h`
/// dismisses the hits.
fn l_and_h_keep_the_hits(fixture: &KeyboardFixture, mode: BrowserMode, folder: &str) {
    let browser = fixture.view.browser();
    let none = ModifierType::empty();
    assert!(fixture.press(Key::l, none));
    wait_until(|| fixture.preview.owns_focus(window_focus(fixture).as_ref()));
    assert!(fixture.press(Key::h, none));
    wait_until(|| fixture.view.item_view_has_focus());
    assert!(fixture.preview.is_open(), "{mode:?}: h keeps the drawer");
    assert_eq!(
        (
            fixture.view.listing_search_active(),
            hit_cursor(fixture).as_deref()
        ),
        (true, Some("alpha-report.txt")),
        "{mode:?}: h returns to the same hit"
    );
    assert!(fixture.press(Key::h, none));
    wait_until(|| !fixture.view.listing_search_active());
    assert!(location_ends_with(browser.active_location(), folder));
    fixture.preview.close();
    focus_files(fixture);
    commit_search(fixture, "report");
    wait_results(fixture, &ALL_REPORTS);
}

fn unknown_chord_keeps_the_hit(fixture: &KeyboardFixture, mode: BrowserMode) {
    let none = ModifierType::empty();
    wait_until(|| hit_cursor(fixture).is_some());
    let cursor = hit_cursor(fixture);
    assert!(fixture.press(Key::g, none));
    assert!(fixture.press(Key::j, none));
    assert_eq!(fixture.shortcuts.feedback_text(), "Unknown chord");
    fixture.shortcuts.dismiss_feedback();
    pump(20);
    assert_eq!(hit_cursor(fixture), cursor, "{mode:?}");
    assert!(fixture.view.listing_search_active(), "{mode:?}");
}

/// `g h` leaves for home, ending the search; the test returns to the fixture.
fn go_home_ends_the_search(fixture: &KeyboardFixture, mode: BrowserMode, folder: &str) {
    let browser = fixture.view.browser();
    let none = ModifierType::empty();
    assert!(fixture.press(Key::g, none));
    assert!(fixture.press(Key::h, none));
    let home = std::env::var_os("HOME").expect("isolated HOME");
    wait_until(|| {
        browser.active_location().is_some_and(|location| {
            location
                .native_path()
                .is_some_and(|path| path.as_os_str() == home)
        })
    });
    wait_loaded(&browser, 0);
    assert!(!fixture.view.listing_search_active(), "{mode:?}");
    browser.navigate(crate::model::Location::local(fixture._directory.path()));
    wait_until(|| location_ends_with(browser.active_location(), folder));
    wait_loaded(&browser, 0);
}

#[test]
fn tenxer_search_hit_keys_peek_preview_and_yield_to_chords() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::search_hits::tenxer_search_hit_keys_peek_preview_and_yield_to_chords",
        || {
            let (fixture, _preferences) = tenxer_filter_fixture();
            let browser = fixture.view.browser();
            let folder = fixture_name(&fixture).to_string();

            for mode in ALL_MODES {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                focus_files(&fixture);
                commit_search(&fixture, "report");
                wait_results(&fixture, &ALL_REPORTS);

                i_peeks_the_hit_folder(&fixture, mode, &folder);
                i_previews_a_file_without_ownership(&fixture, mode);
                if mode != BrowserMode::Icons {
                    l_and_h_keep_the_hits(&fixture, mode, &folder);
                }
                unknown_chord_keeps_the_hit(&fixture, mode);
                go_home_ends_the_search(&fixture, mode, &folder);
            }
        },
    );
}
