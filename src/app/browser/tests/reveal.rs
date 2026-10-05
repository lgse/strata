// SPDX-License-Identifier: MIT

use std::os::unix::ffi::OsStringExt;

use super::{
    location_input::{listing_file, selected_locations},
    *,
};

/// Lists entries per directory; `unmounted` reports itself not mounted.
#[derive(Default)]
struct ListingSource {
    listings: RefCell<HashMap<Location, Vec<FileEntry>>>,
    unmounted: Option<Location>,
}

impl FileSource for ListingSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if self.unmounted.as_ref() == Some(location) {
            return Err(LocationValidationError::NotMounted(location.clone()));
        }
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if let Some(entries) = self.listings.borrow().get(&request.location).cloned() {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries,
            });
        }
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

fn file(path: &str) -> FileEntry {
    let location = location(path);
    let name = path.rsplit('/').next().expect("file name").to_owned();
    listing_file(location, &name, false)
}

fn location(path: &str) -> Location {
    if path.contains("://") {
        Location::uri(path)
    } else {
        Location::local(path)
    }
}

fn listings(listings: Vec<(&str, Vec<FileEntry>)>) -> ListingSource {
    ListingSource {
        listings: RefCell::new(
            listings
                .into_iter()
                .map(|(directory, entries)| (location(directory), entries))
                .collect(),
        ),
        ..ListingSource::default()
    }
}

fn reveal_browser(
    source: ListingSource,
) -> (
    Rc<Browser>,
    Rc<RefCell<Vec<BrowserEvent>>>,
    Rc<ListingSource>,
) {
    let source = Rc::new(source);
    let browser = Browser::new(source.clone());
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    (browser, events, source)
}

#[test]
fn reveal_locations_matches_by_location_not_display_name() {
    let colliding = |bytes: &[u8]| {
        let name = OsString::from_vec(bytes.to_vec());
        let mut entry = listing_file(
            Location::local(Path::new("/fixture").join(&name)),
            "bad\u{FFFD}name.txt",
            false,
        );
        entry.native_name = name;
        entry
    };
    let requested = colliding(b"bad\xe9name.txt");
    let (browser, ..) = reveal_browser(listings(vec![(
        "/fixture",
        vec![colliding(b"bad\xe8name.txt"), requested.clone()],
    )]));
    browser.navigate(location("/start"));

    browser.reveal_locations(location("/fixture"), vec![requested.location.clone()]);

    assert_eq!(browser.active_location(), Some(location("/fixture")));
    assert_eq!(selected_locations(&browser), vec![requested.location]);
    assert_eq!(
        browser.focused_entry().map(|entry| entry.native_name),
        Some(requested.native_name)
    );
}

#[test]
fn reveal_locations_selects_every_listed_target_and_focuses_the_first_requested() {
    let (browser, events, _) = reveal_browser(listings(vec![(
        "/fixture",
        vec![
            file("/fixture/a.txt"),
            file("/fixture/b.txt"),
            file("/fixture/c.txt"),
        ],
    )]));
    browser.navigate(location("/start"));

    browser.reveal_locations(
        location("/fixture"),
        vec![location("/fixture/c.txt"), location("/fixture/a.txt")],
    );

    assert_eq!(
        selected_locations(&browser),
        vec![location("/fixture/a.txt"), location("/fixture/c.txt")]
    );
    assert_eq!(
        browser.focused_entry().map(|entry| entry.location),
        Some(location("/fixture/c.txt"))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::SelectionSetChanged {
            focused: 2,
            take_focus: true,
            ..
        }
    )));
}

#[test]
fn reveal_locations_into_an_open_parent_column_selects_in_place_and_closes_deeper_columns() {
    let (browser, events, _) = reveal_browser(listings(vec![
        (
            "/fixture",
            vec![
                file("/fixture/a.txt"),
                file("/fixture/b.txt"),
                file("/fixture/sub"),
            ],
        ),
        ("/fixture/sub", vec![file("/fixture/sub/x")]),
    ]));
    browser.navigate(location("/fixture"));
    browser.descend(0, location("/fixture/sub"));
    assert_eq!(browser.active_depth(), Some(1));
    events.borrow_mut().clear();

    let selected_in_place = browser.reveal_locations(
        location("/fixture"),
        vec![location("/fixture/b.txt"), location("/fixture/a.txt")],
    );

    assert!(selected_in_place);
    assert_eq!(browser.active_depth(), Some(0));
    assert_eq!(browser.location_at(1), None);
    assert_eq!(
        selected_locations(&browser),
        vec![location("/fixture/a.txt"), location("/fixture/b.txt")]
    );
    assert_eq!(
        browser.focused_entry().map(|entry| entry.location),
        Some(location("/fixture/b.txt"))
    );
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::Reset | BrowserEvent::ColumnAdded { .. }
    )));
}

