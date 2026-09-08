// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::{
    app::BrowserEvent,
    model::{EntryKind, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    test_support::gtk_test,
    ui::{
        browser::{BrowserView, PeekBehavior},
        browser_modes::BrowserMode,
    },
};
use std::{
    cell::Cell,
    ffi::OsString,
    rc::Rc,
    time::{Duration, Instant},
};

mod caret;
mod entries;

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

fn active_rename_label(field: &gtk::Entry) -> gtk::Widget {
    field
        .prev_sibling()
        .or_else(|| field.parent().and_then(|parent| parent.first_child()))
        .expect("active rename label")
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

fn click_away(window: &gtk::Window) {
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

struct RefreshSource {
    loads: Cell<usize>,
    refresh_behavior: RefreshBehavior,
}

#[derive(Clone, Copy)]
enum RefreshBehavior {
    Fail,
    AbandonOnce,
}

impl FileSource for RefreshSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let load = self.loads.get();
        self.loads.set(load + 1);
        if load == 0 {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: vec![fixture_entry("original.txt")],
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: None,
                can_delete: None,
            });
        } else if matches!(self.refresh_behavior, RefreshBehavior::Fail) {
            emit(DirectoryEvent::Failed {
                request_id: request.id,
                message: "refresh failed".to_owned(),
            });
        } else if load > 1 {
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: None,
                can_delete: None,
            });
        }
        LoadHandle::new(|| {})
    }
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

#[test]
fn click_away_rename_keeps_the_requested_name_through_rebind_and_rolls_back_on_failure() {
    gtk_test(
        "ui::browser::inline_edit::tests::click_away_rename_keeps_the_requested_name_through_rebind_and_rolls_back_on_failure",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for (directory, original, replacement) in [
                    (false, "original.txt", "renamed.txt"),
                    (true, "original", "renamed"),
                ] {
                    let fixture = tempfile::tempdir().expect("directory fixture");
                    let path = fixture.path().join(original);
                    if directory {
                        std::fs::create_dir(&path).expect("fixture directory");
                    } else {
                        std::fs::write(&path, b"body").expect("fixture file");
                    }
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
                    view.install_inline_edit_dismissal(&window);
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
                    let field = view
                        .state
                        .active_rename
                        .borrow()
                        .as_ref()
                        .map(|rename| rename.field.clone())
                        .or_else(|| view.state.mode_views.borrow().active_rename_field())
                        .expect("rename field");
                    let label = active_rename_label(&field);
                    field.set_text(replacement);
                    click_away(&window);

                    assert!(view.state.rename_operation_pending());
                    assert_eq!(label_text(&label).as_deref(), Some(replacement));

                    assert!(view.state.cancel_rename());
                    assert_eq!(
                        label_text(&label).as_deref(),
                        Some(original),
                        "{mode:?} must restore the original label after failure"
                    );

                    assert!(view.state.begin_rename());
                    let field = view
                        .state
                        .active_rename
                        .borrow()
                        .as_ref()
                        .map(|rename| rename.field.clone())
                        .or_else(|| view.state.mode_views.borrow().active_rename_field())
                        .expect("rename field");
                    let label = active_rename_label(&field);
                    field.set_text(replacement);
                    click_away(&window);
                    view.state
                        .complete_pending_rename(crate::services::OperationRequestId(0));
                    assert!(
                        view.state.rename_operation_pending(),
                        "the optimistic label must remain protected until refreshed data arrives"
                    );
                    view.state
                        .handle(&BrowserEvent::EntriesReplaced { depth: 0, count: 1 });
                    assert_eq!(
                        label_text(&label).as_deref(),
                        Some(replacement),
                        "{mode:?} must not rebind the old label while rename is pending"
                    );
                    assert!(view.state.cancel_rename());
                    assert!(
                        view.state.rename_operation_pending(),
                        "navigation or a click must not cancel a committed rename"
                    );
                    view.state.fail_pending_rename();
                    assert!(!view.state.rename_operation_pending());
                    browser.clear_observer();
                    window.destroy();
                }
            }
        },
    );
}

