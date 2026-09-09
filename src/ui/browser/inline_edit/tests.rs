// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    app::Browser,
    model::{EntryKind, Location, MetadataValue},
    services::{
        CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest, DirectoryChange,
        DirectoryEvent, DirectoryRequest, ExtractRequest, FileSource, LoadHandle,
        LocationValidationError, OperationEvent, OperationProvider, OperationRequestId,
        PasteRequest, RenameRequest, RestoreRequest, UndoCopyRequest, UndoMoveRequest,
    },
    test_support::gtk_test,
    ui::{
        browser::{BrowserView, PeekBehavior},
        browser_modes::BrowserMode,
    },
};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    rc::Rc,
    time::{Duration, Instant},
};

mod caret;
mod entries;
mod visibility;

#[test]
fn an_empty_name_is_not_flagged_as_an_error() {
    assert!(basename_field_error("bad/name").is_some());
    assert!(
        basename_field_error("").is_none(),
        "an empty field is the normal starting state, not a user mistake"
    );
}

#[test]
fn inline_rename_selects_the_stem_but_keeps_the_extension() {
    assert_eq!(rename_stem_end("report.txt"), 6);
    assert_eq!(rename_stem_end("archive.tar.gz"), 11);
    assert_eq!(rename_stem_end("README"), 6);
    assert_eq!(rename_stem_end(".gitignore"), 10);
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "rename fixture did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn label_text(label: &gtk::Widget) -> Option<String> {
    label
        .downcast_ref::<gtk::Label>()
        .map(|label| label.label().to_string())
        .or_else(|| {
            label
                .downcast_ref::<gtk::Inscription>()
                .and_then(|label| label.text().map(|text| text.to_string()))
        })
}

fn wait_for_current_rename_label(view: &BrowserView, mode: BrowserMode) -> gtk::Widget {
    let labels = Rc::new(RefCell::new(Vec::new()));
    let labels_for_wait = labels.clone();
    wait_until(|| {
        let mut candidates = Vec::new();
        fn visit(widget: &gtk::Widget, mode: BrowserMode, candidates: &mut Vec<gtk::Widget>) {
            if widget.is_mapped() && widget.is_visible() {
                let is_name = match mode {
                    BrowserMode::Columns => {
                        widget.downcast_ref::<gtk::Label>().is_some_and(|label| {
                            label
                                .next_sibling()
                                .and_then(|sibling| sibling.downcast::<gtk::Entry>().ok())
                                .is_some()
                                && !widget.has_css_class("alternate-rename-label")
                        })
                    }
                    BrowserMode::List | BrowserMode::Icons => {
                        widget.has_css_class("alternate-rename-label")
                    }
                };
                if is_name {
                    candidates.push(widget.clone());
                }
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                visit(&current, mode, candidates);
            }
        }
        visit(&view.widget(), mode, &mut candidates);
        labels_for_wait.replace(candidates);
        labels_for_wait.borrow().len() == 1
    });
    let labels = labels.borrow_mut().split_off(0);
    assert_eq!(labels.len(), 1, "exactly one live row name widget");
    let label = labels
        .into_iter()
        .next()
        .expect("one mapped row-name widget");
    match mode {
        BrowserMode::Icons => assert!(label.downcast_ref::<gtk::Inscription>().is_some()),
        BrowserMode::Columns | BrowserMode::List => {
            assert!(label.downcast_ref::<gtk::Label>().is_some())
        }
    }
    label
}

fn click_away(window: &gtk::Window) {
    // This emits the dismissal controller directly; real pointer coverage remains in E2E.
    let controllers = window.observe_controllers();
    let click = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .find(|click| click.button() == 0)
        .expect("inline edit dismissal gesture");
    click.emit_by_name::<()>("pressed", &[&1i32, &1.0f64, &1.0f64]);
}

fn fixture_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: OsString::from(name),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        thumbnail_path: None,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
    }
}

struct PendingDirectoryLoad {
    request_id: crate::services::RequestId,
    emit: Rc<dyn Fn(DirectoryEvent)>,
}

type MonitorNotify = Rc<dyn Fn(DirectoryChange)>;

struct ControlledRenameSource {
    initial: FileEntry,
    reloads: RefCell<Vec<PendingDirectoryLoad>>,
    monitor: RefCell<Option<MonitorNotify>>,
    loads: Cell<usize>,
}

