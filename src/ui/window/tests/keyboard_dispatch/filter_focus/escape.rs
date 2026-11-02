// SPDX-License-Identifier: MIT

use super::*;

fn escape(fixture: &KeyboardFixture) -> bool {
    fixture.press(Key::Escape, ModifierType::empty())
}

fn filter_dismissed(fixture: &KeyboardFixture) -> bool {
    fixture.view.listing_filter().is_none() && !fixture.view.results_replace_listing()
}

fn listing_focused_without_filter(fixture: &KeyboardFixture) -> bool {
    fixture.view.item_view_has_focus() && fixture.view.filter_focus().is_none()
}

fn escape_dismisses_the_filter(fixture: &KeyboardFixture) -> Result<(), String> {
    ensure(escape(fixture), || {
        format!(
            "Escape on a focused result was not handled; filter {:?}, focus on {}",
            fixture.view.listing_filter(),
            describe_focus(fixture)
        )
    })?;
    ensure(settles(|| filter_dismissed(fixture)), || {
        format!(
            "Escape on a focused result left the filter {:?} (results replace the listing: {})",
            fixture.view.listing_filter(),
            fixture.view.results_replace_listing()
        )
    })?;
    ensure(settles(|| listing_focused_without_filter(fixture)), || {
        format!(
            "focus did not return to the listing; it is on {}",
            describe_focus(fixture)
        )
    })
}

fn escape_case(mode: BrowserMode, recursive: bool) -> Result<(), String> {
    let fixture = filtered_fixture(mode, recursive);
    let field = show_filter_with_results(&fixture, "report", expected_reports(recursive))?;
    focus_first_result(&fixture, &field)?;
    let separate_results = fixture.view.results_replace_listing();
    let directory_selection = fixture.selected();

    escape_dismisses_the_filter(&fixture)?;
    ensure(
        !separate_results || fixture.selected() == directory_selection,
        || {
            format!(
                "dismissal changed the directory selection from {directory_selection:?} to {:?}",
                fixture.selected()
            )
        },
    )?;
    listing_settles_on_browser(&fixture)?;
    ensure(
        escape(&fixture) && settles(|| fixture.selected().is_empty()),
        || "a second Escape did not clear the restored selection".to_owned(),
    )
}

fn narrowed_listing_restored(fixture: &KeyboardFixture) -> bool {
    listing_focused_without_filter(fixture)
        && fixture.view.listing_filter().is_none()
        && shown_model(fixture).is_some_and(|rows| rows.n_items() == 3)
}

fn in_place_escape_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = remote_share_fixture(mode);
    let browser = fixture.view.browser();
    browser.select(0, 0);
    focus_files(&fixture);
    ensure(ctrl_f_focuses_the_field(&fixture), || {
        "Ctrl+F did not focus the field".to_owned()
    })?;
    focused_entry(&fixture.window).set_text("report");
    // Past the debounce that narrows the rows.
    pump(400);
    browser.focus_active();
    ensure(
        settles(|| fixture.view.filter_focus() == Some(FilterFocus::Results)),
        || {
            format!(
                "a narrowed row did not count as a result; focus is on {}",
                describe_focus(&fixture)
            )
        },
    )?;

    ensure(escape(&fixture), || {
        format!(
            "Escape on a narrowed row was not handled; focus is on {}",
            describe_focus(&fixture)
        )
    })?;
    ensure(settles(|| narrowed_listing_restored(&fixture)), || {
        format!(
            "Escape left the filter {:?}, the listing showing {:?} rows, focus on {}",
            fixture.view.listing_filter(),
            shown_model(&fixture).map(|rows| rows.n_items()),
            describe_focus(&fixture)
        )
    })?;
    listing_settles_on_browser(&fixture)
}