#[test]
fn successful_click_away_renames_clear_pending_state_after_the_refreshed_entry() {
    gtk_test(
        "ui::browser::inline_edit::tests::successful_click_away_renames_clear_pending_state_after_the_refreshed_entry",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for (directory, original, replacement) in [
                    (false, "original.txt", "renamed.txt"),
                    (true, "original", "renamed"),
                    (false, "visible.txt", ".renamed.txt"),
                ] {
                    let fixture = tempfile::tempdir().expect("directory fixture");
                    let original_path = fixture.path().join(original);
                    let replacement_path = fixture.path().join(replacement);
                    if directory {
                        std::fs::create_dir(&original_path).expect("fixture directory");
                    } else {
                        std::fs::write(&original_path, b"body").expect("fixture file");
                    }
                    let view = BrowserView::new(
                        Rc::new(crate::adapters::LocalFileSource),
                        PeekBehavior::default(),
                    );
                    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
                    view.set_view_mode(mode);
                    let window = gtk::Window::builder()
                        .child(&view.widget())
                        .default_width(800)
                        .default_height(600)
                        .build();
                    view.install_inline_edit_dismissal(&window);
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
                    let field = view
                        .state
                        .active_rename
                        .borrow()
                        .as_ref()
                        .map(|rename| rename.field.clone())
                        .or_else(|| view.state.mode_views.borrow().active_rename_field())
                        .expect("rename field");
                    let label = active_rename_label(&field);
                    field.set_text(replacement);
                    click_away(&window);
                    assert_eq!(label_text(&label).as_deref(), Some(replacement));

                    wait_until(|| {
                        replacement_path.exists() && !view.state.rename_operation_pending()
                    });
                    assert!(!original_path.exists());
                    assert_eq!(replacement_path.is_dir(), directory);
                    browser.clear_observer();
                    window.destroy();
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
            view.state
                .start_pending_rename(&fixture_entry("original.txt"), "renamed.txt".to_owned());

            view.browser().create_new_file(Location::local("/fixture"));

            assert!(!view.state.rename_operation_pending());
        },
    );
}

#[test]
fn completed_rename_is_reconciled_when_its_refresh_fails() {
    gtk_test(
        "ui::browser::inline_edit::tests::completed_rename_is_reconciled_when_its_refresh_fails",
        || {
            let view = BrowserView::new(
                Rc::new(RefreshSource {
                    loads: Cell::new(0),
                    refresh_behavior: RefreshBehavior::Fail,
                }),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            browser.navigate(Location::local("/fixture"));
            let entry = browser.entry_at(0, 0).expect("initial entry");
            view.state
                .start_pending_rename(&entry, "renamed.txt".to_owned());
            view.state
                .complete_pending_rename(crate::services::OperationRequestId(0));
            assert!(view.state.rename_operation_pending());

            browser.reload_active();

            assert!(!view.state.rename_operation_pending());
            assert!(
                browser
                    .column_snapshot(0)
                    .is_some_and(|snapshot| snapshot.error.as_deref() == Some("refresh failed"))
            );
        },
    );
}

#[test]
fn completed_rename_uses_the_replacement_refresh_after_the_first_is_abandoned() {
    gtk_test(
        "ui::browser::inline_edit::tests::completed_rename_uses_the_replacement_refresh_after_the_first_is_abandoned",
        || {
            let view = BrowserView::new(
                Rc::new(RefreshSource {
                    loads: Cell::new(0),
                    refresh_behavior: RefreshBehavior::AbandonOnce,
                }),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            browser.navigate(Location::local("/fixture"));
            let entry = browser.entry_at(0, 0).expect("initial entry");
            view.state
                .start_pending_rename(&entry, "renamed.txt".to_owned());
            view.state
                .complete_pending_rename(crate::services::OperationRequestId(0));

            browser.reload_active();
            assert!(view.state.rename_operation_pending());
            browser.reload_active();

            assert!(!view.state.rename_operation_pending());
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