#[test]
fn reveal_locations_reloads_an_open_directory_that_lists_only_some_targets() {
    let (browser, _, source) =
        reveal_browser(listings(vec![("/fixture", vec![file("/fixture/a.txt")])]));
    browser.navigate(location("/fixture"));
    source.listings.borrow_mut().insert(
        location("/fixture"),
        vec![file("/fixture/a.txt"), file("/fixture/copy.txt")],
    );

    let selected_in_place = browser.reveal_locations(
        location("/fixture"),
        vec![location("/fixture/copy.txt"), location("/fixture/a.txt")],
    );

    assert!(!selected_in_place);
    assert_eq!(
        selected_locations(&browser),
        vec![location("/fixture/a.txt"), location("/fixture/copy.txt")]
    );
}

#[test]
fn reveal_locations_selects_the_listed_targets_without_reporting_the_missing_one() {
    let (browser, events, _) =
        reveal_browser(listings(vec![("/fixture", vec![file("/fixture/a.txt")])]));
    browser.navigate(location("/start"));

    browser.reveal_locations(
        location("/fixture"),
        vec![location("/fixture/missing.txt"), location("/fixture/a.txt")],
    );

    assert_eq!(
        selected_locations(&browser),
        vec![location("/fixture/a.txt")]
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LocationRevealFailed { .. }))
    );
}

#[test]
fn reveal_locations_survives_a_mount_round_trip() {
    let share = "sftp://host/share";
    let (browser, events, _) = reveal_browser(ListingSource {
        unmounted: Some(location(share)),
        ..listings(vec![(share, vec![file("sftp://host/share/report.pdf")])])
    });

    browser.reveal_locations(
        location(share),
        vec![location("sftp://host/share/report.pdf")],
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LocationNavigationRejected {
            error: LocationValidationError::NotMounted(_)
        }
    )));
    assert_eq!(browser.active_location(), None);

    // What the UI mount flow does once the share is mounted.
    browser.navigate(location(share));

    assert_eq!(
        selected_locations(&browser),
        vec![location("sftp://host/share/report.pdf")]
    );
    assert!(browser.deferred_reveal.borrow().is_none());
}

#[test]
fn descend_revealing_selects_the_target_in_a_new_column_after_the_parent() {
    for root in ["/fixture", "sftp://host/fixture"] {
        let (browser, ..) = reveal_browser(listings(vec![
            (root, vec![file(&format!("{root}/sub"))]),
            (
                &format!("{root}/sub"),
                vec![
                    file(&format!("{root}/sub/w")),
                    file(&format!("{root}/sub/x")),
                ],
            ),
        ]));
        browser.navigate(location(root));

        browser.descend_revealing(
            0,
            location(&format!("{root}/sub")),
            vec![location(&format!("{root}/sub/x"))],
        );

        assert_eq!(browser.location_at(0), Some(location(root)));
        assert_eq!(
            browser.location_at(1),
            Some(location(&format!("{root}/sub")))
        );
        assert_eq!(
            selected_locations(&browser),
            vec![location(&format!("{root}/sub/x"))]
        );
    }
}

#[test]
fn moving_the_cursor_ends_a_reveal_still_waiting_for_remote_batches() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let mut source = ScriptedSource::manual(vec![], vec![]);
    source.uri_base = Some("sftp://host/share");
    let (browser, events, source) = scripted_browser(source);
    let [a, b, c] = ["a", "b", "c"].map(|name| source.entry_location(name));
    browser.reveal_locations(
        Location::uri("sftp://host/share"),
        vec![c.clone(), a.clone()],
    );
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![
            source.listed_entry("a", false),
            source.listed_entry("b", false),
        ],
    });
    assert_eq!(selected_locations(&browser), vec![a]);

    browser.place_cursor(0, 1, None);
    events.borrow_mut().clear();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![source.listed_entry("c", false)],
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });

    assert_eq!(browser.focused_entry().map(|entry| entry.location), Some(b));
    assert!(!selected_locations(&browser).contains(&c));
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::SelectionSetChanged {
            take_focus: true,
            ..
        }
    )));
}
