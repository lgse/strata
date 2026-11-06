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

struct RemoteFolders;

impl FileSource for RemoteFolders {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        RemoteShare.validate_location(location)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let remote = request.location.native_path().is_none();
        RemoteShare.enumerate(
            request,
            Rc::new(move |event| {
                let event = match event {
                    DirectoryEvent::Batch {
                        request_id,
                        mut entries,
                    } if remote => {
                        for entry in &mut entries {
                            entry.kind = EntryKind::Directory;
                        }
                        DirectoryEvent::Batch {
                            request_id,
                            entries,
                        }
                    }
                    event => event,
                };
                emit(event);
            }),
        )
    }
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
                assert!(fixture.press(Key::slash, ModifierType::empty()));
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
                assert!(fixture.press(Key::slash, ModifierType::empty()));
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
