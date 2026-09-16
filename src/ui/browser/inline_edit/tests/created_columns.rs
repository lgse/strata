// SPDX-License-Identifier: MIT

use super::*;

fn window_for(view: &BrowserView, width: i32) -> gtk::Window {
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(width)
        .default_height(600)
        .build();
    view.install_inline_edit_dismissal(&window);
    window.present();
    window
}

fn current_field(view: &BrowserView) -> Option<gtk::Entry> {
    view.state
        .active_rename
        .borrow()
        .as_ref()
        .map(|active| active.field.clone())
}

fn assert_focus_within(window: &gtk::Window, widget: &impl IsA<gtk::Widget>) {
    let focus = gtk::prelude::RootExt::focus(window).expect("keyboard focus");
    assert!(
        focus == *widget.as_ref() || focus.is_ancestor(widget),
        "focus={focus:?}, expected within {:?}",
        widget.as_ref()
    );
}

fn with_created_entry(
    directory: bool,
    stale_child: bool,
    run: impl FnOnce(&BrowserView, &gtk::Window, &std::path::Path),
) {
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::create_dir_all(fixture.path().join("existing/nested")).expect("existing descendants");
    std::fs::write(fixture.path().join("sibling.txt"), b"keep").expect("sibling file");
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view.set_view_mode(BrowserMode::Columns);
    let window = window_for(&view, 800);
    view.browser().navigate(Location::local(fixture.path()));
    wait_until(|| {
        view.browser()
            .column_snapshot(0)
            .is_some_and(|s| !s.loading)
    });
    if stale_child {
        view.browser().activate(0, 0);
        wait_until(|| {
            view.browser()
                .column_snapshot(1)
                .is_some_and(|s| !s.loading)
        });
        view.browser().activate(1, 0);
        wait_until(|| {
            view.browser()
                .column_snapshot(2)
                .is_some_and(|s| !s.loading)
        });
    }
    let opened = Rc::new(Cell::new(0));
    let observed = opened.clone();
    view.browser().observe(move |event| {
        if matches!(event, crate::app::BrowserEvent::OpenRequested { .. }) {
            observed.set(observed.get() + 1);
        }
    });
    view.state
        .begin_new_entry(0, Location::local(fixture.path()), directory);
    wait_until(|| current_field(&view).is_some_and(|field| field.is_mapped() && field.width() > 0));
    let field = current_field(&view).expect("created-entry editor");
    let original = if directory { "new folder" } else { "new file" };
    assert_eq!(field.text(), original);
    assert_eq!(field.selection_bounds(), Some((0, original.len() as i32)));
    assert_focus_within(&window, &field);
    assert_eq!(view.browser().active_depth(), Some(0));
    assert_eq!(view.browser().selected_entries()[0].display_name, original);
    assert_eq!(view.browser().location_at(2), None);
    let expected_location = if directory {
        let child = Location::local(fixture.path().join(original));
        assert_eq!(view.browser().location_at(1), Some(child.clone()));
        child
    } else {
        assert_eq!(view.browser().location_at(1), None);
        Location::local(fixture.path())
    };
    assert_eq!(
        view.state.location_entry.text(),
        expected_location.display_path()
    );
    run(&view, &window, fixture.path());
    assert_eq!(
        opened.get(),
        0,
        "creation and completion must never open a file"
    );
    view.browser().clear_observer();
    window.destroy();
}

fn wait_for_renamed(view: &BrowserView, path: &std::path::Path, directory: bool) {
    let renamed = path.join("renamed");
    wait_until(|| renamed.exists());
    wait_until(|| {
        view.browser()
            .column_snapshot(0)
            .is_some_and(|s| !s.loading)
    });
    if directory {
        wait_until(|| {
            view.browser().column_snapshot(1).is_some_and(|snapshot| {
                !snapshot.loading && snapshot.location == Location::local(&renamed)
            })
        });
        assert_eq!(
            std::path::Path::new(view.state.location_entry.text().as_str()),
            renamed
        );
    }
}

