// SPDX-License-Identifier: MIT

use super::*;

fn remote_entry(name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        location: Location::uri(format!("{SHARE}/{name}")),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn normal_filter_slash_rejects_files_and_cancels_on_edit_focus_loss_and_escape() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::filter_slash::normal_filter_slash_rejects_files_and_cancels_on_edit_focus_loss_and_escape",
        || {
            let fixture = KeyboardFixture::new();
            let root = fixture._directory.path();
            for folder in ["Documents", "reports"] {
                std::fs::create_dir(root.join(folder)).expect("fixture folder");
            }
            std::fs::write(root.join("report"), b"file").expect("fixture file");
            PreferenceManager::shared().set_filter_include_subfolders(true);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                fixture.view.refresh();
                wait_loaded(&browser, 0);
                focus_files(&fixture);
                assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                let entry = focused_entry(&fixture.window);
                for query in ["", "no-such-match", "report"] {
                    entry.set_text(query);
                    entry.select_region(0, -1);
                    let generation = browser.navigation_generation();
                    type_slash(&fixture);
                    pump(150);
                    assert_eq!(
                        browser.navigation_generation(),
                        generation,
                        "{mode:?} {query}"
                    );
                    assert_eq!(entry.text(), query);
                    assert!(fixture.view.filter_has_focus());
                }
                entry.set_overwrite_mode(true);
                entry.set_position(0);
                type_slash(&fixture);
                pump(150);
                assert_eq!(entry.text(), "report", "{mode:?}: / overwrote a character");
                assert_eq!(
                    entry.position(),
                    0,
                    "{mode:?}: the cursor stays where it was"
                );
                entry.set_overwrite_mode(false);
                entry.set_text("Documents");
                let generation = browser.navigation_generation();
                crate::test_support::commit_text(&entry, "/Projects");
                pump(150);
                assert_eq!(browser.navigation_generation(), generation, "{mode:?}");
                assert!(
                    entry.text().contains('/'),
                    "{mode:?}: a pasted path is typed"
                );
                assert!(fixture.view.filter_has_focus());
                entry.set_text("doc");
                type_slash(&fixture);
                entry.set_text("report");
                pump(150);
                assert_eq!(browser.active_location(), Some(Location::local(root)));
                entry.set_text("doc");
                type_slash(&fixture);
                fixture.sidebar_toggle.grab_focus();
                assert!(fixture.view.show_filter());
                pump(150);
                assert_eq!(
                    browser.active_location(),
                    Some(Location::local(root)),
                    "{mode:?}: returning focus resumed cancelled navigation"
                );
                assert_eq!(entry.text(), "doc");
                entry.set_text("Documents");
                type_slash(&fixture);
                assert!(fixture.press(Key::Escape, none));
                pump(150);
                assert_eq!(browser.active_location(), Some(Location::local(root)));
                assert!(!fixture.view.filter_has_focus());
            }
        },
    );
}

#[test]
fn normal_filter_slash_descends_and_keeps_filtering() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::filter_slash::normal_filter_slash_descends_and_keeps_filtering",
        || {
            let fixture = KeyboardFixture::new();
            let root = fixture._directory.path();
            for folder in ["Documents/Projects", "nested/Archive"] {
                std::fs::create_dir_all(root.join(folder)).expect("fixture folder");
            }
            let browser = fixture.view.browser();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                for recursive in [false, true] {
                    PreferenceManager::shared().set_filter_include_subfolders(recursive);
                    browser.navigate(Location::local(root));
                    fixture.view.refresh();
                    wait_loaded(&browser, 0);
                    focus_files(&fixture);
                    assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                    let entry = focused_entry(&fixture.window);
                    entry.set_text("doc");
                    // Ctrl+F onto a query selects it, and a commit replaces a selection.
                    entry.select_region(0, -1);
                    type_slash(&fixture);
                    wait_until(|| {
                        browser.active_location() == Some(Location::local(root.join("Documents")))
                            && fixture.view.filter_has_focus()
                    });
                    let entry = focused_entry(&fixture.window);
                    assert_eq!(entry.text(), "");
                    entry.set_text("proj");
                    type_slash(&fixture);
                    wait_until(|| {
                        browser.active_location()
                            == Some(Location::local(root.join("Documents/Projects")))
                            && fixture.view.filter_has_focus()
                    });
                    assert_eq!(focused_entry(&fixture.window).text(), "");
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    if recursive {
                        browser.navigate(Location::local(root));
                        wait_loaded(&browser, 0);
                        focus_files(&fixture);
                        assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                        focused_entry(&fixture.window).set_text("arc");
                        type_slash(&fixture);
                        wait_until(|| {
                            browser.active_location()
                                == Some(Location::local(root.join("nested/Archive")))
                                && fixture.view.filter_has_focus()
                        });
                        assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    }
                }
            }
        },
    );
}

