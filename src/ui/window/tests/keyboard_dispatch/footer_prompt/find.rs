// SPDX-License-Identifier: MIT

use super::*;

/// Sorted: a.txt, alpha-report.txt, b.txt, beta.txt, c.txt, delta.txt,
/// gamma-report.md. "report" matches the second and last entries.
fn seed_report_names(fixture: &KeyboardFixture) {
    for name in [
        "alpha-report.txt",
        "beta.txt",
        "delta.txt",
        "gamma-report.md",
    ] {
        std::fs::write(fixture._directory.path().join(name), b"find").expect("fixture file");
    }
    let browser = fixture.view.browser();
    fixture.view.refresh();
    wait_loaded(&browser, 0);
    wait_until(|| entry_count(&browser) == 7);
}

const REPORTS: [&str; 2] = ["alpha-report.txt", "gamma-report.md"];

/// Chords and the filter and search prompts open from a stranded focus.
fn chords_and_prompts_open_from(
    fixture: &KeyboardFixture,
    refocus: &impl Fn(Option<&gtk::Widget>),
    stranded: Option<&gtk::Widget>,
    mode: BrowserMode,
) {
    refocus(stranded);
    assert_eq!(
        gtk::prelude::RootExt::focus(&fixture.window).as_ref(),
        stranded
    );
    assert!(fixture.press(Key::g, ModifierType::empty()));
    assert_eq!(
        fixture.shortcuts.armed_chord(),
        Some(crate::ui::tenxer_mode::Chord::Go),
        "{mode:?}"
    );
    fixture.press(Key::Escape, ModifierType::empty());
    for (key, kind) in [(Key::f, Prompt::Filter), (Key::s, Prompt::Search)] {
        refocus(stranded);
        assert!(
            fixture.press(key, ModifierType::empty()),
            "{mode:?} {key:?}"
        );
        assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(kind));
        assert!(fixture.shortcuts.prompt_has_focus());
        fixture.press(Key::Escape, ModifierType::empty());
    }
}

/// The location field keeps its own keys even after the window regains focus.
fn location_field_keeps_f(fixture: &KeyboardFixture, other: &gtk::Window) {
    fixture.press(Key::l, ModifierType::CONTROL_MASK);
    assert!(fixture.view.location_has_focus());
    other.present();
    wait_until(|| other.is_active());
    fixture.window.present();
    wait_until(|| fixture.window.is_active());
    fixture.press(Key::f, ModifierType::empty());
    assert!(fixture.view.location_has_focus());
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    fixture.press(Key::Escape, ModifierType::empty());
}

#[test]
fn tenxer_routes_listing_shortcuts_from_non_text_controls() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::find::tenxer_routes_listing_shortcuts_from_non_text_controls",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let container = fixture.overlay.child().expect("content container");
            container.set_focusable(true);
            let other = gtk::Window::new();
            let refocus = |stranded: Option<&gtk::Widget>| {
                other.present();
                wait_until(|| other.is_active() && !fixture.window.is_active());
                fixture.window.set_default_size(820, 540);
                gtk::prelude::RootExt::set_focus(&fixture.window, stranded);
                fixture.window.present();
                wait_until(|| fixture.window.is_active() && !other.is_active());
            };
            for mode in ALL_MODES {
                fixture.view.set_view_mode(mode);
                focus_files(&fixture);
                for stranded in [
                    None,
                    Some(&container),
                    Some(fixture.sidebar_toggle.upcast_ref()),
                ] {
                    chords_and_prompts_open_from(&fixture, &refocus, stranded, mode);
                }
                location_field_keeps_f(&fixture, &other);
            }
            other.destroy();
        },
    );
}

fn assert_find_prompt_open(
    fixture: &KeyboardFixture,
    kind: Prompt,
    label: &str,
    mode: BrowserMode,
) {
    assert_eq!(
        (
            fixture.shortcuts.open_prompt_kind(),
            fixture.shortcuts.prompt_label().as_deref()
        ),
        (Some(kind), Some(label)),
        "{mode:?}"
    );
    assert!(fixture.shortcuts.prompt_has_focus(), "{mode:?}");
    assert!(fixture.shortcuts.prompt().text().is_empty());
}

