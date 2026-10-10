// SPDX-License-Identifier: MIT

use super::footer_prompt::{ALL_REPORTS, IMMEDIATE_REPORTS, hit_cursor, seed_filter_tree};
use super::*;
use crate::{
    adapters::LocalFileSource,
    model::{EntryKind, FileEntry, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LocationValidationError},
    ui::browser::FilterFocus,
};

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

fn settles(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        if Instant::now() >= deadline {
            return false;
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}

fn focus(fixture: &KeyboardFixture) -> Option<gtk::Widget> {
    gtk::prelude::RootExt::focus(&fixture.window)
}

fn describe_focus(fixture: &KeyboardFixture) -> String {
    match focus(fixture) {
        None => "nothing".to_owned(),
        Some(focused) => format!(
            "{} {:?} (mapped: {})",
            focused.type_().name(),
            focused.css_classes(),
            focused.is_mapped()
        ),
    }
}

fn digit(mode: BrowserMode) -> Key {
    match mode {
        BrowserMode::Columns => Key::_1,
        BrowserMode::Icons => Key::_2,
        BrowserMode::List => Key::_3,
    }
}

fn result_names(fixture: &KeyboardFixture) -> Vec<String> {
    let mut names = fixture.view.filter_result_names();
    names.sort();
    names
}

/// A Columns filter without Include subfolders narrows the column in place, so it has
/// no separate result names.
fn filter_applied(fixture: &KeyboardFixture, expected: &[&str]) -> bool {
    if fixture.view.view_mode() == BrowserMode::Columns
        && !PreferenceManager::shared().filter_include_subfolders()
    {
        return fixture.view.selected_search_results().is_some();
    }
    result_names(fixture) == expected
}

fn result_has_focus(fixture: &KeyboardFixture) -> bool {
    focus(fixture).is_some_and(|focused| focused.is_mapped())
        && fixture.view.filter_focus() == Some(FilterFocus::Results)
}

/// Ctrl+F, then `query` typed into the field; returns the field once the hits show.
fn show_filter_with_results(
    fixture: &KeyboardFixture,
    query: &str,
    expected: &[&str],
) -> Result<gtk::Entry, String> {
    if !fixture.press(Key::f, ModifierType::CONTROL_MASK) || !fixture.view.filter_has_focus() {
        return Err(format!(
            "Ctrl+F did not focus the field; focus is on {}",
            describe_focus(fixture)
        ));
    }
    let field = focused_entry(&fixture.window);
    field.set_text(query);
    if !settles(|| filter_applied(fixture, expected)) {
        return Err(format!(
            "filter results {:?} never became {expected:?}",
            result_names(fixture)
        ));
    }
    // An in-place Columns filter narrows the rows before its debounced query settles.
    pump(300);
    Ok(field)
}

/// Down from the field, through the field's own key handlers as a real key press: Icons
/// and List focus the results view, Columns the column's filtered rows.
fn focus_first_result(fixture: &KeyboardFixture, field: &gtk::Entry) -> Result<(), String> {
    let controllers = field.observe_controllers();
    let handled = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .any(|keys| {
            keys.emit_by_name::<bool>("key-pressed", &[&Key::Down, &0u32, &ModifierType::empty()])
        });
    if handled && settles(|| result_has_focus(fixture) && hit_cursor(fixture).is_some()) {
        Ok(())
    } else {
        Err(format!(
            "Down from the field did not focus a result (handled {handled}); focus is on {}",
            describe_focus(fixture)
        ))
    }
}

fn filtered_fixture(mode: BrowserMode, recursive: bool) -> KeyboardFixture {
    let fixture = KeyboardFixture::new();
    PreferenceManager::shared().set_filter_include_subfolders(recursive);
    seed_filter_tree(&fixture);
    fixture.view.set_view_mode(mode);
    focus_files(&fixture);
    fixture.view.browser().select(0, 0);
    focus_files(&fixture);
    fixture
}

/// The selection model of the focused list or grid.
fn shown_model(fixture: &KeyboardFixture) -> Option<gtk::SelectionModel> {
    let focused = focus(fixture)?;
    let view = [gtk::ListView::static_type(), gtk::GridView::static_type()]
        .into_iter()
        .find_map(|kind| {
            focused
                .type_()
                .is_a(kind)
                .then(|| focused.clone())
                .or_else(|| focused.ancestor(kind))
        })?;
    view.property::<Option<gtk::SelectionModel>>("model")
}

/// The selection GTK shows in the focused list or grid, in view positions.
fn shown_selection(fixture: &KeyboardFixture) -> Option<Vec<usize>> {
    let model = shown_model(fixture)?;
    let selected = model.selection();
    Some(
        (0..model.n_items())
            .filter(|position| selected.contains(*position))
            .map(|position| position as usize)
            .collect(),
    )
}

fn shows_text(widget: &gtk::Widget, text: &str) -> bool {
    widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.text() == text)
        || widget
            .downcast_ref::<gtk::Inscription>()
            .is_some_and(|label| label.text().as_deref() == Some(text))
        || std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
            .any(|child| shows_text(&child, text))
}

