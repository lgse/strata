// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{FileEntry, Location};
use crate::services::{DropCommit, MoveRecord, RenameCause, TransferKind};
use crate::ui::browser::Browser;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ffi::OsString,
    path::Path,
};

use crate::test_support::operations::operation_event;

mod choose_folder;
mod chosen_destination;
mod conflicts;
mod destination_reloads;
mod drops;
mod plain_transfers;
mod replay_kinds;
mod send_to;
mod undo;

type Events = Rc<RefCell<Vec<crate::app::BrowserEvent>>>;

fn children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        children.push(widget);
    }
    children
}

/// Visits the stacked widgets depth first, last sibling first, skipping the
/// subtrees `descend` rejects.
fn walk(mut stack: Vec<gtk::Widget>, descend: impl Fn(&gtk::Widget) -> bool) -> Vec<gtk::Widget> {
    let mut visited = Vec::new();
    while let Some(widget) = stack.pop() {
        if !descend(&widget) {
            continue;
        }
        stack.extend(children(&widget));
        visited.push(widget);
    }
    visited
}

fn descendants(overlay: &gtk::Overlay) -> Vec<gtk::Widget> {
    walk(children(overlay), |_| true)
}

fn visible_descendants(overlay: &gtk::Overlay) -> Vec<gtk::Widget> {
    walk(children(overlay), gtk::Widget::is_visible)
}

fn widget_text(widget: &gtk::Widget) -> Option<String> {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        return Some(label.label().to_string());
    }
    widget
        .downcast_ref::<gtk::Button>()
        .and_then(gtk::Button::label)
        .map(|label| label.to_string())
}

fn visible_texts(overlay: &gtk::Overlay) -> Vec<String> {
    visible_descendants(overlay)
        .iter()
        .filter_map(widget_text)
        .collect()
}

fn has_visible_button(overlay: &gtk::Overlay, label: &str) -> bool {
    visible_texts(overlay).iter().any(|text| text == label)
}

fn visible_widget<W: IsA<gtk::Widget>>(
    overlay: &gtk::Overlay,
    matches: impl Fn(&W) -> bool,
) -> Option<W> {
    descendants(overlay)
        .into_iter()
        .filter_map(|widget| widget.downcast::<W>().ok())
        .filter(|found| found.is_visible())
        .find(|found| matches(found))
}

fn button_with_label(overlay: &gtk::Overlay, label: &str) -> Option<gtk::Button> {
    visible_widget(overlay, |button: &gtk::Button| {
        button.label().as_deref() == Some(label)
    })
}

fn check_button_with_label(overlay: &gtk::Overlay, label: &str) -> gtk::CheckButton {
    visible_widget(overlay, |button: &gtk::CheckButton| {
        button.label().as_deref() == Some(label)
    })
    .unwrap_or_else(|| panic!("visible {label:?} check button not found"))
}

fn click_button(overlay: &gtk::Overlay, label: &str) {
    button_with_label(overlay, label)
        .unwrap_or_else(|| panic!("visible {label:?} button not found"))
        .emit_clicked();
}

fn wait_until(condition: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn settle(duration: std::time::Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn iterate_context(times: usize) {
    for _ in 0..times {
        glib::MainContext::default().iteration(false);
    }
}

fn wait_for_modal_layer(overlay: &gtk::Overlay) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
        if children(overlay)
            .iter()
            .any(|widget| widget.has_css_class("app-modal-layer"))
        {
            return true;
        }
    }
    false
}

fn transfer_entry(path: &Path) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        native_name: path.file_name().unwrap_or_default().to_owned(),
        thumbnail_path: None,
        display_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        created_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
        listable: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    }
}

fn drain_context() {
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn destination_chooser(parent: &gtk::Window) -> (gtk::Window, crate::ui::browser::BrowserView) {
    wait_until(
        || crate::ui::chooser::destination_chooser_for(parent).is_some(),
        "floating destination chooser",
    );
    crate::ui::chooser::destination_chooser_for(parent).expect("floating destination chooser")
}

fn chooser_overlay(chooser_window: &gtk::Window) -> gtk::Overlay {
    chooser_window
        .child()
        .expect("chooser content")
        .downcast::<gtk::Overlay>()
        .expect("chooser overlay")
}

fn navigate_destination(view: &crate::ui::browser::BrowserView, path: &Path) {
    view.navigate_location(Location::local(path));
    wait_until(
        || view.browser().active_location() == Some(Location::local(path)),
        "chooser destination navigation",
    );
}

fn destination_accept(window: &gtk::Window) -> gtk::Button {
    window
        .default_widget()
        .expect("chooser default action")
        .downcast::<gtk::Button>()
        .expect("chooser action button")
}

fn confirm_destination(window: &gtk::Window) {
    destination_accept(window).emit_clicked();
}

/// Confirms the chooser the way a keyboard user does: Enter on its focused accept.
fn activate_focused_accept(window: &gtk::Window) {
    let accept = destination_accept(window);
    accept.grab_focus();
    accept.emit_by_name::<()>("activate", &[]);
}

fn is_error_label(label: &gtk::Label) -> bool {
    ["error", "form-field-error"]
        .iter()
        .any(|class| label.has_css_class(class))
}

fn visible_error_message(overlay: &gtk::Overlay) -> Option<String> {
    descendants(overlay)
        .iter()
        .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
        .filter(|label| label.is_visible())
        .find(|label| is_error_label(label))
        .map(|label| label.text().to_string())
}

fn find_widget_with_class(overlay: &gtk::Overlay, class: &str) -> Option<gtk::Widget> {
    descendants(overlay)
        .into_iter()
        .find(|widget| widget.has_css_class(class))
}

fn modal_layer_shown(overlay: &gtk::Overlay) -> bool {
    find_widget_with_class(overlay, "app-modal-layer").is_some()
}

fn local_view() -> crate::ui::browser::BrowserView {
    crate::ui::browser::BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        crate::ui::browser::PeekBehavior::default(),
    )
}

