// SPDX-License-Identifier: MIT

use super::*;

fn rename_field(view: &BrowserView) -> Option<gtk::Entry> {
    view.state
        .active_rename
        .borrow()
        .as_ref()
        .map(|active| active.field.clone())
        .or_else(|| view.state.mode_views.borrow().active_rename_field())
}

fn with_new_entry(
    mode: BrowserMode,
    directory: bool,
    run: impl FnOnce(&BrowserView, &std::path::Path, &str),
) {
    let fixture = tempfile::tempdir().expect("fixture");
    with_new_entry_at(mode, directory, fixture.path(), run);
}

fn with_new_entry_at(
    mode: BrowserMode,
    directory: bool,
    path: &std::path::Path,
    run: impl FnOnce(&BrowserView, &std::path::Path, &str),
) {
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
    view.browser().navigate(Location::local(path));
    wait_until(|| {
        view.browser()
            .column_snapshot(0)
            .is_some_and(|snapshot| !snapshot.loading)
    });
    view.state
        .begin_new_entry(0, Location::local(path), directory);
    run(
        &view,
        path,
        if directory { "new folder" } else { "new file" },
    );
    view.browser().clear_observer();
    window.destroy();
}

#[test]
fn new_entries_scroll_into_view_when_the_default_name_sorts_past_the_initial_viewport() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::new_entries_scroll_into_view_when_the_default_name_sorts_past_the_initial_viewport",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    let fixture = tempfile::tempdir().expect("large fixture");
                    for index in 0..1000 {
                        std::fs::write(
                            fixture.path().join(format!("a-file-{index:04}.txt")),
                            b"body",
                        )
                        .expect("fixture file");
                        std::fs::create_dir(fixture.path().join(format!("a-folder-{index:04}")))
                            .expect("fixture directory");
                    }
                    with_new_entry_at(mode, directory, fixture.path(), |view, path, original| {
                        wait_until(|| rename_field(view).is_some());
                        let field = rename_field(view).expect("offscreen item editor");
                        assert!(field.is_mapped());
                        assert_eq!(field.text(), original);
                        assert!(path.join(original).exists());
                        let bounds = field.compute_bounds(&view.widget()).expect("editor bounds");
                        assert!(
                            bounds.y() >= 0.0 && bounds.y() < view.widget().height() as f32,
                            "{mode:?}: editor is outside the visible pane"
                        );
                    });
                }
            }
        },
    );
}

#[test]
fn new_entries_start_fully_selected_and_invalid_names_retain_the_created_item() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::new_entries_start_fully_selected_and_invalid_names_retain_the_created_item",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    for name in ["", " \u{2003}\u{00a0}", ".", "..", "nested/name"] {
                        with_new_entry(mode, directory, |view, path, original| {
                            wait_until(|| rename_field(view).is_some());
                            let field = rename_field(view).expect("rename field");
                            assert!(path.join(original).exists());
                            assert_eq!(path.join(original).is_dir(), directory);
                            if !directory {
                                assert_eq!(
                                    std::fs::read(path.join(original)).expect("empty file"),
                                    b""
                                );
                            }
                            assert_eq!(field.text(), original);
                            assert_eq!(field.selection_bounds(), Some((0, original.len() as i32)));
                            field.set_text(name);
                            field.emit_activate();
                            wait_until(|| !view.rename_is_active());
                            assert!(path.join(original).exists());
                            assert_eq!(std::fs::read_dir(path).expect("listing").count(), 1);
                        });
                    }
                }
            }
        },
    );
}

#[test]
fn navigating_before_creation_finishes_does_not_open_an_editor_in_the_new_location() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::navigating_before_creation_finishes_does_not_open_an_editor_in_the_new_location",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    with_new_entry(mode, directory, |view, path, original| {
                        let elsewhere = path.join("elsewhere");
                        std::fs::create_dir(&elsewhere).expect("navigation target");
                        view.browser().navigate(Location::local(&elsewhere));
                        wait_until(|| path.join(original).exists());
                        wait_until(|| {
                            view.browser().column_snapshot(0).is_some_and(|snapshot| {
                                !snapshot.loading
                                    && snapshot.location == Location::local(&elsewhere)
                            })
                        });
                        assert!(!view.rename_is_active());
                        assert!(!view.new_entry_is_active());
                        assert!(!elsewhere.join(original).exists());
                    });
                }
            }
        },
    );
}