/// GTK's selection matches the browser's, and focus is on the browser's cursor row
/// (with the Columns cursor ring).
fn listing_matches_browser(fixture: &KeyboardFixture) -> Result<(), String> {
    let shown = shown_selection(fixture);
    if shown.as_deref() != Some(fixture.selected().as_slice()) {
        return Err(format!(
            "the listing shows selection {shown:?} but the browser holds {:?}",
            fixture.selected()
        ));
    }
    let cursor = fixture
        .view
        .browser()
        .focused_entry()
        .map(|entry| entry.display_name);
    let focused = focus(fixture);
    let on_cursor = cursor
        .as_deref()
        .zip(focused.as_ref())
        .is_some_and(|(name, focused)| shows_text(focused, name));
    let ring = fixture.view.view_mode() != BrowserMode::Columns
        || focused.as_ref().is_some_and(|focused| {
            focused.has_css_class("keyboard-cursor")
                || std::iter::successors(focused.first_child(), gtk::Widget::next_sibling)
                    .any(|child| child.has_css_class("keyboard-cursor"))
        });
    if !on_cursor || !ring {
        return Err(format!(
            "focus is on {} rather than the cursor row {cursor:?} (ring shown: {ring})",
            describe_focus(fixture)
        ));
    }
    Ok(())
}

