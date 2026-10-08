// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::tenxer_mode::Prompt;

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

fn highlighted_labels(widget: &gtk::Widget) -> Vec<(String, gtk::pango::AttrList)> {
    fn collect(widget: &gtk::Widget, labels: &mut Vec<(String, gtk::pango::AttrList)>) {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && let Some(attributes) = label.attributes()
        {
            labels.push((label.text().to_string(), attributes));
        }
        if let Some(label) = widget.downcast_ref::<gtk::Inscription>()
            && let Some(attributes) = label.attributes()
        {
            labels.push((label.text().unwrap_or_default().to_string(), attributes));
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            collect(&current, labels);
            child = current.next_sibling();
        }
    }
    let mut labels = Vec::new();
    collect(widget, &mut labels);
    labels
}

pub(super) fn highlighted_names(widget: &gtk::Widget) -> Vec<String> {
    let mut names: Vec<_> = highlighted_labels(widget)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names.dedup();
    names
}

fn highlight_colors(widget: &gtk::Widget) -> Vec<String> {
    highlighted_labels(widget)
        .into_iter()
        .flat_map(|(_, attributes)| attributes.attributes())
        .filter_map(|attribute| {
            let color = attribute.downcast_ref::<gtk::pango::AttrColor>()?;
            Some(color.color().to_str().to_string())
        })
        .collect()
}

pub(super) fn type_and_submit(fixture: &KeyboardFixture, prompt: Key, text: &str) {
    assert!(fixture.press(prompt, ModifierType::empty()));
    assert!(fixture.shortcuts.prompt_has_focus());
    fixture.shortcuts.prompt().set_text(text);
    assert!(fixture.press(Key::Return, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
}

pub(super) fn enable_tenxer(fixture: &KeyboardFixture) -> Rc<PreferenceManager> {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_tenxer_mode(true);
    pump(50);
    preferences
}

#[test]
fn tenxer_routes_listing_shortcuts_from_non_text_controls() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_routes_listing_shortcuts_from_non_text_controls",
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
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                focus_files(&fixture);
                for stranded in [
                    None,
                    Some(&container),
                    Some(fixture.sidebar_toggle.upcast_ref()),
                ] {
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
            other.destroy();
        },
    );
}

#[test]
fn tenxer_slash_covers_the_footer_with_a_focused_find_prompt() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_slash_covers_the_footer_with_a_focused_find_prompt",
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
                for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                    fixture.view.set_view_mode(mode);
                    focus_files(&fixture);
                    let focused = focused_name(&browser);
                    assert!(fixture.press(key, modifiers));
                    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(kind));
                    assert_eq!(
                        fixture.shortcuts.prompt_label().as_deref(),
                        Some(label),
                        "{mode:?}"
                    );
                    assert!(fixture.shortcuts.prompt_has_focus(), "{mode:?}");
                    assert!(fixture.shortcuts.prompt().text().is_empty());

                    fixture.press(Key::j, ModifierType::empty());
                    assert_eq!(
                        focused_name(&browser),
                        focused,
                        "{mode:?}: typing stays in the prompt"
                    );
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    assert_eq!(fixture.shortcuts.prompt_label(), None);
                    assert!(
                        fixture.view.item_view_has_focus(),
                        "{mode:?}: Escape returns focus to the listing"
                    );
                }
            }

            assert!(fixture.press(Key::slash, ModifierType::SHIFT_MASK));
            fixture.shortcuts.prompt().set_text("secret");
            preferences.set_tenxer_mode(false);
            pump(50);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            assert_eq!(fixture.shortcuts.prompt_label(), None);
            assert!(fixture.shortcuts.prompt().text().is_empty());
            assert!(fixture.view.item_view_has_focus());
        },
    );
}