impl ControlledRenameSource {
    fn pending_count(&self) -> usize {
        self.reloads.borrow().len()
    }

    fn request_ids(&self) -> Vec<crate::services::RequestId> {
        self.reloads
            .borrow()
            .iter()
            .map(|load| load.request_id)
            .collect()
    }

    fn emit_monitor_change(&self, change: DirectoryChange) {
        let notify = self.monitor.borrow().clone().expect("directory monitor");
        notify(change);
    }

    fn respond(&self, index: usize, entries: Option<Vec<FileEntry>>) -> crate::services::RequestId {
        self.respond_with(index, entries, false)
    }

    fn respond_with(
        &self,
        index: usize,
        entries: Option<Vec<FileEntry>>,
        truncated: bool,
    ) -> crate::services::RequestId {
        let load = self.reloads.borrow_mut().remove(index);
        let request_id = load.request_id;
        if let Some(entries) = entries {
            (load.emit)(DirectoryEvent::Batch {
                request_id,
                entries,
            });
            (load.emit)(DirectoryEvent::Finished {
                request_id,
                truncated,
                can_trash: None,
                can_delete: None,
            });
        } else {
            (load.emit)(DirectoryEvent::Failed {
                request_id,
                message: "refresh failed".to_owned(),
            });
        }
        request_id
    }
}

impl FileSource for ControlledRenameSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if self.loads.get() == 0 {
            self.loads.set(1);
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: vec![self.initial.clone()],
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: None,
                can_delete: None,
            });
        } else {
            self.reloads.borrow_mut().push(PendingDirectoryLoad {
                request_id: request.id,
                emit,
            });
        }
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        self.monitor.replace(Some(notify));
        Some(LoadHandle::new(|| {}))
    }
}

type OperationEmit = Rc<dyn Fn(OperationEvent)>;

struct DelayedRenameProvider {
    request: RefCell<Option<RenameRequest>>,
    emit: RefCell<Option<OperationEmit>>,
    complete_immediately: bool,
    cancel_immediately: bool,
}

impl DelayedRenameProvider {
    fn request_id(&self) -> OperationRequestId {
        self.request
            .borrow()
            .as_ref()
            .map(|request| request.id)
            .expect("rename request")
    }

    fn succeed(&self) {
        let request_id = self.request_id();
        (self.emit.borrow().as_ref().expect("rename callback"))(OperationEvent::Renamed {
            request_id,
        });
    }

    fn fail(&self) {
        let request_id = self.request_id();
        (self.emit.borrow().as_ref().expect("rename callback"))(OperationEvent::Failed {
            request_id,
            message: "rename failed".to_owned(),
        });
    }
}

macro_rules! unsupported_operation {
    ($method:ident, $request:ty) => {
        fn $method(&self, _request: $request, _emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
            panic!("rename fixture does not support {}", stringify!($method))
        }
    };
}

impl OperationProvider for DelayedRenameProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let request_id = request.id;
        self.request.replace(Some(request));
        self.emit.replace(Some(emit));
        if self.complete_immediately {
            (self.emit.borrow().as_ref().expect("rename callback"))(OperationEvent::Renamed {
                request_id,
            });
        } else if self.cancel_immediately {
            (self.emit.borrow().as_ref().expect("rename callback"))(OperationEvent::Cancelled {
                request_id,
                result: Default::default(),
            });
        }
        LoadHandle::new(|| {})
    }

    unsupported_operation!(create_directory, CreateDirectoryRequest);
    unsupported_operation!(create_file, CreateFileRequest);
    unsupported_operation!(paste, PasteRequest);
    unsupported_operation!(undo_move, UndoMoveRequest);
    unsupported_operation!(undo_copy, UndoCopyRequest);
    unsupported_operation!(delete, DeleteRequest);
    unsupported_operation!(restore, RestoreRequest);
    unsupported_operation!(compress, CompressRequest);
    unsupported_operation!(extract, ExtractRequest);
}

fn remote_entry(name: &str, directory: bool) -> FileEntry {
    entry_at_location(
        Location::uri(format!("smb://host/share/{name}")),
        name,
        directory,
    )
}

