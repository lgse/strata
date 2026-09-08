// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::created_entry::reveal_delta;
use super::entries::{rename_field, with_new_entry_setup};
use super::*;

use crate::services::*;

type PendingRename = (RenameRequest, Rc<dyn Fn(OperationEvent)>);

#[derive(Default)]
struct DelayedRename {
    request: std::cell::RefCell<Option<PendingRename>>,
}

macro_rules! local_operation {
    ($method:ident, $request:ty) => {
        fn $method(&self, request: $request, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
            crate::adapters::LocalOperationProvider.$method(request, emit)
        }
    };
}

impl OperationProvider for DelayedRename {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.request.replace(Some((request, emit)));
        LoadHandle::new(|| {})
    }
    local_operation!(create_directory, CreateDirectoryRequest);
    local_operation!(create_file, CreateFileRequest);
    local_operation!(paste, PasteRequest);
    local_operation!(undo_move, UndoMoveRequest);
    local_operation!(undo_copy, UndoCopyRequest);
    local_operation!(delete, DeleteRequest);
    local_operation!(restore, RestoreRequest);
    local_operation!(compress, CompressRequest);
    local_operation!(extract, ExtractRequest);
}

#[test]
fn delayed_naming_does_not_select_a_collision_or_expire_while_operation_is_pending() {
    gtk_test(
        "ui::browser::inline_edit::tests::reveal::delayed_naming_does_not_select_a_collision_or_expire_while_operation_is_pending",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for success in [false, true] {
                    let fixture = tempfile::tempdir().expect("fixture");
                    let provider = Rc::new(DelayedRename::default());
                    with_new_entry_setup(
                        mode,
                        false,
                        fixture.path(),
                        |view| view.set_operation_provider(provider.clone()),
                        |view, path, original| {
                            wait_until(|| rename_field(view).is_some());
                            let field = rename_field(view).expect("created entry editor");
                            field.set_text("destination");
                            field.emit_activate();
                            wait_until(|| provider.request.borrow().is_some());
                            std::fs::write(path.join("destination"), b"unrelated")
                                .expect("fixture file operation");
                            wait_until(|| {
                                view.browser()
                                    .column_snapshot(0)
                                    .is_some_and(|snapshot| snapshot.count == 2)
                            });
                            let wait = if mode == BrowserMode::Columns && success {
                                Duration::from_millis(5200)
                            } else {
                                Duration::from_millis(120)
                            };
                            let until = std::time::Instant::now() + wait;
                            while std::time::Instant::now() < until {
                                settle();
                                assert!(
                                    !target(view, "destination")
                                        .selection
                                        .is_selected(target(view, "destination").position)
                                );
                                assert!(view.new_entry_is_active());
                            }
                            let (request, emit) = provider
                                .request
                                .borrow_mut()
                                .take()
                                .expect("pending rename request");
                            if success {
                                std::fs::remove_file(path.join("destination"))
                                    .expect("fixture file operation");
                                std::fs::rename(path.join(original), path.join("destination"))
                                    .expect("fixture file operation");
                                emit(OperationEvent::Renamed {
                                    request_id: request.id,
                                });
                                wait_until(|| !view.new_entry_is_active());
                                settle();
                                assert_revealed(view, "destination");
                            } else {
                                emit(OperationEvent::Failed {
                                    request_id: request.id,
                                    message: "Destination exists".into(),
                                });
                                wait_until(|| !view.new_entry_is_active());
                                settle();
                                assert!(path.join(original).exists());
                                assert_eq!(
                                    std::fs::read(path.join("destination"))
                                        .expect("fixture file operation"),
                                    b"unrelated"
                                );
                                let destination = target(view, "destination");
                                assert!(!destination.selection.is_selected(destination.position));
                            }
                        },
                    );
                }
            }
        },
    );
}