fn report(failures: Vec<String>) {
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn escape_case(mode: BrowserMode, recursive: bool) -> Result<(), String> {
    let fixture = filtered_fixture(mode, recursive);
    let expected: &[&str] = if recursive {
        &ALL_REPORTS
    } else {
        &IMMEDIATE_REPORTS
    };
    let field = show_filter_with_results(&fixture, "report", expected)?;
    focus_first_result(&fixture, &field)?;
    let separate_results = fixture.view.results_replace_listing();
    let directory_selection = fixture.selected();

    if !fixture.press(Key::Escape, ModifierType::empty()) {
        return Err(format!(
            "Escape on a focused result was not handled; filter {:?}, focus on {}",
            fixture.view.listing_filter(),
            describe_focus(&fixture)
        ));
    }
    if !settles(|| {
        fixture.view.listing_filter().is_none() && !fixture.view.results_replace_listing()
    }) {
        return Err(format!(
            "Escape on a focused result left the filter {:?} (results replace the listing: {})",
            fixture.view.listing_filter(),
            fixture.view.results_replace_listing()
        ));
    }
    if !settles(|| fixture.view.item_view_has_focus() && fixture.view.filter_focus().is_none()) {
        return Err(format!(
            "focus did not return to the listing; it is on {}",
            describe_focus(&fixture)
        ));
    }
    if separate_results && fixture.selected() != directory_selection {
        return Err(format!(
            "dismissal changed the directory selection from {directory_selection:?} to {:?}",
            fixture.selected()
        ));
    }
    if !settles(|| listing_matches_browser(&fixture).is_ok()) {
        listing_matches_browser(&fixture)?;
    }
    if !fixture.press(Key::Escape, ModifierType::empty())
        || !settles(|| fixture.selected().is_empty())
    {
        return Err("a second Escape did not clear the restored selection".to_owned());
    }
    Ok(())
}

const SHARE: &str = "sftp://share.invalid/reports";

/// A remote share: it has no native path, so Icons and List narrow it in place instead
/// of showing separate results.
struct RemoteShare;

impl FileSource for RemoteShare {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if location.native_path().is_some() {
            return LocalFileSource.validate_location(location);
        }
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if request.location.native_path().is_some() {
            return LocalFileSource.enumerate(request, emit);
        }
        let entries = ["alpha-report.txt", "notes.txt", "zeta.txt"]
            .into_iter()
            .map(|name| FileEntry {
                location: Location::uri(format!("{SHARE}/{name}")),
                thumbnail_path: None,
                native_name: name.into(),
                display_name: name.into(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                recent_unix_seconds: MetadataValue::Unknown,
                recent_uri: None,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
                is_hidden: false,
            })
            .collect();
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries,
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

fn in_place_escape_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = KeyboardFixture::with_parts(Rc::new(TextPreview), || {
        BrowserView::new(
            Rc::new(RemoteShare),
            crate::ui::browser::PeekBehavior::default(),
        )
    });
    fixture.view.set_view_mode(mode);
    let browser = fixture.view.browser();
    let share = Location::uri(SHARE);
    browser.navigate(share.clone());
    wait_until(|| browser.active_location().as_ref() == Some(&share));
    wait_loaded(&browser, 0);
    browser.select(0, 0);
    focus_files(&fixture);
    if !fixture.press(Key::f, ModifierType::CONTROL_MASK) || !fixture.view.filter_has_focus() {
        return Err("Ctrl+F did not focus the field".to_owned());
    }
    focused_entry(&fixture.window).set_text("report");
    // Past the debounce that narrows the rows.
    pump(400);
    browser.focus_active();
    if !settles(|| fixture.view.filter_focus() == Some(FilterFocus::Results)) {
        return Err(format!(
            "a narrowed row did not count as a result; focus is on {}",
            describe_focus(&fixture)
        ));
    }

    if !fixture.press(Key::Escape, ModifierType::empty()) {
        return Err(format!(
            "Escape on a narrowed row was not handled; focus is on {}",
            describe_focus(&fixture)
        ));
    }
    if !settles(|| {
        fixture.view.listing_filter().is_none()
            && fixture.view.filter_focus().is_none()
            && fixture.view.item_view_has_focus()
            && shown_model(&fixture).is_some_and(|rows| rows.n_items() == 3)
    }) {
        return Err(format!(
            "Escape left the filter {:?}, the listing showing {:?} rows, focus on {}",
            fixture.view.listing_filter(),
            shown_model(&fixture).map(|rows| rows.n_items()),
            describe_focus(&fixture)
        ));
    }
    if !settles(|| listing_matches_browser(&fixture).is_ok()) {
        listing_matches_browser(&fixture)?;
    }
    Ok(())
}

#[test]
fn escape_dismisses_the_filter_from_a_focused_result() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::escape_dismisses_the_filter_from_a_focused_result",
        || {
            let mut failures = Vec::new();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for recursive in [false, true] {
                    if let Err(error) = escape_case(mode, recursive) {
                        failures.push(format!("{mode:?}, subfolders {recursive}: {error}"));
                    }
                }
            }
            for mode in [BrowserMode::List, BrowserMode::Icons] {
                if let Err(error) = in_place_escape_case(mode) {
                    failures.push(format!("{mode:?}, remote share: {error}"));
                }
            }
            report(failures);
        },
    );
}

fn preview_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = filtered_fixture(mode, false);
    let expected: &[&str] = &["alpha-report.txt"];
    let field = show_filter_with_results(&fixture, "alpha", expected)?;
    focus_first_result(&fixture, &field)?;
    if !fixture.press(Key::space, ModifierType::empty())
        || !settles(|| fixture.preview.is_enabled())
    {
        return Err("Space on a focused result did not open the preview".to_owned());
    }
    let handled = fixture.press(Key::Escape, ModifierType::empty());
    pump(100);
    if !handled || fixture.view.listing_filter().is_some() || !fixture.preview.is_enabled() {
        return Err(format!(
            "the first Escape should dismiss the filter and keep the preview: handled {handled}, \
             filter {:?}, preview open {}",
            fixture.view.listing_filter(),
            fixture.preview.is_enabled()
        ));
    }
    if !fixture.press(Key::Escape, ModifierType::empty())
        || !settles(|| !fixture.preview.is_enabled())
    {
        return Err("the second Escape did not close the preview".to_owned());
    }
    Ok(())
}

#[test]
fn escape_from_a_focused_result_dismisses_the_filter_before_the_preview() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::escape_from_a_focused_result_dismisses_the_filter_before_the_preview",
        || {
            let failures = [BrowserMode::Columns, BrowserMode::List]
                .into_iter()
                .filter_map(|mode| {
                    preview_case(mode)
                        .err()
                        .map(|error| format!("{mode:?}: {error}"))
                })
                .collect();
            report(failures);
        },
    );
}