fn entry_at_location(location: Location, name: &str, directory: bool) -> FileEntry {
    FileEntry {
        location,
        native_name: OsString::from(name),
        display_name: name.to_owned(),
        kind: if directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        thumbnail_path: None,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
    }
}

fn assert_model_contains_only(browser: &Browser, old: &Location, new: &Location) {
    let snapshot = browser.column_snapshot(0).expect("loaded column");
    let locations = browser
        .with_entries(0, 0..snapshot.count, |entries| {
            entries
                .iter()
                .map(|entry| entry.location.clone())
                .collect::<Vec<_>>()
        })
        .expect("entries");
    assert!(!locations.iter().any(|location| location == old));
    assert!(locations.iter().any(|location| location == new));
    assert_eq!(locations.len(), 1);
}

fn icon_card_bounds(root: &gtk::Widget) -> Vec<(i32, i32, i32, i32)> {
    fn visit(widget: &gtk::Widget, root: &gtk::Widget, bounds: &mut Vec<(i32, i32, i32, i32)>) {
        if widget.has_css_class("icons-card")
            && widget.is_mapped()
            && let Some(rect) = widget.compute_bounds(root)
        {
            bounds.push((
                rect.x().round() as i32,
                rect.y().round() as i32,
                rect.width().round() as i32,
                rect.height().round() as i32,
            ));
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            visit(&current, root, bounds);
        }
    }

    let mut bounds = Vec::new();
    visit(root, root, &mut bounds);
    bounds.sort_unstable();
    bounds
}

#[test]
fn columns_rename_hides_and_restores_the_size_badge() {
    gtk_test(
        "ui::browser::inline_edit::tests::columns_rename_hides_and_restores_the_size_badge",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            let name = "synthetic-quarterly-report-with-a-very-long-descriptive-basename-2026.txt";
            std::fs::write(fixture.path().join(name), b"body").expect("fixture file");

            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(420)
                .default_height(300)
                .build();
            window.present();
            let browser = view.browser();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 1)
            });
            browser.select(0, 0);
            wait_until(|| view.state.begin_rename());
            let size = view
                .state
                .active_rename
                .borrow()
                .as_ref()
                .map(|rename| rename.size.clone())
                .expect("a Columns rename is open");

            wait_until(|| !size.label().is_empty());
            assert!(
                !size.is_visible(),
                "the badge must stay hidden while renaming"
            );
            assert!(view.state.cancel_rename());
            assert!(size.is_visible(), "cancelling must restore the size badge");

            browser.clear_observer();
            window.destroy();
        },
    );
}

#[derive(Clone, Copy)]
enum DelayedRenameResult {
    Success,
    Failure,
    MonitorBeforeCompletion,
    MonitorAfterCompletion,
    QueuedThroughRebuild,
    RefreshFailure,
    Replacement,
    SynchronousSuccess,
    SynchronousCancellation,
}