#[test]
fn tenxer_find_moves_between_matches_and_keeps_rows_visible() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_find_moves_between_matches_and_keeps_rows_visible",
        || {
            let fixture = KeyboardFixture::new();
            seed_report_names(&fixture);
            let preferences = enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            let reports = vec!["alpha-report.txt".to_owned(), "gamma-report.md".to_owned()];

            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                move_to_named(&fixture, &browser, "a.txt");

                type_and_submit(&fixture, Key::slash, "REPORT");
                assert_eq!(focused_name(&browser), "alpha-report.txt", "{mode:?}");
                assert!(fixture.view.item_view_has_focus(), "{mode:?}");
                wait_until(|| highlighted_names(&fixture.view.widget()) == reports);
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
                wait_until(|| highlighted_names(&fixture.view.widget()) == reports);
                assert!(fixture.press(Key::n, none));
                assert!(
                    reports.contains(&focused_name(&browser)),
                    "{mode:?}: n after reload"
                );
                move_to_named(&fixture, &browser, "gamma-report.md");

                type_and_submit(&fixture, Key::question, "report");
                assert_eq!(
                    focused_name(&browser),
                    "alpha-report.txt",
                    "{mode:?}: ? searches backward"
                );
                assert!(fixture.press(Key::n, none));
                assert_eq!(
                    focused_name(&browser),
                    "gamma-report.md",
                    "{mode:?}: n keeps the ? direction"
                );

                type_and_submit(&fixture, Key::slash, "zzz");
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

                type_and_submit(&fixture, Key::slash, "report");
                type_and_submit(&fixture, Key::slash, "");
                assert_eq!(
                    focused_name(&browser),
                    "alpha-report.txt",
                    "{mode:?}: empty Enter does nothing"
                );
                wait_until(|| highlighted_names(&fixture.view.widget()) == reports);

                assert!(fixture.press(Key::Escape, none));
                wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());
                assert_eq!(
                    focused_name(&browser),
                    "alpha-report.txt",
                    "{mode:?}: listing Escape dismisses highlights before the selection"
                );
                assert!(fixture.press(Key::n, none));
                assert_eq!(focused_name(&browser), "gamma-report.md");
                wait_until(|| highlighted_names(&fixture.view.widget()) == reports);

                assert!(fixture.press(Key::slash, none));
                assert!(fixture.press(Key::Escape, none));
                wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());
                assert!(fixture.view.item_view_has_focus());
            }

            type_and_submit(&fixture, Key::slash, "report");
            wait_until(|| !highlighted_names(&fixture.view.widget()).is_empty());
            preferences.set_tenxer_mode(false);
            pump(50);
            wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());
            preferences.set_tenxer_mode(true);
            pump(50);
            assert!(fixture.press(Key::n, none));
            assert_eq!(
                fixture.shortcuts.feedback_text(),
                "No previous find",
                "leaving the mode forgets the query"
            );
        },
    );
}

