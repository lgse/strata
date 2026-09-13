// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Duration, Instant};

const FIXTURE_NAMES: [&str; 2] = ["needle.txt", "other.txt"];

fn wait_until(condition: impl Fn() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn mapped_labels(widget: &gtk::Widget) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.is_mapped()
        && !label.text().is_empty()
    {
        names.push(label.text().to_string());
    }
    if let Some(inscription) = widget.downcast_ref::<gtk::Inscription>()
        && inscription.is_mapped()
        && let Some(text) = inscription.text()
        && !text.is_empty()
    {
        names.push(text.to_string());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        names.extend(mapped_labels(&widget));
        child = widget.next_sibling();
    }
    names
}

fn shows_name(widget: &gtk::Widget, name: &str) -> bool {
    mapped_labels(widget).iter().any(|label| label == name)
}

fn pane_filter(view: &BrowserView) -> (String, bool) {
    match view.view_mode() {
        BrowserMode::Columns => {
            let depth = view.browser().active_depth().expect("active column");
            let columns = view.state.columns.borrow();
            let column = &columns[depth];
            (
                column.filter_entry.text().to_string(),
                column.filter_button.is_active(),
            )
        }
        BrowserMode::Icons | BrowserMode::List => {
            let filter = view.state.mode_views.borrow().capture_active_filter();
            (filter.query, filter.revealed)
        }
    }
}

fn present_filtered_view(
    mode: BrowserMode,
) -> (
    BrowserView,
    gtk::Window,
    tempfile::TempDir,
    Rc<crate::app::Browser>,
) {
    crate::ui::theme::ThemeManager::seed_saved_preferences_for_test();
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("needle.txt"), b"needle").expect("needle");
    std::fs::write(fixture.path().join("other.txt"), b"other").expect("other");
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    view.set_view_mode(mode);
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(900)
        .default_height(500)
        .build();
    window.present();
    let browser = view.browser();
    browser.navigate(Location::local(fixture.path()));
    wait_until(
        || {
            browser
                .column_snapshot(0)
                .is_some_and(|snapshot| !snapshot.loading)
        },
        "directory listing should load",
    );
    (view, window, fixture, browser)
}

fn assert_filtered(view: &BrowserView) {
    wait_until(
        || {
            let (query, revealed) = pane_filter(view);
            query == "needle" && revealed
        },
        &format!(
            "{:?} should keep query 'needle' revealed; got {:?}",
            view.view_mode(),
            pane_filter(view)
        ),
    );
    wait_until(
        || shows_name(&view.widget(), "needle.txt") && !shows_name(&view.widget(), "other.txt"),
        &format!(
            "{:?} listing should stay narrowed to needle.txt; mapped labels: {:?}",
            view.view_mode(),
            mapped_labels(&view.widget())
        ),
    );
}

fn assert_listing_focus(view: &BrowserView) {
    wait_until(
        || !view.filter_has_focus(),
        &format!(
            "{:?} listing should take focus after restoring the filter",
            view.view_mode()
        ),
    );
}

fn assert_unfiltered(view: &BrowserView) {
    wait_until(
        || {
            let (query, revealed) = pane_filter(view);
            query.is_empty() && !revealed
        },
        &format!(
            "{:?} should keep a dismissed filter closed; got {:?}",
            view.view_mode(),
            pane_filter(view)
        ),
    );
    wait_until(
        || {
            FIXTURE_NAMES
                .iter()
                .all(|name| shows_name(&view.widget(), name))
        },
        &format!(
            "{:?} listing should stay unfiltered; mapped labels: {:?}",
            view.view_mode(),
            mapped_labels(&view.widget())
        ),
    );
}