fn run_delayed_rename_handler(mode: BrowserMode, directory: bool, result: DelayedRenameResult) {
    let original = if directory {
        "original"
    } else {
        "original.txt"
    };
    let replacement = if directory { "renamed" } else { "renamed.txt" };
    let native_fixture = matches!(
        result,
        DelayedRenameResult::MonitorBeforeCompletion | DelayedRenameResult::MonitorAfterCompletion
    )
    .then(|| tempfile::tempdir().expect("directory fixture"));
    let location = native_fixture.as_ref().map_or_else(
        || Location::uri("smb://host/share"),
        |fixture| Location::local(fixture.path()),
    );
    let initial_location = location.child(original.as_ref()).expect("initial location");
    let renamed_location = location
        .child(replacement.as_ref())
        .expect("renamed location");
    let initial = if native_fixture.is_some() {
        entry_at_location(initial_location, original, directory)
    } else {
        remote_entry(original, directory)
    };
    let renamed = if native_fixture.is_some() {
        entry_at_location(renamed_location, replacement, directory)
    } else {
        remote_entry(replacement, directory)
    };
    let source = Rc::new(ControlledRenameSource {
        initial: initial.clone(),
        reloads: RefCell::new(Vec::new()),
        monitor: RefCell::new(None),
        loads: Cell::new(0),
    });
    let provider = Rc::new(DelayedRenameProvider {
        request: RefCell::new(None),
        emit: RefCell::new(None),
        complete_immediately: matches!(result, DelayedRenameResult::SynchronousSuccess),
        cancel_immediately: matches!(result, DelayedRenameResult::SynchronousCancellation),
    });
    let view = BrowserView::new(source.clone(), PeekBehavior::default());
    view.set_operation_provider(provider.clone());
    view.set_view_mode(mode);
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(800)
        .default_height(600)
        .build();
    view.install_inline_edit_dismissal(&window);
    window.present();
    let browser = view.browser();
    browser.navigate(location);
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 1)
    });
    browser.select(0, 0);
    wait_until(|| view.state.begin_rename());
    let field = view
        .state
        .active_rename
        .borrow()
        .as_ref()
        .map(|rename| rename.field.clone())
        .or_else(|| view.state.mode_views.borrow().active_rename_field())
        .expect("rename field");
    field.set_text(replacement);
    click_away(&window);

    if matches!(result, DelayedRenameResult::QueuedThroughRebuild) {
        view.set_view_mode(BrowserMode::List);
        browser.reload_active();
    }
    wait_until(|| provider.request.borrow().is_some());
    let operation_id = provider.request_id();
    if matches!(result, DelayedRenameResult::QueuedThroughRebuild) {
        wait_until(|| source.pending_count() == 1);
        source.respond_with(0, Some(vec![initial.clone()]), true);
        assert!(view.state.rename_operation_pending());
        provider.succeed();
        wait_until(|| source.pending_count() == 1);
        source.respond(0, Some(vec![renamed.clone()]));
        wait_until(|| !view.state.rename_operation_pending());
        let current = wait_for_current_rename_label(&view, BrowserMode::List);
        assert_eq!(label_text(&current).as_deref(), Some(replacement));
        assert_model_contains_only(&browser, &initial.location, &renamed.location);
        provider.fail();
        let current = wait_for_current_rename_label(&view, BrowserMode::List);
        assert_eq!(label_text(&current).as_deref(), Some(replacement));
        browser.clear_observer();
        window.destroy();
        return;
    }
    if matches!(result, DelayedRenameResult::MonitorBeforeCompletion) {
        source.emit_monitor_change(DirectoryChange::Move {
            from: initial.location.clone(),
            entry: renamed.clone(),
        });
        assert_eq!(source.pending_count(), 0);
        assert!(view.state.rename_operation_pending());
        provider.succeed();
        wait_until(|| !view.state.rename_operation_pending());
        assert_eq!(source.pending_count(), 0);
        assert_model_contains_only(&browser, &initial.location, &renamed.location);
        browser.clear_observer();
        window.destroy();
        return;
    }
    if matches!(result, DelayedRenameResult::MonitorAfterCompletion) {
        provider.succeed();
        assert_eq!(source.pending_count(), 0);
        source.emit_monitor_change(DirectoryChange::Move {
            from: initial.location.clone(),
            entry: renamed.clone(),
        });
        wait_until(|| !view.state.rename_operation_pending());
        assert_eq!(source.pending_count(), 0);
        assert_model_contains_only(&browser, &initial.location, &renamed.location);
        browser.clear_observer();
        window.destroy();
        return;
    }
    assert_eq!(
        provider
            .request
            .borrow()
            .as_ref()
            .expect("rename request")
            .new_name,
        replacement
    );

    if matches!(result, DelayedRenameResult::SynchronousCancellation) {
        wait_until(|| !view.state.rename_operation_pending());
        assert!(!browser.is_current_operation(provider.request_id()));
        browser.clear_observer();
        window.destroy();
        return;
    }

    view.state
        .complete_pending_rename(OperationRequestId(operation_id.0 + 1));
    assert!(view.state.rename_operation_pending());
    if matches!(result, DelayedRenameResult::SynchronousSuccess) {
        wait_until(|| source.pending_count() == 1);
        source.respond(0, Some(vec![renamed]));
        wait_until(|| !view.state.rename_operation_pending());
        let current = wait_for_current_rename_label(&view, mode);
        assert_eq!(label_text(&current).as_deref(), Some(replacement));
        browser.clear_observer();
        window.destroy();
        return;
    }

    // Rebind while the provider is still holding the operation. The assertion below
    // deliberately looks up the current bound row after the rebind, not the old widget.
    browser.reload_active();
    assert_eq!(source.pending_count(), 1);
    source.respond(0, Some(vec![remote_entry("unrelated", directory)]));
    browser.reload_active();
    wait_until(|| source.pending_count() == 1);
    source.respond(0, Some(vec![initial]));
    let current = wait_for_current_rename_label(&view, mode);
    assert_eq!(label_text(&current).as_deref(), Some(replacement));

    match result {
        DelayedRenameResult::Failure => {
            provider.fail();
            wait_until(|| !view.state.rename_operation_pending());
            let current = wait_for_current_rename_label(&view, mode);
            assert_eq!(label_text(&current).as_deref(), Some(original));
        }
        DelayedRenameResult::Success => {
            provider.succeed();
            wait_until(|| source.pending_count() == 1);
            source.respond(0, Some(vec![renamed]));
            wait_until(|| !view.state.rename_operation_pending());
            let current = wait_for_current_rename_label(&view, mode);
            assert_eq!(label_text(&current).as_deref(), Some(replacement));
        }
        DelayedRenameResult::RefreshFailure => {
            provider.succeed();
            wait_until(|| source.pending_count() == 1);
            source.respond(0, None);
            wait_until(|| !view.state.rename_operation_pending());
            assert!(
                browser.column_snapshot(0).is_some_and(|snapshot| {
                    snapshot.error.as_deref() == Some("refresh failed")
                })
            );
        }
        DelayedRenameResult::Replacement => {
            provider.succeed();
            wait_until(|| source.pending_count() == 1);
            let first_refresh = source.request_ids()[0];
            browser.reload_active();
            wait_until(|| source.pending_count() == 2);
            let second_refresh = source.request_ids()[1];
            assert_ne!(first_refresh, second_refresh);

            assert_eq!(source.respond(0, None), first_refresh);
            assert!(view.state.rename_operation_pending());
            assert_eq!(source.respond(0, Some(vec![renamed])), second_refresh);
            wait_until(|| !view.state.rename_operation_pending());
            let current = wait_for_current_rename_label(&view, mode);
            assert_eq!(label_text(&current).as_deref(), Some(replacement));
        }
        DelayedRenameResult::SynchronousSuccess
        | DelayedRenameResult::SynchronousCancellation
        | DelayedRenameResult::MonitorBeforeCompletion
        | DelayedRenameResult::MonitorAfterCompletion
        | DelayedRenameResult::QueuedThroughRebuild => unreachable!(),
    }

    browser.clear_observer();
    window.destroy();
}