fn mode_label(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Columns => "Columns",
        BrowserMode::Icons => "Icons",
        BrowserMode::List => "List",
    }
}

fn descendant(
    widget: &gtk::Widget,
    matches: &impl Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if matches(widget) {
        return Some(widget.clone());
    }
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
        .find_map(|child| descendant(&child, matches))
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
        widget.is::<gtk::Button>()
            && widget.has_css_class("appearance-option")
            && shows_text(widget, mode_label(to))
    })
    .and_downcast::<gtk::Button>() else {
        return Err(format!("the menu has no {} option", mode_label(to)));
    };
    option.grab_focus();
    option.emit_clicked();
    Ok(())
}

fn switch_case(
    from: BrowserMode,
    to: BrowserMode,
    start: Start,
    route: Route,
) -> Result<(), String> {
    let fixture = filtered_fixture(from, false);
    if start != Start::Listing {
        let field = show_filter_with_results(&fixture, "report", &IMMEDIATE_REPORTS)?;
        if start == Start::Result {
            focus_first_result(&fixture, &field)?;
        }
    }

    match route {
        Route::Keys => {
            if !fixture.press(digit(to), ModifierType::CONTROL_MASK) {
                return Err("Ctrl+digit was not handled".to_owned());
            }
        }
        Route::Menu => switch_from_menu(&fixture, to)?,
    }
    if !settles(|| fixture.view.view_mode() == to) {
        return Err("the view did not switch".to_owned());
    }
    let held = |fixture: &KeyboardFixture| match start {
        Start::Entry => fixture.view.filter_has_focus(),
        Start::Result => result_has_focus(fixture) && hit_cursor(fixture).is_some(),
        Start::Listing => {
            fixture.view.item_view_has_focus() && fixture.view.filter_focus().is_none()
        }
    };
    if !settles(|| held(&fixture)) {
        return Err(format!(
            "focus did not stay with the {start:?} after the switch; it is on {}",
            describe_focus(&fixture)
        ));
    }
    if start == Start::Entry {
        let field = focused_entry(&fixture.window);
        if field.text() != "report" || field.selection_bounds().is_some() || field.position() != 6 {
            return Err(format!(
                "the field reads {:?} with selection {:?} and caret {}; expected the query with \
                 the caret at its end",
                field.text(),
                field.selection_bounds(),
                field.position()
            ));
        }
    }
    // Past the filter debounce, result arrival, the menu's idle and idle refocus.
    pump(400);
    if !held(&fixture) {
        return Err(format!(
            "focus was taken from the {start:?} after the switch settled; it is on {}",
            describe_focus(&fixture)
        ));
    }
    if start != Start::Listing
        && (fixture.view.listing_filter().as_deref() != Some("report")
            || !settles(|| filter_applied(&fixture, &IMMEDIATE_REPORTS)))
    {
        return Err(format!(
            "the switch lost the filter: {:?}, results {:?}",
            fixture.view.listing_filter(),
            result_names(&fixture)
        ));
    }
    Ok(())
}

#[test]
fn switching_view_mode_keeps_focus_on_the_filter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::switching_view_mode_keeps_focus_on_the_filter",
        || {
            use BrowserMode::{Columns, Icons, List};
            let mut cases = Vec::new();
            for (from, to) in [
                (Columns, List),
                (List, Icons),
                (Icons, Columns),
                (Columns, Icons),
            ] {
                for start in [Start::Entry, Start::Result] {
                    cases.push((from, to, start, Route::Keys));
                }
            }
            // The menu takes focus from the field, so only results can keep it.
            for (from, to) in [(List, Icons), (Icons, List), (Columns, List)] {
                cases.push((from, to, Start::Result, Route::Menu));
            }
            cases.push((List, Icons, Start::Listing, Route::Keys));
            let failures = cases
                .into_iter()
                .filter_map(|(from, to, start, route)| {
                    switch_case(from, to, start, route)
                        .err()
                        .map(|error| format!("{from:?} -> {to:?}, {start:?}, {route:?}: {error}"))
                })
                .collect();
            report(failures);
        },
    );
}

/// More than the browser publishes inline, so the rows after the first 128 arrive in
/// an idle tail and the load completes after Ctrl+F.
const STAGED_ENTRIES: usize = 600;

