// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn navigation_reference_matches_each_mode() {
    assert!(
        navigation_shortcuts(BrowserMode::Columns)
            .contains(&("← / →", "Parent pane / enter folder"))
    );
    assert!(
        navigation_shortcuts(BrowserMode::Icons)
            .contains(&("← at left edge", "Focus the visible sidebar"))
    );
    assert!(navigation_shortcuts(BrowserMode::List).contains(&("←", "Focus the visible sidebar")));
}

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
    footer.observe_browser(&view);
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
    root.append(&view.widget());
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
        view.browser().set_selection(depth, &[folder], Some(folder));
        assert!(view.show_filter_with_query(".txt"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while view.search_result_listing().as_ref().map(Vec::len) != Some(2) {
            assert!(
                std::time::Instant::now() < deadline,
                "{mode:?}: expected two .txt results"
            );
            settle();
        }
        assert_eq!(
            footer.count.text(),
            "2 items",
            "{mode:?}: published .txt result count"
        );
        assert_eq!(
            footer.count.tooltip_text().as_deref(),
            Some("2 files, 0 folders")
        );
        assert!(view.focus_first_search_result());
        view.select_all();
        assert_eq!(
            view.selected_search_results().expect("selected hits").len(),
            2
        );
        assert_eq!(
            footer.count.text(),
            "2 items",
            "result total is independent of hit selection"
        );
        assert!(view.show_filter_with_query("no-matching-name"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while view.search_result_listing().as_ref().map(Vec::len) != Some(0) {
            assert!(
                std::time::Instant::now() < deadline,
                "{mode:?}: expected no matching results"
            );
            settle();
        }
        assert_eq!(footer.count.text(), "0 items");
        view.dismiss_hidden_filter();
        assert!(view.search_result_listing().is_none());
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
    entry.grab_focus();
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
        // The footer stays visible for the minimal MIN tag, filter mark,
        // chord label, flash hint, or open prompt even when hints are off.
        let status = visible
            || self.paste.is_visible()
            || self.count.is_visible()
            || self.min_tag.is_visible()
            || self.filter_mark.is_visible()
            || self.chord_label.is_visible()
            || self.flash_label.is_visible()
            || self.prompt_box.is_visible();
        assert_eq!(self.widget().is_visible(), status);
        assert_eq!(self.more.is_visible(), visible);
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

#[test]
fn chord_popups_leave_other_windows_animation_settings_untouched() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::chord_popups_leave_other_windows_animation_settings_untouched",
        || {
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            footer.set_minimal(true);
            let first = gtk::Window::builder().child(footer.widget()).build();
            let other = gtk::Window::new();
            first.present();
            other.present();
            settle();
            let settings = other.settings();
            let previous = settings.is_gtk_enable_animations();
            settings.set_gtk_enable_animations(true);
            let notifications = Rc::new(Cell::new(0));
            let observed = notifications.clone();
            let handler = settings.connect_gtk_enable_animations_notify(move |_| {
                observed.set(observed.get() + 1);
            });
            let preferences = super::super::preferences::PreferenceManager::shared();
            for reduced in [true, false, true] {
                preferences.set_reduce_motion(reduced);
                for chord in [
                    MinimalChord::Go,
                    MinimalChord::Copy,
                    MinimalChord::Sort,
                    MinimalChord::Action,
                ] {
                    footer.set_chord_mark(chord);
                    settle();
                    assert!(footer.chord_hints_visible());
                    footer.clear_chord_mark();
                    settle();
                    assert!(!footer.chord_hints_visible());
                }
                assert_eq!(
                    notifications.get(),
                    0,
                    "popups must not mutate another window's GTK settings"
                );
                assert!(settings.is_gtk_enable_animations());
                assert_eq!(preferences.reduce_motion(), reduced);
            }
            settings.disconnect(handler);
            settings.set_gtk_enable_animations(previous);
            first.destroy();
            other.destroy();
        },
    );
}

#[test]
fn prompt_reopens_after_hide() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::prompt_reopens_after_hide",
        || {
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(footer.widget())
                .default_width(600)
                .default_height(80)
                .build();
            window.present();
            settle();
            footer.show_prompt("/", "find", "", None);
            settle();
            assert_eq!(footer.stack.visible_child_name().as_deref(), Some("prompt"));
            assert!(footer.prompt_box.is_visible());
            footer.hide_prompt();
            settle();
            assert_eq!(footer.stack.visible_child_name().as_deref(), Some("status"));
            footer.show_prompt("/", "find", "", None);
            settle();
            assert_eq!(footer.stack.visible_child_name().as_deref(), Some("prompt"));
            assert!(footer.prompt_box.is_visible());
            assert_eq!(footer.prompt_prefix.label().as_str(), "/");
            window.destroy();
        },
    );
}