#[test]
fn tenxer_find_prompt_steers_the_listing_and_closes_on_focus_loss() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_find_prompt_steers_the_listing_and_closes_on_focus_loss",
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
            assert!(fixture.shortcuts.prompt().text().is_empty());
            assert_eq!(
                focused_name(&browser),
                "b.txt",
                "focus loss keeps the cursor"
            );
            assert!(fixture.view.item_view_has_focus());
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
        "ui::window::tests::keyboard_dispatch::footer_prompt::leaving_tenxer_mode_clears_find_in_every_window",
        || {
            let first = KeyboardFixture::new();
            let second = KeyboardFixture::new();
            let preferences = enable_tenxer(&first);
            second.shortcuts.bind_preferences(&preferences);
            for fixture in [&first, &second] {
                focus_files(fixture);
                type_and_submit(fixture, Key::slash, "b");
                wait_until(|| highlighted_names(&fixture.view.widget()) == ["b.txt"]);
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

/// Adds alpha-report.txt, beta.txt, gamma-report.md, and a reports folder
/// holding deep-report.txt. "report" names two files and the folder here, and
/// the nested file only with Include subfolders on.
pub(super) fn seed_filter_tree(fixture: &KeyboardFixture) {
    let root = fixture._directory.path();
    for name in ["alpha-report.txt", "beta.txt", "gamma-report.md"] {
        std::fs::write(root.join(name), b"filter").expect("fixture file");
    }
    std::fs::create_dir(root.join("reports")).expect("fixture folder");
    std::fs::write(root.join("reports/deep-report.txt"), b"filter").expect("nested file");
    let browser = fixture.view.browser();
    fixture.view.refresh();
    wait_loaded(&browser, 0);
    wait_until(|| entry_count(&browser) == 7);
    fixture.shortcuts.observe_browser(&browser);
}

pub(super) const IMMEDIATE_REPORTS: [&str; 3] = ["alpha-report.txt", "gamma-report.md", "reports"];
pub(super) const ALL_REPORTS: [&str; 4] = [
    "alpha-report.txt",
    "deep-report.txt",
    "gamma-report.md",
    "reports",
];

fn result_names(fixture: &KeyboardFixture) -> Vec<String> {
    let mut names = fixture.view.filter_result_names();
    names.sort();
    names
}

pub(super) fn wait_results(fixture: &KeyboardFixture, expected: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while result_names(fixture) != expected {
        assert!(
            Instant::now() < deadline,
            "filter results {:?} never became {expected:?}",
            result_names(fixture)
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(super) fn selected_result_names(fixture: &KeyboardFixture) -> Vec<String> {
    let mut names: Vec<_> = fixture
        .view
        .selected_search_results()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect();
    names.sort();
    names
}

fn revealed_filter_funnels(widget: &gtk::Widget) -> usize {
    let own = widget
        .downcast_ref::<gtk::Revealer>()
        .is_some_and(|revealer| {
            revealer.has_css_class("tenxer-filter-revealer") && revealer.reveals_child()
        });
    let mut count = usize::from(own);
    let mut child = widget.first_child();
    while let Some(current) = child {
        count += revealed_filter_funnels(&current);
        child = current.next_sibling();
    }
    count
}

pub(super) fn commit_filter(fixture: &KeyboardFixture, text: &str) {
    type_and_submit(fixture, Key::f, text);
    wait_until(|| fixture.view.item_view_has_focus());
}

#[test]
fn tenxer_filter_commits_results_without_touching_the_hidden_directory() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_filter_commits_results_without_touching_the_hidden_directory",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let preferences = enable_tenxer(&fixture);
            preferences.set_filter_include_subfolders(false);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();

            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                select_named(&fixture, "beta.txt");
                let cursor = browser
                    .focused_item()
                    .map(|(depth, position, _)| (depth, position));
                let directory_count = fixture.shortcuts.count_text();

                assert!(fixture.press(Key::f, none), "{mode:?}");
                assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Filter));
                assert_eq!(fixture.shortcuts.prompt_label().as_deref(), Some("filter:"));
                fixture.shortcuts.prompt().set_text("report");
                wait_results(&fixture, &IMMEDIATE_REPORTS);
                wait_until(|| highlighted_names(&fixture.view.widget()) == IMMEDIATE_REPORTS);
                assert!(
                    fixture.shortcuts.prompt_has_focus(),
                    "{mode:?}: typing filters live and stays in the prompt"
                );

                assert!(fixture.press(Key::Return, none));
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                wait_until(|| {
                    fixture.view.item_view_has_focus()
                        && fixture.view.selected_search_result().is_some()
                        && fixture.shortcuts.filter_mark().as_deref() == Some("filter: report")
                });
                wait_until(|| {
                    fixture.shortcuts.count_text() == ("3 items".to_owned(), String::new())
                });
                assert_eq!(revealed_filter_funnels(&fixture.view.widget()), 0);
                assert_eq!(fill_names(&browser), ["beta.txt"], "{mode:?}");

                let first = selected_result_names(&fixture);
                assert!(fixture.press(Key::j, none), "{mode:?}");
                pump(40);
                let second = selected_result_names(&fixture);
                assert_eq!(second.len(), 1, "{mode:?}");
                assert_ne!(first, second, "{mode:?}: j moves among the results");
                assert!(fixture.press(Key::r, ModifierType::CONTROL_MASK));
                pump(20);
                let inverted = selected_result_names(&fixture);
                assert_eq!(inverted.len(), 2, "{mode:?}: Ctrl+R inverts the results");
                assert!(!inverted.contains(&second[0]), "{mode:?}");
                assert!(fixture.press(Key::j, none));
                assert_eq!(
                    selected_result_names(&fixture),
                    inverted,
                    "{mode:?}: motion preserves the inverted result selection"
                );
                assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
                pump(20);
                let all = selected_result_names(&fixture);
                assert_eq!(all.len(), 3, "{mode:?}: Ctrl+A selects the results");
                assert!(fixture.press(Key::k, none));
                assert_eq!(
                    selected_result_names(&fixture),
                    all,
                    "{mode:?}: motion preserves select all"
                );
                assert_eq!(
                    fill_names(&browser),
                    ["beta.txt"],
                    "{mode:?}: the hidden directory fill is untouched"
                );
                assert_eq!(
                    browser
                        .focused_item()
                        .map(|(depth, position, _)| (depth, position)),
                    cursor,
                    "{mode:?}: the hidden directory cursor is untouched"
                );

                assert!(fixture.press(Key::f, none));
                assert_eq!(
                    fixture.shortcuts.prompt().text(),
                    "report",
                    "{mode:?}: f pre-fills the committed query"
                );
                assert!(fixture.press(Key::Escape, none));
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                wait_until(|| fixture.view.selected_search_results().is_none());
                assert_eq!(
                    fixture.view.listing_filter(),
                    None,
                    "{mode:?}: prompt Esc clears"
                );
                wait_until(|| fixture.shortcuts.filter_mark().is_none());
                wait_until(|| fixture.shortcuts.count_text() == directory_count);
                assert!(fixture.view.item_view_has_focus(), "{mode:?}");

                commit_filter(&fixture, "report");
                assert!(fixture.press(Key::Escape, none));
                wait_until(|| fixture.view.selected_search_results().is_none());
                assert_eq!(
                    fixture.view.listing_filter(),
                    None,
                    "{mode:?}: listing Esc clears"
                );
                assert_eq!(fill_names(&browser), ["beta.txt"], "{mode:?}");

                commit_filter(&fixture, "report");
                commit_filter(&fixture, "");
                wait_until(|| fixture.view.selected_search_results().is_none());
                assert_eq!(
                    fixture.view.listing_filter(),
                    None,
                    "{mode:?}: empty Enter clears"
                );

                commit_filter(&fixture, "zzz");
                wait_until(|| fixture.shortcuts.count_text().0 == "0 items");
                assert_eq!(
                    fixture.shortcuts.filter_mark().as_deref(),
                    Some("filter: zzz")
                );
                assert!(
                    fixture.view.item_view_has_focus(),
                    "{mode:?}: zero results keep focus"
                );
                assert!(fixture.press(Key::Escape, none));
                wait_until(|| fixture.view.listing_filter().is_none());

                commit_filter(&fixture, "md rep");
                wait_results(&fixture, &["gamma-report.md"]);
                wait_until(|| highlighted_names(&fixture.view.widget()) == ["gamma-report.md"]);
                assert!(fixture.press(Key::Escape, none));
                wait_until(|| fixture.view.listing_filter().is_none());
                wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());

                commit_filter(&fixture, "reports");
                wait_until(|| {
                    fixture
                        .view
                        .selected_search_result()
                        .is_some_and(|entry| entry.display_name == "reports")
                });
                assert!(
                    location_ends_with(browser.active_location(), fixture_name(&fixture)),
                    "{mode:?}: committing opens nothing"
                );
                assert!(fixture.press(Key::Return, none));
                wait_until(|| location_ends_with(browser.active_location(), "reports"));
                browser.back();
                wait_until(|| {
                    location_ends_with(browser.active_location(), fixture_name(&fixture))
                });
                wait_loaded(&browser, 0);
                fixture.view.clear_listing_filter();
            }
        },
    );
}