fn escape_returns_to_listing(fixture: &KeyboardFixture, mode: BrowserMode) {
    assert!(fixture.press(Key::Escape, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.prompt_label(), None);
    assert!(
        fixture.view.item_view_has_focus(),
        "{mode:?}: Escape returns focus to the listing"
    );
}

/// Leaving 10xer mode discards a pending prompt and returns focus to the listing.
fn assert_prompt_discarded(fixture: &KeyboardFixture) {
    assert_eq!(
        (
            fixture.shortcuts.open_prompt_kind(),
            fixture.shortcuts.prompt_label()
        ),
        (None, None)
    );
    assert!(fixture.shortcuts.prompt().text().is_empty());
    assert!(fixture.view.item_view_has_focus());
}

#[test]
fn tenxer_slash_covers_the_footer_with_a_focused_find_prompt() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::find::tenxer_slash_covers_the_footer_with_a_focused_find_prompt",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            let browser = fixture.view.browser();
            fixture.shortcuts.bind_preferences(&preferences);

            focus_files(&fixture);
            fixture.press(Key::slash, ModifierType::empty());
            assert_eq!(
                fixture.shortcuts.open_prompt_kind(),
                None,
                "outside 10xer mode / keeps its type-to-search meaning"
            );

            preferences.set_tenxer_mode(true);
            pump(50);
            for (key, modifiers, kind, label) in [
                (Key::slash, ModifierType::empty(), Prompt::Find, "/"),
                (
                    Key::question,
                    ModifierType::SHIFT_MASK,
                    Prompt::FindBackward,
                    "?",
                ),
            ] {
                for mode in ALL_MODES {
                    fixture.view.set_view_mode(mode);
                    focus_files(&fixture);
                    let focused = focused_name(&browser);
                    assert!(fixture.press(key, modifiers));
                    assert_find_prompt_open(&fixture, kind, label, mode);

                    fixture.press(Key::j, ModifierType::empty());
                    assert_eq!(
                        focused_name(&browser),
                        focused,
                        "{mode:?}: typing stays in the prompt"
                    );
                    escape_returns_to_listing(&fixture, mode);
                }
            }

            assert!(fixture.press(Key::slash, ModifierType::SHIFT_MASK));
            fixture.shortcuts.prompt().set_text("secret");
            preferences.set_tenxer_mode(false);
            pump(50);
            assert_prompt_discarded(&fixture);
        },
    );
}

#[test]
fn a_prompt_submits_from_its_activate_signal() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::find::a_prompt_submits_from_its_activate_signal",
        || {
            let fixture = KeyboardFixture::new();
            seed_report_names(&fixture);
            let _preferences = enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            move_to_named(&fixture, &browser, "a.txt");

            assert!(fixture.press(Key::slash, ModifierType::empty()));
            fixture.shortcuts.prompt().set_text("REPORT");
            fixture.shortcuts.prompt().emit_activate();
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            assert_eq!(focused_name(&browser), "alpha-report.txt");
        },
    );
}

/// `/` finds the first match; `n` and `N` step between the matches, also after
/// a reload.
fn find_steps_between_matches(fixture: &KeyboardFixture, mode: BrowserMode) {
    let browser = fixture.view.browser();
    let none = ModifierType::empty();
    move_to_named(fixture, &browser, "a.txt");

    type_and_submit(fixture, Key::slash, "REPORT");
    assert_eq!(focused_name(&browser), "alpha-report.txt", "{mode:?}");
    assert!(fixture.view.item_view_has_focus(), "{mode:?}");
    wait_highlights(fixture, &REPORTS);
    assert_eq!(entry_count(&browser), 7, "{mode:?}: find hides no rows");

    for (key, modifiers, expected) in [
        (Key::n, none, "gamma-report.md"),
        (Key::n, none, "alpha-report.txt"),
        (Key::N, ModifierType::SHIFT_MASK, "gamma-report.md"),
    ] {
        assert!(fixture.press(key, modifiers));
        assert_eq!(focused_name(&browser), expected, "{mode:?} {key:?}");
    }

    browser.reload_active();
    wait_loaded(&browser, 0);
    pump(100);
    wait_highlights(fixture, &REPORTS);
    assert!(fixture.press(Key::n, none));
    assert!(
        REPORTS.contains(&focused_name(&browser).as_str()),
        "{mode:?}: n after reload"
    );
    move_to_named(fixture, &browser, "gamma-report.md");
}

/// `?` searches backward and sets the `n` direction; a miss and an empty
/// query leave the cursor alone.
fn find_backward_and_misses(fixture: &KeyboardFixture, mode: BrowserMode) {
    let browser = fixture.view.browser();
    type_and_submit(fixture, Key::question, "report");
    assert_eq!(
        focused_name(&browser),
        "alpha-report.txt",
        "{mode:?}: ? searches backward"
    );
    assert!(fixture.press(Key::n, ModifierType::empty()));
    assert_eq!(
        focused_name(&browser),
        "gamma-report.md",
        "{mode:?}: n keeps the ? direction"
    );

    type_and_submit(fixture, Key::slash, "zzz");
    assert_eq!(
        focused_name(&browser),
        "gamma-report.md",
        "{mode:?}: a miss does not navigate"
    );
    assert_eq!(
        fixture.shortcuts.feedback_text(),
        "No matches for \u{201c}zzz\u{201d}"
    );
    assert_eq!(entry_count(&browser), 7);

    type_and_submit(fixture, Key::slash, "report");
    type_and_submit(fixture, Key::slash, "");
    assert_eq!(
        focused_name(&browser),
        "alpha-report.txt",
        "{mode:?}: empty Enter does nothing"
    );
    wait_highlights(fixture, &REPORTS);
}

