// SPDX-License-Identifier: MIT

use super::footer_prompt::{ALL_REPORTS, IMMEDIATE_REPORTS, hit_cursor, seed_filter_tree};
use super::*;
use crate::{
    adapters::LocalFileSource,
    model::{EntryKind, FileEntry, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LocationValidationError},
    ui::browser::FilterFocus,
};

mod enter_target;
mod escape;
mod filter_slash;
mod live_changes;
mod returning_focus;
mod view_switch;

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

/// `Ok` when `holds`, otherwise the error `describe` builds.
fn ensure(holds: bool, describe: impl FnOnce() -> String) -> Result<(), String> {
    if holds { Ok(()) } else { Err(describe()) }
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

fn result_names(fixture: &KeyboardFixture) -> Vec<String> {
    let mut names = fixture.view.filter_result_names();
    names.sort();
    names
}

fn expected_reports(recursive: bool) -> &'static [&'static str] {
    if recursive {
        &ALL_REPORTS
    } else {
        &IMMEDIATE_REPORTS
    }
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

fn ctrl_f_focuses_the_field(fixture: &KeyboardFixture) -> bool {
    fixture.press(Key::f, ModifierType::CONTROL_MASK) && fixture.view.filter_has_focus()
}

/// Ctrl+F, then `query` typed into the field; returns the field once the hits show.
fn show_filter_with_results(
    fixture: &KeyboardFixture,
    query: &str,
    expected: &[&str],
) -> Result<gtk::Entry, String> {
    ensure(ctrl_f_focuses_the_field(fixture), || {
        format!(
            "Ctrl+F did not focus the field; focus is on {}",
            describe_focus(fixture)
        )
    })?;
    let field = focused_entry(&fixture.window);
    field.set_text(query);
    ensure(settles(|| filter_applied(fixture, expected)), || {
        format!(
            "filter results {:?} never became {expected:?}",
            result_names(fixture)
        )
    })?;
    // An in-place Columns filter narrows the rows before its debounced query settles.
    pump(300);
    Ok(field)
}

/// Emits `key` through the field's own key handlers, as a real press reaches them.
fn field_handles_key(field: &gtk::Entry, key: Key) -> bool {
    let controllers = field.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .any(|keys| {
            keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &ModifierType::empty()])
        })
}

fn focused_hit(fixture: &KeyboardFixture) -> bool {
    result_has_focus(fixture) && hit_cursor(fixture).is_some()
}

/// Down from the field, through the field's own key handlers as a real key press: Icons
/// and List focus the results view, Columns the column's filtered rows.
fn focus_first_result(fixture: &KeyboardFixture, field: &gtk::Entry) -> Result<(), String> {
    let handled = field_handles_key(field, Key::Down);
    ensure(handled && settles(|| focused_hit(fixture)), || {
        format!(
            "Down from the field did not focus a result (handled {handled}); focus is on {}",
            describe_focus(fixture)
        )
    })
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

fn children(widget: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
}

fn label_shows(widget: &gtk::Widget, text: &str) -> bool {
    widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.text() == text)
}

fn inscription_shows(widget: &gtk::Widget, text: &str) -> bool {
    widget
        .downcast_ref::<gtk::Inscription>()
        .is_some_and(|label| label.text().as_deref() == Some(text))
}

fn shows_text(widget: &gtk::Widget, text: &str) -> bool {
    label_shows(widget, text)
        || inscription_shows(widget, text)
        || children(widget).any(|child| shows_text(&child, text))
}

fn carries_cursor_ring(focused: &gtk::Widget) -> bool {
    focused.has_css_class("keyboard-cursor")
        || children(focused).any(|child| child.has_css_class("keyboard-cursor"))
}