#[test]
fn new_folder_from_the_parent_background_replaces_a_stale_child_column() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::new_folder_from_the_parent_background_replaces_a_stale_child_column",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let existing = fixture.path().join("existing");
            std::fs::create_dir(&existing).expect("existing child folder");

            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(800)
                .default_height(600)
                .build();
            view.install_inline_edit_dismissal(&window);
            window.present();
            view.browser().navigate(Location::local(fixture.path()));
            wait_until(|| {
                view.browser()
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading)
            });

            view.browser().select(0, 0);
            view.browser().activate_focused();
            wait_until(|| view.browser().location_at(1) == Some(Location::local(&existing)));

            view.state
                .begin_new_entry(0, Location::local(fixture.path()), true);
            wait_until(|| rename_field(&view).is_some());

            let new_folder = fixture.path().join("new folder");
            assert_eq!(
                view.browser().location_at(1),
                Some(Location::local(&new_folder)),
                "the child column should follow the newly created folder, not stay on the previously opened one"
            );

            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn new_file_from_the_parent_background_closes_a_stale_child_column_without_opening_it() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::new_file_from_the_parent_background_closes_a_stale_child_column_without_opening_it",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let existing = fixture.path().join("existing");
            std::fs::create_dir(&existing).expect("existing child folder");

            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(800)
                .default_height(600)
                .build();
            view.install_inline_edit_dismissal(&window);
            window.present();
            view.browser().navigate(Location::local(fixture.path()));
            wait_until(|| {
                view.browser()
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading)
            });

            view.browser().select(0, 0);
            view.browser().activate_focused();
            wait_until(|| view.browser().location_at(1) == Some(Location::local(&existing)));

            let opened = Rc::new(Cell::new(false));
            let observed_opened = opened.clone();
            view.browser().observe(move |event| {
                if matches!(event, crate::app::BrowserEvent::OpenRequested { .. }) {
                    observed_opened.set(true);
                }
            });

            view.state
                .begin_new_entry(0, Location::local(fixture.path()), false);
            wait_until(|| rename_field(&view).is_some());

            assert_eq!(
                view.browser().location_at(1),
                None,
                "a new file has no children; the stale child column must close"
            );
            assert!(
                !opened.get(),
                "creating a file must never request opening it"
            );
            let field = rename_field(&view).expect("rename field");
            assert_eq!(field.text(), "new file");
            assert_eq!(field.selection_bounds(), Some((0, "new file".len() as i32)));

            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn renaming_the_just_created_folder_updates_the_child_column_again() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::renaming_the_just_created_folder_updates_the_child_column_again",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(800)
                .default_height(600)
                .build();
            view.install_inline_edit_dismissal(&window);
            window.present();
            view.browser().navigate(Location::local(fixture.path()));
            wait_until(|| {
                view.browser()
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading)
            });

            view.state
                .begin_new_entry(0, Location::local(fixture.path()), true);
            wait_until(|| rename_field(&view).is_some());
            let new_folder = fixture.path().join("new folder");
            wait_until(|| view.browser().location_at(1) == Some(Location::local(&new_folder)));

            let field = rename_field(&view).expect("rename field");
            field.set_text("renamed");
            field.emit_activate();
            // Submitting closes the editor synchronously; the rename itself
            // dispatches asynchronously, so wait for it on disk.
            let renamed = fixture.path().join("renamed");
            wait_until(|| renamed.is_dir());
            wait_until(|| view.browser().location_at(1) == Some(Location::local(&renamed)));

            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn creating_and_renaming_a_new_file_never_requests_opening_it() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::creating_and_renaming_a_new_file_never_requests_opening_it",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(800)
                .default_height(600)
                .build();
            view.install_inline_edit_dismissal(&window);
            window.present();
            view.browser().navigate(Location::local(fixture.path()));
            wait_until(|| {
                view.browser()
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading)
            });

            // Registered before creation starts so it catches an open request
            // from the initial select as well as from completing the rename.
            let opened = Rc::new(Cell::new(false));
            let observed_opened = opened.clone();
            view.browser().observe(move |event| {
                if matches!(event, crate::app::BrowserEvent::OpenRequested { .. }) {
                    observed_opened.set(true);
                }
            });

            view.state
                .begin_new_entry(0, Location::local(fixture.path()), false);
            wait_until(|| rename_field(&view).is_some());
            assert!(
                !opened.get(),
                "creating a file must never request opening it"
            );
            let field = rename_field(&view).expect("rename field");
            assert_eq!(field.text(), "new file");

            field.set_text("submitted-file.txt");
            field.emit_activate();
            // Submitting closes the editor synchronously; the rename itself
            // dispatches asynchronously, so wait for it on disk.
            let renamed = fixture.path().join("submitted-file.txt");
            wait_until(|| renamed.exists());
            assert!(!fixture.path().join("new file").exists());
            assert!(
                !opened.get(),
                "Enter must submit the rename, not also activate the item"
            );
            wait_until(|| {
                view.browser()
                    .selected_entries()
                    .iter()
                    .map(|entry| entry.display_name.as_str())
                    .eq(["submitted-file.txt"])
            });

            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn clicking_a_different_row_during_rename_submits_and_selects_the_clicked_row() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::clicking_a_different_row_during_rename_submits_and_selects_the_clicked_row",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::write(fixture.path().join("sibling.txt"), b"body").expect("sibling file");
            with_new_entry_at(
                BrowserMode::Columns,
                false,
                fixture.path(),
                |view, path, original| {
                    wait_until(|| rename_field(view).is_some());
                    let field = rename_field(view).expect("rename field");
                    field.set_text("renamed-file.txt");

                    // The dismissal gesture submits in GTK's capture phase, before the
                    // clicked row's own bubble-phase click handler runs its normal
                    // selection -- reproduce that order directly rather than the click.
                    // The rename itself dispatches asynchronously, so select the
                    // clicked row immediately, the way the row's own handler would
                    // run right after the capture-phase dismissal, well before the
                    // rename resolves on disk.
                    view.state.submit_rename(&field);
                    let sibling_position = (0..2)
                        .find(|position| {
                            view.browser()
                                .entry_at(0, *position)
                                .is_some_and(|entry| entry.display_name == "sibling.txt")
                        })
                        .expect("sibling position");
                    view.browser().select(0, sibling_position);

                    wait_until(|| path.join("renamed-file.txt").exists());
                    assert!(!path.join(original).exists());
                    assert_eq!(
                        view.browser()
                            .selected_entries()
                            .into_iter()
                            .map(|entry| entry.display_name)
                            .collect::<Vec<_>>(),
                        vec!["sibling.txt"],
                        "completing the rename after the fact must not steal back the selection"
                    );
                },
            );
        },
    );
}

