// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser::BrowserView;

const CHILD: &str = "STRATA_SHORTCUT_FOOTER_GTK_CHILD";

/// Reruns `name` in a child process with isolated preferences; in that child,
/// initializes GTK. Returns whether the caller is the child and may continue.
fn runs_in_gtk_child(name: &str) -> bool {
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", name, "--nocapture", "--ignored"])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("XDG_DATA_HOME", sandbox.path().join("data"))
            .status()
            .expect("GTK test starts");
        assert!(status.success());
        return false;
    }
    if gtk::init().is_err() {
        assert!(
            std::env::var_os("STRATA_REQUIRE_GTK_TESTS").is_none(),
            "GTK required"
        );
        return false;
    }
    true
}

fn browse_fixture_folder(view: &BrowserView) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("count fixture");
    std::fs::write(directory.path().join("one.txt"), "one").expect("first file");
    std::fs::write(directory.path().join("two.txt"), "two").expect("second file");
    std::fs::create_dir(directory.path().join("folder")).expect("empty folder");
    view.browser()
        .navigate(crate::model::Location::local(directory.path()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while view
        .browser()
        .column_entry_counts(0)
        .map(|counts| counts.total)
        != Some(3)
        && std::time::Instant::now() < deadline
    {
        settle();
    }
    directory
}

fn present_with_entry(footer: &ShortcutFooter) -> (gtk::Window, gtk::Entry) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let entry = gtk::Entry::new();
    root.append(&entry);
    root.append(footer.widget());
    let window = gtk::Window::builder()
        .child(&gtk::Overlay::builder().child(&root).build())
        .default_width(600)
        .default_height(500)
        .build();
    window.present();
    entry.grab_focus();
    settle();
    (window, entry)
}

fn wait_for_count(footer: &ShortcutFooter, text: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while footer.count.text() != text && std::time::Instant::now() < deadline {
        settle();
    }
    assert_eq!(footer.count.text(), text);
}

fn navigation_heading(footer: &ShortcutFooter) -> gtk::Label {
    footer
        .reference
        .first_child()
        .and_then(|section| section.first_child())
        .and_then(|heading| heading.first_child())
        .and_downcast::<gtk::Label>()
        .expect("navigation reference heading")
}

fn each_mode_names_its_reference_and_counts_the_selection(
    view: &BrowserView,
    footer: &ShortcutFooter,
) {
    for mode in [
        BrowserMode::Icons,
        BrowserMode::List,
        BrowserMode::Compact,
        BrowserMode::Columns,
    ] {
        view.set_view_mode(mode);
        assert_eq!(
            navigation_heading(footer).text(),
            match mode {
                BrowserMode::Columns => "COLUMNS NAVIGATION",
                BrowserMode::Icons => "ICONS NAVIGATION",
                BrowserMode::List => "LIST NAVIGATION",
                BrowserMode::Compact => "COMPACT NAVIGATION",
            }
        );
        assert!(footer.widget().is_visible());
        let depth = view.browser().active_depth().expect("active directory");
        view.browser().set_selection(depth, &[0, 1, 2], Some(2));
        wait_for_count(footer, "1 folder, 2 files selected");
        assert!(footer.count.tooltip_text().is_none());
        let folder = (0..3)
            .find(|position| {
                view.browser()
                    .entry_at(depth, *position)
                    .is_some_and(|entry| entry.is_directory())
            })
            .expect("folder position");
        let file = (0..3)
            .find(|position| *position != folder)
            .expect("file position");
        view.browser().set_selection(depth, &[folder], Some(folder));
        assert_eq!(footer.count.text(), "1 folder selected");
        view.browser().set_selection(depth, &[file], Some(file));
        wait_for_count(footer, "1 file selected (3 B)");
        view.browser().set_selection(depth, &[], None);
        assert_eq!(footer.count.text(), "3 items");
    }
}

fn selection_details_describe_known_and_missing_sizes(view: &BrowserView) {
    use crate::model::MetadataValue::{Known, Unavailable, Unknown};
    let mut files = (0..3)
        .filter_map(|position| view.browser().entry_at(0, position))
        .filter(|entry| !entry.is_directory())
        .collect::<Vec<_>>();
    for (sizes, expected) in [
        ([Known(0), Known(0)], "2 files selected (0 B)"),
        ([Known(64_000_000), Known(0)], "2 files selected (64 MB)"),
        (
            [Known(3), Unknown],
            "2 files selected (3 B known; size incomplete)",
        ),
        (
            [Unavailable, Unknown],
            "2 files selected (size unavailable)",
        ),
    ] {
        for (entry, size) in files.iter_mut().zip(sizes) {
            entry.size = size;
        }
        assert_eq!(selection_details(&files), expected);
    }
}