#[test]
fn click_away_rename_handler_preserves_current_bound_label_through_delayed_callbacks() {
    gtk_test(
        "ui::browser::inline_edit::tests::click_away_rename_handler_preserves_current_bound_label_through_delayed_callbacks",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    run_delayed_rename_handler(mode, directory, DelayedRenameResult::Failure);
                    run_delayed_rename_handler(mode, directory, DelayedRenameResult::Success);
                }
            }
        },
    );
}

#[test]
fn rename_callbacks_only_affect_their_owned_operation() {
    gtk_test(
        "ui::browser::inline_edit::tests::rename_callbacks_only_affect_their_owned_operation",
        || {
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let first = fixture_entry("first.txt");
            let second = fixture_entry("second.txt");
            let first_id = OperationRequestId(41);
            let second_id = OperationRequestId(42);
            view.state.pending_rename.replace(Some(PendingRename {
                old_location: first.location.clone(),
                new_location: None,
                old_name: first.display_name.clone(),
                new_name: "first-renamed.txt".to_owned(),
                generation: 1,
                monitor_has_new_location: false,
                reveal_generation: 0,
                source_position: None,
                scroll_value: None,
                state: PendingRenameState::Running(first_id),
            }));
            view.state.complete_pending_rename(OperationRequestId(99));
            assert_eq!(
                view.state.pending_rename_name(&first),
                Some("first-renamed.txt".to_owned())
            );

            view.state.pending_rename.replace(Some(PendingRename {
                old_location: second.location.clone(),
                new_location: None,
                old_name: second.display_name.clone(),
                new_name: "second-renamed.txt".to_owned(),
                generation: 2,
                monitor_has_new_location: false,
                reveal_generation: 0,
                source_position: None,
                scroll_value: None,
                state: PendingRenameState::Running(second_id),
            }));
            view.state.complete_pending_rename(first_id);
            view.state.fail_pending_rename_from_browser(Some(first_id));
            assert_eq!(
                view.state.pending_rename_name(&second),
                Some("second-renamed.txt".to_owned())
            );
        },
    );
}

