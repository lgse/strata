// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{
    browser::PeekBehavior,
    browser_modes::BrowserMode,
    preferences::{ListColumnId, ListColumns},
};
use acceptance::{request, wait_until};
use std::{
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn open_browser(root: &Path, chooser: bool) -> gtk::Window {
    let (window, browser) = if chooser {
        let state = build_chooser(
            request(root.to_path_buf()),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .expect("chooser");
        (state.window.clone(), state.view.browser())
    } else {
        let view = crate::ui::browser::BrowserView::new(
            Rc::new(crate::adapters::LocalFileSource),
            PeekBehavior::default(),
        );
        let window = gtk::Window::builder().child(&view.widget()).build();
        window.present();
        view.navigate_location(Location::local(root));
        (window, view.browser())
    };
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    window
}

/// A browser window composed like the main window: the preview split publishes
/// the visible browser area that bounds Miller columns.
fn open_split_browser(root: &Path) -> (gtk::Window, PreviewDrawer) {
    let view = crate::ui::browser::BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    let preview = PreviewDrawer::new(
        Rc::new(LocalPreviewProvider::new(Rc::new(|| {
            PreferenceManager::shared().media_preview_backend()
        }))),
        false,
    );
    let content = gtk::Paned::new(gtk::Orientation::Horizontal);
    content.set_end_child(Some(&view.widget()));
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.add_css_class("preview-split");
    split.set_wide_handle(true);
    split.set_start_child(Some(&content));
    split.set_end_child(Some(&preview.widget()));
    split.set_position(i32::MAX);
    let window = gtk::Window::builder()
        .default_width(1200)
        .default_height(700)
        .child(&split)
        .build();
    preview.attach_split(&split, &content, &view, None);
    window.present();
    view.navigate_location(Location::local(root));
    let browser = view.browser();
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    (window, preview)
}

fn open_bounded_browser(root: &Path, chooser: bool) -> (gtk::Window, Option<PreviewDrawer>) {
    let (window, preview) = if chooser {
        (open_browser(root, true), None)
    } else {
        let (window, preview) = open_split_browser(root);
        (window, Some(preview))
    };
    let shell = column_shell(&window);
    wait_until(|| shell.width() > 0);
    settle();
    (window, preview)
}

fn settle() {
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn visible_browser_area_unscaled(window: &gtk::Window) -> i32 {
    let shell = column_shell(window);
    let content = shell
        .ancestor(gtk::Paned::static_type())
        .and_downcast::<gtk::Paned>()
        .expect("sidebar split");
    let split = content
        .parent()
        .and_downcast::<gtk::Paned>()
        .expect("preview split");
    let sidebar = content
        .start_child()
        .filter(|sidebar| sidebar.is_visible())
        .map_or(0, |_| {
            content.position() + crate::ui::preview::separator_width(&content)
        });
    let area = split.width() - crate::ui::preview::separator_width(&split) - sidebar;
    (f64::from(area) / PreferenceManager::shared().interface_scale()).ceil() as i32
}

fn autofit_column(window: &gtk::Window) {
    let shell = column_shell(window);
    let scroller = shell
        .ancestor(gtk::ScrolledWindow::static_type())
        .expect("columns scroller");
    let bounds = shell.compute_bounds(&scroller).expect("column bounds");
    let edge = (
        f64::from(bounds.x() + bounds.width()) - 0.5,
        f64::from(bounds.y()) + 4.0,
    );
    let gesture = resize_drag(&scroller);
    gesture.emit_by_name::<()>("drag-begin", &[&edge.0, &edge.1]);
    gesture.emit_by_name::<()>("drag-begin", &[&edge.0, &edge.1]);
    gesture.emit_by_name::<()>("drag-end", &[&0.0f64, &0.0f64]);
    // Past the autofit animation and the 400 ms double-press window.
    settle();
}

fn descendants_with_class(widget: &gtk::Widget, class: &str, found: &mut Vec<gtk::Widget>) {
    if widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        descendants_with_class(&current, class, found);
        child = current.next_sibling();
    }
}

fn resize_handle(cell: &gtk::Widget) -> gtk::Widget {
    let mut handles = Vec::new();
    descendants_with_class(cell, "list-column-resize-handle", &mut handles);
    handles.pop().expect("heading resize handle")
}

fn resizable_list_headings(window: &gtk::Window) -> Vec<gtk::Widget> {
    let mut cells = Vec::new();
    descendants_with_class(window.upcast_ref(), "list-heading-cell", &mut cells);
    cells.retain(|cell| {
        let mut handles = Vec::new();
        descendants_with_class(cell, "list-column-resize-handle", &mut handles);
        !handles.is_empty()
    });
    wait_until(|| {
        cells
            .iter()
            .all(|cell| cell.is_mapped() && cell.width() > 0)
    });
    cells
}

fn column_shell(window: &gtk::Window) -> gtk::Box {
    let mut columns = Vec::new();
    descendants_with_class(window.upcast_ref(), "directory-column", &mut columns);
    columns
        .first()
        .and_then(|column| column.parent())
        .and_then(|overlay| overlay.parent())
        .and_downcast::<gtk::Box>()
        .expect("column shell")
}

fn resize_drag(widget: &gtk::Widget) -> gtk::GestureDrag {
    let controllers = widget.observe_controllers();
    let gestures: Vec<gtk::GestureDrag> = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_then(|controller| controller.downcast::<gtk::GestureDrag>().ok())
        })
        .collect();
    gestures
        .iter()
        .find(|gesture| gesture.name().as_deref() == Some("column-resize"))
        .or_else(|| (gestures.len() == 1).then(|| &gestures[0]))
        .cloned()
        .expect("resize drag gesture")
}

