// SPDX-License-Identifier: MIT

use super::*;

#[test]
#[ignore = "requires a mapped GTK window; run this test alone"]
fn footer_tracks_modes_and_shields_files_while_open() {
    const CHILD: &str = "STRATA_SHORTCUT_FOOTER_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::shortcut_footer::tests::footer_tracks_modes_and_shields_files_while_open",
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
        assert!(
            std::env::var_os("STRATA_REQUIRE_GTK_TESTS").is_none(),
            "GTK required"
        );
        return;
    }
    crate::assets::prepare().expect("assets");
    let view = super::super::browser::BrowserView::new(
        std::rc::Rc::new(crate::adapters::LocalFileSource),
        super::super::browser::PeekBehavior::default(),
    );
    let footer = ShortcutFooter::new(view.view_mode());
    footer.observe_browser(&view.browser());
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
    view.browser().set_selection(0, &[], None);
    assert_eq!(footer.count.text(), "3 items");
    assert_eq!(
        footer.count.tooltip_text().as_deref(),
        Some("2 files, 1 folder")
    );
    // Other test windows must not compete for the display's global popup grab.
    footer.popover.set_autohide(false);
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
    settle();
    for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns] {
        view.set_view_mode(mode);
        let heading = footer
            .reference
            .first_child()
            .and_then(|section| section.first_child())
            .and_downcast::<gtk::Label>()
            .expect("navigation reference heading");
        assert_eq!(
            heading.text(),
            match mode {
                BrowserMode::Columns => "Columns navigation",
                BrowserMode::Icons => "Icons navigation",
                BrowserMode::List => "List navigation",
            }
        );
        assert!(footer.widget().is_visible());
        let depth = view.browser().active_depth().expect("active directory");
        view.browser().set_selection(depth, &[0, 1, 2], Some(2));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while footer.count.text() != "1 folder, 2 files selected (6 B)"
            && std::time::Instant::now() < deadline
        {
            settle();
        }
        assert_eq!(footer.count.text(), "1 folder, 2 files selected (6 B)");
        assert!(
            footer
                .count
                .tooltip_text()
                .is_some_and(|text| { text.contains("folder contents are not counted") })
        );
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
        assert_eq!(footer.count.text(), "1 file selected (3 B)");
        view.browser().set_selection(depth, &[], None);
        assert_eq!(footer.count.text(), "3 items");
    }
    let mut files = (0..3)
        .filter_map(|position| view.browser().entry_at(0, position))
        .filter(|entry| !entry.is_directory())
        .collect::<Vec<_>>();
    for (sizes, expected) in [
        (
            [
                crate::model::MetadataValue::Known(0),
                crate::model::MetadataValue::Known(0),
            ],
            "2 files selected (0 B)",
        ),
        (
            [
                crate::model::MetadataValue::Known(64_000_000),
                crate::model::MetadataValue::Known(0),
            ],
            "2 files selected (64 MB)",
        ),
        (
            [
                crate::model::MetadataValue::Known(3),
                crate::model::MetadataValue::Unknown,
            ],
            "2 files selected (3 B known; size incomplete)",
        ),
        (
            [
                crate::model::MetadataValue::Unavailable,
                crate::model::MetadataValue::Unknown,
            ],
            "2 files selected (size unavailable)",
        ),
    ] {
        for (entry, size) in files.iter_mut().zip(sizes) {
            entry.size = size;
        }
        assert_eq!(selection_details(&files), expected);
    }
    view.browser().navigate(crate::model::Location::local(
        directory.path().join("folder"),
    ));
    settle();
    assert_eq!(footer.count.text(), "0 items");
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
    assert!(footer.popover.child_focus(gtk::DirectionType::TabForward));
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
    let manager = super::super::preferences::PreferenceManager::shared();
    footer.bind_preferences(&manager);
    let other = ShortcutFooter::new(BrowserMode::Icons);
    other.bind_preferences(&manager);
    for enabled in [false, true, false] {
        manager.set_show_keybinding_hints(enabled);
        footer.assert_hints_visible(enabled);
        other.assert_hints_visible(enabled);
    }
    let settings =
        std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("isolated config"))
            .join("strata/settings.toml");
    let saved: toml::Value =
        toml::from_str(&std::fs::read_to_string(settings).expect("saved preferences"))
            .expect("valid preferences");
    assert_eq!(saved["show_keybinding_hints"].as_bool(), Some(false));
    footer.handle_key(gdk::Key::F1, none);
    settle();
    assert!(footer.popover.is_visible());
    footer.handle_key(gdk::Key::F1, none);
    footer.handle_key(gdk::Key::F1, none);
    settle();
    assert!(footer.popover.is_visible());
    footer.handle_key(gdk::Key::F1, none);
    settle();
    assert!(!footer.popover.is_visible());
    footer.assert_hints_visible(false);
    assert!(!manager.show_keybinding_hints());
    assert!(gtk::prelude::RootExt::focus(&window).is_some_and(|focus| {
        focus == *entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&entry)
    }));
    window.destroy();
    view.browser().clear_observer();
}

