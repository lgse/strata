// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::theme::ThemeManager;
use std::time::{Duration, Instant};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "filter did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn visible_matches(widget: &gtk::Widget) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.is_mapped()
        && ["needle.txt", "needle-folder", "needle-nested.txt"].contains(&label.text().as_str())
    {
        names.push(label.text().to_string());
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
}

#[test]
fn saved_filter_scope_updates_two_windows_and_rebuilt_views_without_settings() {
    crate::test_support::gtk_test(
        "ui::browser::tests::filter_scope::saved_filter_scope_updates_two_windows_and_rebuilt_views_without_settings",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
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
