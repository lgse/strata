// SPDX-License-Identifier: MIT

use super::*;

fn hidden_cursor(fixture: &KeyboardFixture) -> Option<(usize, usize)> {
    fixture
        .view
        .browser()
        .focused_item()
        .map(|(depth, position, _)| (depth, position))
}

/// Typing into the **f** prompt filters live, keeps a stepped hit through
/// edits, and shows no Enter mark.
fn filter_prompt_filters_live(fixture: &KeyboardFixture, mode: BrowserMode) {
    assert!(fixture.press(Key::f, ModifierType::empty()), "{mode:?}");
    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Filter));
    assert_eq!(fixture.shortcuts.prompt_label().as_deref(), Some("filter:"));
    fixture.shortcuts.prompt().set_text("report");
    wait_results(fixture, &IMMEDIATE_REPORTS);
    wait_highlights(fixture, &IMMEDIATE_REPORTS);
    assert!(
        fixture.shortcuts.prompt_has_focus(),
        "{mode:?}: typing filters live and stays in the prompt"
    );
    fixture.view.step_filter_results(1, 1, false);
    let stepped = selected_result_names(fixture);
    assert_eq!(stepped.len(), 1, "{mode:?}");
    fixture.shortcuts.prompt().set_text("repo");
    wait_until(|| fixture.view.listing_filter().as_deref() == Some("repo"));
    pump(300);
    assert_eq!(
        selected_result_names(fixture),
        stepped,
        "{mode:?}: editing the query keeps a stepped hit, unlike the Ctrl+F field"
    );
    assert!(
        marked_rows(fixture.window.upcast_ref()).is_empty(),
        "{mode:?}: the prompt shows no Enter mark"
    );
    fixture.shortcuts.prompt().set_text("report");
    wait_results(fixture, &IMMEDIATE_REPORTS);
}

fn ctrl_r_inverts_results(
    fixture: &KeyboardFixture,
    mode: BrowserMode,
    second: &[String],
) -> Vec<String> {
    assert!(fixture.press(Key::r, ModifierType::CONTROL_MASK));
    pump(20);
    let inverted = selected_result_names(fixture);
    assert_eq!(inverted.len(), 2, "{mode:?}: Ctrl+R inverts the results");
    assert!(!inverted.contains(&second[0]), "{mode:?}");
    inverted
}

fn ctrl_a_selects_results(fixture: &KeyboardFixture, mode: BrowserMode) {
    assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
    pump(20);
    let all = selected_result_names(fixture);
    assert_eq!(all.len(), 3, "{mode:?}: Ctrl+A selects the results");
    assert!(fixture.press(Key::k, ModifierType::empty()));
    assert_eq!(
        selected_result_names(fixture),
        all,
        "{mode:?}: motion preserves select all"
    );
}

/// Enter commits the filter; motion and selection keys act on the results and
/// leave the hidden directory's fill and cursor alone.
fn committed_filter_leaves_hidden_directory(
    fixture: &KeyboardFixture,
    mode: BrowserMode,
    cursor: Option<(usize, usize)>,
) {
    let browser = fixture.view.browser();
    let none = ModifierType::empty();
    assert!(fixture.press(Key::Return, none));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    wait_until(|| {
        fixture.view.item_view_has_focus()
            && fixture.view.selected_search_result().is_some()
            && fixture.shortcuts.filter_mark().as_deref() == Some("filter: report")
    });
    wait_until(|| fixture.shortcuts.count_text() == ("3 items".to_owned(), String::new()));
    assert_eq!(revealed_filter_funnels(&fixture.view.widget()), 0);
    assert_eq!(fill_names(&browser), ["beta.txt"], "{mode:?}");

    let first = selected_result_names(fixture);
    assert!(fixture.press(Key::j, none), "{mode:?}");
    pump(40);
    let second = selected_result_names(fixture);
    assert_eq!(second.len(), 1, "{mode:?}");
    assert_ne!(first, second, "{mode:?}: j moves among the results");
    let inverted = ctrl_r_inverts_results(fixture, mode, &second);
    assert!(fixture.press(Key::j, none));
    assert_eq!(
        selected_result_names(fixture),
        inverted,
        "{mode:?}: motion preserves the inverted result selection"
    );
    ctrl_a_selects_results(fixture, mode);
    assert_eq!(
        fill_names(&browser),
        ["beta.txt"],
        "{mode:?}: the hidden directory fill is untouched"
    );
    assert_eq!(
        hidden_cursor(fixture),
        cursor,
        "{mode:?}: the hidden directory cursor is untouched"
    );
}