fn drag(gesture: &gtk::GestureDrag, start: (f64, f64), offset: f64) {
    gesture.emit_by_name::<()>("drag-begin", &[&start.0, &start.1]);
    gesture.emit_by_name::<()>("drag-update", &[&offset, &0.0f64]);
    gesture.emit_by_name::<()>("drag-end", &[&offset, &0.0f64]);
}

fn resize_column(window: &gtk::Window, offset: f64) {
    let shell = column_shell(window);
    let scroller = shell
        .ancestor(gtk::ScrolledWindow::static_type())
        .expect("columns scroller");
    wait_until(|| {
        shell
            .compute_bounds(&scroller)
            .is_some_and(|bounds| bounds.width() > 0.0)
    });
    let bounds = shell.compute_bounds(&scroller).expect("column bounds");
    let edge = (
        f64::from(bounds.x() + bounds.width()) - 0.5,
        f64::from(bounds.y()) + 4.0,
    );
    drag(&resize_drag(&scroller), edge, offset);
}

fn fixture_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("fixture");
    std::fs::write(root.path().join("report.pdf"), "report").expect("fixture file");
    root
}

fn long_name_fixture_root() -> tempfile::TempDir {
    let root = fixture_root();
    std::fs::write(root.path().join("x".repeat(216)), "long").expect("long-name fixture file");
    root
}

fn saved_list(chooser: bool) -> ListColumns {
    let preferences = PreferenceManager::shared();
    if chooser {
        preferences.chooser_list_columns()
    } else {
        preferences.browser_list_columns()
    }
    .expect("saved list columns")
}

fn saved_column(chooser: bool) -> i32 {
    let preferences = PreferenceManager::shared();
    if chooser {
        preferences.chooser_column_width()
    } else {
        preferences.browser_column_width()
    }
    .expect("saved column width")
}

/// Miller tests use text size 16: it still proves unscaled storage, and the
/// chooser's visible browser area holds a 420 px default plus a drag, which a
/// 402 px sidebar at size 26 leaves no room for in a chooser of at most 1000 px.
fn seed_settings(mode: &str, text_size: u32) {
    let path = crate::ui::preferences::config_directory().join("settings.toml");
    std::fs::create_dir_all(path.parent().expect("settings directory"))
        .expect("create settings directory");
    std::fs::write(path, format!(
        "browser_mode = '{mode}'\ntext_size = {text_size}\nchooser_column_width = 420\nbrowser_column_width = 420\n\
         [chooser_list_columns]\nmode = 100\nsize = 72\ntype = 90\nmodified = 130\n\
         [browser_list_columns]\nmode = 100\nsize = 72\ntype = 90\nmodified = 130\n"
    )).expect("seed saved widths before startup");
}

