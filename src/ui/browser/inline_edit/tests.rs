// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::{
    test_support::gtk_test,
    ui::{
        browser::{BrowserView, PeekBehavior},
        browser_modes::BrowserMode,
    },
};
use std::time::{Duration, Instant};

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

#[test]
fn submitting_an_invalid_rename_flags_the_field_in_every_view_mode() {
    gtk_test(
        "ui::browser::inline_edit::tests::submitting_an_invalid_rename_flags_the_field_in_every_view_mode",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            let file = fixture.path().join("notes.txt");
            std::fs::write(&file, b"body").expect("fixture file");

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
                    .expect("an inline rename field is open");

                for (name, message) in [
                    ("", "Enter a name"),
                    ("bad/name", "Names cannot contain /"),
                    (".", "That name is reserved"),
                ] {
                    field.set_text(name);
                    field.emit_by_name::<()>("activate", &[]);
                    assert!(
                        field.has_css_class("error"),
                        "{mode:?} did not flag {name:?}"
                    );
                    assert_eq!(
                        field.tooltip_text().as_deref(),
                        Some(message),
                        "{mode:?} explains why {name:?} was rejected"
                    );
                    assert!(field.is_sensitive(), "{mode:?} left the field disabled");
                }

                assert!(file.is_file(), "{mode:?} left the entry untouched");
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}