fn f_prefills_the_committed_query(fixture: &KeyboardFixture, mode: BrowserMode) {
    assert!(fixture.press(Key::f, ModifierType::empty()));
    assert_eq!(
        fixture.shortcuts.prompt().text(),
        "report",
        "{mode:?}: f pre-fills the committed query"
    );
}

fn prompt_escape_clears(
    fixture: &KeyboardFixture,
    mode: BrowserMode,
    directory_count: &(String, String),
) {
    f_prefills_the_committed_query(fixture, mode);
    assert!(fixture.press(Key::Escape, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    wait_until(|| fixture.view.selected_search_results().is_none());
    assert_eq!(
        fixture.view.listing_filter(),
        None,
        "{mode:?}: prompt Esc clears"
    );
    wait_until(|| fixture.shortcuts.filter_mark().is_none());
    wait_until(|| fixture.shortcuts.count_text() == *directory_count);
    assert!(fixture.view.item_view_has_focus(), "{mode:?}");
}

fn listing_escape_and_empty_enter_clear(fixture: &KeyboardFixture, mode: BrowserMode) {
    let browser = fixture.view.browser();
    commit_filter(fixture, "report");
    assert!(fixture.press(Key::Escape, ModifierType::empty()));
    wait_until(|| fixture.view.selected_search_results().is_none());
    assert_eq!(
        fixture.view.listing_filter(),
        None,
        "{mode:?}: listing Esc clears"
    );
    assert_eq!(fill_names(&browser), ["beta.txt"], "{mode:?}");

    commit_filter(fixture, "report");
    commit_filter(fixture, "");
    wait_until(|| fixture.view.selected_search_results().is_none());
    assert_eq!(
        fixture.view.listing_filter(),
        None,
        "{mode:?}: empty Enter clears"
    );
}

fn zero_results_and_multiword_queries(fixture: &KeyboardFixture, mode: BrowserMode) {
    let none = ModifierType::empty();
    commit_filter(fixture, "zzz");
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

    commit_filter(fixture, "md rep");
    wait_results(fixture, &["gamma-report.md"]);
    wait_highlights(fixture, &["gamma-report.md"]);
    assert!(fixture.press(Key::Escape, none));
    wait_until(|| fixture.view.listing_filter().is_none());
    wait_until(|| highlighted_names(&fixture.view.widget()).is_empty());
}

fn committing_opens_nothing_until_enter(fixture: &KeyboardFixture, mode: BrowserMode) {
    commit_filter(fixture, "reports");
    wait_until(|| {
        fixture
            .view
            .selected_search_result()
            .is_some_and(|entry| entry.display_name == "reports")
    });
    assert!(
        in_fixture_folder(fixture),
        "{mode:?}: committing opens nothing"
    );
    enter_reports_and_back(fixture);
    fixture.view.clear_listing_filter();
}

#[test]
fn tenxer_filter_commits_results_without_touching_the_hidden_directory() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter::tenxer_filter_commits_results_without_touching_the_hidden_directory",
        || {
            let (fixture, _preferences) = tenxer_filter_fixture();
            let browser = fixture.view.browser();

            for mode in ALL_MODES {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                select_named(&fixture, "beta.txt");
                let cursor = hidden_cursor(&fixture);
                let directory_count = fixture.shortcuts.count_text();

                filter_prompt_filters_live(&fixture, mode);
                committed_filter_leaves_hidden_directory(&fixture, mode, cursor);
                prompt_escape_clears(&fixture, mode, &directory_count);
                listing_escape_and_empty_enter_clear(&fixture, mode);
                zero_results_and_multiword_queries(&fixture, mode);
                committing_opens_nothing_until_enter(&fixture, mode);
            }
        },
    );
}

/// Ctrl+F with a query: the footer describes the directory until the first
/// batch, then shows the result total without a mark.
fn typed_query_counts_results(
    fixture: &KeyboardFixture,
    mode: BrowserMode,
    directory_count: &(String, String),
) -> gtk::Entry {
    assert!(
        fixture.press(Key::f, ModifierType::CONTROL_MASK),
        "{mode:?}"
    );
    let entry = focused_entry(&fixture.window);
    entry.set_text("report");
    fixture.shortcuts.refresh_filter();
    assert_eq!(
        fixture.shortcuts.count_text(),
        *directory_count,
        "{mode:?}: a query still in its debounce describes the directory"
    );
    wait_results(fixture, &IMMEDIATE_REPORTS);
    wait_footer(
        fixture,
        &format!("{mode:?}: the default map shows the displayed result total"),
        || fixture.shortcuts.count_text().0 == "3 items",
    );
    assert_eq!(
        fixture.shortcuts.filter_mark(),
        None,
        "{mode:?}: the default map shows the total only"
    );
    entry
}