impl ShortcutFooter {
    pub(crate) fn assert_hints_visible(&self, visible: bool) {
        assert_eq!(
            self.widget().is_visible(),
            visible || self.paste.is_visible() || self.count.is_visible()
        );
        assert_eq!(self.more.is_visible(), visible);
    }
}

#[test]
fn tenxer_reference_follows_the_active_map() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::tenxer_reference_follows_the_active_map",
        || {
            let manager = super::super::preferences::PreferenceManager::shared();
            manager.set_tenxer_mode(false);
            manager.set_show_keybinding_hints(true);
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            footer.bind_preferences(&manager);
            let entry = gtk::Entry::new();
            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
            root.append(&entry);
            root.append(footer.widget());
            let window = gtk::Window::builder()
                .child(&root)
                .default_width(640)
                .default_height(480)
                .build();
            footer.popover.set_autohide(false);
            window.present();
            entry.grab_focus();
            settle();
            let phrase = crate::ui::shortcut_reference::EXPERIMENTAL_LABEL;
            let none = gdk::ModifierType::empty();
            assert!(!footer.tag_note.is_visible());
            assert!(
                reference_labels(&footer)
                    .iter()
                    .any(|label| label == "Toggle file preview")
            );
            assert_eq!(footer.handle_key(gdk::Key::asciitilde, none), None);
            assert!(!footer.popover.is_visible());

            manager.set_tenxer_mode(true);
            settle();
            assert!(footer.tag_note.is_visible());
            assert_eq!(footer.tag_note.text(), phrase);
            assert!(
                footer
                    .tag
                    .tooltip_text()
                    .is_some_and(|text| text.contains(phrase))
            );
            let columns = reference_labels(&footer);
            assert!(columns.iter().any(|label| label == "Leave 10xer mode"));
            assert!(
                columns
                    .iter()
                    .any(|label| label == "Parent pane / enter folder")
            );
            assert!(!columns.iter().any(|label| label == "Toggle file preview"));
            assert!(
                !columns
                    .iter()
                    .any(|label| label == "Move spatially between tiles")
            );
            footer.set_mode(BrowserMode::Icons);
            let icons = reference_labels(&footer);
            assert!(
                icons
                    .iter()
                    .any(|label| label == "Move spatially between tiles")
            );
            assert!(
                !icons
                    .iter()
                    .any(|label| label == "Parent pane / enter folder")
            );

            footer.handle_key(gdk::Key::F1, none);
            settle();
            assert!(footer.popover.is_visible());
            assert!(
                gtk::prelude::RootExt::focus(&window).is_some_and(|focus| {
                    focus == *footer.scroll.upcast_ref::<gtk::Widget>()
                        || focus.is_ancestor(&footer.scroll)
                }),
                "the open reference takes keyboard focus"
            );
            press_reference(&footer, gdk::Key::F1);
            settle();
            assert!(
                !footer.popover.is_visible(),
                "F1 from the open reference closes it"
            );
            assert!(gtk::prelude::RootExt::focus(&window).is_some_and(|focus| {
                focus == *entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&entry)
            }));
            footer.handle_key(gdk::Key::asciitilde, none);
            settle();
            assert!(footer.popover.is_visible());
            assert_eq!(
                footer.handle_key(gdk::Key::Delete, none),
                Some(glib::Propagation::Stop)
            );
            footer.handle_key(gdk::Key::Escape, none);
            settle();
            assert!(!footer.popover.is_visible());

            let other = ShortcutFooter::new(BrowserMode::List);
            other.bind_preferences(&manager);
            assert!(
                reference_labels(&other)
                    .iter()
                    .any(|label| label == "Leave 10xer mode")
            );
            manager.set_tenxer_mode(false);
            settle();
            assert!(!footer.tag_note.is_visible());
            assert!(
                footer
                    .tag
                    .tooltip_text()
                    .is_none_or(|text| !text.contains(phrase))
            );
            for shown in [&footer, &other] {
                let labels = reference_labels(shown);
                assert!(labels.iter().any(|label| label == "Toggle file preview"));
                assert!(!labels.iter().any(|label| label == "Leave 10xer mode"));
            }
            window.destroy();
        },
    );
}

