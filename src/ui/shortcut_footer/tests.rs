// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn navigation_reference_matches_each_mode() {
    for mode in [
        BrowserMode::Columns,
        BrowserMode::Grid,
        BrowserMode::Explorer,
    ] {
        let navigation = navigation_shortcuts(mode);
        assert!(navigation.contains(&("Alt+↑", "Go to the parent folder")));
        assert!(navigation.contains(&("↑ at top", "Focus the navigation header")));
        assert!(navigation.contains(&("↓ in header", "Return to the files")));
        assert!(!navigation.iter().any(|(key, _)| *key == "Ctrl+Left"));
        assert!(summary_shortcuts(mode).contains(&("Enter", "Open")));
    }
    assert!(
        navigation_shortcuts(BrowserMode::Grid)
            .contains(&("← at left edge", "Focus the visible sidebar"))
    );
    assert!(
        navigation_shortcuts(BrowserMode::Explorer).contains(&("←", "Focus the visible sidebar"))
    );
    assert_ne!(
        summary_shortcuts(BrowserMode::Columns),
        summary_shortcuts(BrowserMode::Grid)
    );
    assert_ne!(
        summary_shortcuts(BrowserMode::Grid),
        summary_shortcuts(BrowserMode::Explorer)
    );
    assert!(TOOLS.contains(&("F1", "Show or hide this reference")));
}

#[test]
fn footer_tracks_modes_and_shields_files_while_open() {
    const CHILD: &str = "STRATA_SHORTCUT_FOOTER_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::shortcut_footer::tests::footer_tracks_modes_and_shields_files_while_open",
                "--nocapture",
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
    let view = super::super::browser::BrowserView::new(
        std::rc::Rc::new(crate::adapters::LocalFileSource),
        super::super::browser::PeekBehavior::default(),
    );
    let footer = ShortcutFooter::new(view.view_mode());
    let updated = footer.clone();
    view.connect_view_mode_changed(move |mode| updated.set_mode(mode));
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let entry = gtk::Entry::new();
    root.append(&entry);
    root.append(footer.widget());
    let window = gtk::Window::builder()
        .child(&root)
        .default_width(600)
        .default_height(500)
        .build();
    window.present();
    entry.grab_focus();
    for mode in [
        BrowserMode::Grid,
        BrowserMode::Explorer,
        BrowserMode::Columns,
    ] {
        view.set_view_mode(mode);
        assert!(
            footer
                .summary
                .text()
                .starts_with(summary_shortcuts(mode)[0].0)
        );
        assert_eq!(footer.summary.ellipsize(), gtk::pango::EllipsizeMode::End);
        assert!(footer.summary.is_single_line_mode());
        assert!(footer.widget().is_visible());
    }
    let none = gdk::ModifierType::empty();
    assert_eq!(footer.handle_key(gdk::Key::Delete, none), None);
    assert_eq!(
        footer.handle_key(gdk::Key::F1, gdk::ModifierType::CONTROL_MASK),
        None
    );
    assert_eq!(
        footer.handle_key(gdk::Key::F1, none),
        Some(glib::Propagation::Stop)
    );
    assert!(footer.popover.is_visible());
    assert_eq!(
        footer.handle_key(gdk::Key::Delete, none),
        Some(glib::Propagation::Stop)
    );
    assert_eq!(
        footer.handle_key(gdk::Key::v, gdk::ModifierType::CONTROL_MASK),
        Some(glib::Propagation::Stop)
    );
    assert_eq!(
        footer.handle_key(gdk::Key::Tab, none),
        Some(glib::Propagation::Proceed)
    );
    assert_eq!(
        footer.handle_key(gdk::Key::Escape, none),
        Some(glib::Propagation::Stop)
    );
    assert!(!footer.popover.is_visible());
    while glib::MainContext::default().iteration(false) {}
    assert!(
        gtk::prelude::RootExt::focus(&window).is_some_and(|focus| {
            focus == *entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&entry)
        }),
        "closing keyboard help must restore the previous editing or browsing focus"
    );
    assert_eq!(footer.handle_key(gdk::Key::Delete, none), None);
    window.destroy();
    view.browser().clear_observer();
}
