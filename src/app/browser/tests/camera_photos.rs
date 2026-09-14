// SPDX-License-Identifier: MIT

use super::*;

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
