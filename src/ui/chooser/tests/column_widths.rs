// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{
    browser::PeekBehavior, browser_modes::BrowserMode, preferences::ChooserListColumns,
};
use acceptance::{request, wait_until};
use std::{
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

fn open_chooser(root: &Path) -> Rc<ChooserState> {
    let state = build_chooser(
        request(root.to_path_buf()),
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .expect("chooser");
    let browser = state.view.browser();
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    state
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

/// The loading skeleton renders headings too. Only real ones carry resize handles.
fn list_heading_cells(window: &gtk::Window) -> Vec<gtk::Widget> {
    let mut cells = Vec::new();
    descendants_with_class(window.upcast_ref(), "list-heading-cell", &mut cells);
    cells.retain(|cell| {
        let mut handles = Vec::new();
        descendants_with_class(cell, "list-column-resize-handle", &mut handles);
        !handles.is_empty()
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

/// The columns scroller also carries GTK's own drag gestures and the marquee.
fn drag_gesture(widget: &gtk::Widget) -> gtk::GestureDrag {
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
        .expect("one resize drag gesture")
}

fn drag(gesture: &gtk::GestureDrag, start: (f64, f64), offset: f64) {
    gesture.emit_by_name::<()>("drag-begin", &[&start.0, &start.1]);
    gesture.emit_by_name::<()>("drag-update", &[&offset, &0.0f64]);
    gesture.emit_by_name::<()>("drag-end", &[&offset, &0.0f64]);
}

fn fixture_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("fixture");
    std::fs::write(root.path().join("report.pdf"), "report").expect("fixture file");
    root
}

#[test]
fn chooser_list_columns_reopen_at_the_remembered_widths() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::chooser_list_columns_reopen_at_the_remembered_widths",
        || {
            crate::ui::prepare_portal_ui();
            let preferences = PreferenceManager::shared();
            preferences.set_browser_mode(BrowserMode::List);
            preferences.set_chooser_list_columns(Some(ChooserListColumns {
                name: None,
                mode: 100,
                size: 72,
                kind: 90,
                modified: 130,
            }));
            let scale = preferences.interface_scale();
            let scaled = |width: i32| (f64::from(width) * scale).round() as i32;
            let unscaled = |width: i32| (f64::from(width) / scale).round() as i32;
            let root = fixture_root();

            let first = open_chooser(root.path());
            let cells = list_heading_cells(&first.window);
            assert_eq!(cells.len(), 5);
            assert!(cells[0].hexpands(), "Name expands until it is resized");
            assert_eq!(cells[1].width_request(), scaled(100));
            assert_eq!(cells[2].width_request(), scaled(72));
            assert_eq!(cells[4].width_request(), scaled(130));

            let mode_before = cells[1].width_request();
            drag(&drag_gesture(&resize_handle(&cells[1])), (0.0, 0.0), 40.0);
            let mode_after = cells[1].width_request();
            assert!(mode_after > mode_before, "Mode grows with the drag");
            drag(&drag_gesture(&resize_handle(&cells[0])), (0.0, 0.0), 120.0);
            assert!(
                !cells[0].hexpands(),
                "a resized Name column stops expanding"
            );
            let name_after = cells[0].width_request();

            let saved = preferences
                .chooser_list_columns()
                .expect("resizing saves the list columns");
            assert_eq!(saved.name, Some(unscaled(name_after)));
            assert_eq!(saved.mode, unscaled(mode_after));
            assert_eq!(
                (saved.size, saved.kind, saved.modified),
                (72, 90, 130),
                "untouched columns keep their saved widths"
            );
            first.window.close();

            let second = open_chooser(root.path());
            let reopened = list_heading_cells(&second.window);
            assert_eq!(reopened.len(), 5);
            assert!(!reopened[0].hexpands());
            assert_eq!(
                reopened[0].width_request(),
                scaled(saved.name.expect("name"))
            );
            assert_eq!(reopened[1].width_request(), scaled(saved.mode));
            assert_eq!(reopened[2].width_request(), scaled(72));
            second.window.close();

            let interactive = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            interactive.set_view_mode(BrowserMode::List);
            let window = gtk::Window::builder().child(&interactive.widget()).build();
            window.present();
            interactive.navigate_location(Location::local(root.path()));
            let browser = interactive.browser();
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            let cells = list_heading_cells(&window);
            assert_eq!(cells.len(), 5);
            assert_eq!(
                cells[1].width_request(),
                scaled(160),
                "interactive panes start from the built-in widths"
            );
            drag(&drag_gesture(&resize_handle(&cells[1])), (0.0, 0.0), 40.0);
            assert_eq!(
                preferences.chooser_list_columns(),
                Some(saved),
                "interactive resizes do not change the chooser widths"
            );
            window.close();
        },
    );
}

#[test]
fn chooser_columns_reopen_at_the_remembered_width() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::chooser_columns_reopen_at_the_remembered_width",
        || {
            crate::ui::prepare_portal_ui();
            let preferences = PreferenceManager::shared();
            preferences.set_browser_mode(BrowserMode::Columns);
            preferences.set_chooser_column_width(Some(420));
            let scale = preferences.interface_scale();
            let scaled = |width: i32| (f64::from(width) * scale).round() as i32;
            let unscaled = |width: i32| (f64::from(width) / scale).round() as i32;
            let root = fixture_root();

            let first = open_chooser(root.path());
            let shell = column_shell(&first.window);
            assert_eq!(shell.width_request(), scaled(420));
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
            let start = shell.width().max(crate::ui::browser::COLUMN_WIDTH);
            drag(&drag_gesture(&scroller), edge, 100.0);
            assert_eq!(shell.width_request(), start + 100);
            let saved = preferences
                .chooser_column_width()
                .expect("resizing saves the column width");
            assert_eq!(saved, unscaled(start + 100));
            first.window.close();

            let second = open_chooser(root.path());
            assert_eq!(column_shell(&second.window).width_request(), scaled(saved));
            second.window.close();
        },
    );
}
