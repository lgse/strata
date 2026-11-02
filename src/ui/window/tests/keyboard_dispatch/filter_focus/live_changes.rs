// SPDX-License-Identifier: MIT

use super::*;

/// More than the browser publishes inline, so the rows after the first 128 arrive in
/// an idle tail and the load completes after Ctrl+F.
const STAGED_ENTRIES: usize = 600;

fn seed_staged_folder(fixture: &KeyboardFixture) -> Location {
    let big = fixture._directory.path().join("big");
    std::fs::create_dir(&big).expect("staged folder");
    for index in 0..STAGED_ENTRIES {
        std::fs::write(big.join(format!("file-{index:04}.txt")), b"x").expect("staged file");
    }
    Location::local(&big)
}

/// Flags set when the first inline publication and the load's end arrive.
fn observe_staged_load(browser: &crate::app::Browser) -> (Rc<Cell<bool>>, Rc<Cell<bool>>) {
    let first_publish = Rc::new(Cell::new(false));
    let finished = Rc::new(Cell::new(false));
    let (seen, done) = (first_publish.clone(), finished.clone());
    browser.observe(move |event| match event {
        BrowserEvent::EntriesReplaced {
            depth: 0,
            count: 128,
        } => seen.set(true),
        BrowserEvent::LoadFinished { depth: 0, .. } => done.set(true),
        _ => {}
    });
    (first_publish, finished)
}

fn field_keeps_editing_keys(fixture: &KeyboardFixture, big: &Location) -> Result<(), String> {
    fixture.press(Key::a, ModifierType::CONTROL_MASK);
    ensure(fixture.selected() == [0], || {
        format!(
            "Ctrl+A selected {} hidden rows instead of the field text",
            fixture.selected().len()
        )
    })?;
    fixture.press(Key::BackSpace, ModifierType::empty());
    pump(100);
    ensure(
        fixture.view.browser().active_location().as_ref() == Some(big),
        || "Backspace left the folder instead of editing the field".to_owned(),
    )
}

fn staged_load_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = KeyboardFixture::new();
    let big = seed_staged_folder(&fixture);
    fixture.view.set_view_mode(mode);
    focus_files(&fixture);
    let browser = fixture.view.browser();
    let (first_publish, finished) = observe_staged_load(&browser);
    browser.navigate(big.clone());
    wait_until(|| first_publish.get());
    ensure(ctrl_f_focuses_the_field(&fixture), || {
        format!(
            "Ctrl+F during the load did not focus the field; focus is on {}",
            describe_focus(&fixture)
        )
    })?;
    // The load reports done before its idle tail completes the publication.
    wait_until(|| finished.get());
    wait_until(|| entry_count(&browser) == STAGED_ENTRIES);
    pump(100);
    ensure(
        fixture.view.filter_has_focus() && !fixture.view.item_view_has_focus(),
        || {
            format!(
                "the load took focus from the field; it is on {}",
                describe_focus(&fixture)
            )
        },
    )?;
    field_keeps_editing_keys(&fixture, &big)
}

#[test]
fn staged_load_completion_keeps_focus_in_the_filter_entry() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::live_changes::staged_load_completion_keeps_focus_in_the_filter_entry",
        || check_modes(&[BrowserMode::Icons, BrowserMode::List], staged_load_case),
    );
}

/// What an outside deletion of a hit left wrong while the field had focus.
fn deletion_failures(
    fixture: &KeyboardFixture,
    field: &gtk::Entry,
    expected: &[&str],
    separate_results: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if !fixture.view.filter_has_focus() {
        failures.push(format!(
            "the deletion took focus from the field to {}",
            describe_focus(fixture)
        ));
    }
    if field.text() != "report" {
        failures.push(format!("the filter text became {:?}", field.text()));
    }
    if separate_results && !settles(|| result_names(fixture) == expected[1..]) {
        failures.push(format!(
            "the hits are {:?}, not {:?}",
            result_names(fixture),
            &expected[1..]
        ));
    }
    match focus_first_result(fixture, field) {
        Ok(()) if hit_cursor(fixture).as_deref() == Some("alpha-report.txt") => {
            failures.push("Down focused the deleted hit".to_owned());
        }
        Ok(()) => {}
        Err(error) => failures.push(error),
    }
    failures
}

/// Another program deletes the cursor's file, one of the hits, while the pane filter's
/// field has focus.
fn external_delete_case(mode: BrowserMode, recursive: bool) -> Result<(), String> {
    let fixture = filtered_fixture(mode, recursive);
    let browser = fixture.view.browser();
    select_named(&fixture, "alpha-report.txt");
    let expected = expected_reports(recursive);
    let field = show_filter_with_results(&fixture, "report", expected)?;
    std::fs::remove_file(fixture._directory.path().join("alpha-report.txt"))
        .expect("delete the hit");
    ensure(settles(|| entry_count(&browser) == 6), || {
        "the monitor never removed the deleted file from the listing".to_owned()
    })?;
    // Focus that must stay put has no settle condition.
    pump(300);
    // An in-place Columns filter narrows the listing itself, which drops the row.
    let separate_results = mode != BrowserMode::Columns || recursive;
    let failures = deletion_failures(&fixture, &field, expected, separate_results);
    ensure(failures.is_empty(), || failures.join("; "))
}