#[test]
fn enter_preserves_created_entry_selection_and_parent_keyboard_focus() {
    gtk_test(
        "ui::browser::inline_edit::tests::created_columns::enter_preserves_created_entry_selection_and_parent_keyboard_focus",
        || {
            for directory in [false, true] {
                for stale_child in [false, true] {
                    with_created_entry(directory, stale_child, |view, window, path| {
                        let field = current_field(view).expect("editor");
                        field.set_text("renamed");
                        field.emit_activate();
                        wait_for_renamed(view, path, directory);
                        wait_until(|| {
                            view.browser()
                                .selected_entries()
                                .iter()
                                .map(|entry| entry.display_name.as_str())
                                .eq(["renamed"])
                        });
                        assert_eq!(view.browser().active_depth(), Some(0));
                        assert_eq!(view.browser().selected_positions(0).len(), 1);
                        let list = view.state.columns.borrow()[0].list.clone();
                        wait_until(|| {
                            gtk::prelude::RootExt::focus(window)
                                .is_some_and(|focus| focus == list || focus.is_ancestor(&list))
                        });
                        wait_until(|| view.state.begin_rename());
                        let reopened = current_field(view).expect("reopened editor");
                        assert_eq!(reopened.text(), "renamed");
                        assert_focus_within(window, &reopened);
                    });
                }
            }
        },
    );
}

#[test]
fn completing_rename_recovers_the_panes_fallback_keyboard_focus() {
    gtk_test(
        "ui::browser::inline_edit::tests::created_columns::completing_rename_recovers_the_panes_fallback_keyboard_focus",
        || {
            with_created_entry(false, false, |view, window, path| {
                let field = current_field(view).expect("editor");
                field.set_text("renamed");
                field.emit_activate();
                // GTK can return focus to the pane stack while replacing the native row.
                let column = view.state.columns.borrow()[0].clone();
                gtk::prelude::RootExt::set_focus(window, Some(&column.presentation.stack));
                wait_for_renamed(view, path, false);
                wait_until(|| {
                    gtk::prelude::RootExt::focus(window).is_some_and(|focus| {
                        focus == column.list || focus.is_ancestor(&column.list)
                    })
                });
                assert_eq!(view.browser().selected_entries()[0].display_name, "renamed");
            });
        },
    );
}

#[test]
fn folder_click_away_keeps_focus_in_the_clicked_control() {
    gtk_test(
        "ui::browser::inline_edit::tests::created_columns::folder_click_away_keeps_focus_in_the_clicked_control",
        || {
            with_created_entry(true, true, |view, window, path| {
                current_field(view).expect("editor").set_text("renamed");
                click_away(window);
                let control = view.state.columns.borrow()[0].filter_button.clone();
                assert!(control.grab_focus());
                wait_for_renamed(view, path, true);
                assert_focus_within(window, &control);
            });
        },
    );
}

#[test]
fn folder_rename_completion_does_not_restore_selection_or_reopen_a_closed_path() {
    gtk_test(
        "ui::browser::inline_edit::tests::created_columns::folder_rename_completion_does_not_restore_selection_or_reopen_a_closed_path",
        || {
            for navigate in [false, true] {
                with_created_entry(true, true, |view, window, path| {
                    current_field(view).expect("editor").set_text("renamed");
                    click_away(window);
                    if navigate {
                        view.browser()
                            .navigate(Location::local(path.join("existing")));
                    } else {
                        let position = (0..view
                            .browser()
                            .column_snapshot(0)
                            .expect("parent column")
                            .count)
                            .find(|position| {
                                view.browser()
                                    .entry_at(0, *position)
                                    .is_some_and(|entry| entry.display_name == "sibling.txt")
                            })
                            .expect("sibling position");
                        view.browser().select(0, position);
                    }
                    wait_until(|| path.join("renamed").is_dir());
                    wait_until(|| !view.state.rename_operation_pending());
                    if navigate {
                        assert_eq!(
                            view.browser().location_at(0),
                            Some(Location::local(path.join("existing")))
                        );
                        assert_eq!(view.browser().location_at(1), None);
                    } else {
                        wait_for_renamed(view, path, true);
                        assert_eq!(
                            view.browser().selected_entries()[0].display_name,
                            "sibling.txt"
                        );
                        assert_eq!(view.browser().active_depth(), Some(0));
                    }
                });
            }
        },
    );
}

