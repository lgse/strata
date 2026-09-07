// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::time::{Duration, Instant};

fn settle() {
    let deadline = Instant::now() + Duration::from_millis(120);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "browser did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn present_single_pane(
    mode: BrowserMode,
) -> (
    BrowserView,
    Rc<crate::app::Browser>,
    gtk::Window,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let home = tempfile::tempdir().expect("home fixture");
    let place = tempfile::tempdir().expect("place fixture");
    for index in 0..6 {
        std::fs::write(place.path().join(format!("file-{index:02}.txt")), "fixture")
            .expect("fixture file");
    }
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    let browser = view.browser();
    view.set_view_mode(mode);
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(1000)
        .default_height(650)
        .build();
    window.present();
    browser.navigate(Location::local(home.path()));
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|snapshot| !snapshot.loading)
    });
    settle();
    (view, browser, window, home, place)
}

fn assert_navigate_lands_on_first_item(view: &BrowserView, browser: &crate::app::Browser) {
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|snapshot| !snapshot.loading)
            && browser.selected_positions(0) == [0]
    });
    settle();
    assert_eq!(
        browser
            .focused_entry()
            .expect("first item after navigate")
            .display_name,
        "file-00.txt"
    );
    assert!(
        view.item_view_has_focus(),
        "navigate must leave keyboard focus in the file view"
    );
    assert!(
        view.at_left_edge(),
        "the first item must be a usable cursor so Right stays in the listing"
    );
}

#[test]
#[ignore = "requires a mapped GTK window; run this test alone"]
fn icons_navigate_focuses_first_item_so_arrows_move_without_left_right() {
    const CHILD: &str = "STRATA_ICONS_NAVIGATE_FOCUS_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated settings");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::navigate::icons_navigate_focuses_first_item_so_arrows_move_without_left_right",
                "--nocapture",
                "--ignored",
            ])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("XDG_DATA_HOME", sandbox.path().join("data"))
            .status()
            .expect("GTK test starts");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    crate::assets::prepare().expect("assets");
    crate::assets::register_icon_theme();
    let (view, browser, window, _home, place) = present_single_pane(BrowserMode::Icons);
    browser.navigate(Location::local(place.path()));
    assert_navigate_lands_on_first_item(&view, &browser);
    window.destroy();
    browser.clear_observer();
}

#[test]
#[ignore = "requires a mapped GTK window; run this test alone"]
fn list_navigate_focuses_first_item_so_arrows_move() {
    const CHILD: &str = "STRATA_LIST_NAVIGATE_FOCUS_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated settings");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::navigate::list_navigate_focuses_first_item_so_arrows_move",
                "--nocapture",
                "--ignored",
            ])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("XDG_DATA_HOME", sandbox.path().join("data"))
            .status()
            .expect("GTK test starts");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    crate::assets::prepare().expect("assets");
    crate::assets::register_icon_theme();
    let (view, browser, window, _home, place) = present_single_pane(BrowserMode::List);
    browser.navigate(Location::local(place.path()));
    assert_navigate_lands_on_first_item(&view, &browser);
    window.destroy();
    browser.clear_observer();
}

fn assert_parent_paste_stays_in_current_directory(
    view: &BrowserView,
    browser: &crate::app::Browser,
    current: &Location,
    mode: BrowserMode,
) {
    if let Some(other) = (0..2).find(|position| {
        browser
            .entry_at(0, *position)
            .is_some_and(|entry| entry.display_name == "documents")
    }) {
        browser.set_selection(0, &[other], Some(other));
    }
    view.state.sync_mode_selection();
    let selected = browser.selected_entries();
    let names: Vec<&str> = selected
        .iter()
        .map(|entry| entry.display_name.as_str())
        .collect();
    let destination = paste_destination(
        &selected,
        browser.active_location(),
        browser.selection_is_load_cursor(),
    );
    assert_eq!(
        destination,
        Some(current.clone()),
        "{mode:?}: paste after parent must target the parent, not {names:?} (load_cursor={})",
        browser.selection_is_load_cursor()
    );
}

fn wait_for_loaded_count(browser: &crate::app::Browser, location: &Location, count: usize) {
    wait_until(|| {
        browser.column_snapshot(0).is_some_and(|snapshot| {
            !snapshot.loading && snapshot.count == count && snapshot.location == *location
        })
    });
}

#[test]
fn parent_paste_uses_the_current_directory_in_single_pane_modes() {
    crate::test_support::gtk_test(
        "ui::browser::tests::navigate::parent_paste_uses_the_current_directory_in_single_pane_modes",
        || {
            for mode in [BrowserMode::Icons, BrowserMode::List] {
                let fixture = tempfile::tempdir().expect("paste fixture");
                std::fs::create_dir_all(fixture.path().join("archive")).expect("archive");
                std::fs::create_dir_all(fixture.path().join("documents")).expect("documents");
                std::fs::write(fixture.path().join("documents/notes.txt"), "notes").expect("notes");
                let root = Location::local(fixture.path());
                let documents = Location::local(fixture.path().join("documents"));
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                let browser = view.browser();
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(650)
                    .build();
                window.present();
                browser.navigate(documents.clone());
                wait_for_loaded_count(&browser, &documents, 1);
                settle();
                browser.select(0, 0);
                browser.parent();
                wait_for_loaded_count(&browser, &root, 2);
                settle();
                assert_parent_paste_stays_in_current_directory(&view, &browser, &root, mode);

                if mode == BrowserMode::List {
                    let documents_index = (0..2)
                        .find(|position| {
                            browser
                                .entry_at(0, *position)
                                .is_some_and(|entry| entry.display_name == "documents")
                        })
                        .expect("documents");
                    browser.activate(0, documents_index);
                    wait_until(|| {
                        browser.active_location() == Some(documents.clone())
                            && browser
                                .column_snapshot(1)
                                .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 1)
                    });
                    settle();
                    browser.select(1, 0);
                    browser.parent();
                    wait_for_loaded_count(&browser, &root, 2);
                    settle();
                    assert_parent_paste_stays_in_current_directory(&view, &browser, &root, mode);
                }

                window.destroy();
                browser.clear_observer();
            }
        },
    );
}
