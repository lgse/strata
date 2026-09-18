// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn camera_device_order_appends_batches_and_keeps_selection_through_completion_and_refresh() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async test lock");
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::uri("gphoto2://camera/"));
    let (request_id, emit) = captured.borrow().clone().expect("initial Photos request");
    let entries = |names: &[&str]| {
        names
            .iter()
            .map(|name| FileEntry {
                location: Location::uri(format!("gphoto2://camera/202609_a/{name}")),
                ..batch_entry(name)
            })
            .collect()
    };
    emit(DirectoryEvent::Batch {
        request_id,
        entries: entries(&["z.jpg", "m.jpg"]),
    });
    browser.flush_coalesced_capped(None);
    browser.select(0, 1);
    emit(DirectoryEvent::Batch {
        request_id,
        entries: entries(&["b.jpg", "a.jpg"]),
    });
    browser.flush_coalesced_capped(None);
    assert_eq!(
        column_names(&browser, 0),
        ["z.jpg", "m.jpg", "b.jpg", "a.jpg"]
    );
    assert_eq!(browser.state.borrow().selected_positions(0), [1]);
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: Some(false),
        can_delete: Some(true),
    });
    browser.flush_coalesced_capped(None);
    assert_eq!(
        column_names(&browser, 0),
        ["z.jpg", "m.jpg", "b.jpg", "a.jpg"]
    );
    assert_eq!(browser.state.borrow().selected_positions(0), [1]);
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesReplaced { .. }))
    );
    let positions: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::EntriesInserted { insertions, .. } => Some(insertions),
            _ => None,
        })
        .flatten()
        .map(|insertion| insertion.position)
        .collect();
    assert_eq!(positions, [0, 2]);

    browser.retry_column(0);
    let (request_id, emit) = captured.borrow().clone().expect("refreshed Photos request");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: entries(&["z.jpg", "m.jpg", "b.jpg", "a.jpg"]),
    });
    browser.flush_coalesced_capped(None);
    assert_eq!(
        column_names(&browser, 0),
        ["z.jpg", "m.jpg", "b.jpg", "a.jpg"]
    );
    assert_eq!(browser.state.borrow().selected_positions(0), [1]);

    browser.set_sort_key(0, SortKey::Name);
    browser.set_sort_key(0, SortKey::DeviceOrder);
    let (request_id, emit) = captured.borrow().clone().expect("device-order reload");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: entries(&["z.jpg", "m.jpg", "b.jpg", "a.jpg"]),
    });
    browser.flush_coalesced_capped(None);
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(30)));
    assert_eq!(
        column_names(&browser, 0),
        ["z.jpg", "m.jpg", "b.jpg", "a.jpg"]
    );
    assert_eq!(
        browser
            .column_preferences(0)
            .expect("Photos preferences")
            .sort_key,
        SortKey::DeviceOrder
    );
}

#[test]
fn camera_explicit_sorts_work_and_device_order_can_be_restored_without_leaking_to_folders() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async test lock");
    for (key, expected) in [
        (SortKey::Name, ["a.jpg", "b.jpg", "z.jpg"]),
        (SortKey::Modified, ["b.jpg", "z.jpg", "a.jpg"]),
    ] {
        let captured: CapturedLoad = Rc::new(RefCell::new(None));
        let browser = Browser::new(Rc::new(BatchReplaySource {
            captured: captured.clone(),
        }));
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        browser.navigate(Location::uri("gphoto2://camera/"));
        let (request_id, emit) = captured.borrow().clone().expect("initial Photos request");
        let entries = || {
            [("z.jpg", 20), ("b.jpg", 10), ("a.jpg", 30)]
                .into_iter()
                .map(|(name, modified)| FileEntry {
                    location: Location::uri(format!("gphoto2://camera/202609_a/{name}")),
                    size: MetadataValue::Known(1),
                    modified_unix_seconds: MetadataValue::Known(modified),
                    ..batch_entry(name)
                })
                .collect()
        };
        emit(DirectoryEvent::Batch {
            request_id,
            entries: entries(),
        });
        browser.flush_coalesced_capped(None);
        browser.select(0, 0);
        browser.set_sort(0, key, SortDirection::Ascending);
        pump_until(|| finish_count(&events) == 1);
        assert_eq!(column_names(&browser, 0), expected);
        assert_eq!(
            browser.state.borrow().selected_entries()[0].display_name,
            "z.jpg"
        );
        browser.set_sort_key(0, SortKey::DeviceOrder);
        let (new_request, new_emit) = captured.borrow().clone().expect("device-order reload");
        assert_ne!(new_request, request_id);
        emit(DirectoryEvent::Batch {
            request_id,
            entries: vec![batch_entry("stale.jpg")],
        });
        new_emit(DirectoryEvent::Batch {
            request_id: new_request,
            entries: entries(),
        });
        browser.flush_coalesced_capped(None);
        assert_eq!(column_names(&browser, 0), ["z.jpg", "b.jpg", "a.jpg"]);
        assert_eq!(browser.state.borrow().selected_positions(0), [0]);
        assert_eq!(browser.preferences.get().sort_key, key);
        browser.navigate(Location::local("/fixture"));
        assert_eq!(
            browser
                .column_preferences(0)
                .expect("folder preferences")
                .sort_key,
            key
        );
        browser.set_sort_key(0, SortKey::DeviceOrder);
        assert_eq!(
            browser
                .column_preferences(0)
                .expect("folder preferences")
                .sort_key,
            key
        );
    }
}