#[test]
fn normal_filter_slash_enters_a_remote_folder_from_in_place_results() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::filter_slash::normal_filter_slash_enters_a_remote_folder_from_in_place_results",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let fixture = KeyboardFixture::with_parts(Rc::new(TextPreview), || {
                    BrowserView::new(
                        Rc::new(RemoteFolders),
                        crate::ui::browser::PeekBehavior::default(),
                    )
                });
                fixture.view.set_view_mode(mode);
                let browser = fixture.view.browser();
                browser.navigate(Location::uri(SHARE));
                wait_loaded(&browser, 0);
                focus_files(&fixture);
                assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                focused_entry(&fixture.window).set_text("notes");
                type_slash(&fixture);
                wait_until_or(
                    || {
                        browser.active_location()
                            == Some(Location::uri(format!("{SHARE}/notes.txt")))
                            && fixture.view.filter_has_focus()
                    },
                    || {
                        format!(
                            "{mode:?}: location {:?}, filter focus {}, query {:?}",
                            browser.active_location(),
                            fixture.view.filter_has_focus(),
                            fixture.view.listing_filter()
                        )
                    },
                );
                assert_eq!(focused_entry(&fixture.window).text(), "");
            }
        },
    );
}

/// A share whose first listing streams `notes.txt`, then stays loading until released
/// with a `notes` folder that sorts ahead of it.
#[derive(Default)]
struct StreamingShare {
    held: Cell<bool>,
    release: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl FileSource for Rc<StreamingShare> {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        RemoteShare.validate_location(location)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if request.location != Location::uri(SHARE) || self.held.replace(true) {
            return RemoteShare.enumerate(request, emit);
        }
        let id = request.id;
        emit(DirectoryEvent::Batch {
            request_id: id,
            entries: vec![remote_entry("notes.txt", EntryKind::File)],
        });
        self.release.replace(Some(Box::new(move || {
            emit(DirectoryEvent::Batch {
                request_id: id,
                entries: vec![remote_entry("notes", EntryKind::Directory)],
            });
            emit(DirectoryEvent::Finished {
                request_id: id,
                truncated: false,
                can_trash: None,
                can_delete: None,
            });
        })));
        LoadHandle::new(|| {})
    }
}

#[test]
fn normal_filter_slash_waits_for_a_folder_still_listing() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_focus::filter_slash::normal_filter_slash_waits_for_a_folder_still_listing",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let share = Rc::new(StreamingShare::default());
                let source = share.clone();
                let fixture = KeyboardFixture::with_parts(Rc::new(TextPreview), || {
                    BrowserView::new(Rc::new(source), crate::ui::browser::PeekBehavior::default())
                });
                fixture.view.set_view_mode(mode);
                let browser = fixture.view.browser();
                browser.navigate(Location::uri(SHARE));
                wait_until(|| {
                    browser.active_location() == Some(Location::uri(SHARE))
                        && browser
                            .column_snapshot(0)
                            .is_some_and(|column| column.loading && column.count == 1)
                });
                assert!(fixture.view.show_filter(), "{mode:?}");
                focused_entry(&fixture.window).set_text("notes");
                type_slash(&fixture);
                pump(400);
                assert_eq!(
                    browser.active_location(),
                    Some(Location::uri(SHARE)),
                    "{mode:?}: a file match while listing cancelled or entered nothing"
                );
                let release = share.release.take().expect("held listing");
                release();
                wait_until_or(
                    || {
                        browser.active_location() == Some(Location::uri(format!("{SHARE}/notes")))
                            && fixture.view.filter_has_focus()
                    },
                    || {
                        format!(
                            "{mode:?}: location {:?}, filter focus {}",
                            browser.active_location(),
                            fixture.view.filter_has_focus()
                        )
                    },
                );
                assert_eq!(focused_entry(&fixture.window).text(), "");
            }
        },
    );
}
