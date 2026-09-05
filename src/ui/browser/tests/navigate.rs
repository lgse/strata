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
