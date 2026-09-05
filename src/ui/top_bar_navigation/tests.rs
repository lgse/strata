// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::cell::Cell;

#[test]
fn sidebar_top_reaches_navigation_bar_and_restores_focus() {
    const CHILD: &str = "STRATA_TOP_BAR_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated GTK configuration");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "ui::top_bar_navigation::tests::sidebar_top_reaches_navigation_bar_and_restores_focus", "--nocapture"])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("XDG_DATA_HOME", sandbox.path().join("data"))
            .status().expect("GTK child process");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let toggle = gtk::ToggleButton::with_label("Sidebar");
    toggle.set_active(true);
    let disabled = gtk::Button::with_label("Disabled");
    disabled.set_sensitive(false);
    let location = gtk::Button::with_label("Location");
    let settings = gtk::Button::with_label("Settings");
    header.append(&toggle);
    header.append(&disabled);
    header.append(&location);
    header.append(&settings);
    let places = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let home = gtk::Button::with_label("Home");
    let trash = gtk::Button::with_label("Trash");
    places.append(&home);
    places.append(&trash);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&places)
        .vexpand(true)
        .build();
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.append(&scroll);
    let navigation = TopBarNavigation::new(&header, sidebar.upcast_ref(), &toggle);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&sidebar);
    let window = gtk::Window::builder()
        .child(&root)
        .default_width(600)
        .default_height(450)
        .build();
    let activations = Rc::new(Cell::new(0));
    for button in [&home, &trash, &location, &settings] {
        let activations = activations.clone();
        button.connect_clicked(move |_| activations.set(activations.get() + 1));
    }
    window.present();
    settle();
    trash.grab_focus();
    assert!(navigation.move_up_from_sidebar());
    assert_focus(&window, &home);
    assert!(!navigation.has_focus());
    assert!(navigation.move_up_from_sidebar());
    assert_focus(&window, &toggle);
    assert!(navigation.has_focus());
    assert!(!navigation.move_focus(gtk::DirectionType::Left));
    assert_focus(&window, &toggle);
    assert!(navigation.move_focus(gtk::DirectionType::Right));
    assert_focus(&window, &location);
    assert!(navigation.move_focus(gtk::DirectionType::Right));
    assert_focus(&window, &settings);
    assert!(!navigation.move_focus(gtk::DirectionType::Right));
    assert!(navigation.return_to_sidebar());
    assert_focus(&window, &home);
    assert_eq!(activations.get(), 0);
    assert!(toggle.is_active());

    assert!(navigation.move_up_from_sidebar());
    places.remove(&home);
    places.remove(&trash);
    let replacement = gtk::Button::with_label("Home after sidebar rebuild");
    places.append(&replacement);
    settle();
    assert!(navigation.return_to_sidebar());
    assert_focus(&window, &replacement);

    assert!(navigation.move_up_from_sidebar());
    sidebar.set_visible(false);
    assert!(
        !navigation.return_to_sidebar(),
        "hidden sidebars must yield to file focus"
    );
    assert_focus(&window, &toggle);
    window.destroy();
}

fn assert_focus(window: &gtk::Window, widget: &impl IsA<gtk::Widget>) {
    assert!(
        gtk::prelude::RootExt::focus(window)
            .is_some_and(|focused| { focused == *widget.as_ref() || focused.is_ancestor(widget) })
    );
}

fn settle() {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < until {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