#[test]
fn pending_naming_allows_scrolling_and_clears_when_cancelled_or_superseded() {
    gtk_test(
        "ui::browser::inline_edit::tests::reveal::pending_naming_allows_scrolling_and_clears_when_cancelled_or_superseded",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for outcome in ["cancel", "supersede", "success"] {
                    let fixture = tempfile::tempdir().expect("large fixture");
                    for index in 0..150 {
                        std::fs::write(fixture.path().join(format!("a-{index:04}")), b"")
                            .expect("fixture file");
                    }
                    let provider = Rc::new(DelayedRename::default());
                    with_new_entry_setup(
                        mode,
                        false,
                        fixture.path(),
                        |view| view.set_operation_provider(provider.clone()),
                        |view, path, original| {
                            wait_until(|| {
                                rename_field(view).is_some() && !view.new_entry_is_active()
                            });
                            let scroll = target(view, original)
                                .view
                                .ancestor(gtk::ScrolledWindow::static_type())
                                .and_downcast::<gtk::ScrolledWindow>()
                                .expect("created item scroller");
                            let field = rename_field(view).expect("created entry editor");
                            field.set_text("a-0000-new");
                            field.emit_activate();
                            wait_until(|| provider.request.borrow().is_some());
                            settle();
                            let adjustment = scroll.vadjustment();
                            assert!(adjustment.upper() > adjustment.page_size());
                            let requested = 0.0;
                            assert!((requested - adjustment.value()).abs() > 1.0);
                            adjustment.set_value(requested);
                            settle();
                            assert_eq!(
                                adjustment.value(),
                                requested,
                                "{mode:?}: pending rename blocked scrolling"
                            );
                            assert!(view.new_entry_is_active());
                            let (request, retained) = provider
                                .request
                                .borrow_mut()
                                .take()
                                .expect("retained pending rename callback");
                            if outcome == "success" {
                                std::fs::rename(path.join(original), path.join("a-0000-new"))
                                    .expect("complete delayed rename");
                                retained(OperationEvent::Renamed {
                                    request_id: request.id,
                                });
                                wait_until(|| !view.new_entry_is_active());
                                settle();
                                assert_revealed(view, "a-0000-new");
                                assert_eq!(
                                    adjustment.value(),
                                    requested,
                                    "{mode:?}: completion undid pending-operation scrolling"
                                );
                                return;
                            }
                            if outcome == "supersede" {
                                let entry = view.browser().entry_at(0, 0).expect("other entry");
                                view.browser().rename(entry, "other-renamed".into());
                            } else {
                                view.browser().cancel_file_operation();
                            }
                            assert!(!view.new_entry_is_active());
                            assert!(!view.rename_is_active());
                            retained(OperationEvent::Renamed {
                                request_id: request.id,
                            });
                            settle();
                            assert!(!view.new_entry_is_active());
                            assert_eq!(adjustment.value(), requested);
                            view.browser().cancel_file_operation();
                        },
                    );
                }
            }
        },
    );
}

#[test]
fn cancelling_created_naming_without_navigation_never_selects_another_item() {
    gtk_test(
        "ui::browser::inline_edit::tests::reveal::cancelling_created_naming_without_navigation_never_selects_another_item",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let fixture = tempfile::tempdir().expect("fixture directory");
                std::fs::write(fixture.path().join("destination"), b"unrelated")
                    .expect("fixture file operation");
                super::entries::with_new_entry_at(
                    mode,
                    false,
                    fixture.path(),
                    |view, path, original| {
                        wait_until(|| rename_field(view).is_some());
                        rename_field(view)
                            .expect("created entry editor")
                            .set_text("destination");
                        assert!(view.cancel_rename());
                        settle();
                        assert!(!view.new_entry_is_active());
                        assert!(!view.rename_is_active());
                        assert!(path.join(original).exists());
                        let destination = target(view, "destination");
                        assert!(!destination.selection.is_selected(destination.position));
                    },
                );
            }
        },
    );
}