#[test]
fn escape_after_submission_does_not_cancel_the_rename() {
    gtk_test(
        "ui::browser::inline_edit::tests::escape_after_submission_does_not_cancel_the_rename",
        || {
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let entry = fixture_entry("original.txt");
            view.state
                .start_pending_rename(&entry, "renamed.txt".to_owned());

            assert!(!view.state.cancel_rename());
            assert!(view.state.rename_operation_pending());
        },
    );
}

#[test]
fn monitor_completion_is_safe_in_both_event_orders() {
    gtk_test(
        "ui::browser::inline_edit::tests::monitor_completion_is_safe_in_both_event_orders",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    run_delayed_rename_handler(
                        mode,
                        directory,
                        DelayedRenameResult::MonitorBeforeCompletion,
                    );
                    run_delayed_rename_handler(
                        mode,
                        directory,
                        DelayedRenameResult::MonitorAfterCompletion,
                    );
                }
            }
        },
    );
}

#[test]
fn queued_rename_survives_truncation_and_mode_rebuild() {
    gtk_test(
        "ui::browser::inline_edit::tests::queued_rename_survives_truncation_and_mode_rebuild",
        || {
            for directory in [false, true] {
                run_delayed_rename_handler(
                    BrowserMode::Columns,
                    directory,
                    DelayedRenameResult::QueuedThroughRebuild,
                );
            }
        },
    );
}

#[test]
fn synchronous_rename_handler_completion_enters_the_refresh_lifecycle() {
    gtk_test(
        "ui::browser::inline_edit::tests::synchronous_rename_handler_completion_enters_the_refresh_lifecycle",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    run_delayed_rename_handler(
                        mode,
                        directory,
                        DelayedRenameResult::SynchronousSuccess,
                    );
                }
            }
        },
    );
}

#[test]
fn successful_rename_handler_clears_pending_state_after_the_refreshed_entry() {
    gtk_test(
        "ui::browser::inline_edit::tests::successful_rename_handler_clears_pending_state_after_the_refreshed_entry",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    run_delayed_rename_handler(mode, directory, DelayedRenameResult::Success);
                }
            }
        },
    );
}

#[test]
fn another_operation_abandons_a_queued_rename() {
    gtk_test(
        "ui::browser::inline_edit::tests::another_operation_abandons_a_queued_rename",
        || {
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let entry = fixture_entry("original.txt");
            let generation = view
                .state
                .start_pending_rename(&entry, "renamed.txt".to_owned());
            view.state
                .queue_pending_rename(entry, "renamed.txt".to_owned(), generation);

            view.browser().create_new_file(Location::local("/fixture"));
            wait_until(|| !view.state.rename_operation_pending());
        },
    );
}

#[test]
fn synchronous_cancellation_abandons_dispatching_rename() {
    gtk_test(
        "ui::browser::inline_edit::tests::synchronous_cancellation_abandons_dispatching_rename",
        || {
            run_delayed_rename_handler(
                BrowserMode::Columns,
                false,
                DelayedRenameResult::SynchronousCancellation,
            )
        },
    );
}

#[test]
fn completed_rename_reconciles_a_real_refresh_failure() {
    gtk_test(
        "ui::browser::inline_edit::tests::completed_rename_reconciles_a_real_refresh_failure",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    run_delayed_rename_handler(
                        mode,
                        directory,
                        DelayedRenameResult::RefreshFailure,
                    );
                }
            }
        },
    );
}

#[test]
fn completed_rename_uses_the_real_replacement_refresh_after_supersession() {
    gtk_test(
        "ui::browser::inline_edit::tests::completed_rename_uses_the_real_replacement_refresh_after_supersession",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    run_delayed_rename_handler(mode, directory, DelayedRenameResult::Replacement);
                }
            }
        },
    );
}