fn persisted_settings() -> toml::Table {
    crate::storage::writer::settle();
    let path = crate::ui::preferences::config_directory().join("settings.toml");
    toml::from_str(&std::fs::read_to_string(path).expect("persisted settings"))
        .expect("settings table")
}

#[test]
fn list_resizes_persist_across_windows_without_changing_the_other_scope() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::list_resizes_persist_across_windows_without_changing_the_other_scope",
        || {
            seed_settings("list", 26);
            crate::ui::prepare_portal_ui();
            let preferences = PreferenceManager::shared();
            let root = fixture_root();
            for chooser in [true, false] {
                let other = saved_list(!chooser);
                let first = open_browser(root.path(), chooser);
                let cells = resizable_list_headings(&first);
                drag(&resize_drag(&resize_handle(&cells[1])), (0.0, 0.0), 80.0);
                let after_mode = saved_list(chooser);
                assert_eq!(after_mode.name, None);
                assert_eq!(
                    after_mode.mode,
                    100 + (80.0 / preferences.interface_scale()).round() as i32
                );
                assert_eq!(
                    (after_mode.size, after_mode.kind, after_mode.modified),
                    (72, 90, 130)
                );
                drag(&resize_drag(&resize_handle(&cells[0])), (0.0, 0.0), 120.0);
                let saved = saved_list(chooser);
                assert!(saved.name.is_some_and(|width| width > 0));
                assert_eq!(saved_list(!chooser), other);
                let key = if chooser {
                    "chooser_list_columns"
                } else {
                    "browser_list_columns"
                };
                assert_eq!(
                    persisted_settings()[key]["mode"].as_integer(),
                    Some(i64::from(saved.mode))
                );
                first.close();

                preferences.set_text_size(crate::ui::preferences::TextSize::new(13));
                let second = open_browser(root.path(), chooser);
                preferences.set_browser_mode(BrowserMode::Icons);
                preferences.set_browser_mode(BrowserMode::List);
                let cells = resizable_list_headings(&second);
                drag(&resize_drag(&resize_handle(&cells[1])), (0.0, 0.0), 40.0);
                let reopened = saved_list(chooser);
                assert_eq!(
                    reopened.mode,
                    saved.mode + (40.0 / preferences.interface_scale()).round() as i32
                );
                assert_eq!(reopened.name, saved.name);
                assert_eq!(saved_list(!chooser), other);
                second.close();
                preferences.set_text_size(crate::ui::preferences::TextSize::new(26));
            }
        },
    );
}

#[test]
fn miller_resizes_persist_across_windows_without_changing_the_other_scope() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::miller_resizes_persist_across_windows_without_changing_the_other_scope",
        || {
            seed_settings("columns", 16);
            crate::ui::prepare_portal_ui();
            let preferences = PreferenceManager::shared();
            let root = fixture_root();
            for chooser in [true, false] {
                let other = saved_column(!chooser);
                let (first, _preview) = open_bounded_browser(root.path(), chooser);
                resize_column(&first, 200.0);
                let saved = saved_column(chooser);
                assert!(saved > 420, "resize saves an unscaled width");
                assert_eq!(saved_column(!chooser), other);
                let key = if chooser {
                    "chooser_column_width"
                } else {
                    "browser_column_width"
                };
                assert_eq!(
                    persisted_settings()[key].as_integer(),
                    Some(i64::from(saved))
                );
                first.close();

                preferences.set_text_size(crate::ui::preferences::TextSize::new(13));
                let (second, _preview) = open_bounded_browser(root.path(), chooser);
                resize_column(&second, 100.0);
                assert!(
                    saved_column(chooser) > saved,
                    "the next resize starts from the restored default"
                );
                assert_eq!(saved_column(!chooser), other);
                second.close();
                preferences.set_text_size(crate::ui::preferences::TextSize::new(16));
            }
        },
    );
}