fn settle() {
    let until = std::time::Instant::now() + Duration::from_millis(90);
    while std::time::Instant::now() < until {
        while gtk::glib::MainContext::default().pending() {
            gtk::glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn target(view: &BrowserView, name: &str) -> CreatedEntryTarget {
    let snapshot = view.browser().column_snapshot(0).expect("snapshot");
    let entry = view
        .browser()
        .with_entries(0, 0..snapshot.count, |entries| {
            entries
                .iter()
                .find(|entry| entry.display_name == name)
                .cloned()
        })
        .flatten()
        .expect("named entry");
    view.state
        .created_entry_target(0, &entry)
        .expect("displayed entry")
}

fn assert_revealed(view: &BrowserView, name: &str) {
    let target = target(view, name);
    assert!(target.selection.is_selected(target.position));
    assert_eq!(view.browser().selected_entries()[0].display_name, name);
    let widget = target.widget.expect("allocated item");
    let scroll = target
        .view
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("scroller");
    let bounds = widget
        .parent()
        .expect("item container")
        .compute_bounds(&scroll)
        .expect("bounds");
    assert!(bounds.y() >= -0.5, "top: {}", bounds.y());
    assert!(
        f64::from(bounds.y() + bounds.height()) <= scroll.vadjustment().page_size() + 0.5,
        "bottom: {}, viewport: {}",
        bounds.y() + bounds.height(),
        scroll.vadjustment().page_size()
    );
    let focused = target
        .view
        .root()
        .and_then(|root| root.focus())
        .expect("keyboard focus");
    assert!(
        focused == widget || focused.is_ancestor(&widget) || widget.is_ancestor(&focused),
        "focus is not on {name}"
    );
}

#[test]
fn columns_creation_uses_the_live_gtk_model_when_its_order_diverges() {
    gtk_test(
        "ui::browser::inline_edit::tests::reveal::columns_creation_uses_the_live_gtk_model_when_its_order_diverges",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            for name in ["a", "b"] {
                std::fs::write(fixture.path().join(name), b"").expect("fixture file");
            }
            with_new_entry_setup(
                BrowserMode::Columns,
                false,
                fixture.path(),
                |view| {
                    let columns = view.state.columns.borrow();
                    let column = &columns[0];
                    let sorter = gtk::CustomSorter::new(|left, right| {
                        right
                            .downcast_ref::<gtk::StringObject>()
                            .expect("model string")
                            .string()
                            .cmp(
                                &left
                                    .downcast_ref::<gtk::StringObject>()
                                    .expect("model string")
                                    .string(),
                            )
                            .into()
                    });
                    let model =
                        gtk::SortListModel::new(Some(column.filtered_model.clone()), Some(sorter));
                    column.selection.set_model(Some(&model));
                },
                |view, _, original| {
                    wait_until(|| rename_field(view).is_some());
                    let target = target(view, original);
                    assert_eq!(target.position, 0);
                    assert_eq!(
                        view.browser()
                            .entry_at(0, 2)
                            .expect("source entry")
                            .display_name,
                        original
                    );
                    let field = rename_field(view).expect("editor");
                    assert_eq!(field.text(), original);
                    assert!(field.is_ancestor(&target.widget.expect("allocated row")));
                },
            );
        },
    );
}

#[test]
fn cancelling_or_navigating_away_from_created_naming_never_reveals_a_late_target() {
    gtk_test(
        "ui::browser::inline_edit::tests::reveal::cancelling_or_navigating_away_from_created_naming_never_reveals_a_late_target",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for submit in [false, true] {
                    let fixture = tempfile::tempdir().expect("fixture");
                    let elsewhere = tempfile::tempdir().expect("navigation target");
                    super::entries::with_new_entry_at(
                        mode,
                        false,
                        fixture.path(),
                        |view, path, original| {
                            wait_until(|| rename_field(view).is_some());
                            let field = rename_field(view).expect("editor");
                            field.set_text("finished");
                            if submit {
                                field.emit_activate();
                            } else {
                                assert!(view.cancel_rename());
                            }
                            view.browser().navigate(Location::local(elsewhere.path()));
                            wait_until(|| {
                                view.browser()
                                    .column_snapshot(0)
                                    .is_some_and(|snapshot| !snapshot.loading)
                            });
                            settle();
                            assert!(!view.new_entry_is_active());
                            assert!(!view.rename_is_active());
                            assert!(view.browser().selected_entries().is_empty());
                            assert!(
                                path.join(if submit { "finished" } else { original })
                                    .exists()
                            );
                        },
                    );
                }
            }
        },
    );
}

#[test]
fn accepting_the_default_name_returns_focus_to_the_created_item() {
    gtk_test(
        "ui::browser::inline_edit::tests::reveal::accepting_the_default_name_returns_focus_to_the_created_item",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let fixture = tempfile::tempdir().expect("fixture");
                super::entries::with_new_entry_at(
                    mode,
                    false,
                    fixture.path(),
                    |view, _, original| {
                        wait_until(|| rename_field(view).is_some() && !view.new_entry_is_active());
                        rename_field(view).expect("editor").emit_activate();
                        wait_until(|| !view.new_entry_is_active());
                        settle();
                        assert_revealed(view, original);
                    },
                );
            }
        },
    );
}

#[test]
fn minimal_reveal_delta_preserves_both_visible_directions() {
    assert_eq!(reveal_delta(20.0, 30.0, 100.0), 0.0);
    assert_eq!(reveal_delta(60.0, 30.0, 100.0), 0.0);
    assert_eq!(reveal_delta(-12.0, 30.0, 100.0), -12.0);
    assert_eq!(reveal_delta(85.0, 30.0, 100.0), 15.0);
}