fn the_open_reference_shields_file_keys(
    footer: &ShortcutFooter,
    window: &gtk::Window,
    entry: &gtk::Entry,
) {
    let none = gdk::ModifierType::empty();
    let ctrl = gdk::ModifierType::CONTROL_MASK;
    expect_key(footer, gdk::Key::Delete, none, None);
    expect_key(footer, gdk::Key::F1, ctrl, None);
    expect_key(footer, gdk::Key::F1, none, Some(glib::Propagation::Stop));
    assert!(footer.session.is_open());
    settle();
    assert!(footer.panel.child_focus(gtk::DirectionType::TabForward));
    footer.search.grab_focus();
    expect_key(
        footer,
        gdk::Key::Delete,
        none,
        Some(glib::Propagation::Proceed),
    );
    expect_key(footer, gdk::Key::v, ctrl, Some(glib::Propagation::Proceed));
    footer.scroll.grab_focus();
    expect_key(
        footer,
        gdk::Key::Delete,
        none,
        Some(glib::Propagation::Stop),
    );
    expect_key(footer, gdk::Key::Tab, none, Some(glib::Propagation::Stop));
    assert!(focus_within(window, &footer.search));
    expect_key(
        footer,
        gdk::Key::Escape,
        none,
        Some(glib::Propagation::Stop),
    );
    assert!(!footer.session.is_open());
    while glib::MainContext::default().iteration(false) {}
    assert!(
        focus_within(window, entry),
        "closing keyboard help must restore the previous editing or browsing focus"
    );
    expect_key(footer, gdk::Key::Delete, none, None);
}

fn hints_follow_the_saved_preference(
    footer: &ShortcutFooter,
    window: &gtk::Window,
    entry: &gtk::Entry,
) {
    let none = gdk::ModifierType::empty();
    let manager = crate::ui::preferences::PreferenceManager::shared();
    footer.bind_preferences(&manager);
    let other = ShortcutFooter::new(BrowserMode::Icons);
    other.bind_preferences(&manager);
    for enabled in [false, true, false] {
        manager.set_show_keybinding_hints(enabled);
        footer.assert_hints_visible(enabled);
        other.assert_hints_visible(enabled);
    }
    crate::storage::writer::settle();
    let settings =
        std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("isolated config"))
            .join("strata/settings.toml");
    let saved: toml::Value =
        toml::from_str(&std::fs::read_to_string(settings).expect("saved preferences"))
            .expect("valid preferences");
    assert_eq!(saved["show_keybinding_hints"].as_bool(), Some(false));
    footer.handle_key(&press(gdk::Key::F1, none));
    settle();
    assert!(footer.session.is_open());
    footer.handle_key(&press(gdk::Key::F1, none));
    footer.handle_key(&press(gdk::Key::F1, none));
    settle();
    assert!(footer.session.is_open());
    footer.handle_key(&press(gdk::Key::F1, none));
    settle();
    assert!(!footer.session.is_open());
    footer.assert_hints_visible(false);
    assert!(!manager.show_keybinding_hints());
    assert!(focus_within(window, entry));
}

#[test]
#[ignore = "requires a mapped GTK window; run this test alone"]
fn footer_tracks_modes_and_shields_files_while_open() {
    if !runs_in_gtk_child(
        "ui::shortcut_footer::tests::modes::footer_tracks_modes_and_shields_files_while_open",
    ) {
        return;
    }
    crate::assets::prepare().expect("assets");
    let view = BrowserView::new(
        std::rc::Rc::new(crate::adapters::LocalFileSource),
        crate::ui::browser::PeekBehavior::default(),
    );
    let footer = ShortcutFooter::new(view.view_mode());
    footer.observe_browser(&view.browser());
    let directory = browse_fixture_folder(&view);
    view.browser().set_selection(0, &[], None);
    assert_eq!(footer.count.text(), "3 items");
    assert!(footer.count.tooltip_text().is_none());
    let updated = footer.clone();
    view.connect_view_mode_changed(move |mode| updated.set_mode(mode));
    let (window, entry) = present_with_entry(&footer);
    each_mode_names_its_reference_and_counts_the_selection(&view, &footer);
    selection_details_describe_known_and_missing_sizes(&view);
    view.browser().navigate(crate::model::Location::local(
        directory.path().join("folder"),
    ));
    settle();
    assert_eq!(footer.count.text(), "0 items");
    the_open_reference_shields_file_keys(&footer, &window, &entry);
    hints_follow_the_saved_preference(&footer, &window, &entry);
    window.destroy();
    view.browser().clear_observer();
}
