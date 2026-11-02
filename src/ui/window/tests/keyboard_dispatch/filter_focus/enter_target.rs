// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug)]
enum Pick {
    /// Nothing picked: Enter opens the marked top result.
    None,
    /// Another result picked and focus back in the field: Enter opens the pick.
    Kept,
    /// Another result picked, then the query edited: Enter opens the new top.
    Retyped,
    /// Another result picked, then only the query's letter case changed: the filter
    /// matches the same rows, but Enter still opens the top again.
    Recased,
}

/// Presses `key` through the field's own key handlers, as a real press reaches them,
/// falling back to the field's `activate` for Enter.
fn press_in_field(field: &gtk::Entry, key: Key) {
    if !field_handles_key(field, key) && key == Key::Return {
        field.emit_activate();
    }
}

fn record_open_requests(browser: &crate::app::Browser) -> Rc<RefCell<Option<Location>>> {
    let opened = Rc::new(RefCell::new(None));
    let record = opened.clone();
    browser.observe(move |event| {
        if let BrowserEvent::OpenRequested { location } = event {
            record.replace(Some(location.clone()));
        }
    });
    opened
}

/// The name of what Enter in `field` opened: a launched file, or the folder it entered.
fn enter_in_field(fixture: &KeyboardFixture, field: &gtk::Entry) -> Result<String, String> {
    let browser = fixture.view.browser();
    let opened = record_open_requests(&browser);
    let before = browser.active_location();
    press_in_field(field, Key::Return);
    settles(|| opened.borrow().is_some() || browser.active_location() != before);
    let location = opened
        .borrow()
        .clone()
        .or_else(|| {
            browser
                .active_location()
                .filter(|now| Some(now) != before.as_ref())
        })
        .ok_or_else(|| "Enter in the field opened nothing".to_owned())?;
    Ok(location
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default())
}

fn enter_opens(fixture: &KeyboardFixture, field: &gtk::Entry, wanted: &str) -> Result<(), String> {
    let opened = enter_in_field(fixture, field)?;
    ensure(opened == wanted, || {
        format!("Enter opened {opened}, not {wanted}")
    })
}

fn ctrl_f_returns_to_the_field(fixture: &KeyboardFixture) -> bool {
    fixture.press(Key::f, ModifierType::CONTROL_MASK) && settles(|| fixture.view.filter_has_focus())
}

/// Moves off the focused top result, then returns to the field; returns the pick.
fn pick_another(fixture: &KeyboardFixture, top: &str) -> Result<String, String> {
    fixture.view.step_filter_results(1, 1, true);
    let landed = || hit_cursor(fixture).filter(|name| name != top);
    ensure(settles(|| landed().is_some()), || {
        format!(
            "moving from the top result stayed on {:?}",
            hit_cursor(fixture)
        )
    })?;
    let picked = landed().unwrap_or_default();
    ensure(ctrl_f_returns_to_the_field(fixture), || {
        format!(
            "Ctrl+F did not return to the field; focus is on {}",
            describe_focus(fixture)
        )
    })?;
    Ok(picked)
}

fn nothing_marked(fixture: &KeyboardFixture) -> bool {
    marked_rows(fixture.window.upcast_ref()).is_empty()
}

/// Down from the field lands on the marked top, then another result is picked.
fn down_then_pick_another(
    fixture: &KeyboardFixture,
    field: &gtk::Entry,
    top: &str,
) -> Result<String, String> {
    focus_first_result(fixture, field)?;
    ensure(hit_cursor(fixture).as_deref() == Some(top), || {
        format!(
            "Down from the field landed on {:?}, not the marked top {top}",
            hit_cursor(fixture)
        )
    })?;
    ensure(settles(|| nothing_marked(fixture)), || {
        format!("a focused result left {:?} marked", marked_names(fixture))
    })?;
    let picked = pick_another(fixture, top)?;
    pump(100);
    ensure(nothing_marked(fixture), || {
        format!("the pick {picked} left {:?} marked", marked_names(fixture))
    })?;
    Ok(picked)
}

/// Editing the query drops the pick and marks the new top, which it returns.
fn requery_drops_the_pick(fixture: &KeyboardFixture, field: &gtk::Entry) -> Result<String, String> {
    field.set_text("repo");
    ensure(
        settles(|| {
            fixture
                .view
                .selected_search_results()
                .is_some_and(|selected| selected.is_empty())
        }),
        || "editing the query kept the pick selected".to_owned(),
    )?;
    marks_only_top(fixture)?;
    Ok(fixture.view.filter_result_names()[0].clone())
}

fn enter_target_case(mode: BrowserMode, recursive: bool, pick: Pick) -> Result<(), String> {
    let fixture = filtered_fixture(mode, recursive);
    let field = show_filter_with_results(&fixture, "report", expected_reports(recursive))?;
    marks_only_top(&fixture)?;
    let top = fixture.view.filter_result_names()[0].clone();
    if let Pick::None = pick {
        let opened = enter_in_field(&fixture, &field)?;
        return ensure(opened == top, || {
            format!("Enter opened {opened}, not the marked top {top}")
        });
    }
    let picked = down_then_pick_another(&fixture, &field, &top)?;
    let wanted = match pick {
        Pick::Kept => picked,
        _ => requery_drops_the_pick(&fixture, &field)?,
    };
    enter_opens(&fixture, &field, &wanted)
}

/// The visible list or grid narrowed in place, found from the window since the field
/// keeps focus.
fn narrowed_rows(widget: &gtk::Widget) -> Option<gtk::SelectionModel> {
    let is_view = widget.is::<gtk::ListView>() || widget.is::<gtk::GridView>();
    if is_view && widget.is_mapped() {
        return widget.property::<Option<gtk::SelectionModel>>("model");
    }
    children(widget).find_map(|child| narrowed_rows(&child))
}