#[test]
fn naming_created_items_reveals_final_sorted_identity_without_moving_visible_items() {
    gtk_test(
        "ui::browser::inline_edit::tests::reveal::naming_created_items_reveals_final_sorted_identity_without_moving_visible_items",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    for descending in [false, true] {
                        for name in ["m-0048-final", "p-0000-final", "a-first", "z-last"] {
                            let fixture = tempfile::tempdir().expect("fixture");
                            for prefix in ["m", "p"] {
                                for index in 0..50 {
                                    let path = fixture.path().join(format!("{prefix}-{index:04}"));
                                    if directory {
                                        std::fs::create_dir(path).expect("fixture folder");
                                    } else {
                                        std::fs::write(path, b"").expect("fixture file");
                                    }
                                }
                            }
                            // Keep the other kind present to exercise folders-first ordering.
                            if directory {
                                std::fs::write(fixture.path().join("other-file"), b"")
                                    .expect("other file");
                            } else {
                                std::fs::create_dir(fixture.path().join("other-folder"))
                                    .expect("other folder");
                            }
                            with_new_entry_setup(
                                mode,
                                directory,
                                fixture.path(),
                                |view| {
                                    view.browser()
                                        .set_folders_first(0, !directory || descending);
                                    view.browser().set_sort_direction(
                                        0,
                                        if descending {
                                            crate::model::SortDirection::Descending
                                        } else {
                                            crate::model::SortDirection::Ascending
                                        },
                                    );
                                    settle();
                                },
                                |view, path, original| {
                                    wait_until(|| rename_field(view).is_some());
                                    let field = rename_field(view).expect("editor");
                                    let initial = target(view, original);
                                    let scroll = initial
                                        .view
                                        .ancestor(gtk::ScrolledWindow::static_type())
                                        .and_downcast::<gtk::ScrolledWindow>()
                                        .expect("scroller");
                                    let adjustment = scroll.vadjustment();
                                    if let Some(widget) = initial.widget {
                                        let bounds =
                                            widget.compute_bounds(&scroll).expect("bounds");
                                        adjustment.set_value(
                                            adjustment.value()
                                                + f64::from(bounds.y() + bounds.height() / 2.0)
                                                - adjustment.page_size() / 2.0,
                                        );
                                    }
                                    settle();
                                    let before = adjustment.value();
                                    let observed = Rc::new(std::cell::RefCell::new(Vec::new()));
                                    let values = observed.clone();
                                    let observed_adjustment = adjustment.clone();
                                    let listener = view.widget().add_tick_callback(move |_, _| {
                                        values.borrow_mut().push(observed_adjustment.value());
                                        gtk::glib::ControlFlow::Continue
                                    });
                                    field.set_text(name);
                                    field.emit_activate();
                                    wait_until(|| {
                                        path.join(name).exists() && !view.new_entry_is_active()
                                    });
                                    settle();
                                    listener.remove();
                                    assert_revealed(view, name);
                                    if name == "a-first" || name == "z-last" {
                                        let item = target(view, name).widget.expect("final item");
                                        let bounds = item
                                            .parent()
                                            .expect("item container")
                                            .compute_bounds(&scroll)
                                            .expect("final bounds");
                                        if (name == "a-first") != descending {
                                            assert!(
                                                bounds.y().abs() <= 0.5,
                                                "{mode:?}, dir={directory}, descending={descending}, {name}: must reveal only to top, bounds={bounds:?}, scroll={} baseline={before}",
                                                adjustment.value()
                                            );
                                        } else {
                                            assert!(
                                                (f64::from(bounds.y() + bounds.height())
                                                    - adjustment.page_size())
                                                .abs()
                                                    <= 0.5,
                                                "{mode:?}, dir={directory}, descending={descending}, {name}: must reveal only to bottom, bounds={bounds:?}, scroll={} baseline={before}",
                                                adjustment.value()
                                            );
                                        }
                                    }
                                    if name.starts_with("m-") || name.starts_with("p-") {
                                        assert!(
                                            observed.borrow().iter().all(|value| *value == before),
                                            "{mode:?}, dir={directory}, descending={descending}, {name}: viewport moved during naming: {:?}",
                                            observed.borrow()
                                        );
                                        assert_eq!(
                                            adjustment.value(),
                                            before,
                                            "{mode:?}, directory={directory}, descending={descending}, {name}"
                                        );
                                    }
                                },
                            );
                        }
                    }
                }
            }
        },
    );
}