#[test]
fn escape_dismisses_the_filter_from_a_focused_result() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::escape::escape_dismisses_the_filter_from_a_focused_result",
        || {
            let mut failures = failures_of(
                [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons]
                    .into_iter()
                    .flat_map(|mode| [false, true].map(|recursive| (mode, recursive))),
                |(mode, recursive)| format!("{mode:?}, subfolders {recursive}"),
                |(mode, recursive)| escape_case(mode, recursive),
            );
            failures.extend(failures_of(
                [BrowserMode::List, BrowserMode::Icons],
                |mode| format!("{mode:?}, remote share"),
                in_place_escape_case,
            ));
            report(failures);
        },
    );
}

fn preview_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = filtered_fixture(mode, false);
    let expected: &[&str] = &["alpha-report.txt"];
    let field = show_filter_with_results(&fixture, "alpha", expected)?;
    focus_first_result(&fixture, &field)?;
    ensure(
        fixture.press(Key::space, ModifierType::empty())
            && settles(|| fixture.preview.is_enabled()),
        || "Space on a focused result did not open the preview".to_owned(),
    )?;
    let handled = escape(&fixture);
    pump(100);
    let filter_dismissed = fixture.view.listing_filter().is_none();
    let preview_open = fixture.preview.is_enabled();
    ensure(handled && filter_dismissed && preview_open, || {
        format!(
            "the first Escape should dismiss the filter and keep the preview: handled {handled}, \
             filter {:?}, preview open {}",
            fixture.view.listing_filter(),
            fixture.preview.is_enabled()
        )
    })?;
    ensure(
        escape(&fixture) && settles(|| !fixture.preview.is_enabled()),
        || "the second Escape did not close the preview".to_owned(),
    )
}

#[test]
fn escape_from_a_focused_result_dismisses_the_filter_before_the_preview() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::escape::escape_from_a_focused_result_dismisses_the_filter_before_the_preview",
        || check_modes(&[BrowserMode::Columns, BrowserMode::List], preview_case),
    );
}

#[derive(Clone, Copy, Debug)]
enum SpaceFocus {
    Location,
    Sidebar,
    Toolbar,
}

fn move_focus_to(fixture: &KeyboardFixture, target: SpaceFocus) -> bool {
    match target {
        SpaceFocus::Location => {
            fixture.press(Key::l, ModifierType::CONTROL_MASK)
                && settles(|| fixture.view.location_has_focus())
        }
        SpaceFocus::Sidebar => {
            fixture.sidebar.state.focus_active_place();
            settles(|| {
                focus(fixture).is_some_and(|focused| focused.is_ancestor(&fixture.sidebar.widget))
            })
        }
        SpaceFocus::Toolbar => fixture.sidebar_toggle.grab_focus(),
    }
}

fn space_case(mode: BrowserMode, target: SpaceFocus) -> Result<(), String> {
    let fixture = filtered_fixture(mode, false);
    let field = show_filter_with_results(&fixture, "alpha", &["alpha-report.txt"])?;
    focus_first_result(&fixture, &field)?;
    ensure(move_focus_to(&fixture, target), || {
        format!(
            "{target:?} did not take focus; focus is on {}",
            describe_focus(&fixture)
        )
    })?;
    ensure(
        fixture
            .view
            .selected_search_results()
            .is_some_and(|hits| !hits.is_empty()),
        || format!("focusing {target:?} dropped the selected hit"),
    )?;
    ensure(!fixture.press(Key::space, ModifierType::empty()), || {
        format!("Space on {target:?} acted on the selected hit")
    })?;
    pump(100);
    ensure(!fixture.preview.is_enabled(), || {
        format!("Space on {target:?} opened the preview")
    })
}

#[test]
fn space_acts_on_a_selected_hit_only_from_the_filter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::escape::space_acts_on_a_selected_hit_only_from_the_filter",
        || {
            let cases = [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons]
                .into_iter()
                .flat_map(|mode| {
                    [
                        SpaceFocus::Location,
                        SpaceFocus::Sidebar,
                        SpaceFocus::Toolbar,
                    ]
                    .map(|target| (mode, target))
                });
            report(failures_of(
                cases,
                |(mode, _)| format!("{mode:?}"),
                |(mode, target)| space_case(mode, target),
            ));
        },
    );
}