/// A query marks the top row without touching the listing's selection.
fn query_marks_the_top_row(fixture: &KeyboardFixture) -> Result<gtk::Entry, String> {
    ensure(ctrl_f_focuses_the_field(fixture), || {
        "Ctrl+F did not focus the field".to_owned()
    })?;
    let field = focused_entry(&fixture.window);
    field.set_text("t");
    ensure(settles(|| only_marked(fixture, "alpha-report.txt")), || {
        format!(
            "expected only the top row alpha-report.txt marked; {} rows marked",
            marked_rows(fixture.window.upcast_ref()).len()
        )
    })?;
    ensure(fixture.selected() == [2], || {
        format!(
            "the query changed the listing's selection to {:?}",
            fixture.selected()
        )
    })?;
    Ok(field)
}

/// After pointer motion, Down from the field takes the pane back to keyboard mode on
/// the marked top row.
fn down_reclaims_the_pane_from_the_pointer(
    fixture: &KeyboardFixture,
    field: &gtk::Entry,
) -> Result<(), String> {
    let keyboard_navigation = || fixture.view.widget().has_css_class("keyboard-navigation");
    fixture.view.record_pointer_hover((12.0, 24.0), Some(0));
    ensure(!keyboard_navigation(), || {
        "pointer motion did not take over the pane".to_owned()
    })?;
    press_in_field(field, Key::Down);
    ensure(
        settles(|| result_has_focus(fixture) && fixture.selected() == [0]),
        || {
            format!(
                "Down from the field did not land on the marked top row: selection {:?}, \
                 focus on {}",
                fixture.selected(),
                describe_focus(fixture)
            )
        },
    )?;
    ensure(keyboard_navigation(), || {
        "Down from the field left the pane in pointer mode".to_owned()
    })
}

fn retype_query(
    fixture: &KeyboardFixture,
    field: &gtk::Entry,
    edit: &str,
) -> Result<&'static str, String> {
    field.set_text(edit);
    ensure(
        settles(|| marked_rows(fixture.window.upcast_ref()).len() == 1),
        || format!("editing the query to {edit:?} did not mark the top row again"),
    )?;
    Ok("alpha-report.txt")
}

/// Picks notes.txt in the narrowed listing; returns what Enter must then open.
fn pick_in_place(
    fixture: &KeyboardFixture,
    field: &gtk::Entry,
    pick: Pick,
) -> Result<&'static str, String> {
    let rows = narrowed_rows(fixture.window.upcast_ref())
        .ok_or_else(|| "no narrowed listing is shown".to_owned())?;
    let kept = matches!(pick, Pick::Kept);
    if kept {
        down_reclaims_the_pane_from_the_pointer(fixture, field)?;
    }
    rows.select_item(1, true);
    ensure(settles(|| nothing_marked(fixture)), || {
        "picking notes.txt left the top row marked".to_owned()
    })?;
    ensure(!kept || ctrl_f_returns_to_the_field(fixture), || {
        "Ctrl+F did not return to the field".to_owned()
    })?;
    match pick {
        Pick::Retyped => retype_query(fixture, field, "txt"),
        Pick::Recased => retype_query(fixture, field, "T"),
        _ => Ok("notes.txt"),
    }
}

/// Without a native folder the filter narrows the listing in place; its selection is the
/// listing's own, so a query keeps it but stops it counting as the pick.
fn in_place_enter_target_case(mode: BrowserMode, pick: Pick) -> Result<(), String> {
    let fixture = remote_share_fixture(mode);
    fixture.view.browser().select(0, 2);
    focus_files(&fixture);
    let field = query_marks_the_top_row(&fixture)?;
    let wanted = match pick {
        Pick::None => "alpha-report.txt",
        Pick::Kept | Pick::Retyped | Pick::Recased => pick_in_place(&fixture, &field, pick)?,
    };
    enter_opens(&fixture, &field, wanted)
}

#[derive(Clone, Copy)]
enum EnterCase {
    Native(BrowserMode, bool, Pick),
    RemoteShare(BrowserMode, Pick),
}

fn enter_cases() -> Vec<EnterCase> {
    let mut cases = Vec::new();
    for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
        for pick in [Pick::None, Pick::Kept, Pick::Retyped] {
            cases.extend([false, true].map(|recursive| EnterCase::Native(mode, recursive, pick)));
        }
        // Only the in-place filter skips a case-only edit, which leaves its
        // folded query unchanged.
        cases.extend(
            [Pick::None, Pick::Kept, Pick::Retyped, Pick::Recased]
                .map(|pick| EnterCase::RemoteShare(mode, pick)),
        );
    }
    cases
}

fn label_enter_case(case: EnterCase) -> String {
    match case {
        EnterCase::Native(mode, recursive, pick) => {
            format!("{mode:?}, subfolders {recursive}, {pick:?}")
        }
        EnterCase::RemoteShare(mode, pick) => format!("{mode:?}, remote share, {pick:?}"),
    }
}

fn run_enter_case(case: EnterCase) -> Result<(), String> {
    match case {
        EnterCase::Native(mode, recursive, pick) => enter_target_case(mode, recursive, pick),
        EnterCase::RemoteShare(mode, pick) => in_place_enter_target_case(mode, pick),
    }
}

#[test]
fn enter_in_the_filter_opens_the_marked_top_result_until_a_pick() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::enter_target::enter_in_the_filter_opens_the_marked_top_result_until_a_pick",
        || report(failures_of(enter_cases(), label_enter_case, run_enter_case)),
    );
}