fn fixture_name(fixture: &KeyboardFixture) -> &str {
    fixture
        ._directory
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("fixture name")
}

#[test]
fn tenxer_filter_ignores_include_subfolders_and_survives_view_rebuilds() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_filter_ignores_include_subfolders_and_survives_view_rebuilds",
        || {
            let first = KeyboardFixture::new();
            let second = KeyboardFixture::new();
            let preferences = enable_tenxer(&first);
            second.shortcuts.bind_preferences(&preferences);
            preferences.set_filter_include_subfolders(true);
            for fixture in [&first, &second] {
                seed_filter_tree(fixture);
                focus_files(fixture);
                commit_filter(fixture, "report");
                wait_results(fixture, &IMMEDIATE_REPORTS);
            }

            for include_subfolders in [false, true] {
                preferences.set_filter_include_subfolders(include_subfolders);
                pump(200);
                for fixture in [&first, &second] {
                    wait_results(fixture, &IMMEDIATE_REPORTS);
                    wait_until(|| fixture.shortcuts.count_text().0 == "3 items");
                }
            }

            assert!(first.press(Key::f, ModifierType::empty()));
            first.shortcuts.prompt().set_text("gamma");
            assert!(first.press(Key::Return, ModifierType::empty()));
            for (mode, accent, drawn) in [
                (BrowserMode::List, "#aa0000", "#aaaa00000000"),
                (BrowserMode::Icons, "#00aa00", "#0000aaaa0000"),
                (BrowserMode::Columns, "#0000aa", "#00000000aaaa"),
            ] {
                first.view.set_view_mode(mode);
                wait_results(&first, &["gamma-report.md"]);
                assert_eq!(
                    first.view.listing_filter().as_deref(),
                    Some("gamma"),
                    "{mode:?}"
                );
                crate::ui::browser::find::apply_theme(accent, "#000000");
                wait_until(|| highlight_colors(&first.view.widget()).contains(&drawn.to_owned()));
                wait_until(|| first.shortcuts.filter_mark().as_deref() == Some("filter: gamma"));
                assert_eq!(
                    revealed_filter_funnels(&first.view.widget()),
                    0,
                    "{mode:?}: a rebuild keeps the funnel closed"
                );

                let browser = first.view.browser();
                browser.reload_active();
                wait_loaded(&browser, 0);
                pump(200);
                wait_results(&first, &["gamma-report.md"]);
                assert_eq!(
                    first.view.listing_filter().as_deref(),
                    Some("gamma"),
                    "{mode:?} after reload"
                );
            }

            preferences.set_tenxer_mode(false);
            pump(50);
            for fixture in [&first, &second] {
                wait_until(|| fixture.view.listing_filter().is_none());
                wait_until(|| fixture.view.selected_search_results().is_none());
                assert_eq!(fixture.shortcuts.filter_mark(), None);
                assert_eq!(revealed_filter_funnels(&fixture.view.widget()), 0);
            }
        },
    );
}

fn type_search(fixture: &KeyboardFixture, text: &str) {
    assert!(fixture.press(Key::s, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Search));
    fixture.shortcuts.prompt().set_text(text);
}