fn subfolder_scope_restarts_the_count(
    fixture: &KeyboardFixture,
    preferences: &PreferenceManager,
    mode: BrowserMode,
    directory_count: &(String, String),
) {
    preferences.set_filter_include_subfolders(true);
    fixture.shortcuts.refresh_filter();
    assert_eq!(
        fixture.shortcuts.count_text(),
        *directory_count,
        "{mode:?}: a restarted search counts nothing before its first batch"
    );
    wait_results(fixture, &ALL_REPORTS);
    wait_footer(fixture, &format!("{mode:?}: the wider total"), || {
        fixture.shortcuts.count_text().0 == "4 items"
    });
    preferences.set_filter_include_subfolders(false);
    wait_results(fixture, &IMMEDIATE_REPORTS);
    wait_footer(fixture, &format!("{mode:?}: the narrower total"), || {
        fixture.shortcuts.count_text().0 == "3 items"
    });
}

fn selection_and_miss_keep_totals(
    fixture: &KeyboardFixture,
    entry: &gtk::Entry,
    mode: BrowserMode,
) {
    assert!(fixture.view.extend_result_selection(1), "{mode:?}");
    pump(40);
    assert_eq!(
        fixture.shortcuts.count_text().0,
        "3 items",
        "{mode:?}: selecting a result keeps the total"
    );
    assert_eq!(hidden_selection(fixture), ["beta.txt"], "{mode:?}");

    entry.set_text("zzz");
    wait_footer(
        fixture,
        &format!("{mode:?}: a miss reports 0 items"),
        || fixture.shortcuts.count_text().0 == "0 items",
    );
}

fn mark_follows_tenxer_mode(
    fixture: &KeyboardFixture,
    preferences: &PreferenceManager,
    mode: BrowserMode,
    directory_count: &(String, String),
) {
    preferences.set_tenxer_mode(true);
    wait_footer(fixture, &format!("{mode:?}: 10xer shows the mark"), || {
        fixture.shortcuts.filter_mark().as_deref() == Some("filter: zzz")
    });
    // The hidden field drops focus; its blur check must run while still in 10xer.
    pump(40);
    preferences.set_tenxer_mode(false);
    wait_footer(
        fixture,
        &format!("{mode:?}: leaving 10xer hides the mark, keeps the total"),
        || {
            fixture.shortcuts.filter_mark().is_none()
                && fixture.shortcuts.count_text().0 == "0 items"
        },
    );

    fixture.view.clear_listing_filter();
    wait_until(|| fixture.view.listing_filter().is_none());
    wait_footer(
        fixture,
        &format!("{mode:?}: dismissing restores the selection summary"),
        || fixture.shortcuts.count_text() == *directory_count,
    );
}

#[test]
fn default_filter_shows_the_result_total_without_a_footer_mark() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter::default_filter_shows_the_result_total_without_a_footer_mark",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let preferences = PreferenceManager::shared();
            fixture.shortcuts.bind_preferences(&preferences);
            assert!(!preferences.tenxer_mode());
            preferences.set_filter_include_subfolders(false);
            let browser = fixture.view.browser();

            for mode in ALL_MODES {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                select_named(&fixture, "beta.txt");
                focus_files(&fixture);
                wait_footer(&fixture, &format!("{mode:?}: seed selection"), || {
                    fixture.shortcuts.count_text().0 == "1 file selected (6 B)"
                });
                let directory_count = fixture.shortcuts.count_text();
                assert_eq!(fixture.shortcuts.filter_mark(), None, "{mode:?}");

                let entry = typed_query_counts_results(&fixture, mode, &directory_count);
                subfolder_scope_restarts_the_count(&fixture, &preferences, mode, &directory_count);
                selection_and_miss_keep_totals(&fixture, &entry, mode);
                mark_follows_tenxer_mode(&fixture, &preferences, mode, &directory_count);
            }
        },
    );
}

/// A rebuilt view and a reload keep the committed filter and its highlights.
fn rebuild_keeps_filter(first: &KeyboardFixture, mode: BrowserMode, accent: &str, drawn: &str) {
    first.view.set_view_mode(mode);
    wait_results(first, &["gamma-report.md"]);
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
    wait_results(first, &["gamma-report.md"]);
    assert_eq!(
        first.view.listing_filter().as_deref(),
        Some("gamma"),
        "{mode:?} after reload"
    );
}

#[test]
fn tenxer_filter_ignores_include_subfolders_and_survives_view_rebuilds() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter::tenxer_filter_ignores_include_subfolders_and_survives_view_rebuilds",
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
                rebuild_keeps_filter(&first, mode, accent, drawn);
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
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter::tenxer_filter_prompts_keep_focus_through_an_outside_deletion",
        || {
            let mut failures = Vec::new();
            for mode in ALL_MODES {
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
