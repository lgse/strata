// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{browser::PeekBehavior, browser_modes::BrowserMode, preferences::ListColumns};
use acceptance::{request, wait_until};
use std::{
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
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

fn seed_settings(mode: &str) {
    let path = crate::ui::preferences::config_directory().join("settings.toml");
    std::fs::create_dir_all(path.parent().expect("settings directory"))
        .expect("create settings directory");
    std::fs::write(path, format!(
        "browser_mode = '{mode}'\ntext_size = 26\nchooser_column_width = 420\nbrowser_column_width = 420\n\
         [chooser_list_columns]\nmode = 100\nsize = 72\ntype = 90\nmodified = 130\n\
         [browser_list_columns]\nmode = 100\nsize = 72\ntype = 90\nmodified = 130\n"
    )).expect("seed saved widths before startup");
}

fn persisted_settings() -> toml::Table {
    let path = crate::ui::preferences::config_directory().join("settings.toml");
    toml::from_str(&std::fs::read_to_string(path).expect("persisted settings"))
        .expect("settings table")
}

#[test]
fn list_resizes_persist_across_windows_without_changing_the_other_scope() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::column_widths::list_resizes_persist_across_windows_without_changing_the_other_scope",
        || {
            seed_settings("list");
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
            seed_settings("columns");
            crate::ui::prepare_portal_ui();
            let preferences = PreferenceManager::shared();
            let root = fixture_root();
            for chooser in [true, false] {
                let other = saved_column(!chooser);
                let first = open_browser(root.path(), chooser);
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
                let second = open_browser(root.path(), chooser);
                resize_column(&second, 100.0);
                assert!(
                    saved_column(chooser) > saved,
                    "the next resize starts from the restored default"
                );
                assert_eq!(saved_column(!chooser), other);
                second.close();
                preferences.set_text_size(crate::ui::preferences::TextSize::new(26));
            }
        },
    );
}