pub(super) fn commit_search(fixture: &KeyboardFixture, text: &str) {
    type_search(fixture, text);
    assert!(fixture.press(Key::Return, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    wait_until(|| fixture.view.item_view_has_focus());
}

fn wait_filter_restored(fixture: &KeyboardFixture, query: &str, results: &[&str]) {
    wait_results(fixture, results);
    wait_until(|| !fixture.view.listing_search_active());
    assert_eq!(fixture.view.listing_filter().as_deref(), Some(query));
    wait_until(|| fixture.shortcuts.filter_mark() == Some(format!("filter: {query}")));
    assert_eq!(
        fixture.shortcuts.current_hit(),
        None,
        "filters show no hit path"
    );
}

#[test]
fn tenxer_search_covers_the_current_tree_and_restores_the_filter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_search_covers_the_current_tree_and_restores_the_filter",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let preferences = enable_tenxer(&fixture);
            preferences.set_filter_include_subfolders(false);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();

            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                // A mistaken activation of the hidden cursor would open this folder.
                select_named(&fixture, "reports");
                commit_filter(&fixture, "gamma");
                wait_results(&fixture, &["gamma-report.md"]);

                type_search(&fixture, "report");
                assert_eq!(fixture.shortcuts.prompt_label().as_deref(), Some("search:"));
                wait_results(&fixture, &ALL_REPORTS);
                assert!(
                    fixture.shortcuts.prompt_has_focus(),
                    "{mode:?}: typing searches live and stays in the prompt"
                );
                assert!(
                    !preferences.filter_include_subfolders(),
                    "{mode:?}: the saved scope is untouched"
                );
                assert!(fixture.press(Key::Return, none));
                wait_until(|| {
                    fixture.view.item_view_has_focus()
                        && fixture.view.selected_search_result().is_some()
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
                    let path = if name == "deep-report.txt" {
                        "reports/deep-report.txt".to_string()
                    } else {
                        name
                    };
                    wait_until(|| {
                        fixture.shortcuts.current_hit() == Some((path.clone(), path.clone()))
                    });
                    assert!(fixture.press(Key::j, none));
                }
                assert!(
                    location_ends_with(browser.active_location(), fixture_name(&fixture)),
                    "{mode:?}: applying the search opens nothing"
                );
                wait_until(|| highlighted_names(&fixture.view.widget()) == ALL_REPORTS);

                type_and_submit(&fixture, Key::slash, "deep-report");
                assert_eq!(
                    fixture
                        .view
                        .selected_search_result()
                        .expect("find focuses a visible hit")
                        .display_name,
                    "deep-report.txt",
                    "{mode:?}: find searches visible hits, not the hidden directory"
                );
                wait_until(|| highlighted_names(&fixture.view.widget()) == ["deep-report.txt"]);
                assert!(fixture.press(Key::Escape, none));
                wait_until(|| highlighted_names(&fixture.view.widget()) == ALL_REPORTS);

                let selected = selected_result_names(&fixture);
                assert!(fixture.press(Key::r, ModifierType::CONTROL_MASK));
                pump(20);
                assert_eq!(
                    selected_result_names(&fixture),
                    selected,
                    "{mode:?}: Ctrl+R is inactive for search hits"
                );

                assert!(fixture.press(Key::s, none));
                assert_eq!(
                    fixture.shortcuts.prompt().text(),
                    "report",
                    "{mode:?}: s pre-fills the showing search"
                );
                assert!(fixture.press(Key::Escape, none));
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                wait_until(|| fixture.view.item_view_has_focus());
                assert_eq!(
                    result_names(&fixture),
                    ALL_REPORTS,
                    "{mode:?}: prompt Esc keeps hits"
                );
                assert!(fixture.view.listing_search_active());

                assert!(fixture.press(Key::Escape, none));
                wait_filter_restored(&fixture, "gamma", &["gamma-report.md"]);
                assert!(fixture.view.item_view_has_focus(), "{mode:?}");

                type_search(&fixture, "report");
                wait_results(&fixture, &ALL_REPORTS);
                fixture.shortcuts.prompt().set_text("");
                assert!(fixture.press(Key::Escape, none));
                wait_filter_restored(&fixture, "gamma", &["gamma-report.md"]);

                type_search(&fixture, "deep reports");
                wait_results(&fixture, &["deep-report.txt"]);
                wait_until(|| highlighted_names(&fixture.view.widget()) == ["deep-report.txt"]);
                assert!(fixture.press(Key::Escape, none));
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(fixture.press(Key::Escape, none));
                wait_filter_restored(&fixture, "gamma", &["gamma-report.md"]);

                commit_search(&fixture, "zzz");
                wait_until(|| {
                    fixture.shortcuts.filter_mark().as_deref() == Some("search: zzz")
                        && fixture.shortcuts.count_text().0 == "0 items"
                });
                assert!(fixture.press(Key::Return, none));
                pump(100);
                assert!(
                    location_ends_with(browser.active_location(), fixture_name(&fixture)),
                    "{mode:?}: Enter with no hits opens no hidden item"
                );
                assert!(fixture.press(Key::Escape, none));
                wait_filter_restored(&fixture, "gamma", &["gamma-report.md"]);

                if mode != BrowserMode::Icons {
                    commit_search(&fixture, "report");
                    wait_results(&fixture, &ALL_REPORTS);
                    assert!(fixture.press(Key::h, none));
                    wait_filter_restored(&fixture, "gamma", &["gamma-report.md"]);
                    assert!(
                        location_ends_with(browser.active_location(), fixture_name(&fixture)),
                        "{mode:?}: h dismisses the hits rather than leaving the folder"
                    );
                }

                commit_search(&fixture, "reports");
                wait_until(|| {
                    fixture
                        .view
                        .selected_search_result()
                        .is_some_and(|entry| entry.display_name == "reports")
                });
                assert!(fixture.press(Key::Return, none));
                wait_until(|| location_ends_with(browser.active_location(), "reports"));
                wait_loaded(&browser, 0);
                assert!(!fixture.view.listing_search_active(), "{mode:?}");
                browser.back();
                wait_until(|| {
                    location_ends_with(browser.active_location(), fixture_name(&fixture))
                });
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
        },
    );
}

#[test]
fn tenxer_search_caps_hits_and_retires_stale_queries() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_search_caps_hits_and_retires_stale_queries",
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

