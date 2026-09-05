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

fn find_grid(widget: &gtk::Widget) -> Option<gtk::GridView> {
    if let Ok(icons) = widget.clone().downcast::<gtk::GridView>()
        && icons.is_mapped()
    {
        return Some(icons);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(icons) = find_grid(&widget) {
            return Some(icons);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
#[ignore = "requires a mapped GTK window; run this test alone"]
fn sidebar_boundary_tracks_icons_layout_and_empty_views() {
    const CHILD: &str = "STRATA_SIDEBAR_BOUNDARY_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated settings");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::sidebar::sidebar_boundary_tracks_icons_layout_and_empty_views",
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
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::create_dir(fixture.path().join("Child")).expect("folder group");
    for index in 0..12 {
        std::fs::write(
            fixture.path().join(format!("file-{index:02}.txt")),
            "fixture",
        )
        .expect("fixture file");
    }
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    let browser = view.browser();
    view.set_view_mode(BrowserMode::Icons);
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let sidebar = gtk::Button::with_label("Sidebar");
    root.append(&sidebar);
    root.append(&view.widget());
    let window = gtk::Window::builder()
        .child(&root)
        .default_width(1000)
        .default_height(650)
        .build();
    window.present();
    browser.navigate(Location::local(fixture.path()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !browser
        .column_snapshot(0)
        .is_some_and(|snapshot| !snapshot.loading)
    {
        assert!(Instant::now() < deadline, "directory load");
        glib::MainContext::default().iteration(false);
    }
    settle();
    let icons = find_grid(&view.widget()).expect("icons view");
    icons.set_max_columns(3);
    icons.set_min_columns(3);
    settle();
    for (position, edge) in [(0, true), (1, false), (3, true), (4, false)] {
        icons.scroll_to(
            position,
            gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
            None,
        );
        settle();
        assert!(view.item_view_has_focus());
        assert_eq!(view.at_left_edge(), edge, "position {position}");
        let selection = browser.selected_positions(0);
        assert_eq!(view.focus_header_from_top_item(), position < 3);
        if position < 3 {
            assert!(view.header_actions_have_focus());
            assert!(view.move_header_focus(gtk::DirectionType::Right));
            assert!(view.focus_items_from_header());
            assert!(view.item_view_has_focus());
            assert_eq!(browser.selected_positions(0), selection);
        }
    }
    icons.set_min_columns(1);
    icons.set_max_columns(1);
    settle();
    icons.scroll_to(
        1,
        gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
        None,
    );
    settle();
    assert!(
        view.at_left_edge(),
        "resizing to one column changes the sidebar boundary"
    );

    browser.focus_active();
    sidebar.grab_focus();
    settle();
    assert!(
        gtk::prelude::RootExt::focus(&window).as_ref() == Some(sidebar.upcast_ref()),
        "deferred item focus must not steal sidebar focus"
    );

    view.set_group_by_type(true);
    settle();
    let group = find_grid(&view.widget()).expect("folder group");
    group.scroll_to(
        0,
        gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
        None,
    );
    settle();
    assert!(view.at_left_edge());
    group.grab_focus();
    assert!(view.move_icons_group(gtk::DirectionType::Down));
    settle();
    assert_eq!(
        browser
            .focused_entry()
            .expect("file group cursor")
            .display_name,
        "file-00.txt"
    );
    assert!(view.at_left_edge());
    assert!(view.move_icons_group(gtk::DirectionType::Up));
    settle();
    assert_eq!(
        browser
            .focused_entry()
            .expect("folder group cursor")
            .display_name,
        "Child"
    );

    view.set_group_by_type(false);
    view.set_view_mode(BrowserMode::List);
    settle();
    browser.select(0, 0);
    browser.focus_active();
    settle();
    assert!(view.focus_header_from_top_item());
    assert!(view.header_actions_have_focus());
    assert!(view.move_header_focus(gtk::DirectionType::Right));
    assert!(view.focus_items_from_header());
    assert!(view.item_view_has_focus());
    let list = gtk::prelude::RootExt::focus(&window)
        .and_then(|focus| focus.ancestor(gtk::ListView::static_type()))
        .and_downcast::<gtk::ListView>()
        .expect("focused List view");
    list.scroll_to(
        2,
        gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
        None,
    );
    settle();
    assert!(!view.focus_header_from_top_item());

    let empty = tempfile::tempdir().expect("empty fixture");
    for mode in [BrowserMode::Icons, BrowserMode::List] {
        view.set_view_mode(mode);
        browser.navigate(Location::local(empty.path()));
        settle();
        browser.focus_active();
        settle();
        assert!(
            view.item_view_has_focus(),
            "empty {mode:?} needs a keyboard target"
        );
        assert!(view.at_left_edge());
        assert!(view.focus_header_from_top_item());
        assert!(view.header_actions_have_focus());
        assert!(view.focus_items_from_header());
        assert!(view.item_view_has_focus());
    }
    window.destroy();
    browser.clear_observer();
}