#[test]
fn minimal_mode_tag_and_reference_name_the_experimental_feature() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::minimal_mode_tag_and_reference_name_the_experimental_feature",
        || {
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            let manager = super::super::preferences::PreferenceManager::shared();
            footer.bind_minimal_mode(&manager);
            let rows = || {
                let mut rows = Vec::new();
                let mut section = footer.reference.first_child();
                while let Some(widget) = section {
                    section = widget.next_sibling();
                    let mut row = widget.first_child();
                    while let Some(widget) = row {
                        row = widget.next_sibling();
                        if let Some(key) = widget.first_child().and_downcast::<gtk::Label>() {
                            let action = key
                                .next_sibling()
                                .and_downcast::<gtk::Label>()
                                .expect("rendered shortcut action");
                            rows.push((key.text().to_string(), action.text().to_string()));
                        }
                    }
                }
                rows
            };
            for minimal in [false, true, false, true] {
                manager.set_minimal_mode(minimal);
                let rows = rows();
                assert!(rows.iter().any(|(key, action)| key == "Ctrl+Shift+M"
                    && action == "Toggle minimal mode (also while editing text)"));
                assert_eq!(rows.iter().any(|(key, action)| key == "l / →"
                    && action == "Open directory / enter preview (List and Columns; repetition keeps the preview open)"), minimal);
                assert_eq!(
                    rows.iter().any(|(key, action)| key
                        == "h / j / k / l / ← / → / ↑ / ↓"
                        && action
                            == "Move among icon tiles (Icons; stays in the current folder, never previews)"),
                    minimal
                );
                assert_eq!(
                    rows.iter().any(|(key, action)| key == "i"
                        && action
                            == "Toggle the preview drawer (Icons keeps listing focus; no preview-focus hotkey)"),
                    minimal
                );
                assert_eq!(
                    rows.iter()
                        .any(|(key, action)| key == "Ctrl+D" && action == "Duplicate"),
                    !minimal
                );
            }
            let tooltip = footer
                .min_tag
                .tooltip_text()
                .expect("MIN tag tooltip")
                .as_str()
                .to_string();
            assert!(
                tooltip.contains("Minimal mode")
                    && tooltip.contains(crate::ui::minimal_mode::EXPERIMENTAL_NOTE),
                "MIN tooltip names the experimental feature, got {tooltip:?}"
            );
            let headings: Vec<String> = {
                let mut labels = Vec::new();
                let mut child = footer.reference.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    let mut nested = widget.first_child();
                    while let Some(inner) = nested {
                        nested = inner.next_sibling();
                        if let Ok(label) = inner.downcast::<gtk::Label>()
                            && label.has_css_class("shortcut-reference-heading")
                        {
                            labels.push(label.text().to_string());
                        }
                    }
                }
                labels
            };
            assert!(
                headings
                    .iter()
                    .any(|text| text == crate::ui::minimal_mode::LABELED_TITLE),
                "F1 heading includes the experimental note, got {headings:?}"
            );
        },
    );
}