#[test]
fn tenxer_search_survives_view_rebuilds_and_ends_with_the_mode() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_search_survives_view_rebuilds_and_ends_with_the_mode",
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
                first.view.set_view_mode(mode);
                wait_results(&first, &ALL_REPORTS);
                assert!(first.view.listing_search_active(), "{mode:?}");
                wait_until(|| first.shortcuts.filter_mark().as_deref() == Some("search: report"));
                assert_eq!(revealed_filter_funnels(&first.view.widget()), 0, "{mode:?}");

                let browser = first.view.browser();
                browser.reload_active();
                wait_loaded(&browser, 0);
                pump(200);
                wait_results(&first, &ALL_REPORTS);
                assert!(first.view.listing_search_active(), "{mode:?} after reload");
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

#[test]
fn tenxer_go_hit_folder_reveals_the_cursor_hit() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_go_hit_folder_reveals_the_cursor_hit",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let preferences = enable_tenxer(&fixture);
            preferences.set_filter_include_subfolders(false);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            let selected_names = || {
                browser
                    .selected_entries()
                    .into_iter()
                    .map(|entry| entry.display_name)
                    .collect::<Vec<_>>()
            };

            for mode in [BrowserMode::Icons, BrowserMode::Columns, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                for (query, folder, hit) in [
                    ("deep", "reports", "deep-report.txt"),
                    ("gamma", fixture_name(&fixture), "gamma-report.md"),
                ] {
                    focus_files(&fixture);
                    commit_search(&fixture, query);
                    wait_results(&fixture, &[hit]);
                    wait_until(|| fixture.view.selected_search_result().is_some());
                    assert!(fixture.press(Key::g, none));
                    assert!(fixture.press(Key::f, none));
                    wait_until(|| {
                        location_ends_with(browser.active_location(), folder)
                            && selected_names() == [hit]
                    });
                    wait_until(|| !fixture.view.listing_search_active());
                    assert_eq!(fixture.view.listing_filter(), None, "{mode:?} {query}");
                    wait_until(|| fixture.shortcuts.filter_mark().is_none());
                    if folder == "reports" {
                        browser.back();
                        wait_until(|| {
                            location_ends_with(browser.active_location(), fixture_name(&fixture))
                        });
                        wait_loaded(&browser, 0);
                    }
                }
            }
        },
    );
}

pub(super) fn hit_cursor(fixture: &KeyboardFixture) -> Option<String> {
    fixture
        .view
        .selected_search_result()
        .map(|entry| entry.display_name)
}

fn hidden_selection(fixture: &KeyboardFixture) -> Vec<String> {
    fixture
        .view
        .browser()
        .selected_entries()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect()
}

fn hit_names(order: &[String], positions: &[usize]) -> Vec<String> {
    let mut names: Vec<_> = positions.iter().map(|&at| order[at].clone()).collect();
    names.sort();
    names
}

/// Moves the cursor onto `name` with real keys where the view has a linear
/// order; Icons focus the hit directly.
pub(super) fn cursor_to_hit(fixture: &KeyboardFixture, name: &str) {
    if fixture.view.view_mode() == BrowserMode::Icons {
        let path = fixture
            ._directory
            .path()
            .join(if name == "deep-report.txt" {
                "reports/deep-report.txt"
            } else {
                name
            });
        assert!(fixture.view.focus_search_result(&path), "{name}");
    } else {
        assert!(fixture.press(Key::g, ModifierType::empty()));
        assert!(fixture.press(Key::g, ModifierType::empty()));
        for _ in ALL_REPORTS {
            if hit_cursor(fixture).as_deref() == Some(name) {
                break;
            }
            assert!(fixture.press(Key::j, ModifierType::empty()));
        }
    }
    wait_until(|| hit_cursor(fixture).as_deref() == Some(name));
}