/// Escape in the listing clears the highlights first; `n` brings them back.
fn listing_escape_dismisses_highlights(fixture: &KeyboardFixture, mode: BrowserMode) {
    let browser = fixture.view.browser();
    let none = ModifierType::empty();
    assert!(fixture.press(Key::Escape, none));
    wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());
    assert_eq!(
        focused_name(&browser),
        "alpha-report.txt",
        "{mode:?}: listing Escape dismisses highlights before the selection"
    );
    assert!(fixture.press(Key::n, none));
    assert_eq!(focused_name(&browser), "gamma-report.md");
    wait_highlights(fixture, &REPORTS);

    assert!(fixture.press(Key::slash, none));
    assert!(fixture.press(Key::Escape, none));
    wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());
    assert!(fixture.view.item_view_has_focus());
}

#[test]
fn tenxer_find_moves_between_matches_and_keeps_rows_visible() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::find::tenxer_find_moves_between_matches_and_keeps_rows_visible",
        || {
            let fixture = KeyboardFixture::new();
            seed_report_names(&fixture);
            let preferences = enable_tenxer(&fixture);

            for mode in ALL_MODES {
                fixture.view.set_view_mode(mode);
                find_steps_between_matches(&fixture, mode);
                find_backward_and_misses(&fixture, mode);
                listing_escape_dismisses_highlights(&fixture, mode);
            }

            type_and_submit(&fixture, Key::slash, "report");
            wait_until(|| !highlighted_names(&fixture.view.widget()).is_empty());
            preferences.set_tenxer_mode(false);
            pump(50);
            wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());
            preferences.set_tenxer_mode(true);
            pump(50);
            assert!(fixture.press(Key::n, ModifierType::empty()));
            assert_eq!(
                fixture.shortcuts.feedback_text(),
                "No previous find",
                "leaving the mode forgets the query"
            );
        },
    );
}

fn assert_focus_loss_closed_prompt(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    assert!(fixture.shortcuts.prompt().text().is_empty());
    assert_eq!(
        focused_name(&browser),
        "b.txt",
        "focus loss keeps the cursor"
    );
    assert!(fixture.view.item_view_has_focus());
}

#[test]
fn tenxer_find_prompt_steers_the_listing_and_closes_on_focus_loss() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::find::tenxer_find_prompt_steers_the_listing_and_closes_on_focus_loss",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            move_to_named(&fixture, &browser, "a.txt");

            assert!(fixture.press(Key::slash, none));
            fixture.shortcuts.prompt().set_text("b");
            for (key, expected) in [
                (Key::Down, "b.txt"),
                (Key::Down, "c.txt"),
                (Key::Up, "b.txt"),
            ] {
                assert!(fixture.press(key, none));
                pump(20);
                assert_eq!(focused_name(&browser), expected, "{key:?}");
                assert!(
                    fixture.shortcuts.prompt_has_focus(),
                    "{key:?} leaves the prompt focused"
                );
                assert_eq!(fixture.shortcuts.prompt().text(), "b");
            }

            let names = directory_names(fixture._directory.path());
            browser.focus_active();
            wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
            assert_focus_loss_closed_prompt(&fixture);
            assert_eq!(directory_names(fixture._directory.path()), names);
            assert!(fixture.press(Key::n, none));
            assert_eq!(
                fixture.shortcuts.feedback_text(),
                "No previous find",
                "an abandoned prompt commits nothing"
            );
        },
    );
}

#[test]
fn leaving_tenxer_mode_clears_find_in_every_window() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::find::leaving_tenxer_mode_clears_find_in_every_window",
        || {
            let first = KeyboardFixture::new();
            let second = KeyboardFixture::new();
            let preferences = enable_tenxer(&first);
            second.shortcuts.bind_preferences(&preferences);
            for fixture in [&first, &second] {
                focus_files(fixture);
                type_and_submit(fixture, Key::slash, "b");
                wait_highlights(fixture, &["b.txt"]);
            }
            focus_files(&second);
            assert!(second.press(Key::question, ModifierType::SHIFT_MASK));
            second.shortcuts.prompt().set_text("draft");

            preferences.set_tenxer_mode(false);
            pump(50);
            for fixture in [&first, &second] {
                wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                assert!(fixture.shortcuts.prompt().text().is_empty());
                assert_eq!(fixture.view.find_query(), None);
            }
        },
    );
}