#[test]
fn miller_autofit_saves_a_width_within_the_visible_browser_area() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::miller_autofit_saves_a_width_within_the_visible_browser_area",
        || {
            seed_settings("columns", 16);
            crate::ui::prepare_portal_ui();
            let root = long_name_fixture_root();
            for chooser in [true, false] {
                let other = saved_column(!chooser);
                let (window, _preview) = open_bounded_browser(root.path(), chooser);
                autofit_column(&window);
                let saved = saved_column(chooser);
                let area = visible_browser_area_unscaled(&window);
                assert!(saved > 420, "autofit widened the column to {saved}");
                assert!(
                    saved <= area,
                    "autofit stops at the visible browser area: saved {saved}, area {area} (chooser: {chooser})"
                );
                assert_eq!(saved_column(!chooser), other);
                window.close();
            }
        },
    );
}

#[test]
fn miller_drag_cannot_exceed_the_visible_browser_area() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::miller_drag_cannot_exceed_the_visible_browser_area",
        || {
            seed_settings("columns", 16);
            crate::ui::prepare_portal_ui();
            let root = fixture_root();
            for chooser in [true, false] {
                let other = saved_column(!chooser);
                let (window, _preview) = open_bounded_browser(root.path(), chooser);
                resize_column(&window, 5000.0);
                let saved = saved_column(chooser);
                let area = visible_browser_area_unscaled(&window);
                assert!(saved > 420, "the drag widened the column to {saved}");
                assert!(
                    saved <= area,
                    "a drag stops at the visible browser area: saved {saved}, area {area} (chooser: {chooser})"
                );

                let preferences = PreferenceManager::shared();
                preferences.set_text_size(crate::ui::preferences::TextSize::new(26));
                settle();
                preferences.set_text_size(crate::ui::preferences::TextSize::new(16));
                settle();
                resize_column(&window, -1.0);
                let restored = saved_column(chooser);
                assert!(
                    restored >= saved - 2,
                    "a text-size round trip keeps the width: saved {saved}, then {restored} (chooser: {chooser})"
                );

                preferences.set_text_size(crate::ui::preferences::TextSize::new(20));
                settle();
                resize_column(&window, -1.0);
                let larger_text = saved_column(chooser);
                let area = visible_browser_area_unscaled(&window);
                assert!(
                    larger_text <= area,
                    "a larger text size narrows the column to the area: saved {larger_text}, area {area} (chooser: {chooser})"
                );
                preferences.set_text_size(crate::ui::preferences::TextSize::new(16));
                assert_eq!(saved_column(!chooser), other);
                window.close();
            }
        },
    );
}

#[test]
fn failed_chooser_saves_are_only_logged() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::failed_chooser_saves_are_only_logged",
        || {
            seed_settings("list", 26);
            crate::ui::prepare_portal_ui();
            let root = fixture_root();
            let window = open_browser(root.path(), true);
            wait_until(|| window.is_active());
            let path = crate::ui::preferences::config_directory().join("settings.toml");
            std::fs::remove_file(&path).expect("seeded settings");
            std::fs::create_dir(&path).expect("block the settings file with a directory");
            let before = saved_list(true);

            let cells = resizable_list_headings(&window);
            drag(&resize_drag(&resize_handle(&cells[1])), (0.0, 0.0), 80.0);
            // The notice would open on an idle callback.
            while gtk::glib::MainContext::default().iteration(false) {}

            assert_ne!(
                saved_list(true).mode,
                before.mode,
                "the resize still applies"
            );
            assert!(path.is_dir(), "the save failed");
            assert!(
                crate::ui::window::visible_modal_layer(&window).is_none(),
                "the chooser shows no save notice"
            );
        },
    );
}

