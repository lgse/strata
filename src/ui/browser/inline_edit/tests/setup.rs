// SPDX-License-Identifier: MIT

use super::*;

fn columns_fixture() -> tempfile::TempDir {
    let fixture = tempfile::tempdir().expect("directory fixture");
    let file = fixture.path().join("quarterly-report.txt");
    std::fs::write(&file, b"body").expect("fixture file");
    std::fs::create_dir(fixture.path().join("folder")).expect("fixture folder");
    fixture
}

fn open_columns_rename(view: &BrowserView, position: usize) -> gtk::Entry {
    let browser = view.browser();
    browser.select(0, position);
    wait_until(|| view.state.begin_rename());
    let field = view
        .state
        .active_rename
        .borrow()
        .as_ref()
        .expect("active Columns rename")
        .field
        .clone();
    assert!(gtk::prelude::WidgetExt::is_visible(&field));
    field
}

#[test]
fn columns_setup_selects_file_stem_and_folder_name() {
    gtk_test(
        "ui::browser::inline_edit::tests::setup::columns_setup_selects_file_stem_and_folder_name",
        || {
            let fixture = columns_fixture();
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(600)
                .default_height(300)
                .build();
            window.present();
            let browser = view.browser();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 2)
            });

            let file = open_columns_rename(&view, 1);
            assert_eq!(file.text(), "quarterly-report.txt");
            assert_eq!(file.selection_bounds(), Some((0, 16)));
            assert!(view.state.cancel_rename());

            let folder = open_columns_rename(&view, 0);
            assert_eq!(folder.text(), "folder");
            assert_eq!(folder.selection_bounds(), Some((0, 6)));
            assert!(view.state.cancel_rename());
            assert!(!view.rename_is_active());
            browser.clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn columns_setup_resets_validation_and_cancel_restores_widgets() {
    gtk_test(
        "ui::browser::inline_edit::tests::setup::columns_setup_resets_validation_and_cancel_restores_widgets",
        || {
            let fixture = columns_fixture();
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(600)
                .default_height(300)
                .build();
            window.present();
            let browser = view.browser();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 2)
            });

            let field = open_columns_rename(&view, 0);
            let (label, spacer, size) = {
                let active = view.state.active_rename.borrow();
                let active = active.as_ref().expect("active Columns rename");
                (
                    active.label.clone(),
                    active.spacer.clone(),
                    active.size.clone(),
                )
            };
            size.set_label("1 KB");
            field.set_text("bad/name");
            assert!(field.has_css_class("error"));
            assert!(field.tooltip_text().is_some());
            assert!(!label.is_visible());
            assert!(!spacer.is_visible());
            assert!(!size.is_visible());
            assert!(view.state.cancel_rename());
            assert_eq!(field.margin_start(), 0);
            assert_eq!(field.margin_end(), 0);
            assert!(!gtk::prelude::WidgetExt::is_visible(&field));
            assert!(label.is_visible());
            assert!(spacer.is_visible());
            assert!(size.is_visible());

            let reopened = open_columns_rename(&view, 0);
            assert_eq!(reopened.as_ptr(), field.as_ptr());
            assert_eq!(reopened.text(), "folder");
            assert!(!reopened.has_css_class("error"));
            assert!(reopened.tooltip_text().is_none());
            assert!(view.state.cancel_rename());
            browser.clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn columns_setup_rejects_missing_depth_or_filtered_mapping() {
    gtk_test(
        "ui::browser::inline_edit::tests::setup::columns_setup_rejects_missing_depth_or_filtered_mapping",
        || {
            let fixture = columns_fixture();
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(600)
                .default_height(300)
                .build();
            window.present();
            let browser = view.browser();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 2)
            });
            let entry = fixture_entry("orphan.txt");
            assert!(!view.state.begin_rename_item(99, 0, entry.clone()));
            assert!(!view.rename_is_active());
            assert!(!view.state.begin_rename_item(0, 99, entry));
            assert!(!view.rename_is_active());
            browser.clear_observer();
            window.destroy();
        },
    );
}
