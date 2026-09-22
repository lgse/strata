// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::preferences::PreferenceManager;
use std::time::{Duration, Instant};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "filter did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

const MATCH_NAMES: &[&str] = &[
    "needle.txt",
    "needle-folder",
    "needle-nested.txt",
    ".needle-secret.txt",
    "needle-hidden.txt",
];

fn visible_matches(widget: &gtk::Widget) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.is_mapped()
        && MATCH_NAMES.contains(&label.text().as_str())
    {
        names.push(label.text().to_string());
    } else if let Some(label) = widget.downcast_ref::<gtk::Inscription>()
        && label.is_mapped()
        && label
            .text()
            .as_deref()
            .is_some_and(|text| MATCH_NAMES.contains(&text))
        && let Some(text) = label.text()
    {
        names.push(text.to_string());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        names.extend(visible_matches(&widget));
        child = widget.next_sibling();
    }
    names.sort();
    names.dedup();
    names
}

fn visible_path_count(widget: &gtk::Widget) -> usize {
    let mut count = usize::from(widget.has_css_class("file-search-path") && widget.is_mapped());
    let mut child = widget.first_child();
    while let Some(widget) = child {
        count += visible_path_count(&widget);
        child = widget.next_sibling();
    }
    count
}

fn assert_matches(views: &[BrowserView], recursive: bool) {
    let expected = if recursive {
        vec!["needle-folder", "needle-nested.txt", "needle.txt"]
    } else {
        vec!["needle-folder", "needle.txt"]
    };
    wait_until(|| {
        views
            .iter()
            .all(|view| visible_matches(&view.widget()) == expected)
    });
    for view in views {
        assert_eq!(
            visible_path_count(&view.widget()),
            if recursive { expected.len() } else { 0 },
            "path subtitles must only appear in recursive results ({:?})",
            view.view_mode(),
        );
    }
}

#[test]
fn saved_filter_scope_updates_two_windows_and_rebuilt_views_without_settings() {
    crate::test_support::gtk_test(
        "ui::browser::tests::filter_scope::saved_filter_scope_updates_two_windows_and_rebuilt_views_without_settings",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            assert!(!manager.filter_include_subfolders());
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(fixture.path().join("needle-folder")).expect("folder");
            std::fs::write(fixture.path().join("needle.txt"), "fixture").expect("file");
            std::fs::write(
                fixture.path().join("needle-folder/needle-nested.txt"),
                "fixture",
            )
            .expect("nested file");
            let views: Vec<_> = (0..2)
                .map(|_| {
                    BrowserView::new(
                        Rc::new(crate::adapters::LocalFileSource),
                        PeekBehavior::default(),
                    )
                })
                .collect();
            let windows: Vec<_> = views
                .iter()
                .map(|view| {
                    let window = gtk::Window::builder()
                        .child(&view.widget())
                        .default_width(900)
                        .default_height(500)
                        .build();
                    window.present();
                    view.browser().navigate(Location::local(fixture.path()));
                    window
                })
                .collect();
            wait_until(|| {
                views.iter().all(|view| {
                    view.browser()
                        .column_snapshot(0)
                        .is_some_and(|s| !s.loading)
                })
            });
            // Revisiting modes exercises cached panes as well as lazy construction.
            for mode in [
                BrowserMode::List,
                BrowserMode::Icons,
                BrowserMode::Columns,
                BrowserMode::List,
            ] {
                manager.set_browser_mode(mode);
                for view in &views {
                    assert!(view.show_filter_with_query("needle"));
                }
                assert_matches(&views, false);
                for recursive in [true, false] {
                    manager.set_filter_include_subfolders(recursive);
                    assert_matches(&views, recursive);
                }
                for view in &views {
                    view.browser().reload_active();
                }
                wait_until(|| {
                    views.iter().all(|view| {
                        view.browser()
                            .column_snapshot(0)
                            .is_some_and(|s| !s.loading)
                    })
                });
                for view in &views {
                    assert!(view.show_filter_with_query("needle"));
                }
                assert_matches(&views, false);
                manager.set_filter_include_subfolders(true);
                manager.set_filter_include_subfolders(false);
                assert_matches(&views, false);
                for view in &views {
                    assert!(view.show_filter_with_query("needle-nested"));
                }
                manager.set_filter_include_subfolders(true);
                manager.set_filter_include_subfolders(false);
                let drained = Rc::new(Cell::new(false));
                let done = drained.clone();
                glib::timeout_add_local_once(Duration::from_millis(250), move || done.set(true));
                wait_until(|| drained.get());
                for view in &views {
                    assert!(
                        visible_matches(&view.widget()).is_empty(),
                        "stale recursive results reappeared"
                    );
                    assert!(view.show_filter_with_query(""));
                }
            }
            let saved =
                std::fs::read_to_string(glib::user_config_dir().join("strata/settings.toml"))
                    .expect("saved preference");
            assert!(saved.contains("filter_include_subfolders = false"));
            for window in windows {
                window.close();
            }
        },
    );
}

#[test]
fn hidden_search_input_updates_two_windows_and_rebuilt_modes_without_settings() {
    crate::test_support::gtk_test(
        "ui::browser::tests::filter_scope::hidden_search_input_updates_two_windows_and_rebuilt_modes_without_settings",
        || {
            let manager = PreferenceManager::shared();
            let mut preferences = manager.sort_preferences();
            preferences.show_hidden = false;
            manager.set_sort_preferences(preferences);
            manager.set_filter_include_subfolders(true);
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(fixture.path().join(".hidden")).expect("hidden directory");
            for name in [
                "needle.txt",
                ".needle-secret.txt",
                ".hidden/needle-hidden.txt",
            ] {
                std::fs::write(fixture.path().join(name), "fixture").expect("match");
            }
            let views: Vec<_> = (0..2)
                .map(|_| {
                    BrowserView::new(
                        Rc::new(crate::adapters::LocalFileSource),
                        PeekBehavior::default(),
                    )
                })
                .collect();
            let windows: Vec<_> = views
                .iter()
                .map(|view| {
                    let window = gtk::Window::builder()
                        .child(&view.widget())
                        .default_width(900)
                        .default_height(500)
                        .build();
                    window.present();
                    view.browser().navigate(Location::local(fixture.path()));
                    window
                })
                .collect();
            wait_until(|| {
                views.iter().all(|view| {
                    view.browser()
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading)
                })
            });
            for mode in [
                BrowserMode::Columns,
                BrowserMode::List,
                BrowserMode::Icons,
                BrowserMode::Columns,
            ] {
                manager.set_browser_mode(mode);
                for view in &views {
                    assert!(view.show_filter_with_query("needle"));
                }
                wait_until(|| {
                    views
                        .iter()
                        .all(|view| visible_matches(&view.widget()) == vec!["needle.txt"])
                });
                for show_hidden in [true, false] {
                    let mut preferences = manager.sort_preferences();
                    preferences.show_hidden = show_hidden;
                    manager.set_sort_preferences(preferences);
                    let expected = if show_hidden {
                        vec![".needle-secret.txt", "needle-hidden.txt", "needle.txt"]
                    } else {
                        vec!["needle.txt"]
                    };
                    wait_until(|| {
                        views
                            .iter()
                            .all(|view| visible_matches(&view.widget()) == expected)
                    });
                }
            }
            for window in windows {
                window.close();
            }
        },
    );
}