fn operating_view() -> crate::ui::browser::BrowserView {
    let view = local_view();
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view
}

fn observe_events(view: &crate::ui::browser::BrowserView) -> Events {
    let events = Events::default();
    let observed = events.clone();
    view.browser()
        .observe(move |event| observed.borrow_mut().push(operation_event(event).clone()));
    events
}

fn finished(events: &Events) -> bool {
    events
        .borrow()
        .iter()
        .any(|event| matches!(event, crate::app::BrowserEvent::TransferFinished { .. }))
}

fn finished_count(events: &Events) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches!(event, crate::app::BrowserEvent::TransferFinished { .. }))
        .count()
}

fn started(events: &Events) -> bool {
    events
        .borrow()
        .iter()
        .any(|event| matches!(event, crate::app::BrowserEvent::TransferStarted { .. }))
}

fn revealed(events: &Events) -> bool {
    events
        .borrow()
        .iter()
        .any(|event| matches!(event, crate::app::BrowserEvent::TransferReveal { .. }))
}

fn started_totals(events: &Events) -> Vec<(usize, bool)> {
    events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            crate::app::BrowserEvent::TransferStarted { total, moving } => Some((*total, *moving)),
            _ => None,
        })
        .collect()
}

fn conflict_view() -> (crate::ui::browser::BrowserView, gtk::Overlay, gtk::Window) {
    let view = operating_view();
    let browser_widget = view.widget();
    let root = crate::ui::blur::BlurBin::new(&browser_widget);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&root));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    (view, overlay, window)
}

fn present_transfer_browser(
    view: crate::ui::browser::BrowserView,
) -> (
    crate::ui::browser::BrowserView,
    gtk::Overlay,
    gtk::Window,
    Events,
) {
    let overlay = view.overlay();
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    let events = observe_events(&view);
    (view, overlay, window, events)
}

fn open_transfer_browser(
    source_dir: &std::path::Path,
) -> (
    crate::ui::browser::BrowserView,
    gtk::Overlay,
    gtk::Window,
    Events,
) {
    let view = operating_view();
    view.navigate_location(Location::local(source_dir));
    present_transfer_browser(view)
}

fn open_unnavigated_transfer_browser() -> (
    crate::ui::browser::BrowserView,
    gtk::Overlay,
    gtk::Window,
    Events,
) {
    present_transfer_browser(operating_view())
}

fn focused_enter_destination_fixture(
    name: &str,
) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let fixture = tempfile::tempdir().expect(name);
    let source_dir = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    std::fs::create_dir_all(&source_dir).expect("source directory");
    std::fs::create_dir_all(&destination).expect("destination directory");
    std::fs::write(source_dir.join("photo.txt"), b"photo").expect("source file");
    (fixture, source_dir, destination)
}

fn write_files(root: &Path, files: &[(&str, &[u8])]) {
    for (name, contents) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture folder");
        std::fs::write(path, contents).expect("fixture file");
    }
}

fn holds(path: &Path, contents: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|held| held == contents)
}

fn transfer_row_shown(overlay: &gtk::Overlay, moving: bool) -> bool {
    if moving {
        return visible_widget(overlay, |layer: &gtk::Box| {
            layer.has_css_class("app-modal-layer") && !layer.has_css_class("dismissing")
        })
        .is_some();
    }
    let docked = visible_widget(overlay, |card: &gtk::Box| {
        card.has_css_class("file-operation-card")
    });
    !modal_layer_shown(overlay) && docked.is_some()
}

fn assert_skip_offered_alone(overlay: &gtk::Overlay, skip_reason: &str, apply_reason: &str) {
    assert!(has_visible_button(overlay, "Skip"), "{skip_reason}");
    assert!(
        !has_visible_button(overlay, "Apply to All"),
        "{apply_reason}"
    );
}