fn staged_load_case(mode: BrowserMode) -> Result<(), String> {
    let fixture = KeyboardFixture::new();
    let big = fixture._directory.path().join("big");
    std::fs::create_dir(&big).expect("staged folder");
    for index in 0..STAGED_ENTRIES {
        std::fs::write(big.join(format!("file-{index:04}.txt")), b"x").expect("staged file");
    }
    fixture.view.set_view_mode(mode);
    focus_files(&fixture);
    let browser = fixture.view.browser();
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
    let big = Location::local(&big);
    browser.navigate(big.clone());
    wait_until(|| first_publish.get());
    if !fixture.press(Key::f, ModifierType::CONTROL_MASK) || !fixture.view.filter_has_focus() {
        return Err(format!(
            "Ctrl+F during the load did not focus the field; focus is on {}",
            describe_focus(&fixture)
        ));
    }
    // The load reports done before its idle tail completes the publication.
    wait_until(|| finished.get());
    wait_until(|| entry_count(&browser) == STAGED_ENTRIES);
    pump(100);
    if !fixture.view.filter_has_focus() || fixture.view.item_view_has_focus() {
        return Err(format!(
            "the load took focus from the field; it is on {}",
            describe_focus(&fixture)
        ));
    }
    fixture.press(Key::a, ModifierType::CONTROL_MASK);
    if fixture.selected() != [0] {
        return Err(format!(
            "Ctrl+A selected {} hidden rows instead of the field text",
            fixture.selected().len()
        ));
    }
    fixture.press(Key::BackSpace, ModifierType::empty());
    pump(100);
    if browser.active_location().as_ref() != Some(&big) {
        return Err("Backspace left the folder instead of editing the field".to_owned());
    }
    Ok(())
}

#[test]
fn staged_load_completion_keeps_focus_in_the_filter_entry() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::staged_load_completion_keeps_focus_in_the_filter_entry",
        || {
            let failures = [BrowserMode::Icons, BrowserMode::List]
                .into_iter()
                .filter_map(|mode| {
                    staged_load_case(mode)
                        .err()
                        .map(|error| format!("{mode:?}: {error}"))
                })
                .collect();
            report(failures);
        },
    );
}

/// Another program deletes the cursor's file, one of the hits, while the pane filter's
/// field has focus.
fn external_delete_case(mode: BrowserMode, recursive: bool) -> Result<(), String> {
    let fixture = filtered_fixture(mode, recursive);
    let browser = fixture.view.browser();
    select_named(&fixture, "alpha-report.txt");
    let expected: &[&str] = if recursive {
        &ALL_REPORTS
    } else {
        &IMMEDIATE_REPORTS
    };
    let field = show_filter_with_results(&fixture, "report", expected)?;
    std::fs::remove_file(fixture._directory.path().join("alpha-report.txt"))
        .expect("delete the hit");
    if !settles(|| entry_count(&browser) == 6) {
        return Err("the monitor never removed the deleted file from the listing".to_owned());
    }
    // Focus that must stay put has no settle condition.
    pump(300);
    let mut failures = Vec::new();
    if !fixture.view.filter_has_focus() {
        failures.push(format!(
            "the deletion took focus from the field to {}",
            describe_focus(&fixture)
        ));
    }
    if field.text() != "report" {
        failures.push(format!("the filter text became {:?}", field.text()));
    }
    // An in-place Columns filter narrows the listing itself, which drops the row.
    let separate_results = !(mode == BrowserMode::Columns && !recursive);
    if separate_results && !settles(|| result_names(&fixture) == expected[1..]) {
        failures.push(format!(
            "the hits are {:?}, not {:?}",
            result_names(&fixture),
            &expected[1..]
        ));
    }
    match focus_first_result(&fixture, &field) {
        Ok(()) if hit_cursor(&fixture).as_deref() == Some("alpha-report.txt") => {
            failures.push("Down focused the deleted hit".to_owned());
        }
        Ok(()) => {}
        Err(error) => failures.push(error),
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[test]
fn an_outside_deletion_keeps_focus_in_the_filter_entry() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::an_outside_deletion_keeps_focus_in_the_filter_entry",
        || {
            let mut failures = Vec::new();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for recursive in [false, true] {
                    if let Err(error) = external_delete_case(mode, recursive) {
                        failures.push(format!("{mode:?}, subfolders {recursive}: {error}"));
                    }
                }
            }
            report(failures);
        },
    );
}
