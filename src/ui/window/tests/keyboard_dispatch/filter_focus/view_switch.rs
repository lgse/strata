// SPDX-License-Identifier: MIT

use super::*;

/// Where focus was before an action: in the pane filter's field, on one of its results,
/// or on a listing with no filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    Entry,
    Result,
    Listing,
}

/// How the view is switched: Ctrl+digit keeps focus where it is, the View menu takes it
/// into its popover first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Keys,
    Menu,
}

fn digit(mode: BrowserMode) -> Key {
    match mode {
        BrowserMode::Columns => Key::_1,
        BrowserMode::Icons => Key::_2,
        BrowserMode::List => Key::_3,
        BrowserMode::Compact => Key::_4,
    }
}

fn mode_label(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Columns => "Columns",
        BrowserMode::Icons => "Icons",
        BrowserMode::List => "List",
        BrowserMode::Compact => "Compact",
    }
}

fn descendant(
    widget: &gtk::Widget,
    matches: &impl Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if matches(widget) {
        return Some(widget.clone());
    }
    children(widget).find_map(|child| descendant(&child, matches))
}

fn is_appearance_option(widget: &gtk::Widget, label: &str) -> bool {
    widget.is::<gtk::Button>()
        && widget.has_css_class("appearance-option")
        && shows_text(widget, label)
}

/// Picks `to` from the Appearance menu the way a click does: focus moves into the menu,
/// which closes after the switch.
fn switch_from_menu(fixture: &KeyboardFixture, to: BrowserMode) -> Result<(), String> {
    let menu = build_appearance_menu(
        &fixture.view,
        &fixture.view.browser(),
        PreferenceManager::shared(),
        &fixture.preview,
    );
    fixture.overlay.add_overlay(&menu);
    menu.popup();
    let Some(popover) = menu
        .popover()
        .filter(|popover| settles(|| popover.is_visible()))
    else {
        return Err("the Appearance menu did not open".to_owned());
    };
    let Some(option) = descendant(popover.upcast_ref(), &|widget| {
        is_appearance_option(widget, mode_label(to))
    })
    .and_downcast::<gtk::Button>() else {
        return Err(format!("the menu has no {} option", mode_label(to)));
    };
    option.grab_focus();
    option.emit_clicked();
    Ok(())
}

fn open_filter_at(fixture: &KeyboardFixture, start: Start) -> Result<(), String> {
    if start == Start::Listing {
        return Ok(());
    }
    let field = show_filter_with_results(fixture, "report", &IMMEDIATE_REPORTS)?;
    if start == Start::Result {
        focus_first_result(fixture, &field)?;
    }
    Ok(())
}

fn switch_view(fixture: &KeyboardFixture, to: BrowserMode, route: Route) -> Result<(), String> {
    match route {
        Route::Keys => ensure(fixture.press(digit(to), ModifierType::CONTROL_MASK), || {
            "Ctrl+digit was not handled".to_owned()
        })?,
        Route::Menu => switch_from_menu(fixture, to)?,
    }
    ensure(settles(|| fixture.view.view_mode() == to), || {
        "the view did not switch".to_owned()
    })
}

fn focus_held(fixture: &KeyboardFixture, start: Start) -> bool {
    match start {
        Start::Entry => fixture.view.filter_has_focus(),
        Start::Result => focused_hit(fixture),
        Start::Listing => {
            fixture.view.item_view_has_focus() && fixture.view.filter_focus().is_none()
        }
    }
}

fn caret_at_query_end(field: &gtk::Entry) -> bool {
    field.text() == "report" && field.selection_bounds().is_none() && field.position() == 6
}

fn field_keeps_the_query(fixture: &KeyboardFixture) -> Result<(), String> {
    let field = focused_entry(&fixture.window);
    ensure(caret_at_query_end(&field), || {
        format!(
            "the field reads {:?} with selection {:?} and caret {}; expected the query with \
             the caret at its end",
            field.text(),
            field.selection_bounds(),
            field.position()
        )
    })
}

fn filter_survives(fixture: &KeyboardFixture) -> bool {
    fixture.view.listing_filter().as_deref() == Some("report")
        && settles(|| filter_applied(fixture, &IMMEDIATE_REPORTS))
}

fn marks_after_switch(fixture: &KeyboardFixture, start: Start) -> Result<(), String> {
    match start {
        Start::Entry => marks_only_top(fixture),
        Start::Result => ensure(marked_rows(fixture.window.upcast_ref()).is_empty(), || {
            "a focused result left the Enter mark showing".to_owned()
        }),
        Start::Listing => Ok(()),
    }
}

fn switch_case(
    from: BrowserMode,
    to: BrowserMode,
    start: Start,
    route: Route,
) -> Result<(), String> {
    let fixture = filtered_fixture(from, false);
    open_filter_at(&fixture, start)?;
    switch_view(&fixture, to, route)?;
    ensure(settles(|| focus_held(&fixture, start)), || {
        format!(
            "focus did not stay with the {start:?} after the switch; it is on {}",
            describe_focus(&fixture)
        )
    })?;
    if start == Start::Entry {
        field_keeps_the_query(&fixture)?;
    }
    // Past the filter debounce, result arrival, the menu's idle and idle refocus.
    pump(400);
    ensure(focus_held(&fixture, start), || {
        format!(
            "focus was taken from the {start:?} after the switch settled; it is on {}",
            describe_focus(&fixture)
        )
    })?;
    ensure(start == Start::Listing || filter_survives(&fixture), || {
        format!(
            "the switch lost the filter: {:?}, results {:?}",
            fixture.view.listing_filter(),
            result_names(&fixture)
        )
    })?;
    marks_after_switch(&fixture, start)
}

#[test]
fn switching_view_mode_keeps_focus_on_the_filter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::view_switch::switching_view_mode_keeps_focus_on_the_filter",
        || {
            use BrowserMode::{Columns, Compact, Icons, List};
            let mut cases = Vec::new();
            for (from, to) in [
                (Columns, List),
                (List, Icons),
                (Icons, Columns),
                (Columns, Icons),
                (Icons, Compact),
                (Compact, List),
            ] {
                for start in [Start::Entry, Start::Result] {
                    cases.push((from, to, start, Route::Keys));
                }
            }
            // The menu takes focus from the field, so only results can keep it.
            for (from, to) in [
                (List, Icons),
                (Icons, List),
                (Columns, List),
                (List, Compact),
            ] {
                cases.push((from, to, Start::Result, Route::Menu));
            }
            cases.push((List, Icons, Start::Listing, Route::Keys));
            report(failures_of(
                cases,
                |(from, to, start, route)| format!("{from:?} -> {to:?}, {start:?}, {route:?}"),
                |(from, to, start, route)| switch_case(from, to, start, route),
            ));
        },
    );
}