#[test]
fn tenxer_search_hits_fill_and_range_apart_from_the_hidden_directory() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_search_hits_fill_and_range_apart_from_the_hidden_directory",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let preferences = enable_tenxer(&fixture);
            preferences.set_filter_include_subfolders(false);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            let shift = ModifierType::SHIFT_MASK;

            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                select_named(&fixture, "beta.txt");
                let hidden = hidden_selection(&fixture);
                assert_eq!(hidden, ["beta.txt"], "{mode:?}");
                assert!(fixture.press(Key::v, none));
                wait_until(|| fixture.shortcuts.visual_text().as_deref() == Some("VISUAL"));
                commit_search(&fixture, "report");
                wait_results(&fixture, &ALL_REPORTS);
                wait_until(|| fixture.shortcuts.filter_mark().as_deref() == Some("search: report"));
                assert_eq!(
                    fixture.shortcuts.visual_text(),
                    None,
                    "{mode:?}: hits end the hidden directory's range"
                );
                let order = fixture.view.filter_result_names();
                let at = |position: usize| Some(order[position].clone());
                wait_until(|| hit_cursor(&fixture) == at(0));

                assert!(fixture.press(Key::G, shift));
                wait_until(|| hit_cursor(&fixture) == at(3));
                for key in [Key::Home, Key::Page_Up] {
                    assert!(fixture.press(key, none));
                    assert!(fixture.press(Key::u, ModifierType::CONTROL_MASK));
                    pump(20);
                    assert_eq!(hit_cursor(&fixture), at(3), "{mode:?} {key:?} is swallowed");
                }
                assert!(fixture.press(Key::g, none));
                assert!(fixture.press(Key::g, none));
                wait_until(|| hit_cursor(&fixture) == at(0));
                for key in [Key::End, Key::Page_Down] {
                    assert!(fixture.press(key, none));
                    pump(20);
                    assert_eq!(hit_cursor(&fixture), at(0), "{mode:?} {key:?} is swallowed");
                }

                assert!(fixture.press(Key::space, none));
                wait_until(|| hit_cursor(&fixture) == at(1));
                assert_eq!(selected_result_names(&fixture), hit_names(&order, &[0]));
                assert!(fixture.press(Key::space, none));
                wait_until(|| hit_cursor(&fixture) == at(2));
                assert_eq!(selected_result_names(&fixture), hit_names(&order, &[0, 1]));
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

                if mode == BrowserMode::Icons {
                    // Spatial moves keep the fill wherever the grid puts the cursor.
                    assert!(fixture.press(Key::h, none));
                    pump(20);
                    assert_eq!(selected_result_names(&fixture), hit_names(&order, &[0, 1]));
                    assert!(fixture.press(Key::v, none));
                    wait_until(|| fixture.shortcuts.visual_text().as_deref() == Some("VISUAL"));
                    assert!(fixture.press(Key::v, none));
                    wait_until(|| fixture.shortcuts.visual_text().is_none());
                    assert!(fixture.view.listing_search_active(), "{mode:?}");
                } else {
                    assert!(fixture.press(Key::j, none));
                    wait_until(|| hit_cursor(&fixture) == at(3));
                    assert_eq!(selected_result_names(&fixture), hit_names(&order, &[0, 1]));
                    assert!(fixture.press(Key::k, none));
                    wait_until(|| hit_cursor(&fixture) == at(2));

                    assert!(fixture.press(Key::v, none));
                    wait_until(|| fixture.shortcuts.visual_text().as_deref() == Some("VISUAL"));
                    assert_eq!(
                        selected_result_names(&fixture),
                        hit_names(&order, &[0, 1, 2])
                    );
                    assert!(fixture.press(Key::j, none));
                    wait_until(|| hit_cursor(&fixture) == at(3));
                    assert_eq!(
                        selected_result_names(&fixture),
                        hit_names(&order, &[0, 1, 2, 3])
                    );
                    assert!(fixture.press(Key::k, none));
                    wait_until(|| hit_cursor(&fixture) == at(2));
                    assert_eq!(
                        selected_result_names(&fixture),
                        hit_names(&order, &[0, 1, 2])
                    );

                    assert!(fixture.press(Key::Escape, none));
                    wait_until(|| fixture.shortcuts.visual_text().is_none());
                    assert!(fixture.view.listing_search_active(), "{mode:?}");
                    assert_eq!(
                        selected_result_names(&fixture),
                        hit_names(&order, &[0, 1, 2])
                    );

                    assert!(fixture.press(Key::k, none));
                    wait_until(|| hit_cursor(&fixture) == at(1));
                    assert!(fixture.press(Key::V, shift));
                    wait_until(|| fixture.shortcuts.visual_text().as_deref() == Some("UNSET"));
                    assert_eq!(selected_result_names(&fixture), hit_names(&order, &[0, 2]));
                    assert!(fixture.press(Key::V, shift));
                    wait_until(|| fixture.shortcuts.visual_text().is_none());
                    assert_eq!(selected_result_names(&fixture), hit_names(&order, &[0, 2]));
                }
                let cursor = hit_cursor(&fixture).expect("a cursor hit");
                let mut expected = selected_result_names(&fixture);
                let extra = order
                    .iter()
                    .position(|name| *name != cursor && !expected.contains(name))
                    .expect("an unselected hit");
                expected.push(order[extra].clone());
                match expected.iter().position(|name| *name == cursor) {
                    Some(at) => {
                        expected.remove(at);
                    }
                    None => expected.push(cursor),
                }
                expected.sort();
                assert!(fixture.view.extend_result_selection(extra as u32));
                assert!(fixture.press(Key::space, none));
                wait_until(|| selected_result_names(&fixture) == expected);

                assert_eq!(
                    hidden_selection(&fixture),
                    hidden,
                    "{mode:?}: hits never touch the hidden directory's fill"
                );
                assert_eq!(fixture.shortcuts.count_text().0, "4 items", "{mode:?}");

                assert!(fixture.press(Key::Escape, none));
                wait_until(|| !fixture.view.listing_search_active());
                wait_until(|| fixture.view.selected_search_results().is_none());
                assert_eq!(hidden_selection(&fixture), hidden, "{mode:?}");
            }
        },
    );
}