fn press_reference(footer: &ShortcutFooter, key: gdk::Key) {
    let controllers = footer.popover.observe_controllers();
    let controller = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .next()
        .expect("reference key controller");
    assert!(
        controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &gdk::ModifierType::empty()]),
        "{key:?} should be handled by the open reference"
    );
}

fn reference_labels(footer: &ShortcutFooter) -> Vec<String> {
    let mut labels = Vec::new();
    collect_label_text(footer.reference.upcast_ref(), &mut labels);
    labels
}

fn collect_label_text(widget: &gtk::Widget, labels: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        labels.push(label.text().to_string());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        collect_label_text(&current, labels);
        child = current.next_sibling();
    }
}

fn settle() {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < until {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn paste_availability_tracks_file_clipboard() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::paste_availability_tracks_file_clipboard",
        || {
            let clipboard = gdk::Display::default().expect("test display").clipboard();
            let files = gdk::FileList::from_array(&[gtk::gio::File::for_path(
                "/tmp/strata-clipboard-fixture.txt",
            )]);
            let provider = gdk::ContentProvider::for_value(&files.to_value());
            clipboard
                .set_content(Some(&provider))
                .expect("file clipboard");
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            let handler = footer.connect_clipboard(&clipboard);
            let manager = super::super::preferences::PreferenceManager::shared();
            manager.set_show_keybinding_hints(false);
            footer.bind_preferences(&manager);
            footer.assert_hints_visible(false);
            settle();
            assert!(
                footer.paste.is_visible(),
                "existing files on the clipboard enable paste"
            );
            assert!(footer.widget().is_visible());
            clipboard.set_text("plain text is not a file clipboard");
            settle();
            assert!(!footer.paste.is_visible());
            assert!(!footer.widget().is_visible());
            let uri = gdk::ContentProvider::for_bytes(
                "text/uri-list",
                &glib::Bytes::from_static(b"file:///tmp/strata-clipboard-fixture.txt\r\n"),
            );
            clipboard.set_content(Some(&uri)).expect("URI clipboard");
            settle();
            assert!(
                footer.paste.is_visible(),
                "external URI lists also enable paste"
            );
            assert!(footer.widget().is_visible());
            clipboard
                .set_content(Some(&provider))
                .expect("pending file clipboard");
            clipboard.set_text("newer clipboard replaces a pending file read");
            settle();
            assert!(!footer.paste.is_visible());
            clipboard
                .set_content(Some(&provider))
                .expect("cut clipboard");
            settle();
            assert!(footer.paste.is_visible());
            clipboard
                .set_content(None::<&gdk::ContentProvider>)
                .expect("cleared clipboard");
            settle();
            assert!(
                !footer.paste.is_visible(),
                "consuming a cut clears paste availability"
            );
            assert!(!footer.widget().is_visible());
            let empty: Option<gdk::FileList> = None;
            clipboard
                .set_content(Some(&gdk::ContentProvider::for_value(&empty.to_value())))
                .expect("empty file clipboard");
            settle();
            assert!(!footer.paste.is_visible());
            clipboard.disconnect(handler);
            clipboard
                .set_content(None::<&gdk::ContentProvider>)
                .expect("fixture cleanup");
        },
    );
}
