// SPDX-License-Identifier: MIT

use super::*;

const ALL_MODES: [BrowserMode; 4] = [
    BrowserMode::Columns,
    BrowserMode::List,
    BrowserMode::Icons,
    BrowserMode::Compact,
];

/// The hit the selection holds, the cursor while focus is outside the results.
fn selected_hit(fixture: &KeyboardFixture) -> Option<String> {
    match fixture.view.selected_search_results()?.as_slice() {
        [hit] => Some(hit.display_name.clone()),
        _ => None,
    }
}

fn focus_shows_hit(fixture: &KeyboardFixture, hit: &str) -> bool {
    focus(fixture).is_some_and(|focused| focused.is_mapped() && shows_text(&focused, hit))
        && fixture.view.filter_focus() == Some(FilterFocus::Results)
}

/// Focus returned to results whose cursor moved while they lacked focus, as
/// Find moves it, lands on that hit rather than the row GTK last focused or
/// the hidden listing.
fn return_to_hit_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = filtered_fixture(mode, true);
    show_filter_with_results(&fixture, "report", &ALL_REPORTS)?;
    fixture.view.step_filter_results(1, 1, false);
    let first_hit = selected_hit(&fixture);
    fixture.view.step_filter_results(1, 1, false);
    let Some(cursor) = selected_hit(&fixture).filter(|hit| Some(hit) != first_hit.as_ref()) else {
        return Err(format!(
            "stepping without focus left the cursor on {:?}",
            selected_hit(&fixture)
        ));
    };

    // A closed dialog leaves the window without a focused widget.
    gtk::prelude::GtkWindowExt::set_focus(&fixture.window, None::<&gtk::Widget>);
    fixture.view.browser().focus_active();
    ensure(settles(|| focus_shows_hit(&fixture, &cursor)), || {
        format!(
            "focus returned to {} rather than the cursor hit {cursor:?}",
            describe_focus(&fixture)
        )
    })?;
    ensure(selected_hit(&fixture).as_ref() == Some(&cursor), || {
        format!(
            "returning focus changed the selection to {:?}",
            fixture.view.selected_search_results()
        )
    })
}

#[test]
fn returning_focus_lands_on_the_cursor_hit() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::returning_focus::returning_focus_lands_on_the_cursor_hit",
        || check_modes(&ALL_MODES, return_to_hit_case),
    );
}

fn editing(fixture: &KeyboardFixture) -> bool {
    focus(fixture).is_some_and(|focused| crate::ui::focus_navigation::editable(&focused))
}

/// A request to return focus to the listing leaves a hit's rename field focused
/// and the hit unrenamed.
fn rename_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = filtered_fixture(mode, true);
    let field = show_filter_with_results(&fixture, "report", &ALL_REPORTS)?;
    focus_first_result(&fixture, &field)?;
    let hit = hit_cursor(&fixture).expect("cursor hit");
    ensure(fixture.view.begin_rename(), || {
        format!("no rename began on {hit:?}")
    })?;
    ensure(settles(|| editing(&fixture)), || {
        format!(
            "the rename field never took focus; focus is on {}",
            describe_focus(&fixture)
        )
    })?;
    focus(&fixture)
        .and_then(|focused| focused.dynamic_cast::<gtk::Editable>().ok())
        .expect("rename field")
        .set_text("half-typed");
    fixture.view.browser().focus_active();
    pump(100);
    ensure(editing(&fixture), || {
        format!(
            "focus left the rename field for {}",
            describe_focus(&fixture)
        )
    })?;
    ensure(result_names(&fixture).contains(&hit), || {
        format!("{hit:?} was renamed; hits are {:?}", result_names(&fixture))
    })
}

#[test]
fn returning_focus_keeps_a_hit_rename() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::returning_focus::returning_focus_keeps_a_hit_rename",
        || check_modes(&ALL_MODES, rename_case),
    );
}

fn tab_from_result_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = filtered_fixture(mode, false);
    let field = show_filter_with_results(&fixture, "report", &IMMEDIATE_REPORTS)?;
    focus_first_result(&fixture, &field)?;
    let first_hit = hit_cursor(&fixture);

    tab(&fixture, gtk::DirectionType::TabForward);
    ensure(!result_has_focus(&fixture), || {
        format!(
            "Tab on a focused result stayed in the results view; focus on {}",
            describe_focus(&fixture)
        )
    })?;

    tab(&fixture, gtk::DirectionType::TabBackward);
    ensure(result_has_focus(&fixture), || {
        format!(
            "Shift+Tab did not return to the results; focus on {}",
            describe_focus(&fixture)
        )
    })?;
    ensure(hit_cursor(&fixture) == first_hit, || {
        format!(
            "Shift+Tab landed on {:?} rather than previous cursor {:?}",
            hit_cursor(&fixture),
            first_hit
        )
    })
}

#[test]
fn tab_from_focused_result_leaves_in_one_press() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::returning_focus::tab_from_focused_result_leaves_in_one_press",
        || {
            check_modes(
                &[BrowserMode::List, BrowserMode::Icons],
                tab_from_result_case,
            )
        },
    );
}