#[test]
fn failed_folder_rename_keeps_the_original_child_path_and_reports_the_error() {
    gtk_test(
        "ui::browser::inline_edit::tests::created_columns::failed_folder_rename_keeps_the_original_child_path_and_reports_the_error",
        || {
            with_created_entry(true, true, |view, _, path| {
                let errors = Rc::new(Cell::new(0));
                let observed = errors.clone();
                view.browser().observe(move |event| {
                    if matches!(event, crate::app::BrowserEvent::RenameFailed { .. }) {
                        observed.set(observed.get() + 1);
                    }
                });
                let field = current_field(view).expect("editor");
                field.set_text("existing");
                field.emit_activate();
                wait_until(|| errors.get() == 1);
                assert!(path.join("new folder").is_dir());
                assert!(path.join("existing/nested").is_dir());
                assert_eq!(
                    view.browser().location_at(1),
                    Some(Location::local(path.join("new folder")))
                );
                assert_eq!(
                    view.state.location_entry.text(),
                    path.join("new folder").display().to_string()
                );
                assert_eq!(
                    view.browser().selected_entries()[0].display_name,
                    "new folder"
                );
                assert!(!view.rename_is_active());
            });
        },
    );
}

type Validation = Rc<dyn Fn(Result<(), LocationValidationError>)>;

#[derive(Default)]
struct AsyncFolderSource {
    pending: RefCell<Option<Validation>>,
}

impl FileSource for AsyncFolderSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn validate_location_async(&self, location: Location, emit: Validation) -> LoadHandle {
        if location == Location::uri("smb://host/share") {
            emit(Ok(()));
        } else {
            self.pending.replace(Some(emit));
        }
        LoadHandle::new(|| {})
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if request.location == Location::uri("smb://host/share") {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: vec![remote_entry("new folder", true)],
            });
        }
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn created_folder_reveal_does_not_start_a_late_navigation() {
    gtk_test(
        "ui::browser::inline_edit::tests::created_columns::created_folder_reveal_does_not_start_a_late_navigation",
        || {
            let source = Rc::new(AsyncFolderSource::default());
            let view = BrowserView::new(source.clone(), PeekBehavior::default());
            view.set_view_mode(BrowserMode::Columns);
            let window = window_for(&view, 800);
            let parent = Location::uri("smb://host/share");
            view.browser().navigate(parent.clone());
            wait_until(|| {
                view.browser()
                    .column_snapshot(0)
                    .is_some_and(|s| !s.loading)
            });
            // The state at EntryCreated, after the parent listing has refreshed.
            view.state
                .pending_new_entry
                .replace(Some(Rc::new(PendingEntryRename {
                    depth: 0,
                    parent,
                    reveal_generation: view.state.rename_reveal_generation.get(),
                })));
            view.state
                .rename_created_entry(&remote_entry("new folder", true).location);
            wait_until(|| current_field(&view).is_some());
            assert!(source.pending.borrow().is_none());
            assert_eq!(
                view.browser().location_at(1),
                Some(remote_entry("new folder", true).location)
            );
            assert_focus_within(&window, &current_field(&view).expect("editor"));
            view.state.cancel_rename();
            view.browser().navigate(Location::uri("smb://host/share"));
            assert!(!view.new_entry_is_active());
            assert!(!view.rename_is_active());
            view.browser().clear_observer();
            window.destroy();
        },
    );
}