#[test]
fn an_outside_deletion_keeps_focus_in_the_filter_entry() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::live_changes::an_outside_deletion_keeps_focus_in_the_filter_entry",
        || {
            let cases = [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons]
                .into_iter()
                .flat_map(|mode| [false, true].map(|recursive| (mode, recursive)));
            report(failures_of(
                cases,
                |(mode, recursive)| format!("{mode:?}, subfolders {recursive}"),
                |(mode, recursive)| external_delete_case(mode, recursive),
            ));
        },
    );
}

/// Delete trashes the hit at once; Shift+Delete asks first, so its focus comes back
/// through the dismissed dialog's fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeleteRoute {
    Trash,
    Permanent,
}

const DELETE_CASES: [(BrowserMode, DeleteRoute); 4] = [
    (BrowserMode::List, DeleteRoute::Trash),
    (BrowserMode::List, DeleteRoute::Permanent),
    (BrowserMode::Icons, DeleteRoute::Trash),
    (BrowserMode::Icons, DeleteRoute::Permanent),
];

fn confirm_permanent_deletion(fixture: &KeyboardFixture) -> Result<(), String> {
    ensure(
        settles(|| {
            widget_with_class(fixture.overlay.upcast_ref(), "action-dialog-confirm").is_some()
        }),
        || "Shift+Delete never asked for confirmation".to_owned(),
    )?;
    ensure(
        click_class(&fixture.overlay, "action-dialog-confirm"),
        || "failed to confirm the deletion".to_owned(),
    )
}

fn delete_focused_hit(fixture: &KeyboardFixture, route: DeleteRoute) -> Result<(), String> {
    let hit = hit_cursor(fixture).ok_or("no focused hit to delete")?;
    let modifiers = match route {
        DeleteRoute::Trash => ModifierType::empty(),
        DeleteRoute::Permanent => ModifierType::SHIFT_MASK,
    };
    ensure(fixture.press(Key::Delete, modifiers), || {
        format!(
            "{route:?} delete on a focused result was not handled; focus on {}",
            describe_focus(fixture)
        )
    })?;
    if route == DeleteRoute::Permanent {
        confirm_permanent_deletion(fixture)?;
    }
    ensure(settles(|| !result_names(fixture).contains(&hit)), || {
        format!(
            "deleted hit {hit:?} never left the results {:?}",
            result_names(fixture)
        )
    })
}

fn escape_closes_filter(fixture: &KeyboardFixture) -> Result<(), String> {
    ensure(fixture.press(Key::Escape, ModifierType::empty()), || {
        format!(
            "Escape after the deletion was not handled; focus on {}",
            describe_focus(fixture)
        )
    })?;
    ensure(
        settles(|| {
            fixture.view.listing_filter().is_none() && !fixture.view.results_replace_listing()
        }),
        || {
            format!(
                "Escape after the deletion did not close the filter; filter {:?}",
                fixture.view.listing_filter()
            )
        },
    )
}

fn delete_result_case(mode: BrowserMode, route: DeleteRoute) -> Result<(), String> {
    let fixture = filtered_fixture(mode, false);
    let field = show_filter_with_results(&fixture, "report", &IMMEDIATE_REPORTS)?;
    focus_first_result(&fixture, &field)?;
    let first_hit = hit_cursor(&fixture).expect("focused hit");
    let next_hit = fixture
        .view
        .filter_result_names()
        .into_iter()
        .find(|name| name != &first_hit)
        .expect("next hit");

    delete_focused_hit(&fixture, route)?;
    ensure(settles(|| result_has_focus(&fixture)), || {
        format!(
            "after deleting a result, focus was lost; focus on {}",
            describe_focus(&fixture)
        )
    })?;
    ensure(hit_cursor(&fixture).as_deref() == Some(&next_hit), || {
        format!(
            "focus moved to {:?} rather than next hit {next_hit:?}",
            hit_cursor(&fixture)
        )
    })?;
    escape_closes_filter(&fixture)
}

fn check_delete_cases(run: impl Fn(BrowserMode, DeleteRoute) -> Result<(), String>) {
    report(failures_of(
        DELETE_CASES,
        |(mode, route)| format!("{mode:?}, {route:?}"),
        |(mode, route)| run(mode, route),
    ));
}

#[test]
fn deleting_focused_result_moves_focus_to_next_result() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::live_changes::deleting_focused_result_moves_focus_to_next_result",
        || check_delete_cases(delete_result_case),
    );
}

fn field_or_its_text_focused(fixture: &KeyboardFixture, field: &gtk::Entry) -> bool {
    focus(fixture).is_some_and(|focused| {
        focused == *field.upcast_ref::<gtk::Widget>() || focused.is_ancestor(field)
    })
}

fn delete_only_result_case(mode: BrowserMode, route: DeleteRoute) -> Result<(), String> {
    let fixture = filtered_fixture(mode, false);
    let field = show_filter_with_results(&fixture, "gamma", &["gamma-report.md"])?;
    focus_first_result(&fixture, &field)?;

    delete_focused_hit(&fixture, route)?;
    ensure(
        settles(|| fixture.view.filter_has_focus() && field_or_its_text_focused(&fixture, &field)),
        || {
            format!(
                "after deleting the only result, focus was not in the filter field; focus on {}",
                describe_focus(&fixture)
            )
        },
    )?;
    escape_closes_filter(&fixture)
}

#[test]
fn deleting_only_result_moves_focus_to_filter_entry() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::live_changes::deleting_only_result_moves_focus_to_filter_entry",
        || check_delete_cases(delete_only_result_case),
    );
}