#[test]
fn invalid_renames_retain_the_original_file_in_every_view_mode() {
    gtk_test(
        "ui::browser::inline_edit::tests::invalid_renames_retain_the_original_file_in_every_view_mode",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            let file = fixture.path().join("notes.txt");
            std::fs::write(&file, b"body").expect("fixture file");
            for index in 0..5 {
                std::fs::write(fixture.path().join(format!("sample-{index}.txt")), b"body")
                    .expect("fixture file");
            }

            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(800)
                    .default_height(600)
                    .build();
                window.present();
                let browser = view.browser();
                browser.navigate(Location::local(fixture.path()));
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 6)
                });
                let widget = view.widget();
                let bounds_before = (mode == BrowserMode::Icons).then(|| {
                    wait_until(|| {
                        let bounds = icon_card_bounds(&widget);
                        bounds.len() == 6
                            && bounds
                                .iter()
                                .all(|(_, _, width, height)| *width > 0 && *height > 0)
                    });
                    icon_card_bounds(&widget)
                });
                browser.select(0, 0);
                wait_until(|| view.state.begin_rename());
                let field = view
                    .state
                    .active_rename
                    .borrow()
                    .as_ref()
                    .map(|rename| rename.field.clone())
                    .or_else(|| view.state.mode_views.borrow().active_rename_field())
                    .expect("an inline rename field is open");

                if let Some(bounds_before) = bounds_before {
                    wait_until(|| field.is_mapped());
                    let deadline = Instant::now() + Duration::from_millis(100);
                    while Instant::now() < deadline {
                        glib::MainContext::default().iteration(false);
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    assert_eq!(
                        icon_card_bounds(&widget),
                        bounds_before,
                        "opening the Icons rename field must not reflow the grid"
                    );
                }

                for name in ["", "   ", "bad/name", "."] {
                    assert!(view.state.begin_rename());
                    let field = view
                        .state
                        .active_rename
                        .borrow()
                        .as_ref()
                        .map(|active| active.field.clone())
                        .or_else(|| view.state.mode_views.borrow().active_rename_field())
                        .expect("rename field");
                    field.set_text(name);
                    if !name.is_empty() {
                        assert!(field.has_css_class("error"));
                    }
                    field.emit_activate();
                    assert!(!view.rename_is_active());
                    assert!(!gtk::prelude::WidgetExt::is_visible(&field));
                    assert_eq!(std::fs::read(&file).expect("original contents"), b"body");
                }

                assert!(file.is_file(), "{mode:?} left the entry untouched");
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn lifecycle_rejects_invalid_transitions_and_stale_operations() {
    let mut pending = pending();
    let current = OperationRequestId(7);
    let stale = OperationRequestId(6);

    assert!(!pending.finish_dispatch(current));
    assert!(!pending.complete(current, None));
    assert!(pending.begin_dispatch());
    assert!(!pending.begin_dispatch());
    assert!(!pending.owns_operation(stale, Some(current)));
    assert!(pending.finish_dispatch(current));
    assert!(pending.owns_operation(current, None));
    assert!(!pending.complete(stale, Some(current)));
    assert!(pending.complete(current, Some(current)));
    assert!(matches!(
        pending.state,
        PendingRenameState::AwaitingRefresh { .. }
    ));
}

#[test]
fn dispatching_operation_is_owned_by_latest_started_request() {
    let mut pending = pending();
    let current = OperationRequestId(11);
    assert!(pending.begin_dispatch());
    assert!(pending.owns_operation(current, Some(current)));
    assert!(!pending.owns_operation(current, None));
    assert!(pending.complete(current, Some(current)));
}

#[test]
fn synchronous_completion_cannot_be_overwritten_by_dispatch_finish() {
    let mut pending = pending();
    let operation = OperationRequestId(12);
    assert!(pending.begin_dispatch());
    assert!(pending.complete(operation, Some(operation)));
    assert!(!pending.finish_dispatch(operation));
    assert!(matches!(
        pending.state,
        PendingRenameState::AwaitingRefresh { .. }
    ));
}

fn pending() -> PendingRename {
    PendingRename {
        old_location: Location::local("/home/test/old"),
        new_location: None,
        old_name: "old".into(),
        new_name: "new".into(),
        generation: 1,
        monitor_has_new_location: false,
        reveal_generation: 0,
        scroll_value: None,
        source_position: None,
        state: PendingRenameState::Queued,
    }
}
