// SPDX-License-Identifier: MIT

use super::*;

fn collect(root: &Path, max_entries: usize, time_budget: Duration) -> Vec<DirectoryEvent> {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let handle = enumerate(
        DirectoryRequest {
            id: RequestId(901),
            location: Location::uri(gio::File::for_path(root).uri()),
            batch_size: 1,
            include_metadata: true,
            max_entries,
            time_budget,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    context.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !events.borrow().iter().any(|event| {
            matches!(
                event,
                DirectoryEvent::Finished { .. } | DirectoryEvent::Failed { .. }
            )
        }) && Instant::now() < deadline
        {
            glib::timeout_future(Duration::from_millis(1)).await;
        }
    });
    drop(handle);
    Rc::try_unwrap(events)
        .expect("released event callback")
        .into_inner()
}

fn files(events: &[DirectoryEvent]) -> Vec<FileEntry> {
    events
        .iter()
        .flat_map(|event| match event {
            DirectoryEvent::Batch { entries, .. } => entries.clone(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn camera_media_filter_keeps_requested_formats_without_counting_sidecars() {
    let root = tempfile::tempdir().expect("camera tree");
    let included = [
        "photo.JPG",
        "photo.jpeg",
        "photo.HEIC",
        "photo.heif",
        "clip.MOV",
        "clip.mp4",
        "raw.DNG",
        "raw.CR3",
        "raw.nef",
        "raw.ARW",
    ];
    let excluded = [
        "photo.AAE",
        "photo.jpg.aae",
        "photo.PNG",
        "animated.GIF",
        "clip.MKV",
        "metadata.xml",
        "README",
        ".AAE",
    ];
    for name in included.into_iter().chain(excluded) {
        fs::write(root.path().join(name), b"unchanged original").expect("camera file");
    }
    let events = collect(root.path(), included.len(), Duration::from_secs(4));
    let entries = files(&events);
    let names: HashSet<_> = entries
        .iter()
        .map(|entry| entry.display_name.as_str())
        .collect();
    assert_eq!(names, included.into_iter().collect());
    assert!(matches!(
        events.last(),
        Some(DirectoryEvent::Finished {
            truncated: false,
            ..
        })
    ));
    for name in included.into_iter().chain(excluded) {
        assert_eq!(
            fs::read(root.path().join(name)).expect("original retained"),
            b"unchanged original"
        );
    }
}

#[test]
fn discovers_only_files_across_date_folders_without_collapsing_duplicate_names() {
    let root = tempfile::tempdir().expect("camera tree");
    for folder in ["202401_a", "202402_a/nested", "empty", ".private"] {
        fs::create_dir_all(root.path().join(folder)).expect("camera directory");
    }
    let paths = [
        "202401_a/IMG_0001.JPG",
        "202402_a/nested/IMG_0001.JPG",
        "clip.mov",
        ".private/hidden.JPG",
    ];
    for path in paths {
        fs::write(root.path().join(path), b"image").expect("camera file");
    }
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).expect("cycle symlink");
    std::os::unix::fs::symlink(root.path().join(paths[0]), root.path().join("image-link"))
        .expect("file symlink");
    let events = collect(root.path(), usize::MAX, Duration::MAX);
    assert!(matches!(
        events.last(),
        Some(DirectoryEvent::Finished {
            truncated: false,
            ..
        })
    ));
    let entries = files(&events);
    assert_eq!(entries.len(), paths.len());
    assert!(
        entries
            .iter()
            .all(|entry| entry.kind == EntryKind::File && entry.size == MetadataValue::Known(5))
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.native_name == "IMG_0001.JPG")
            .count(),
        2
    );
    let recent = Location::uri(gio::File::for_path(root.path().join(paths[1])).uri());
    let old = Location::uri(gio::File::for_path(root.path().join(paths[0])).uri());
    assert!(
        entries
            .iter()
            .position(|entry| entry.location == recent)
            .expect("recent photo")
            < entries
                .iter()
                .position(|entry| entry.location == old)
                .expect("older photo")
    );
    let identities: HashSet<_> = entries.iter().map(|entry| entry.location.clone()).collect();
    assert_eq!(identities.len(), paths.len());
    assert!(
        entries
            .iter()
            .find(|entry| entry.native_name == "hidden.JPG")
            .expect("hidden descendant")
            .is_hidden
    );
    for path in paths {
        assert!(identities.contains(&Location::uri(
            gio::File::for_path(root.path().join(path)).uri()
        )));
    }
}

#[test]
fn bounded_camera_peeks_keep_limits_but_complete_scans_reach_deep_files() {
    let root = tempfile::tempdir().expect("camera tree");
    fs::write(root.path().join("a.jpg"), b"a").expect("first image");
    fs::write(root.path().join("b.jpg"), b"b").expect("second image");
    let capped = collect(root.path(), 1, Duration::from_secs(4));
    assert_eq!(files(&capped).len(), 1);
    assert!(matches!(
        capped.last(),
        Some(DirectoryEvent::Finished {
            truncated: true,
            ..
        })
    ));
    let timed = collect(root.path(), 100, Duration::ZERO);
    assert!(matches!(
        timed.last(),
        Some(DirectoryEvent::Finished {
            truncated: true,
            ..
        })
    ));
    let mut deep = root.path().to_path_buf();
    for _ in 0..18 {
        deep = deep.join("nested");
    }
    fs::create_dir_all(&deep).expect("deep camera tree");
    fs::write(deep.join("deep.jpg"), b"c").expect("deep image");
    let complete = collect(root.path(), usize::MAX, Duration::MAX);
    let entries = files(&complete);
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().any(|entry| entry.native_name == "deep.jpg"));
    assert!(matches!(
        complete.last(),
        Some(DirectoryEvent::Finished {
            truncated: false,
            ..
        })
    ));
}

#[test]
fn camera_scans_finish_all_folders_beyond_the_previous_directory_limit() {
    let root = tempfile::tempdir().expect("camera tree");
    let folder_count = 4_100;
    for i in 0..folder_count {
        let folder = root.path().join(format!("folder-{i}"));
        fs::create_dir(&folder).expect("camera folder");
        fs::write(folder.join("photo.jpg"), b"photo").expect("camera photo");
    }
    let events = collect(root.path(), usize::MAX, Duration::MAX);
    assert!(matches!(
        events.last(),
        Some(DirectoryEvent::Finished {
            truncated: false,
            ..
        })
    ));
    let entries = files(&events);
    assert_eq!(entries.len(), folder_count);
    let identities: HashSet<_> = entries.iter().map(|entry| &entry.location).collect();
    assert_eq!(identities.len(), folder_count);
}

#[test]
fn inaccessible_camera_roots_report_failure_instead_of_an_empty_library() {
    let root = tempfile::tempdir().expect("camera tree");
    let events = collect(&root.path().join("disconnected"), usize::MAX, Duration::MAX);
    assert!(matches!(events.last(), Some(DirectoryEvent::Failed { .. })));
    assert!(files(&events).is_empty());
}

#[test]
fn cancellation_after_first_batch_stops_recursive_discovery() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let root = tempfile::tempdir().expect("camera tree");
    for i in 0..20 {
        fs::write(root.path().join(format!("{i}.jpg")), b"image").expect("photo");
    }
    let handle = Rc::new(RefCell::new(None::<LoadHandle>));
    let cancel = handle.clone();
    let count = Rc::new(Cell::new(0));
    let emitted = count.clone();
    handle.replace(Some(enumerate(
        DirectoryRequest {
            id: RequestId(902),
            location: Location::uri(gio::File::for_path(root.path()).uri()),
            batch_size: 1,
            include_metadata: false,
            max_entries: usize::MAX,
            time_budget: Duration::MAX,
        },
        Rc::new(move |event| {
            assert!(matches!(event, DirectoryEvent::Batch { .. }));
            emitted.set(emitted.get() + 1);
            cancel.borrow_mut().take();
        }),
    )));
    context.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(4);
        while count.get() == 0 && Instant::now() < deadline {
            glib::timeout_future(Duration::from_millis(1)).await;
        }
        glib::timeout_future(Duration::from_millis(20)).await;
    });
    assert_eq!(count.get(), 1);
}