#[test]
fn tenxer_search_hit_keys_peek_preview_and_yield_to_chords() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_search_hit_keys_peek_preview_and_yield_to_chords",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let preferences = enable_tenxer(&fixture);
            preferences.set_filter_include_subfolders(false);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            let folder = fixture_name(&fixture).to_string();
            let focus = || gtk::prelude::RootExt::focus(&fixture.window);

            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                focus_files(&fixture);
                commit_search(&fixture, "report");
                wait_results(&fixture, &ALL_REPORTS);

                cursor_to_hit(&fixture, "reports");
                assert!(fixture.press(Key::i, none));
                if mode == BrowserMode::Columns {
                    wait_until(|| location_ends_with(browser.location_at(1), "reports"));
                } else {
                    wait_until(|| fixture.view.widget().has_css_class("peek-open"));
                    assert!(fixture.press(Key::i, none));
                    wait_until(|| !fixture.view.widget().has_css_class("peek-open"));
                }
                assert!(fixture.view.listing_search_active(), "{mode:?}");
                assert!(fixture.view.item_view_has_focus(), "{mode:?}");
                assert_eq!(hit_cursor(&fixture).as_deref(), Some("reports"), "{mode:?}");
                assert!(location_ends_with(browser.active_location(), &folder));

                cursor_to_hit(&fixture, "alpha-report.txt");
                assert!(fixture.press(Key::i, none));
                wait_until(|| fixture.preview.is_enabled());
                assert!(
                    !fixture.preview.owns_focus(focus().as_ref()),
                    "{mode:?}: i never takes preview ownership"
                );
                assert!(fixture.view.item_view_has_focus(), "{mode:?}");
                assert!(fixture.press(Key::i, none));
                wait_until(|| !fixture.preview.is_enabled());

                if mode != BrowserMode::Icons {
                    assert!(fixture.press(Key::l, none));
                    wait_until(|| fixture.preview.owns_focus(focus().as_ref()));
                    assert!(fixture.press(Key::h, none));
                    wait_until(|| fixture.view.item_view_has_focus());
                    assert!(fixture.preview.is_open(), "{mode:?}: h keeps the drawer");
                    assert!(fixture.view.listing_search_active(), "{mode:?}");
                    assert_eq!(
                        hit_cursor(&fixture).as_deref(),
                        Some("alpha-report.txt"),
                        "{mode:?}: h returns to the same hit"
                    );
                    assert!(fixture.press(Key::h, none));
                    wait_until(|| !fixture.view.listing_search_active());
                    assert!(location_ends_with(browser.active_location(), &folder));
                    fixture.preview.close();
                    focus_files(&fixture);
                    commit_search(&fixture, "report");
                    wait_results(&fixture, &ALL_REPORTS);
                }

                wait_until(|| hit_cursor(&fixture).is_some());
                let cursor = hit_cursor(&fixture);
                assert!(fixture.press(Key::g, none));
                assert!(fixture.press(Key::j, none));
                assert_eq!(fixture.shortcuts.feedback_text(), "Unknown chord");
                fixture.shortcuts.dismiss_feedback();
                pump(20);
                assert_eq!(hit_cursor(&fixture), cursor, "{mode:?}");
                assert!(fixture.view.listing_search_active(), "{mode:?}");

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
                wait_until(|| location_ends_with(browser.active_location(), &folder));
                wait_loaded(&browser, 0);
            }
        },
    );
}

/// Another program deletes the cursor's file, one of the hits, while the **f** or **s**
/// prompt filters the listing.
fn outside_deletion_case(mode: BrowserMode, prompt: Prompt) -> Result<(), String> {
    let fixture = KeyboardFixture::new();
    seed_filter_tree(&fixture);
    enable_tenxer(&fixture);
    fixture.view.set_view_mode(mode);
    let browser = fixture.view.browser();
    wait_loaded(&browser, 0);
    select_named(&fixture, "alpha-report.txt");
    let (key, expected): (Key, &[&str]) = if prompt == Prompt::Filter {
        (Key::f, &IMMEDIATE_REPORTS)
    } else {
        (Key::s, &ALL_REPORTS)
    };
    if !fixture.press(key, ModifierType::empty()) || !fixture.shortcuts.prompt_has_focus() {
        return Err("setup: the prompt did not open with focus".to_owned());
    }
    fixture.shortcuts.prompt().set_text("report");
    wait_results(&fixture, expected);
    std::fs::remove_file(fixture._directory.path().join("alpha-report.txt"))
        .expect("delete the hit");
    wait_until(|| entry_count(&browser) == 6);
    // Focus that must stay put has no settle condition.
    pump(300);
    if !fixture.shortcuts.prompt_has_focus() || fixture.shortcuts.open_prompt_kind() != Some(prompt)
    {
        return Err(format!(
            "the deletion took focus from the prompt (open prompt {:?})",
            fixture.shortcuts.open_prompt_kind()
        ));
    }
    if fixture.shortcuts.prompt_text() != "report" {
        return Err(format!(
            "the prompt text became {:?}",
            fixture.shortcuts.prompt_text()
        ));
    }
    Ok(())
}

#[test]
fn tenxer_filter_prompts_keep_focus_through_an_outside_deletion() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::tenxer_filter_prompts_keep_focus_through_an_outside_deletion",
        || {
            let mut failures = Vec::new();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for prompt in [Prompt::Filter, Prompt::Search] {
                    if let Err(error) = outside_deletion_case(mode, prompt) {
                        failures.push(format!("{mode:?} {prompt:?}: {error}"));
                    }
                }
            }
            assert!(failures.is_empty(), "\n{}", failures.join("\n"));
        },
    );
}