#[test]
fn switching_view_modes_keeps_the_active_pane_filter() {
    crate::test_support::gtk_test(
        "ui::browser::tests::view_mode_filter::switching_view_modes_keeps_the_active_pane_filter",
        || {
            let (view, window, fixture, browser) = present_filtered_view(BrowserMode::Columns);
            assert!(view.show_filter_with_query("needle"));
            assert_filtered(&view);

            for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns] {
                view.set_view_mode(mode);
                assert_filtered(&view);
                assert_listing_focus(&view);
            }

            let settings = glib::user_config_dir().join("strata/settings.toml");
            if let Ok(saved) = std::fs::read_to_string(settings) {
                assert!(
                    !saved.contains("needle"),
                    "pane filter must stay window-local"
                );
            }
            browser.clear_observer();
            window.close();
            drop(fixture);
        },
    );
}

#[test]
fn reused_panes_drop_a_dismissed_filter() {
    crate::test_support::gtk_test(
        "ui::browser::tests::view_mode_filter::reused_panes_drop_a_dismissed_filter",
        || {
            let (view, window, fixture, browser) = present_filtered_view(BrowserMode::Icons);
            assert!(view.show_filter_with_query("needle"));
            assert_filtered(&view);

            view.set_view_mode(BrowserMode::List);
            assert_filtered(&view);
            assert!(view.show_filter());
            assert!(view.dismiss_focused_filter());
            assert_unfiltered(&view);

            view.set_view_mode(BrowserMode::Icons);
            assert_unfiltered(&view);

            browser.clear_observer();
            window.close();
            drop(fixture);
        },
    );
}

#[test]
fn view_switch_copies_the_active_column_filter_not_the_hovered_parent() {
    crate::test_support::gtk_test(
        "ui::browser::tests::view_mode_filter::view_switch_copies_the_active_column_filter_not_the_hovered_parent",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::write(fixture.path().join("keep.txt"), b"parent").expect("parent file");
            let child = fixture.path().join("child");
            std::fs::create_dir(&child).expect("child");
            std::fs::write(child.join("needle.txt"), b"needle").expect("needle");
            std::fs::write(child.join("other.txt"), b"other").expect("other");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(900)
                .default_height(500)
                .build();
            window.present();
            let browser = view.browser();
            browser.navigate(Location::local(fixture.path()));
            wait_until(
                || {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading)
                },
                "root listing should load",
            );
            let child_position = (0..browser.column_snapshot(0).expect("root").count)
                .find_map(|position| {
                    browser
                        .entry_at(0, position)
                        .filter(|entry| entry.display_name == "child")
                        .map(|_| position)
                })
                .expect("child folder");
            browser.set_selection(0, &[child_position], Some(child_position));
            browser.enter_focused_directory();
            wait_until(
                || {
                    browser
                        .column_snapshot(1)
                        .is_some_and(|snapshot| !snapshot.loading)
                },
                "child listing should load",
            );
            browser.set_active_column(1);
            browser.focus_active();
            view.keyboard_navigation();
            assert_eq!(browser.active_depth(), Some(1));
            assert!(view.show_filter_with_query("needle"));
            wait_until(
                || {
                    let columns = view.state.columns.borrow();
                    columns.get(1).is_some_and(|column| {
                        column.filter_button.is_active() && column.filter_entry.text() == "needle"
                    }) && columns
                        .first()
                        .is_some_and(|column| !column.filter_button.is_active())
                },
                "the active child column should hold the filter",
            );

            view.state.hovered_column.set(Some(0));
            view.state.pointer_navigation();
            view.set_view_mode(BrowserMode::Icons);
            assert_eq!(browser.active_depth(), Some(1));
            assert_filtered(&view);

            view.set_view_mode(BrowserMode::Columns);
            assert_eq!(browser.active_depth(), Some(1));
            wait_until(
                || view.state.columns.borrow().len() > 1,
                "Columns should rebuild both miller panes",
            );
            let columns = view.state.columns.borrow();
            assert!(columns[1].filter_button.is_active());
            assert_eq!(columns[1].filter_entry.text(), "needle");
            assert!(!columns[0].filter_button.is_active());
            drop(columns);
            assert_filtered(&view);

            browser.clear_observer();
            window.close();
        },
    );
}