#[test]
fn clicking_away_with_an_unchanged_name_just_closes_the_editor() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::clicking_away_with_an_unchanged_name_just_closes_the_editor",
        || {
            with_new_entry(BrowserMode::Columns, true, |view, path, original| {
                wait_until(|| rename_field(view).is_some());
                let field = rename_field(view).expect("rename field");
                view.state.submit_rename(&field);
                wait_until(|| !view.rename_is_active());

                assert!(path.join(original).is_dir());
                assert_eq!(
                    std::fs::read_dir(path).expect("listing").count(),
                    1,
                    "an unchanged name must not create or duplicate anything"
                );
            });
        },
    );
}

#[test]
fn a_rename_conflict_retains_the_item_under_its_original_name() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::a_rename_conflict_retains_the_item_under_its_original_name",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::write(fixture.path().join("taken.txt"), b"body").expect("colliding name");
            with_new_entry_at(
                BrowserMode::Columns,
                false,
                fixture.path(),
                |view, path, original| {
                    wait_until(|| rename_field(view).is_some());
                    let field = rename_field(view).expect("rename field");
                    field.set_text("taken.txt");
                    field.emit_activate();
                    wait_until(|| !view.state.rename_operation_pending());

                    assert!(
                        path.join(original).exists(),
                        "a rename conflict must retain the item under its original name"
                    );
                    assert_eq!(
                        std::fs::read(path.join("taken.txt")).expect("original file untouched"),
                        b"body",
                        "the pre-existing file must not be overwritten"
                    );
                },
            );
        },
    );
}

#[test]
fn cancelling_before_creation_finishes_does_not_open_a_late_editor_or_delete_the_item() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::cancelling_before_creation_finishes_does_not_open_a_late_editor_or_delete_the_item",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    with_new_entry(mode, directory, |view, path, original| {
                        assert!(view.cancel_new_entry());
                        wait_until(|| path.join(original).exists());
                        wait_until(|| {
                            view.browser()
                                .column_snapshot(0)
                                .is_some_and(|snapshot| snapshot.count == 1)
                        });
                        while gtk::glib::MainContext::default().pending() {
                            gtk::glib::MainContext::default().iteration(false);
                        }
                        assert!(!view.rename_is_active());
                        assert!(!view.new_entry_is_active());
                        assert!(path.join(original).exists());
                    });
                }
            }
        },
    );
}
