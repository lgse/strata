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
fn submitting_an_invalid_rename_flags_the_field_in_every_view_mode() {
    gtk_test(
        "ui::browser::inline_edit::tests::submitting_an_invalid_rename_flags_the_field_in_every_view_mode",
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