/// GTK's selection matches the browser's, and focus is on the browser's cursor row
/// (with the Columns cursor ring).
fn listing_matches_browser(fixture: &KeyboardFixture) -> Result<(), String> {
    let shown = shown_selection(fixture);
    ensure(
        shown.as_deref() == Some(fixture.selected().as_slice()),
        || {
            format!(
                "the listing shows selection {shown:?} but the browser holds {:?}",
                fixture.selected()
            )
        },
    )?;
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
        || focused.as_ref().is_some_and(carries_cursor_ring);
    ensure(on_cursor && ring, || {
        format!(
            "focus is on {} rather than the cursor row {cursor:?} (ring shown: {ring})",
            describe_focus(fixture)
        )
    })
}

/// Waits for [`listing_matches_browser`], reporting its last mismatch.
fn listing_settles_on_browser(fixture: &KeyboardFixture) -> Result<(), String> {
    if settles(|| listing_matches_browser(fixture).is_ok()) {
        return Ok(());
    }
    listing_matches_browser(fixture)
}

/// Runs every case, labelling each failure with `label`.
fn failures_of<C: Copy>(
    cases: impl IntoIterator<Item = C>,
    label: impl Fn(C) -> String,
    run: impl Fn(C) -> Result<(), String>,
) -> Vec<String> {
    cases
        .into_iter()
        .filter_map(|case| {
            run(case)
                .err()
                .map(|error| format!("{}: {error}", label(case)))
        })
        .collect()
}

fn report(failures: Vec<String>) {
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn check_modes(modes: &[BrowserMode], run: impl Fn(BrowserMode) -> Result<(), String>) {
    report(failures_of(
        modes.iter().copied(),
        |mode| format!("{mode:?}"),
        run,
    ));
}

pub(super) const SHARE: &str = "sftp://share.invalid/reports";

/// A remote share: it has no native path, so Icons and List narrow it in place instead
/// of showing separate results.
struct RemoteShare;

fn share_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::uri(format!("{SHARE}/{name}")),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

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
            .map(share_entry)
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

/// [`RemoteShare`] with every entry under the share a folder.
pub(super) struct RemoteFolders;

impl FileSource for RemoteFolders {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        RemoteShare.validate_location(location)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let remote = request.location.native_path().is_none();
        RemoteShare.enumerate(
            request,
            Rc::new(move |event| {
                let event = match event {
                    DirectoryEvent::Batch {
                        request_id,
                        mut entries,
                    } if remote => {
                        for entry in &mut entries {
                            entry.kind = EntryKind::Directory;
                        }
                        DirectoryEvent::Batch {
                            request_id,
                            entries,
                        }
                    }
                    event => event,
                };
                emit(event);
            }),
        )
    }
}

fn remote_share_fixture(mode: BrowserMode) -> KeyboardFixture {
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
    focus_files(&fixture);
    fixture
}

fn marked_names(fixture: &KeyboardFixture) -> Vec<String> {
    let names = fixture.view.filter_result_names();
    marked_rows(fixture.window.upcast_ref())
        .iter()
        .map(|row| {
            names
                .iter()
                .find(|name| shows_text(row, name))
                .cloned()
                .unwrap_or_else(|| format!("{:?}", row.css_classes()))
        })
        .collect()
}

/// Exactly one row carries the Enter mark, and it shows `name`.
fn only_marked(fixture: &KeyboardFixture, name: &str) -> bool {
    let marked = marked_rows(fixture.window.upcast_ref());
    marked.len() == 1 && shows_text(&marked[0], name)
}

/// Only the top result carries the Enter mark, and the mark selected nothing.
fn marks_only_top(fixture: &KeyboardFixture) -> Result<(), String> {
    let top = || fixture.view.filter_result_names().first().cloned();
    ensure(
        settles(|| top().is_some_and(|top| only_marked(fixture, &top))),
        || {
            format!(
                "expected only the top result {:?} marked; marked {:?}",
                top(),
                marked_names(fixture)
            )
        },
    )?;
    match fixture.view.selected_search_results() {
        Some(selected) if !selected.is_empty() => Err(format!(
            "the Enter mark selected {:?}",
            selected
                .iter()
                .map(|entry| entry.display_name.clone())
                .collect::<Vec<_>>()
        )),
        _ => Ok(()),
    }
}
