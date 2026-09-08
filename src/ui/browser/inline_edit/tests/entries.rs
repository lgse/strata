// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{SortDirection, SortKey};

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
    with_new_entry_at_config(mode, directory, path, |_| {}, run);
}

fn with_new_entry_at_config(
    mode: BrowserMode,
    directory: bool,
    path: &std::path::Path,
    configure: impl FnOnce(&BrowserView),
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
    configure(&view);
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
                        wait_until(|| rename_field(view).is_some_and(|field| field.is_mapped()));
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

fn scroll_positions(root: &gtk::Widget) -> Vec<f64> {
    let mut positions = Vec::new();
    if let Some(scroller) = root.downcast_ref::<gtk::ScrolledWindow>() {
        positions.push(scroller.vadjustment().value());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        positions.extend(scroll_positions(&current));
    }
    positions
}

fn assert_descending_folders_first(view: &BrowserView, final_name: &str, directory: bool) {
    let browser = view.browser();
    browser.with_entries(
        0,
        0..browser.column_snapshot(0).expect("column").count,
        |entries| {
            let position = entries
                .iter()
                .position(|entry| entry.display_name == final_name)
                .expect("final name in sorted listing");
            assert_eq!(entries[position].is_directory(), directory);
            let first_directory = entries
                .iter()
                .position(crate::model::FileEntry::is_directory);
            let first_file = entries.iter().position(|entry| !entry.is_directory());
            if directory {
                assert_eq!(Some(position), first_directory);
            } else {
                assert_eq!(Some(position), first_file);
                assert!(entries[..position].iter().all(FileEntry::is_directory));
            }
        },
    );
}

fn finish_creation_rename(
    view: &BrowserView,
    path: &std::path::Path,
    original: &str,
    final_name: &str,
    mode: BrowserMode,
    submit_on_focus_loss: bool,
) {
    wait_until(|| rename_field(view).is_some());
    let field = rename_field(view).expect("creation rename field");
    field.set_text(final_name);
    if submit_on_focus_loss {
        let focus_target = gtk::Button::new();
        view.state.overlay.add_overlay(&focus_target);
        assert!(focus_target.grab_focus());
    } else {
        field.emit_activate();
    }
    wait_until(|| path.join(final_name).exists());
    wait_until(|| {
        view.browser()
            .focused_entry()
            .is_some_and(|entry| entry.location == Location::local(path.join(final_name)))
    });
    let browser = view.browser();
    let position = browser
        .with_entries(
            0,
            0..browser.column_snapshot(0).expect("column").count,
            |entries| {
                entries
                    .iter()
                    .position(|entry| entry.location == Location::local(path.join(final_name)))
            },
        )
        .flatten()
        .expect("final name in sorted listing");
    assert_eq!(browser.selected_positions(0), vec![position], "{mode:?}");
    wait_until(|| view.item_view_has_focus());
    wait_until(|| view.state.created_entry_is_visible(0, position));
    assert!(path.join(original).exists() == (original == final_name));
    wait_until(|| view.state.pending_created_rename.borrow().is_none());
}

#[test]
fn final_names_keep_created_items_selected_focused_and_visible_in_every_view_mode() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::final_names_keep_created_items_selected_focused_and_visible_in_every_view_mode",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    let fixture = tempfile::tempdir().expect("large fixture");
                    for index in 0..240 {
                        std::fs::write(
                            fixture.path().join(format!("m-file-{index:04}.txt")),
                            b"body",
                        )
                        .expect("fixture file");
                        std::fs::create_dir(fixture.path().join(format!("m-folder-{index:04}")))
                            .expect("fixture directory");
                    }
                    let final_name = if directory {
                        "a-created-folder"
                    } else {
                        "a-created-file.txt"
                    };
                    with_new_entry_at(mode, directory, fixture.path(), |view, path, original| {
                        finish_creation_rename(view, path, original, final_name, mode, false);
                    });
                }
            }
        },
    );
}

#[test]
fn list_and_icons_creation_rename_activation_and_focus_loss_clear_tracking() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::list_and_icons_creation_rename_activation_and_focus_loss_clear_tracking",
        || {
            for mode in [BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    for submit_on_focus_loss in [false, true] {
                        with_new_entry(mode, directory, |view, path, original| {
                            let final_name = if directory {
                                "renamed-folder"
                            } else {
                                "renamed-file.txt"
                            };
                            finish_creation_rename(
                                view,
                                path,
                                original,
                                final_name,
                                mode,
                                submit_on_focus_loss,
                            );
                        });
                    }
                }
            }
        },
    );
}

#[test]
fn final_name_keeps_an_already_visible_created_item_viewport_in_place() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::final_name_keeps_an_already_visible_created_item_viewport_in_place",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let fixture = tempfile::tempdir().expect("fixture");
                for index in 0..120 {
                    std::fs::write(
                        fixture.path().join(format!("m-file-{index:04}.txt")),
                        b"body",
                    )
                    .expect("fixture file");
                    std::fs::create_dir(fixture.path().join(format!("m-folder-{index:04}")))
                        .expect("fixture directory");
                }
                with_new_entry_at(mode, false, fixture.path(), |view, path, original| {
                    wait_until(|| rename_field(view).is_some());
                    let before = scroll_positions(&view.widget());
                    let final_name = "new file renamed.txt";
                    finish_creation_rename(view, path, original, final_name, mode, false);
                    wait_until(|| {
                        scroll_positions(&view.widget())
                            .iter()
                            .zip(before.iter())
                            .all(|(actual, expected)| (actual - expected).abs() <= 32.0)
                    });
                });
            }
        },
    );
}

#[test]
fn final_names_follow_descending_folders_first_sorting_in_every_view_mode() {
    gtk_test(
        "ui::browser::inline_edit::tests::entries::final_names_follow_descending_folders_first_sorting_in_every_view_mode",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for directory in [false, true] {
                    let fixture = tempfile::tempdir().expect("large fixture");
                    for index in 0..120 {
                        std::fs::write(
                            fixture.path().join(format!("m-file-{index:04}.txt")),
                            b"body",
                        )
                        .expect("fixture file");
                        std::fs::create_dir(fixture.path().join(format!("m-folder-{index:04}")))
                            .expect("fixture directory");
                    }
                    let final_name = if directory {
                        "z-created-folder"
                    } else {
                        "z-created-file.txt"
                    };
                    with_new_entry_at_config(
                        mode,
                        directory,
                        fixture.path(),
                        |view| {
                            view.browser()
                                .set_sort(0, SortKey::Name, SortDirection::Descending);
                            wait_until(|| {
                                view.browser()
                                    .column_preferences(0)
                                    .is_some_and(|preferences| {
                                        preferences.sort_key == SortKey::Name
                                            && preferences.sort_direction
                                                == SortDirection::Descending
                                    })
                            });
                            view.browser().set_folders_first(0, true);
                            wait_until(|| {
                                view.browser()
                                    .column_preferences(0)
                                    .is_some_and(|preferences| {
                                        preferences.sort_key == SortKey::Name
                                            && preferences.sort_direction
                                                == SortDirection::Descending
                                            && preferences.folders_first
                                    })
                            });
                            wait_until(|| view.browser().column_is_settled(0));
                        },
                        |view, path, original| {
                            finish_creation_rename(view, path, original, final_name, mode, false);
                            assert_descending_folders_first(view, final_name, directory);
                        },
                    );
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