#[test]
fn list_column_customizations_update_live_across_windows() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::list_column_customizations_update_live_across_windows",
        || {
            seed_settings("list", 16);
            crate::ui::prepare_portal_ui();
            let root = fixture_root();
            let first = open_browser(root.path(), false);
            let second = open_browser(root.path(), false);

            let first_headings = resizable_list_headings(&first);
            let second_headings = resizable_list_headings(&second);
            assert_eq!(first_headings.len(), 5);
            assert_eq!(second_headings.len(), 5);
            assert!(first_headings[1].is_visible());
            assert!(second_headings[1].is_visible());

            let preferences = PreferenceManager::shared();
            preferences.set_list_column_visible(ListColumnId::Mode, false);
            settle();

            assert!(!first_headings[1].is_visible());
            assert!(!second_headings[1].is_visible());

            preferences.set_list_column_visible(ListColumnId::Mode, true);
            settle();

            assert!(first_headings[1].is_visible());
            assert!(second_headings[1].is_visible());

            let headings_box = first_headings[0]
                .parent()
                .and_downcast::<gtk::Box>()
                .expect("headings box");
            let controllers = headings_box.observe_controllers();
            let drag_gesture = (0..controllers.n_items())
                .find_map(|index| {
                    controllers
                        .item(index)
                        .and_then(|c| c.downcast::<gtk::GestureDrag>().ok())
                        .filter(|g| g.name().as_deref() == Some("column-drag"))
                })
                .expect("column drag gesture");
            let mode_bounds = first_headings[1]
                .compute_bounds(&headings_box)
                .expect("mode heading bounds");
            let start_x = f64::from(mode_bounds.x() + 10.0);
            let original_order = preferences.list_column_order();
            let original_settings = persisted_settings();
            drag_gesture.emit_by_name::<()>("drag-begin", &[&start_x, &0.0f64]);
            drag_gesture.emit_by_name::<()>("drag-update", &[&180.0f64, &0.0f64]);
            settle();
            assert_eq!(
                preferences.list_column_order(),
                original_order,
                "hovering previews the destination without reordering any window"
            );
            assert_eq!(
                persisted_settings(),
                original_settings,
                "hovering must not save"
            );
            drag_gesture.emit_by_name::<()>("drag-end", &[&180.0f64, &0.0f64]);
            settle();

            assert_eq!(
                preferences.list_column_order(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Size,
                    ListColumnId::Mode,
                    ListColumnId::Type,
                    ListColumnId::Modified,
                ]
            );

            let committed_order = preferences.list_column_order();
            let committed_settings = persisted_settings();
            for dismiss in ["cancel", "unmap", "outside"] {
                let bounds = first_headings[1]
                    .compute_bounds(&headings_box)
                    .expect("reordered Mode heading");
                let start_x = f64::from(bounds.x() + 10.0);
                drag_gesture.emit_by_name::<()>("drag-begin", &[&start_x, &0.0f64]);
                drag_gesture.emit_by_name::<()>("drag-update", &[&-start_x, &0.0f64]);
                assert_eq!(
                    preferences.list_column_order(),
                    committed_order,
                    "{dismiss}"
                );
                match dismiss {
                    "cancel" => drag_gesture
                        .emit_by_name::<()>("cancel", &[&None::<gtk::gdk::EventSequence>]),
                    "unmap" => {
                        headings_box.set_visible(false);
                        settle();
                        headings_box.set_visible(true);
                        settle();
                    }
                    "outside" => drag_gesture.emit_by_name::<()>(
                        "drag-end",
                        &[&-start_x, &(f64::from(headings_box.height()) + 20.0)],
                    ),
                    _ => unreachable!(),
                }
                drag_gesture.emit_by_name::<()>("drag-end", &[&-start_x, &0.0f64]);
                assert_eq!(
                    preferences.list_column_order(),
                    committed_order,
                    "{dismiss}"
                );
                assert_eq!(persisted_settings(), committed_settings, "{dismiss}");
            }

            first.close();
            second.close();
        },
    );
}