struct CameraLoadSource(Rc<RefCell<Vec<DirectoryRequest>>>);

impl FileSource for CameraLoadSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        request: DirectoryRequest,
        _emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        self.0.borrow_mut().push(request);
        LoadHandle::new(|| {})
    }
}

#[test]
fn camera_browsing_and_refresh_request_complete_scans_but_peeks_remain_bounded() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let browser = Browser::new(Rc::new(CameraLoadSource(requests.clone())));
    let camera = Location::uri("gphoto2://camera/");
    for location in [
        Location::local("/fixture"),
        Location::uri("mtp://phone/"),
        Location::uri("afc://phone/"),
        Location::uri("gphoto2://camera/DCIM/"),
    ] {
        browser.navigate(location.clone());
        let request = requests.borrow_mut().pop().expect("directory load");
        assert_eq!(request.location, location);
        assert!(request.max_entries < usize::MAX);
        assert!(request.time_budget < Duration::MAX);
    }
    browser.begin_peek(0, camera.clone());
    let peek = requests.borrow_mut().pop().expect("camera peek");
    assert_eq!(peek.location, camera);
    assert!(peek.max_entries < usize::MAX);
    assert!(peek.time_budget < Duration::MAX);

    browser.navigate(camera.clone());
    browser.refresh_all();
    let requests = requests.borrow();
    assert_eq!(requests.len(), 2, "initial Photos load and refresh");
    for request in requests.iter() {
        assert_eq!(request.location, camera);
        assert_eq!(request.max_entries, usize::MAX);
        assert_eq!(request.time_budget, Duration::MAX);
    }
}

#[test]
fn deleting_a_nested_camera_file_removes_only_its_original_uri_from_the_flat_view() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let root = Location::uri("gphoto2://camera/");
    let first = FileEntry {
        location: Location::uri("gphoto2://camera/202401_a/IMG_0001.JPG"),
        ..batch_entry("IMG_0001.JPG")
    };
    let second = FileEntry {
        location: Location::uri("gphoto2://camera/202402_a/IMG_0001.JPG"),
        ..batch_entry("IMG_0001.JPG")
    };
    {
        let mut state = browser.state.borrow_mut();
        state.navigate(root, RequestId(1));
        let _ = state.apply_batch(RequestId(1), vec![first.clone(), second.clone()]);
    }
    browser.remove_deleted_locations(&[first.location]);
    assert_eq!(
        browser.column_snapshot(0).map(|column| column.count),
        Some(1)
    );
    assert_eq!(
        browser.entry_at(0, 0).map(|entry| entry.location),
        Some(second.location)
    );
}

#[test]
fn large_camera_deletions_rescan_each_open_library_only_once() {
    let requests = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(RecordingFileSource {
        request_count: requests.clone(),
    }));
    browser
        .state
        .borrow_mut()
        .navigate(Location::uri("gphoto2://camera/"), RequestId(1));
    let deleted: Vec<_> = (0..=MAX_INCREMENTAL_OPERATION_UPDATES)
        .map(|i| Location::uri(format!("gphoto2://camera/folder-{i}/image.jpg")))
        .collect();
    browser.remove_deleted_locations(&deleted);
    assert_eq!(requests.get(), 1);
    let parents: Vec<_> = deleted.iter().filter_map(Location::parent).collect();
    browser.refresh_columns_at_many(&parents);
    assert_eq!(requests.get(), 2);
}

#[test]
fn operations_in_camera_subfolders_refresh_the_library_but_not_other_devices() {
    for cancelled in [false, true] {
        let requests = Rc::new(Cell::new(0));
        let browser = Browser::new(Rc::new(RecordingFileSource {
            request_count: requests.clone(),
        }));
        browser
            .state
            .borrow_mut()
            .navigate(Location::uri("gphoto2://camera/"), RequestId(1));
        for (parent, expected) in [
            ("gphoto2://other/202401_a", 0),
            ("gphoto2://camera/202401_a", 1),
        ] {
            let parent = Location::uri(parent);
            if cancelled {
                browser.refresh_after_cancellation(&HashSet::from([parent]));
            } else {
                browser.refresh_columns_at(&parent);
            }
            assert_eq!(requests.get(), expected);
        }
    }
}
