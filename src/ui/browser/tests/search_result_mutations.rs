// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Duration, Instant};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "search result did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn shows_match(widget: &gtk::Widget, name: &str) -> bool {
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.is_mapped()
        && label.text() == name
    {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if shows_match(&widget, name) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn with_filtered_result(
    mode: BrowserMode,
    run: impl FnOnce(&BrowserView, &std::path::Path, FileEntry),
) {
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("needle.txt"), b"body").expect("fixture file");
    // Keep empty-directory handling from masking stale search results.
    std::fs::write(fixture.path().join("other.txt"), b"body").expect("fixture file");
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view.set_view_mode(mode);
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(900)
        .default_height(500)
        .build();
    window.present();
    view.browser().navigate(Location::local(fixture.path()));
    wait_until(|| {
        view.browser()
            .column_snapshot(0)
            .is_some_and(|snapshot| !snapshot.loading)
    });
    let entry = (0..2)
        .filter_map(|position| view.browser().entry_at(0, position))
        .find(|entry| entry.display_name == "needle.txt")
        .expect("needle.txt entry");
    assert!(view.show_filter_with_query("needle"));
    wait_until(|| shows_match(&view.widget(), "needle.txt"));
    run(&view, fixture.path(), entry);
    assert!(view.show_filter_with_query("nee"));
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        assert!(!shows_match(&view.widget(), "needle.txt"));
        std::thread::sleep(Duration::from_millis(2));
    }
    view.browser().clear_observer();
    window.close();
}

#[test]
fn renaming_a_filtered_result_clears_the_stale_hit_in_every_view_mode() {
    crate::test_support::gtk_test(
        "ui::browser::tests::search_result_mutations::renaming_a_filtered_result_clears_the_stale_hit_in_every_view_mode",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                with_filtered_result(mode, |view, path, entry| {
                    view.browser().rename(entry, "renamed.txt".to_owned());
                    wait_until(|| path.join("renamed.txt").exists());
                    wait_until(|| !shows_match(&view.widget(), "needle.txt"));
                });
            }
        },
    );
}

#[test]
fn matching_rename_refreshes_the_result_and_subsequent_queries_in_every_view_mode() {
    crate::test_support::gtk_test(
        "ui::browser::tests::search_result_mutations::matching_rename_refreshes_the_result_and_subsequent_queries_in_every_view_mode",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                with_filtered_result(mode, |view, path, entry| {
                    view.browser()
                        .rename(entry, "needle-renamed.txt".to_owned());
                    wait_until(|| shows_match(&view.widget(), "needle-renamed.txt"));
                    assert!(!shows_match(&view.widget(), "needle.txt"));
                    assert!(!path.join("needle.txt").exists());
                    assert_eq!(
                        std::fs::read(path.join("needle-renamed.txt")).expect("renamed file"),
                        b"body"
                    );
                    assert!(view.show_filter_with_query("nothing-matches"));
                    wait_until(|| !shows_match(&view.widget(), "needle-renamed.txt"));
                    assert!(view.show_filter_with_query("renamed"));
                    wait_until(|| shows_match(&view.widget(), "needle-renamed.txt"));
                });
            }
        },
    );
}

#[test]
fn undo_after_navigation_refreshes_an_existing_search_session() {
    crate::test_support::gtk_test(
        "ui::browser::tests::search_result_mutations::undo_after_navigation_refreshes_an_existing_search_session",
        || {
            with_filtered_result(BrowserMode::List, |view, path, entry| {
                let (search, events) =
                    crate::services::index_filter(path.to_path_buf(), false, true);
                search.query("needle");
                let await_result = |name: &str| {
                    wait_until(|| {
                        events.try_iter().any(
                            |crate::services::SearchEvent::Results {
                                 items, indexing, ..
                             }| {
                                !indexing && items.len() == 1 && items[0].path == path.join(name)
                            },
                        )
                    });
                };
                await_result("needle.txt");
                view.browser()
                    .rename(entry, "needle-renamed.txt".to_owned());
                await_result("needle-renamed.txt");
                let elsewhere = tempfile::tempdir().expect("another directory");
                view.browser().navigate(Location::local(elsewhere.path()));
                wait_until(|| {
                    view.browser().column_snapshot(0).is_some_and(|snapshot| {
                        !snapshot.loading && snapshot.location == Location::local(elsewhere.path())
                    })
                });
                let (generation, _, _) = view.browser().pending_undo_rename().expect("rename undo");
                assert!(view.browser().undo_rename(generation));
                await_result("needle.txt");
                assert!(!path.join("needle-renamed.txt").exists());
                assert_eq!(
                    std::fs::read(path.join("needle.txt")).expect("restored file"),
                    b"body"
                );
            });
        },
    );
}

#[test]
fn deleting_a_filtered_result_clears_the_stale_hit_in_every_view_mode() {
    crate::test_support::gtk_test(
        "ui::browser::tests::search_result_mutations::deleting_a_filtered_result_clears_the_stale_hit_in_every_view_mode",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                with_filtered_result(mode, |view, path, entry| {
                    view.browser().delete(vec![entry], true);
                    wait_until(|| !path.join("needle.txt").exists());
                    wait_until(|| !shows_match(&view.widget(), "needle.txt"));
                });
            }
        },
    );
}
