// SPDX-License-Identifier: MIT

mod browse;
mod trash;

use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::{ErrorKind, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{OpenOptionsExt, PermissionsExt},
    },
    process::Command,
    time::{Instant, SystemTime},
};

use super::*;
use crate::{
    model::{Location, MetadataValue},
    test_support::{ASYNC_MAIN_CONTEXT_DEFAULT, Gate, capture_logs},
};

fn capture_directory_start_logs(locations: &[(RequestId, &Location)]) -> String {
    capture_logs(|| {
        for (request_id, location) in locations {
            log_directory_load_started(*request_id, location);
        }
    })
}

fn captured_event<'a>(output: &'a str, request_id: RequestId, message: &str) -> &'a str {
    let request_id = format!("request_id={}", request_id.0);
    output
        .lines()
        .find(|line| line.contains(message) && line.contains(&request_id))
        .unwrap_or_else(|| panic!("missing {message:?} event for {request_id}"))
}

#[test]
fn directory_logging_respects_default_and_diagnostic_privacy() {
    let native_path = "/home/alice/sentinel-private-directory";
    let native = Location::local(native_path);
    let remote = Location::uri(
        "sftp://alice:password;key=secret@example.com/private?token=secret#private-fragment",
    );
    let output =
        capture_directory_start_logs(&[(RequestId(42), &native), (RequestId(43), &remote)]);

    let native_default = captured_event(&output, RequestId(42), "directory load started");
    assert_eq!(native_default.split_whitespace().next(), Some("INFO"));
    assert!(native_default.contains("backend=native"));
    assert!(!native_default.contains(native_path));

    let native_diagnostic = captured_event(&output, RequestId(42), "directory load location");
    assert_eq!(native_diagnostic.split_whitespace().next(), Some("DEBUG"));
    assert!(native_diagnostic.contains(native_path));

    let remote_default = captured_event(&output, RequestId(43), "directory load started");
    assert_eq!(remote_default.split_whitespace().next(), Some("INFO"));
    assert!(remote_default.contains("backend=sftp"));
    assert!(!remote_default.contains("example.com"));

    let remote_diagnostic = captured_event(&output, RequestId(43), "directory load location");
    assert_eq!(remote_diagnostic.split_whitespace().next(), Some("DEBUG"));
    assert!(remote_diagnostic.contains("sftp://example.com/private"));
    for secret in [
        "alice",
        "password",
        "key=secret",
        "token=secret",
        "private-fragment",
    ] {
        assert!(!remote_diagnostic.contains(secret));
    }
}

#[test]
fn remote_backend_failures_hide_uri_user_info_in_views_and_diagnostic_logs() {
    let location = Location::uri("sftp://alice@host.example/private");
    let error = glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Unable to read sftp://alice:secret@host.example/private?token=hidden#fragment",
    );
    let validation = uri_validation_result(&location, Err(error.clone()));
    let Err(LocationValidationError::Unavailable(validation_message)) = validation else {
        panic!("remote validation should report a sanitized failure");
    };
    let DirectoryEvent::Failed { message, .. } = remote_directory_failure(RequestId(42), &error)
    else {
        panic!("remote enumeration should report a sanitized failure");
    };
    let diagnostic = capture_logs(|| log_monitor_metadata_error(&location, &error));

    assert_eq!(validation_message, crate::services::UNEXPECTED_SYSTEM_ERROR);
    assert_eq!(message, crate::services::UNEXPECTED_SYSTEM_ERROR);
    assert!(diagnostic.contains("sftp://host.example/private"));
    for text in [&validation_message, &message, &diagnostic] {
        for secret in ["alice", "secret", "token", "hidden", "fragment"] {
            assert!(!text.contains(secret), "{text}");
        }
    }
}

#[test]
fn a_refused_admin_prompt_is_not_authorized_while_other_denials_stay_errors() {
    let denied = || glib::Error::new(gio::IOErrorEnum::PermissionDenied, "Permission denied");

    assert_eq!(
        uri_validation_result(&Location::uri("admin:///root"), Err(denied())),
        Err(LocationValidationError::NotAuthorized)
    );
    assert!(matches!(
        uri_validation_result(&Location::uri("sftp://host/root"), Err(denied())),
        Err(LocationValidationError::Unavailable(_))
    ));
}

#[test]
fn validation_accepts_readable_directories_and_rejects_files_and_missing_paths()
-> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-location-test-{unique}"));
    let file = directory.join("file.txt");
    let missing = directory.join("missing");
    fs::create_dir(&directory)?;
    fs::write(&file, b"fixture")?;

    let source = LocalFileSource;
    assert_eq!(
        source.validate_location(&Location::local(&directory)),
        Ok(())
    );
    assert_eq!(
        source.validate_location(&Location::local(&file)),
        Err(LocationValidationError::NotDirectory)
    );
    assert_eq!(
        source.validate_location(&Location::local(file.join("child"))),
        Err(LocationValidationError::NotDirectory)
    );
    assert_eq!(
        source.validate_location(&Location::local(&missing)),
        Err(LocationValidationError::Missing)
    );

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn invalid_utf8_names_keep_their_native_bytes() -> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-native-name-test-{unique}"));
    fs::create_dir(&directory)?;
    let native_name = OsString::from_vec(b"invalid-\xff".to_vec());
    let path = directory.join(&native_name);
    fs::write(&path, b"fixture")?;

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&directory),
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    let entry = batched_entries(&events)
        .into_iter()
        .next()
        .expect("the invalid UTF-8 entry should be listed");

    assert_eq!(entry.native_name.as_bytes(), native_name.as_bytes());
    assert_eq!(entry.location.native_path(), Some(path.as_path()));
    assert!(!entry.display_name.is_empty());

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn missing_optional_attributes_use_safe_defaults() {
    let info = gio::FileInfo::new();

    assert!(!info_is_hidden(&info));
    assert!(!info_is_symlink(&info));
    assert_eq!(info_mode(&info), MetadataValue::Unavailable);

    info.set_is_hidden(true);
    info.set_is_symlink(true);

    assert!(info_is_hidden(&info));
    assert!(info_is_symlink(&info));

    info.set_attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE, 0);
    assert_eq!(info_mode(&info), MetadataValue::Known(0));
}

#[test]
fn symlink_targets_and_broken_links_are_distinguished() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-symlink-test-{unique}"));
    fs::create_dir(&directory)?;
    fs::create_dir(directory.join("directory"))?;
    fs::write(directory.join("file"), b"fixture")?;
    symlink("directory", directory.join("directory-link"))?;
    symlink("file", directory.join("file-link"))?;
    symlink("missing", directory.join("broken-link"))?;

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&directory),
        batch_size: 64,
        include_metadata: true,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    let entries = batched_entries(&events);
    let kind = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.native_name == name)
            .map(|entry| entry.kind)
            .expect("the symlink should be listed")
    };

    assert_eq!(kind("directory-link"), EntryKind::DirectorySymbolicLink);
    assert_eq!(kind("file-link"), EntryKind::FileSymbolicLink);
    assert_eq!(kind("broken-link"), EntryKind::SymbolicLink);

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn coalescing_preserves_a_move_when_metadata_follows_it() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: Location::local("/fixture/old"),
            to: Location::local("/fixture/new"),
        },
        PendingMonitorChange::Upsert(Location::local("/fixture/new")),
    );

    assert!(matches!(change, PendingMonitorChange::Move { .. }));
}

#[test]
fn hidden_monitor_changes_are_skipped_but_atomic_publication_stays_visible() {
    let watched = Location::local("/fixture");
    let hidden = Location::local("/fixture/.strata-replacement-123");
    let visible = Location::local("/fixture/report.pdf");

    assert!(
        visible_monitor_change(
            &watched,
            PendingMonitorChange::Upsert(hidden.clone()),
            false
        )
        .is_none()
    );
    assert!(matches!(
        visible_monitor_change(
            &watched,
            PendingMonitorChange::Move {
                from: hidden,
                to: visible.clone(),
            },
            false,
        ),
        Some(PendingMonitorChange::Upsert(location)) if location == visible
    ));
    assert!(
        visible_monitor_change(
            &watched,
            PendingMonitorChange::Upsert(Location::local("/fixture/.env")),
            true,
        )
        .is_some()
    );
    let hidden_folder = Location::local("/fixture/.config");
    assert!(matches!(
        visible_monitor_change(
            &hidden_folder,
            PendingMonitorChange::Remove(hidden_folder.clone()),
            false,
        ),
        Some(PendingMonitorChange::Remove(location)) if location == hidden_folder
    ));
}

#[test]
fn recent_monitor_events_rescan_without_querying_virtual_children() {
    let watched = Location::uri("recent:///");
    let child = Location::uri("recent:///virtual-child");
    let events = [
        gio::FileMonitorEvent::Created,
        gio::FileMonitorEvent::Changed,
        gio::FileMonitorEvent::Deleted,
        gio::FileMonitorEvent::Moved,
        gio::FileMonitorEvent::Renamed,
        gio::FileMonitorEvent::AttributeChanged,
        gio::FileMonitorEvent::ChangesDoneHint,
        gio::FileMonitorEvent::PreUnmount,
        gio::FileMonitorEvent::Unmounted,
    ];

    let mut pending = HashMap::new();
    for (index, event) in events.into_iter().enumerate() {
        let change = pending_monitor_change(
            &watched,
            Some(child.clone()),
            Some(Location::uri(format!("recent:///other-{index}"))),
            event,
        )
        .expect("Recent monitor activity should invalidate the collection");
        assert!(matches!(change, PendingMonitorChange::Rescan));
        assert_eq!(queue_monitor_change(&mut pending, None, change), index == 0);
    }

    let notified: Rc<RefCell<Vec<DirectoryChange>>> = Rc::new(RefCell::new(Vec::new()));
    let collected = notified.clone();
    let notify: Rc<dyn Fn(DirectoryChange)> =
        Rc::new(move |change| collected.borrow_mut().push(change));
    flush_monitor_changes(&RefCell::new(pending), &notify, &Rc::new(Cell::new(false)));

    assert!(matches!(
        notified.borrow().as_slice(),
        [DirectoryChange::Rescan]
    ));
}

#[test]
fn metadata_update_followed_by_move_preserves_source_removal() {
    let mut pending = HashMap::new();
    let temp = Location::local("/fixture/file.tmp");
    let dest = Location::local("/fixture/file");
    assert!(queue_monitor_change(
        &mut pending,
        Some(temp.clone()),
        PendingMonitorChange::Upsert(temp),
    ));
    assert!(queue_monitor_change(
        &mut pending,
        Some(dest.clone()),
        PendingMonitorChange::Move {
            from: Location::local("/fixture/file.tmp"),
            to: dest.clone(),
        },
    ));
    assert_eq!(pending.len(), 1);
    assert!(matches!(
        pending.get(&Some(dest)),
        Some(PendingMonitorChange::Move { from, .. })
            if *from == Location::local("/fixture/file.tmp")
    ));
}

#[test]
fn move_does_not_discard_a_pending_source_removal() {
    let mut pending = HashMap::new();
    let from = Location::local("/fixture/source");
    let to = Location::local("/fixture/destination");
    queue_monitor_change(
        &mut pending,
        Some(from.clone()),
        PendingMonitorChange::Remove(from.clone()),
    );
    queue_monitor_change(
        &mut pending,
        Some(to.clone()),
        PendingMonitorChange::Move {
            from: from.clone(),
            to,
        },
    );
    assert!(matches!(
        pending.get(&Some(from)),
        Some(PendingMonitorChange::Remove(_))
    ));
    assert_eq!(pending.len(), 2);
}

#[test]
fn large_monitor_bursts_collapse_to_one_rescan() {
    let mut pending = HashMap::new();
    for index in 0..=MAX_PENDING_MONITOR_CHANGES {
        let location = Location::local(format!("/fixture/{index}"));
        assert!(queue_monitor_change(
            &mut pending,
            Some(location.clone()),
            PendingMonitorChange::Upsert(location),
        ));
    }

    assert_eq!(pending.len(), 1);
    assert!(matches!(
        pending.get(&None),
        Some(PendingMonitorChange::Rescan)
    ));
    assert!(!queue_monitor_change(
        &mut pending,
        Some(Location::local("/fixture/ignored")),
        PendingMonitorChange::Remove(Location::local("/fixture/ignored")),
    ));
}

#[test]
fn conflicting_move_events_fall_back_to_a_rescan() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    for recreate_source in [false, true] {
        let from = Location::local("/fixture/old");
        let to = Location::local("/fixture/new");
        let mut pending = HashMap::new();
        queue_monitor_change(
            &mut pending,
            Some(to.clone()),
            PendingMonitorChange::Move {
                from: from.clone(),
                to: to.clone(),
            },
        );
        if recreate_source {
            queue_monitor_change(
                &mut pending,
                Some(from.clone()),
                PendingMonitorChange::Upsert(from),
            );
        } else {
            queue_monitor_change(
                &mut pending,
                Some(to.clone()),
                PendingMonitorChange::Remove(to),
            );
        }
        let changes = Rc::new(RefCell::new(Vec::new()));
        let observed = changes.clone();
        let notify: Rc<dyn Fn(DirectoryChange)> =
            Rc::new(move |change| observed.borrow_mut().push(change));

        flush_monitor_changes(&RefCell::new(pending), &notify, &Rc::new(Cell::new(false)));

        assert!(
            matches!(changes.borrow().as_slice(), [DirectoryChange::Rescan]),
            "recreate source: {recreate_source}"
        );
    }
}

#[test]
fn uri_monitor_changes_keep_their_uri_locations() {
    let mut pending = HashMap::new();
    let trashed = Location::uri("trash:///report.txt");
    assert!(queue_monitor_change(
        &mut pending,
        Some(trashed.clone()),
        PendingMonitorChange::Remove(trashed.clone()),
    ));

    let notified: Rc<RefCell<Vec<DirectoryChange>>> = Rc::new(RefCell::new(Vec::new()));
    let collected = notified.clone();
    let notify: Rc<dyn Fn(DirectoryChange)> =
        Rc::new(move |change| collected.borrow_mut().push(change));
    flush_monitor_changes(&RefCell::new(pending), &notify, &Rc::new(Cell::new(false)));

    let changes = notified.borrow();
    assert!(
        matches!(changes.as_slice(), [DirectoryChange::Remove(location)] if location == &trashed),
        "the trash URI should survive the flush unchanged"
    );
}

#[test]
fn permission_errors_are_reported_as_inaccessible() {
    let error = std::io::Error::from(ErrorKind::PermissionDenied);
    assert_eq!(
        LocationValidationError::from(error),
        LocationValidationError::Inaccessible
    );
}

#[test]
fn a_directory_reporting_changes_against_itself_is_not_its_own_child() {
    let watched = Location::uri("trash:///");
    let child = Location::uri("trash:///report.txt");

    assert_eq!(
        monitored_change_target(
            &watched,
            Some(watched.clone()),
            gio::FileMonitorEvent::Changed
        ),
        None
    );
    assert_eq!(
        monitored_change_target(
            &watched,
            Some(watched.clone()),
            gio::FileMonitorEvent::Deleted
        ),
        Some(watched.clone())
    );
    assert_eq!(
        monitored_change_target(
            &watched,
            Some(child.clone()),
            gio::FileMonitorEvent::Created
        ),
        Some(child)
    );
}

fn live_watch(location: Location, notify: Rc<dyn Fn(DirectoryChange)>) -> LoadHandle {
    let reported = Rc::new(Cell::new(None));
    let report = reported.clone();
    let handle = LocalFileSource.start_watch(
        location,
        false,
        notify,
        Rc::new(move |watching| report.set(Some(watching))),
    );
    drain_until(|| reported.get().is_some());
    assert_eq!(reported.get(), Some(Watching::Live));
    handle
}

#[test]
fn exchanged_files_stay_listed_unless_they_are_deleted() -> Result<(), Box<dyn Error>> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for deleted in [true, false] {
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("note.txt");
        let hidden = directory.path().join(".strata-originals-test");
        fs::create_dir(&hidden)?;
        let staged = hidden.join("note.txt");
        fs::write(&original, b"old")?;
        fs::write(&staged, b"replacement")?;
        let changes: Rc<RefCell<Vec<DirectoryChange>>> = Rc::default();
        let collected = changes.clone();
        let handle = live_watch(
            Location::local(directory.path()),
            Rc::new(move |change| collected.borrow_mut().push(change)),
        );

        renameat_with(CWD, &staged, CWD, &original, RenameFlags::EXCHANGE)?;
        assert_eq!(fs::read(&original)?, b"replacement");
        if deleted {
            fs::remove_file(&original)?;
        }
        drain_until(|| !changes.borrow().is_empty());
        drop(handle);

        let location = Location::local(&original);
        let observed = changes.borrow();
        if deleted {
            assert!(
                matches!(observed.as_slice(), [DirectoryChange::Remove(path)] if path == &location),
                "a genuine deletion must remove the row: {observed:?}"
            );
        } else {
            assert!(
                matches!(observed.as_slice(), [DirectoryChange::Upsert(entry)]
                    if entry.location == location && entry.size == MetadataValue::Known(11)),
                "the replacement must stay listed with its new metadata: {observed:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn change_events_for_unnamed_temporary_files_are_ignored() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let directory = unique_fixture_root("monitor-unnamed");
    fs::create_dir_all(&directory)?;
    let listed = directory.join("#1234");
    let arrived = directory.join("#5678");
    fs::write(&listed, b"a")?;
    let changes: Rc<RefCell<Vec<DirectoryChange>>> = Rc::default();
    let collected = changes.clone();
    let handle = live_watch(
        Location::local(&directory),
        Rc::new(move |change| collected.borrow_mut().push(change)),
    );
    let write_unnamed = || -> Result<Location, Box<dyn Error>> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_TMPFILE)
            .open(&directory)?;
        file.write_all(b"never linked")?;
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        Ok(Location::local(
            directory.join(format!("#{}", file.metadata()?.ino())),
        ))
    };
    let upserted = |path: &Path, size: u64| {
        let location = Location::local(path);
        let changes = changes.clone();
        move || {
            changes.borrow().iter().any(|change| {
                matches!(change, DirectoryChange::Upsert(entry)
                    if entry.location == location && entry.size == MetadataValue::Known(size))
            })
        }
    };

    let mut unnamed = vec![write_unnamed()?];
    fs::write(&listed, b"updated")?;
    drain_until(upserted(&listed, 7));
    unnamed.push(write_unnamed()?);
    fs::write(&arrived, b"")?;
    drain_until(upserted(&arrived, 0));
    fs::write(&arrived, b"grown")?;
    drain_until(upserted(&arrived, 5));
    let settled = Instant::now() + Duration::from_millis(300);
    drain_until(|| Instant::now() >= settled);
    drop(handle);
    fs::remove_dir_all(&directory)?;

    let changes = changes.borrow();
    assert!(
        changes.iter().all(|change| match change {
            DirectoryChange::Upsert(entry) => !unnamed.contains(&entry.location),
            DirectoryChange::Remove(location) => !unnamed.contains(location),
            DirectoryChange::Move { .. } | DirectoryChange::Rescan => false,
        }),
        "only the named files should be queried: {changes:?}"
    );
    Ok(())
}

#[test]
fn a_file_linked_into_place_is_queried_once() -> Result<(), Box<dyn Error>> {
    use std::os::fd::AsRawFd;

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let directory = unique_fixture_root("monitor-linked");
    fs::create_dir_all(&directory)?;
    let changes: Rc<RefCell<Vec<DirectoryChange>>> = Rc::default();
    let collected = changes.clone();
    let handle = live_watch(
        Location::local(&directory),
        Rc::new(move |change| collected.borrow_mut().push(change)),
    );
    let linked = Location::local(directory.join("linked.txt"));
    let queries = || {
        changes
            .borrow()
            .iter()
            .filter(|change| matches!(change, DirectoryChange::Upsert(entry) if entry.location == linked))
            .count()
    };

    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_TMPFILE)
        .open(&directory)?;
    file.write_all(b"contents")?;
    rustix::fs::linkat(
        rustix::fs::CWD,
        format!("/proc/self/fd/{}", file.as_raw_fd()),
        rustix::fs::CWD,
        directory.join("linked.txt"),
        rustix::fs::AtFlags::SYMLINK_FOLLOW,
    )?;
    drop(file);
    drain_until(|| queries() > 0);
    // GLib sends the linked name's changes-done hint two seconds later.
    let settled = Instant::now() + Duration::from_millis(2_500);
    drain_until(|| Instant::now() >= settled);
    drop(handle);
    fs::remove_dir_all(&directory)?;

    assert_eq!(queries(), 1, "{:?}", changes.borrow());
    Ok(())
}

#[test]
fn a_dangling_link_arriving_in_a_watched_folder_is_reported_as_a_link() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let directory = unique_fixture_root("monitor-dangling-link");
    fs::create_dir_all(&directory)?;
    let changes: Rc<RefCell<Vec<DirectoryChange>>> = Rc::default();
    let collected = changes.clone();
    let handle = live_watch(
        Location::local(&directory),
        Rc::new(move |change| collected.borrow_mut().push(change)),
    );
    let link = Location::local(directory.join("broken-link.txt"));

    std::os::unix::fs::symlink(
        directory.join("missing.txt"),
        directory.join("broken-link.txt"),
    )?;
    drain_until(|| {
        changes.borrow().iter().any(|change| {
            matches!(change, DirectoryChange::Upsert(entry)
                if entry.location == link && entry.kind == EntryKind::SymbolicLink)
        })
    });
    drop(handle);
    fs::remove_dir_all(&directory)?;
    Ok(())
}

#[test]
fn sustained_native_monitor_bursts_flush_without_rescanning() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let directory = unique_fixture_root("monitor-burst");
    fs::create_dir_all(&directory).expect("the fixture directory should be created");
    let changes: Rc<RefCell<Vec<DirectoryChange>>> = Rc::new(RefCell::new(Vec::new()));
    let collected = changes.clone();
    let handle = live_watch(
        Location::local(&directory),
        Rc::new(move |change| collected.borrow_mut().push(change)),
    );

    for index in 0..1_000 {
        fs::write(
            directory.join(format!("file-{index:04}.txt")),
            b"real contents",
        )
        .expect("the fixture file should be written");
    }

    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        while context.iteration(false) {}
        let observed = changes.borrow();
        assert!(
            observed
                .iter()
                .all(|change| !matches!(change, DirectoryChange::Rescan)),
            "a bounded native burst should stay incremental"
        );
        if observed
            .iter()
            .filter(|change| matches!(change, DirectoryChange::Upsert(_)))
            .count()
            == 1_000
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "all monitored entries should arrive"
        );
        drop(observed);
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(handle);
    fs::remove_dir_all(&directory).expect("the fixture directory should be removed");
}

#[test]
fn watching_a_uri_location_reports_created_entries() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let directory = unique_fixture_root("uri-watch");
    fs::create_dir_all(&directory).expect("the fixture directory should be created");
    let uri = glib::filename_to_uri(&directory, None).expect("the fixture path should have a URI");

    let changes: Rc<RefCell<Vec<DirectoryChange>>> = Rc::new(RefCell::new(Vec::new()));
    let collected = changes.clone();
    let handle = live_watch(
        Location::uri(uri.as_str()),
        Rc::new(move |change| collected.borrow_mut().push(change)),
    );

    fs::write(directory.join("arrival.txt"), b"arrived")
        .expect("the fixture file should be written");

    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while changes.borrow().is_empty() && Instant::now() < deadline {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(handle);
    let observed = changes.borrow().clone();
    fs::remove_dir_all(&directory).expect("the fixture directory should be removed");

    assert!(
        observed.iter().any(|change| matches!(
            change,
            DirectoryChange::Upsert(entry) if entry.native_name == "arrival.txt"
        )),
        "the monitor should report the new entry: {observed:?}"
    );
}

fn unique_fixture_root(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-local-files-{label}-{unique}"))
}

struct RecentFixtureSource {
    entries: RefCell<VecDeque<RecentEntryResolution>>,
    requests: Rc<RefCell<Vec<usize>>>,
    pending_after_first_batch: bool,
}

impl RecentFixtureSource {
    fn new(
        entries: Vec<RecentEntryResolution>,
        pending_after_first_batch: bool,
    ) -> (Self, Rc<RefCell<Vec<usize>>>) {
        let requests = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                entries: RefCell::new(entries.into_iter().collect()),
                requests: requests.clone(),
                pending_after_first_batch,
            },
            requests,
        )
    }
}

impl RecentEnumerationSource for RecentFixtureSource {
    fn open(&self, _deadline: Instant) -> RecentEnumerationFuture<Result<(), RecentSourceError>> {
        Box::pin(async { Ok(()) })
    }

    fn next_batch(
        &self,
        batch_size: usize,
        _include_metadata: bool,
        deadline: Instant,
    ) -> RecentEnumerationFuture<Result<Option<Vec<RecentEntryResolution>>, RecentSourceError>>
    {
        let request_number = {
            let mut requests = self.requests.borrow_mut();
            requests.push(batch_size);
            requests.len()
        };
        if Instant::now() >= deadline {
            return Box::pin(async { Err(RecentSourceError::TimedOut) });
        }
        if self.pending_after_first_batch && request_number > 1 {
            return Box::pin(std::future::pending());
        }
        let entries = {
            let mut remaining = self.entries.borrow_mut();
            (0..batch_size)
                .filter_map(|_| remaining.pop_front())
                .collect::<Vec<_>>()
        };
        let batch = (!entries.is_empty()).then_some(entries);
        Box::pin(async move { Ok(batch) })
    }
}

fn recent_fixture_entry(name: &str) -> FileEntry {
    let info = gio::FileInfo::new();
    info.set_name(name);
    info.set_display_name(name);
    info.set_file_type(gio::FileType::Regular);
    let mut entry = entry_from_info(Location::local(format!("/fixture/{name}")), info);
    entry.recent_unix_seconds = MetadataValue::Known(42);
    entry
}

/// `enumerate()` spawns its work on `glib::MainContext::default()` internally (not whatever
/// context happens to be thread-default), so it can only be driven via that same shared context
/// -- a private context pushed as thread-default would never see the spawned task at all. Bridge
/// the callback-based API into a future with `poll_fn` and drive it with `block_on`. The shared
/// lock is still required: concurrent `block_on(default())` calls from different test-harness
/// threads panic with a GLib thread-affinity error, same as concurrent `spawn_local`/`iteration()`
/// would.
fn run_enumerate(request: DirectoryRequest) -> Vec<DirectoryEvent> {
    run_events(|emit| LocalFileSource.enumerate(request, emit))
}

fn run_recent_enumerate(
    request: DirectoryRequest,
    source: Box<dyn RecentEnumerationSource>,
) -> Vec<DirectoryEvent> {
    run_events(|emit| enumerate_recent_with_source(request, emit, Instant::now(), source))
}

fn run_events(start: impl FnOnce(Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle) -> Vec<DirectoryEvent> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    glib::MainContext::default().block_on(async move {
        let events: Rc<RefCell<Vec<DirectoryEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let waker: Rc<RefCell<Option<std::task::Waker>>> = Rc::new(RefCell::new(None));
        let collected = events.clone();
        let collected_waker = waker.clone();
        let emit: Rc<dyn Fn(DirectoryEvent)> = Rc::new(move |event| {
            let is_terminal = matches!(
                event,
                DirectoryEvent::Finished { .. } | DirectoryEvent::Failed { .. }
            );
            collected.borrow_mut().push(event);
            if is_terminal && let Some(waker) = collected_waker.borrow_mut().take() {
                waker.wake();
            }
        });
        let handle = start(emit);
        std::future::poll_fn(|cx| {
            let has_terminal_event = events.borrow().iter().any(|event| {
                matches!(
                    event,
                    DirectoryEvent::Finished { .. } | DirectoryEvent::Failed { .. }
                )
            });
            if has_terminal_event {
                std::task::Poll::Ready(())
            } else {
                *waker.borrow_mut() = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
        .await;
        drop(handle);
        events.borrow().clone()
    })
}

fn batched_entry_count(events: &[DirectoryEvent]) -> usize {
    events
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::Batch { entries, .. } => Some(entries.len()),
            _ => None,
        })
        .sum()
}

fn batched_entries(events: &[DirectoryEvent]) -> Vec<FileEntry> {
    events
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::Batch { entries, .. } => Some(entries.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn finished_truncated(events: &[DirectoryEvent]) -> Option<bool> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::Finished { truncated, .. } => Some(*truncated),
        _ => None,
    })
}

fn finished_can_trash(events: &[DirectoryEvent]) -> Option<Option<bool>> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::Finished { can_trash, .. } => Some(*can_trash),
        _ => None,
    })
}

fn finished_can_delete(events: &[DirectoryEvent]) -> Option<Option<bool>> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::Finished { can_delete, .. } => Some(*can_delete),
        _ => None,
    })
}

fn recent_resolution(name: &str) -> RecentEntryResolution {
    RecentEntryResolution::Entry(Box::new(recent_fixture_entry(name)))
}

#[test]
fn recent_enumeration_publishes_valid_targets_and_skips_stale_entries() {
    let (source, requests) = RecentFixtureSource::new(
        vec![
            recent_resolution("valid-one"),
            RecentEntryResolution::Stale,
            recent_resolution("valid-two"),
        ],
        false,
    );
    let events = run_recent_enumerate(
        DirectoryRequest {
            id: RequestId(1),
            location: Location::uri("recent:///"),
            batch_size: 2,
            include_metadata: true,
            max_entries: 10,
            time_budget: Duration::from_secs(10),
            background: false,
        },
        Box::new(source),
    );

    assert_eq!(
        batched_entries(&events)
            .iter()
            .map(|entry| entry.native_name.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["valid-one", "valid-two"]
    );
    assert_eq!(finished_truncated(&events), Some(false));
    assert_eq!(finished_can_trash(&events), Some(None));
    assert_eq!(finished_can_delete(&events), Some(None));
    assert_eq!(&*requests.borrow(), &[2, 2, 2]);
}

#[test]
fn recent_enumeration_finishes_normally_when_empty() {
    let (source, _) = RecentFixtureSource::new(Vec::new(), false);
    let events = run_recent_enumerate(
        DirectoryRequest {
            id: RequestId(1),
            location: Location::uri("recent:///"),
            batch_size: 2,
            include_metadata: false,
            max_entries: 10,
            time_budget: Duration::from_secs(10),
            background: false,
        },
        Box::new(source),
    );

    assert!(batched_entries(&events).is_empty());
    assert_eq!(finished_truncated(&events), Some(false));
    assert_eq!(finished_can_trash(&events), Some(None));
    assert_eq!(finished_can_delete(&events), Some(None));
}

#[test]
fn recent_enumeration_respects_the_request_batch_size() {
    let (source, requests) = RecentFixtureSource::new(
        (0..5)
            .map(|index| recent_resolution(&format!("entry-{index}")))
            .collect(),
        false,
    );
    let events = run_recent_enumerate(
        DirectoryRequest {
            id: RequestId(1),
            location: Location::uri("recent:///"),
            batch_size: 2,
            include_metadata: false,
            max_entries: 10,
            time_budget: Duration::from_secs(10),
            background: false,
        },
        Box::new(source),
    );

    let batch_sizes = events
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::Batch { entries, .. } => Some(entries.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(batch_sizes, [2, 2, 1]);
    assert!(requests.borrow().iter().all(|size| *size == 2));
}

#[test]
fn recent_enumeration_respects_max_entries_and_reports_truncation() {
    let (source, _) = RecentFixtureSource::new(
        (0..5)
            .map(|index| recent_resolution(&format!("entry-{index}")))
            .collect(),
        false,
    );
    let events = run_recent_enumerate(
        DirectoryRequest {
            id: RequestId(1),
            location: Location::uri("recent:///"),
            batch_size: 2,
            include_metadata: false,
            max_entries: 3,
            time_budget: Duration::from_secs(10),
            background: false,
        },
        Box::new(source),
    );

    assert_eq!(batched_entry_count(&events), 3);
    assert_eq!(finished_truncated(&events), Some(true));
}

struct FailingRecentSource {
    fail_on_open: bool,
}

impl RecentEnumerationSource for FailingRecentSource {
    fn open(&self, _deadline: Instant) -> RecentEnumerationFuture<Result<(), RecentSourceError>> {
        let fail = self.fail_on_open;
        Box::pin(async move {
            if fail {
                Err(RecentSourceError::Failed(
                    "recent backend is gone".to_owned(),
                ))
            } else {
                Ok(())
            }
        })
    }

    fn next_batch(
        &self,
        _batch_size: usize,
        _include_metadata: bool,
        _deadline: Instant,
    ) -> RecentEnumerationFuture<Result<Option<Vec<RecentEntryResolution>>, RecentSourceError>>
    {
        Box::pin(async { Err(RecentSourceError::Failed("recent read failed".to_owned())) })
    }
}

fn recent_failure_message(events: &[DirectoryEvent]) -> Option<&str> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::Failed { message, .. } => Some(message.as_str()),
        _ => None,
    })
}

#[test]
fn a_failing_recent_open_reports_the_backend_error_instead_of_an_empty_collection() {
    let events = run_recent_enumerate(
        recent_request(),
        Box::new(FailingRecentSource { fail_on_open: true }),
    );

    assert_eq!(
        recent_failure_message(&events),
        Some("recent backend is gone")
    );
    assert_eq!(finished_truncated(&events), None);
    assert!(batched_entries(&events).is_empty());
}

#[test]
fn a_failing_recent_read_reports_the_backend_error_instead_of_finishing_cleanly() {
    let events = run_recent_enumerate(
        recent_request(),
        Box::new(FailingRecentSource {
            fail_on_open: false,
        }),
    );

    assert_eq!(recent_failure_message(&events), Some("recent read failed"));
    assert_eq!(finished_truncated(&events), None);
}

#[test]
fn an_unresolvable_target_does_not_discard_the_entries_beside_it() {
    let (source, _) = RecentFixtureSource::new(
        vec![
            recent_resolution("before"),
            RecentEntryResolution::TimedOut,
            recent_resolution("after"),
        ],
        false,
    );

    let events = run_recent_enumerate(
        DirectoryRequest {
            batch_size: 3,
            ..recent_request()
        },
        Box::new(source),
    );

    assert_eq!(
        batched_entries(&events)
            .iter()
            .map(|entry| entry.display_name.clone())
            .collect::<Vec<_>>(),
        ["before", "after"]
    );
    assert_eq!(finished_truncated(&events), Some(true));
}

fn recent_request() -> DirectoryRequest {
    DirectoryRequest {
        id: RequestId(1),
        location: Location::uri("recent:///"),
        batch_size: 2,
        include_metadata: false,
        max_entries: 10,
        time_budget: Duration::from_secs(5),
        background: false,
    }
}

#[test]
fn recent_enumeration_honors_a_zero_time_budget() {
    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::uri("recent:///"),
        batch_size: 2,
        include_metadata: false,
        max_entries: 10,
        time_budget: Duration::ZERO,
        background: false,
    });

    assert!(batched_entries(&events).is_empty());
    assert_eq!(finished_truncated(&events), Some(true));
    assert_eq!(finished_can_trash(&events), Some(None));
    assert_eq!(finished_can_delete(&events), Some(None));
}

#[test]
fn cancelling_recent_enumeration_stops_further_publication() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (source, _) = RecentFixtureSource::new(vec![recent_resolution("first")], true);
    let events: Rc<RefCell<Vec<DirectoryEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let collected = events.clone();
    let handle = enumerate_recent_with_source(
        DirectoryRequest {
            id: RequestId(1),
            location: Location::uri("recent:///"),
            batch_size: 1,
            include_metadata: false,
            max_entries: 10,
            time_budget: Duration::from_secs(10),
            background: false,
        },
        Rc::new(move |event| collected.borrow_mut().push(event)),
        Instant::now(),
        Box::new(source),
    );
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, DirectoryEvent::Batch { .. }))
        && Instant::now() < deadline
    {
        context.iteration(true);
    }

    assert_eq!(batched_entry_count(&events.borrow()), 1);
    drop(handle);
    for _ in 0..3 {
        while context.iteration(false) {}
    }

    assert_eq!(batched_entry_count(&events.borrow()), 1);
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, DirectoryEvent::Finished { .. }))
    );
}

#[test]
fn enumerate_reports_truncated_once_the_entry_budget_is_exceeded() {
    let root = unique_fixture_root("entry-budget");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    for index in 0..5 {
        fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the fixture file should be written");
    }

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 2,
        include_metadata: false,
        max_entries: 3,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    assert_eq!(
        finished_truncated(&events),
        Some(true),
        "exceeding the entry budget should be reported as truncated"
    );
    assert_eq!(
        batched_entry_count(&events),
        3,
        "loading should retain exactly the configured maximum"
    );
}

#[test]
fn native_loads_list_through_the_filesystem_seam() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};
    use rustix::io::Errno;

    for (quirks, listed, chunks, failure) in [
        (
            Quirks {
                listing_chunk: Some(1),
                ..Quirks::default()
            },
            3,
            4,
            None,
        ),
        (
            Quirks {
                fail: vec![(FsOp::ListChunks, "*", 1, Errno::IO)],
                listing_chunk: Some(1),
                ..Quirks::default()
            },
            0,
            2,
            Some("A device input/output error occurred"),
        ),
        (
            Quirks {
                fail: vec![(FsOp::FolderStamp, "*", 0, Errno::ACCESS)],
                ..Quirks::default()
            },
            0,
            0,
            Some("Permission denied"),
        ),
    ] {
        let root = tempfile::tempdir()?;
        for name in ["a", "b", "c"] {
            fs::write(root.path().join(name), b"content")?;
        }
        let scope = FsQuirks::install(root.path(), quirks);

        let events = run_enumerate(DirectoryRequest {
            id: RequestId(1),
            location: Location::local(root.path()),
            batch_size: 64,
            include_metadata: false,
            max_entries: 100,
            time_budget: Duration::from_secs(10),
            background: false,
        });

        let failed = events.iter().find_map(|event| match event {
            DirectoryEvent::Failed { message, .. } => Some(message.as_str()),
            _ => None,
        });
        let revalidated_stamps = scope
            .calls()
            .iter()
            .filter(|call| call.op == FsOp::FolderStamp && call.forced)
            .count();
        assert_eq!(
            (
                batched_entry_count(&events),
                revalidated_stamps,
                scope.count(FsOp::ListChunks),
                failed,
            ),
            (listed, 1, chunks, failure)
        );
    }
    Ok(())
}

fn load_into(
    events: &Rc<RefCell<Vec<DirectoryEvent>>>,
    root: &Path,
    id: u64,
) -> crate::services::LoadHandle {
    let events = events.clone();
    LocalFileSource.enumerate(
        DirectoryRequest {
            id: RequestId(id),
            location: Location::local(root),
            batch_size: 64,
            include_metadata: false,
            max_entries: 100,
            time_budget: Duration::from_secs(30),
            background: false,
        },
        Rc::new(move |event| events.borrow_mut().push(event)),
    )
}

fn outcome_of(events: &Rc<RefCell<Vec<DirectoryEvent>>>, id: u64) -> Option<Result<usize, String>> {
    let events = events.borrow();
    let listed = events
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::Batch {
                request_id,
                entries,
            } if request_id.0 == id => Some(entries.len()),
            _ => None,
        })
        .sum();
    events.iter().find_map(|event| match event {
        DirectoryEvent::Finished { request_id, .. } if request_id.0 == id => Some(Ok(listed)),
        DirectoryEvent::Failed {
            request_id,
            message,
            ..
        } if request_id.0 == id => Some(Err(message.clone())),
        _ => None,
    })
}

#[test]
fn a_local_folder_is_validated_on_a_worker_while_the_window_keeps_running()
-> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(FsOp::FolderStamp, "folder", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let answer = Rc::new(RefCell::new(None));
    let slot = answer.clone();
    let ticked = Rc::new(Cell::new(false));
    let tick = ticked.clone();
    glib::timeout_add_local_once(Duration::from_millis(10), move || tick.set(true));

    let _validation = LocalFileSource.validate_location_async(
        Location::local(&folder),
        Rc::new(move |result| {
            slot.replace(Some(result));
        }),
    );
    drain_until(|| gate.reached() && ticked.get());
    assert!(answer.borrow().is_none());
    gate.release();
    drain_until(|| answer.borrow().is_some());

    assert_eq!(answer.take(), Some(Ok(())));
    Ok(())
}

struct ReportsDrop(std::sync::mpsc::Sender<std::thread::ThreadId>);

impl Drop for ReportsDrop {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}

type Threads = std::sync::Arc<std::sync::Mutex<Vec<std::thread::ThreadId>>>;

fn thread_recorder() -> (Threads, Box<dyn FnMut() + Send>) {
    let threads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = threads.clone();
    let record = Box::new(move || {
        recorded
            .lock()
            .expect("thread record")
            .push(std::thread::current().id());
    });
    (threads, record)
}

#[test]
fn a_monitor_is_made_and_released_on_a_worker_while_the_window_keeps_running()
-> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for dropped_while_parked in [false, true] {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("folder");
        fs::create_dir(&folder)?;
        let gate = Gate::default();
        let (released_on, released) = thread_recorder();
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![(FsOp::Watch, "folder", 0, gate.racer())],
                after: vec![(FsOp::Unwatch, "folder", 0, released)],
                ..Quirks::default()
            },
        );
        let (dropped, drops) = std::sync::mpsc::channel();
        let probe = ReportsDrop(dropped);
        let changes: Rc<RefCell<Vec<DirectoryChange>>> = Rc::default();
        let collected = changes.clone();
        let reported = Rc::new(Cell::new(None));
        let report = reported.clone();
        let ticked = Rc::new(Cell::new(false));
        let tick = ticked.clone();
        glib::timeout_add_local_once(Duration::from_millis(10), move || tick.set(true));

        let handle = LocalFileSource.start_watch(
            Location::local(&folder),
            false,
            Rc::new(move |change| {
                let _held = &probe;
                collected.borrow_mut().push(change);
            }),
            Rc::new(move |watching| report.set(Some(watching))),
        );
        drain_until(|| gate.reached() && ticked.get());
        assert_eq!(reported.get(), None, "parked");
        if dropped_while_parked {
            drop(handle);
            gate.release();
        } else {
            gate.release();
            drain_until(|| reported.get().is_some());
            assert_eq!(reported.get(), Some(Watching::Live));
            fs::write(folder.join("arrival"), b"arrived")?;
            drain_until(|| !changes.borrow().is_empty());
            drop(handle);
            assert_eq!(
                drops.try_recv().ok(),
                Some(std::thread::current().id()),
                "the handler's state goes with the handle, on this thread"
            );
        }
        drain_until(|| scope.count(FsOp::Unwatch) == 1);

        assert_eq!(reported.get().is_none(), dropped_while_parked);
        assert!(
            !released_on
                .lock()
                .expect("thread record")
                .contains(&std::thread::current().id()),
            "dropped while parked: {dropped_while_parked}"
        );
    }
    Ok(())
}

#[test]
fn a_folder_on_a_mount_that_stopped_answering_gets_no_monitor() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks, mount_lanes};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    let gate = Gate::default();
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(FsOp::FolderStamp, "folder", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let watch = || {
        let reported = Rc::new(Cell::new(None));
        let report = reported.clone();
        let handle = LocalFileSource.start_watch(
            Location::local(&folder),
            false,
            Rc::new(|_| {}),
            Rc::new(move |watching| report.set(Some(watching))),
        );
        drain_until(|| reported.get().is_some());
        (reported.get(), handle)
    };
    let events = Rc::new(RefCell::new(Vec::new()));
    let load = load_into(&events, &folder, 1);
    drain_until(|| mount_lanes::responsiveness(&folder) == mount_lanes::Responsiveness::Stalled);

    let (stalled, _unwatched) = watch();
    gate.release();
    drain_until(|| outcome_of(&events, 1).is_some());
    let (answering, _watched) = watch();

    assert_eq!(
        (stalled, answering, scope.count(FsOp::Watch)),
        (Some(Watching::Unavailable), Some(Watching::Live), 1)
    );
    drop(load);
    Ok(())
}

#[test]
fn repeated_loads_of_a_stuck_folder_share_one_call_and_then_time_out() -> Result<(), Box<dyn Error>>
{
    use crate::adapters::{FsOp, FsQuirks, Quirks, mount_lanes};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("file"), b"content")?;
    let gate = Gate::default();
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(FsOp::FolderStamp, "*", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));

    let first = load_into(&events, root.path(), 1);
    drain_until(|| gate.reached());
    let second = load_into(&events, root.path(), 2);
    drain_until(|| {
        mount_lanes::responsiveness(root.path()) == mount_lanes::Responsiveness::Stalled
    });
    assert_eq!(scope.count(FsOp::FolderStamp), 1);
    let third = load_into(&events, root.path(), 3);
    drain_until(|| outcome_of(&events, 3).is_some());
    assert_eq!(
        outcome_of(&events, 3),
        Some(Err("The operation timed out".to_owned()))
    );
    assert_eq!(
        (outcome_of(&events, 1), outcome_of(&events, 2)),
        (None, None)
    );

    gate.release();
    drain_until(|| outcome_of(&events, 1).is_some() && outcome_of(&events, 2).is_some());
    assert_eq!(
        (outcome_of(&events, 1), outcome_of(&events, 2)),
        (Some(Ok(1)), Some(Ok(1)))
    );
    assert_eq!(scope.count(FsOp::FolderStamp), 1);
    assert_eq!(
        mount_lanes::responsiveness(root.path()),
        mount_lanes::Responsiveness::Answering
    );
    drop((first, second, third));
    Ok(())
}

#[test]
fn a_slow_listing_that_keeps_answering_never_stalls_its_mount() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    for name in ["a", "b", "c", "d", "e"] {
        fs::write(root.path().join(name), b"content")?;
    }
    let slow_chunk = || -> Box<dyn FnMut() + Send> {
        Box::new(|| std::thread::sleep(Duration::from_millis(700)))
    };
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            listing_chunk: Some(1),
            before: (0..5)
                .map(|_| (FsOp::ListChunks, "*", 0, slow_chunk()))
                .collect(),
            ..Quirks::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));

    let started = Instant::now();
    let first = load_into(&events, root.path(), 1);
    drain_until(|| started.elapsed() >= Duration::from_millis(2500));
    let second = load_into(&events, root.path(), 2);
    drain_until(|| outcome_of(&events, 1).is_some() && outcome_of(&events, 2).is_some());

    assert_eq!(
        (outcome_of(&events, 1), outcome_of(&events, 2)),
        (Some(Ok(5)), Some(Ok(5)))
    );
    drop((first, second));
    Ok(())
}

#[test]
fn a_load_asked_for_while_the_folder_is_being_read_lists_it_afresh() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for names in [&["a", "b"][..], &[]] {
        let root = tempfile::tempdir()?;
        for name in names {
            fs::write(root.path().join(name), b"content")?;
        }
        let gate = Gate::default();
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                listing_chunk: Some(1),
                before: vec![(FsOp::ListChunks, "*", 0, gate.racer())],
                ..Quirks::default()
            },
        );
        let events = Rc::new(RefCell::new(Vec::new()));

        let first = load_into(&events, root.path(), 1);
        drain_until(|| gate.reached());
        fs::write(root.path().join("c"), b"content")?;
        let second = load_into(&events, root.path(), 2);
        for _ in 0..50 {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(scope.count(FsOp::FolderStamp), 1, "one call at a time");

        gate.release();
        drain_until(|| outcome_of(&events, 1).is_some() && outcome_of(&events, 2).is_some());
        assert_eq!(
            (outcome_of(&events, 1), outcome_of(&events, 2)),
            (Some(Ok(names.len())), Some(Ok(names.len() + 1))),
            "{names:?}"
        );
        assert_eq!(scope.count(FsOp::FolderStamp), 2);
        drop((first, second));
    }
    Ok(())
}

#[test]
fn a_link_whose_target_stops_answering_trips_only_that_link() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks, mount_lanes};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let sibling = root.path().join("sibling");
    fs::create_dir(root.path().join("target"))?;
    fs::create_dir(&sibling)?;
    fs::write(sibling.join("file"), b"content")?;
    std::os::unix::fs::symlink(root.path().join("target"), root.path().join("nas"))?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(FsOp::FolderStamp, "nas", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));

    let warm = load_into(&events, &sibling, 1);
    drain_until(|| outcome_of(&events, 1).is_some());
    let parked = load_into(&events, root.path(), 2);
    drain_until(|| gate.reached());
    let reached = Instant::now();
    drain_until(|| reached.elapsed() > mount_lanes::STALLED_AFTER + Duration::from_millis(200));
    let again = load_into(&events, &sibling, 3);
    drain_until(|| outcome_of(&events, 3).is_some());

    assert_eq!(
        (
            outcome_of(&events, 3),
            mount_lanes::responsiveness(&sibling),
            mount_lanes::responsiveness(&root.path().join("nas"))
        ),
        (
            Some(Ok(1)),
            mount_lanes::Responsiveness::Answering,
            mount_lanes::Responsiveness::Stalled
        )
    );
    gate.release();
    drain_until(|| outcome_of(&events, 2).is_some());
    assert_eq!(outcome_of(&events, 2), Some(Ok(3)));
    drop((warm, parked, again));
    Ok(())
}

#[test]
fn a_load_or_folder_check_on_a_gvfs_backend_that_stopped_answering_is_refused_at_once()
-> Result<(), Box<dyn Error>> {
    use crate::adapters::mount_lanes;

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let stopped = [
        Location::uri("sftp://strata-stopped-backend.invalid/share"),
        Location::uri("trash://strata-stopped-trash.invalid/"),
    ];
    let answering = Location::uri("sftp://strata-answering-backend.invalid/share");
    let stuck = stopped
        .iter()
        .map(|location| {
            mount_lanes::admit_remote(
                mount_lanes::authority(location.uri_value().unwrap_or_default()),
                mount_lanes::Class::Interactive,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|refusal| format!("{refusal:?}"))?;
    let admitted = Instant::now();
    drain_until(|| admitted.elapsed() > mount_lanes::STALLED_AFTER + Duration::from_millis(200));
    let events = Rc::new(RefCell::new(Vec::new()));
    let load = |location: &Location, id| {
        let events = events.clone();
        LocalFileSource.enumerate(
            DirectoryRequest {
                id: RequestId(id),
                location: location.clone(),
                batch_size: 64,
                include_metadata: false,
                max_entries: 100,
                time_budget: Duration::from_secs(10),
                background: false,
            },
            Rc::new(move |event| events.borrow_mut().push(event)),
        )
    };

    let checked = Rc::new(RefCell::new(HashMap::new()));
    let check = |location: &Location| {
        let checked = checked.clone();
        let key = location.clone();
        LocalFileSource.validate_location_async(
            location.clone(),
            Rc::new(move |result| {
                checked.borrow_mut().insert(key.clone(), result);
            }),
        )
    };

    let refused = [load(&stopped[0], 1), load(&stopped[1], 3)];
    let elsewhere = load(&answering, 2);
    let checks = [&stopped[0], &stopped[1], &answering].map(check);
    drain_until(|| {
        (1..=3).all(|id| outcome_of(&events, id).is_some()) && checked.borrow().len() == 3
    });

    let timed_out = Some(Err("The operation timed out".to_owned()));
    assert_eq!(
        (
            [outcome_of(&events, 1), outcome_of(&events, 3)],
            stopped
                .each_ref()
                .map(|location| LocalFileSource.responsiveness(location)),
            LocalFileSource.responsiveness(&answering),
        ),
        (
            [timed_out.clone(), timed_out.clone()],
            [Responsiveness::Stalled; 2],
            Responsiveness::Answering
        )
    );
    assert_ne!(outcome_of(&events, 2), timed_out);
    let unavailable = Err(LocationValidationError::Unavailable(
        crate::services::io_error_detail(&ErrorKind::TimedOut.into()),
    ));
    let checked = checked.take();
    assert_eq!(
        stopped.each_ref().map(|location| checked.get(location)),
        [Some(&unavailable); 2]
    );
    assert_ne!(checked.get(&answering), Some(&unavailable));
    drop((stuck, refused, elsewhere, checks));
    Ok(())
}

fn listing_request(path: &Path, max_entries: usize) -> DirectoryRequest {
    DirectoryRequest {
        id: RequestId(1),
        location: Location::local(path),
        batch_size: 64,
        include_metadata: false,
        max_entries,
        time_budget: Duration::from_secs(10),
        background: false,
    }
}

fn load_stamp(events: &[DirectoryEvent]) -> Option<DirectoryStamp> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::Stamp { request_id, stamp } if *request_id == RequestId(1) => {
            Some((**stamp).clone())
        }
        _ => None,
    })
}

fn names_digest(events: &[DirectoryEvent]) -> Option<u64> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::ListingDigest {
            request_id,
            names_digest,
        } if *request_id == RequestId(1) => Some(*names_digest),
        _ => None,
    })
}

fn probed_stamp(path: &Path) -> StampAnswer {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let answer = Rc::new(RefCell::new(None));
    let slot = answer.clone();
    let _probe = LocalFileSource.probe_stamp(
        Location::local(path),
        Rc::new(move |stamp| {
            slot.replace(Some(stamp));
        }),
    );
    drain_until(|| answer.borrow().is_some());
    answer.take().expect("answered")
}

fn stamp_of(answer: StampAnswer) -> DirectoryStamp {
    match answer {
        StampAnswer::Stamp(stamp) => *stamp,
        other => panic!("no stamp: {other:?}"),
    }
}

fn wait_past(probe: &Path, than: &DirectoryStamp) {
    let than = than.changed.expect("a native change time");
    loop {
        fs::write(probe, b"probe").expect("write the probe");
        let stamped = fs::metadata(probe).expect("stat the probe");
        fs::remove_file(probe).expect("remove the probe");
        if (stamped.ctime(), stamped.ctime_nsec() as u32) > than {
            break;
        }
    }
}

#[test]
fn a_load_reports_its_stamp_first_and_a_digest_of_its_names_last() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    for name in ["a", "b"] {
        fs::write(folder.join(name), b"content")?;
    }
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        &folder,
        Quirks {
            before: vec![(FsOp::ListChunks, "*", 0, gate.racer())],
            ..Quirks::ext4()
        },
    );

    let events = Rc::new(RefCell::new(Vec::new()));
    {
        let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .map_err(|error| error.to_string())?;
        let _load = load_into(&events, &folder, 1);
        drain_until(|| gate.reached() && load_stamp(&events.borrow()).is_some());
        assert_eq!(outcome_of(&events, 1), None);
        gate.release();
        drain_until(|| outcome_of(&events, 1).is_some());
    }
    let first = events.take();
    let again = run_enumerate(listing_request(&folder, 100));
    fs::rename(folder.join("b"), folder.join("c"))?;
    let renamed = run_enumerate(listing_request(&folder, 100));
    let truncated = run_enumerate(listing_request(&folder, 1));

    assert!(matches!(first.first(), Some(DirectoryEvent::Stamp { .. })));
    assert!(matches!(
        &first[first.len() - 2..],
        [
            DirectoryEvent::ListingDigest { .. },
            DirectoryEvent::Finished { .. }
        ]
    ));
    let stamp = load_stamp(&first).ok_or("no stamp")?;
    assert_eq!(
        stamp.identity.map(|identity| identity.evidence.kind),
        Some(rustix::fs::FileType::Directory)
    );
    assert_eq!(names_digest(&again), names_digest(&first));
    assert_ne!(names_digest(&renamed), names_digest(&first));
    assert!(load_stamp(&truncated).is_some());
    assert_eq!(names_digest(&truncated), None);
    Ok(())
}

#[test]
fn a_check_sees_a_change_the_attribute_cache_hides() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsQuirks, Quirks};

    for checked_by_a_load in [false, true] {
        let root = tempfile::tempdir()?;
        let share = root.path().join("share");
        fs::create_dir(&share)?;
        let _scope = FsQuirks::install(&share, Quirks::nfs42());
        let check = || match checked_by_a_load {
            true => load_stamp(&run_enumerate(listing_request(&share, 100))).ok_or("no stamp"),
            false => Ok(stamp_of(probed_stamp(&share))),
        };

        let baseline = check()?;
        wait_past(&root.path().join("probe"), &baseline);
        fs::write(share.join("added elsewhere"), b"content")?;
        let cached = super::super::local_operations::folder_stamp(&share, false)
            .map_err(|error| error.to_string())?;
        let checked = check()?;

        assert_eq!(
            (cached.modified, Some(cached.changed)),
            (baseline.modified, baseline.changed),
            "a plain stat answers from the cache"
        );
        assert!(
            checked.moved_since(&baseline),
            "checked by a load: {checked_by_a_load}"
        );
    }
    Ok(())
}

#[test]
fn a_stamp_classifies_the_folder_by_its_mount_not_its_path() -> Result<(), Box<dyn Error>> {
    use crate::{
        adapters::{FsQuirks, Quirks},
        services::InodeNumbers,
    };

    for no_mount_ids in [false, true] {
        let root = tempfile::tempdir()?;
        let (home, nas, ssh) = (
            root.path().join("home"),
            root.path().join("nas"),
            root.path().join("ssh"),
        );
        fs::create_dir_all(home.join("documents"))?;
        fs::create_dir_all(nas.join("music"))?;
        fs::create_dir_all(&ssh)?;
        std::os::unix::fs::symlink(nas.join("music"), home.join("music"))?;
        let _home = FsQuirks::install(
            &home,
            Quirks {
                no_mount_ids,
                ..Quirks::ext4()
            },
        );
        let _nas = FsQuirks::install(
            &nas,
            Quirks {
                no_mount_ids,
                ..Quirks::nfs42()
            },
        );
        let _ssh = FsQuirks::install(
            &ssh,
            Quirks {
                no_mount_ids,
                ..Quirks::sshfs()
            },
        );

        let stamp = |path: &Path| load_stamp(&run_enumerate(listing_request(path, 100)));
        let (documents, linked, music) = (
            stamp(&home.join("documents")).ok_or("no stamp")?,
            stamp(&home.join("music")).ok_or("no stamp")?,
            stamp(&nas.join("music")).ok_or("no stamp")?,
        );

        assert_eq!(
            (
                documents.outside_writers,
                linked.outside_writers,
                music.outside_writers,
                linked.identity.map(|identity| identity.traits.numbers),
            ),
            (false, true, true, Some(InodeNumbers::Kept)),
            "no mount ids: {no_mount_ids}"
        );
        assert_eq!(
            (
                documents.daemon_cache,
                music.daemon_cache,
                stamp(&ssh).ok_or("no stamp")?.daemon_cache
            ),
            (None, None, Some(Duration::from_secs(21))),
            "no mount ids: {no_mount_ids}"
        );
        assert!(music.mount.is_some(), "no mount ids: {no_mount_ids}");
        assert_eq!(linked.mount, music.mount, "no mount ids: {no_mount_ids}");
        assert_ne!(
            linked.mount, documents.mount,
            "no mount ids: {no_mount_ids}"
        );
    }
    Ok(())
}

#[test]
fn a_folder_put_back_to_its_old_time_still_reads_as_changed() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;

    let baseline = stamp_of(probed_stamp(&folder));
    wait_past(&root.path().join("probe"), &baseline);
    fs::write(folder.join("added"), b"content")?;
    let (seconds, nanoseconds) = baseline.modified;
    fs::File::open(&folder)?.set_modified(
        SystemTime::UNIX_EPOCH + Duration::new(u64::try_from(seconds)?, nanoseconds),
    )?;
    let after = stamp_of(probed_stamp(&folder));

    assert_eq!(after.modified, baseline.modified);
    assert!(after.moved_since(&baseline));
    Ok(())
}

#[test]
fn a_hung_mount_holds_one_worker_and_answers_unresponsive() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let (hung, healthy) = (root.path().join("hung"), root.path().join("healthy"));
    fs::create_dir(&hung)?;
    fs::create_dir(&healthy)?;
    let gate = Gate::default();
    let scope = FsQuirks::install(
        &hung,
        Quirks {
            before: vec![(FsOp::FolderStamp, "*", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let answers: Rc<RefCell<Vec<(Location, StampAnswer)>>> = Rc::default();
    let check = |folder: &Path| {
        let answers = answers.clone();
        LocalFileSource.check_stamps(
            vec![Location::local(folder)],
            Rc::new(move |location, answer| answers.borrow_mut().push((location, answer))),
        )
    };
    let answered = |folder: &Path| {
        answers
            .borrow()
            .iter()
            .filter(|(location, _)| *location == Location::local(folder))
            .map(|(_, answer)| match answer {
                StampAnswer::Stamp(_) => "stamp",
                StampAnswer::Unresponsive => "unresponsive",
                _ => "other",
            })
            .collect::<Vec<_>>()
    };

    let relisted = Rc::new(RefCell::new(Vec::new()));
    let relist = || {
        let events = relisted.clone();
        LocalFileSource.enumerate(
            DirectoryRequest {
                background: true,
                ..listing_request(&hung, 100)
            },
            Rc::new(move |event| events.borrow_mut().push(event)),
        )
    };

    let first = check(&hung);
    drain_until(|| gate.reached());
    let repeats = [check(&hung), check(&hung)];
    let elsewhere = check(&healthy);
    let refused_relist = relist();
    drain_until(|| answers.borrow().len() == 3 && outcome_of(&relisted, 1).is_some());
    let while_hung = (
        answered(&hung),
        answered(&healthy),
        outcome_of(&relisted, 1),
    );
    gate.release();
    drain_until(|| answers.borrow().len() == 4);

    assert_eq!(
        while_hung,
        (
            vec!["unresponsive"; 2],
            vec!["stamp"],
            Some(Err("The operation timed out".to_owned()))
        )
    );
    assert_eq!(answered(&hung), ["unresponsive", "unresponsive", "stamp"]);
    assert_eq!(scope.count(FsOp::FolderStamp), 1);
    drop((first, repeats, elsewhere, refused_relist));
    Ok(())
}

#[test]
fn a_check_answer_can_relist_while_the_check_reads_another_folder_of_its_mount()
-> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let (changed, slow) = (root.path().join("changed"), root.path().join("slow"));
    fs::create_dir(&changed)?;
    fs::create_dir(&slow)?;
    fs::write(changed.join("added elsewhere"), b"x")?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        &slow,
        Quirks {
            before: vec![(FsOp::FolderStamp, "*", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let relisted = Rc::new(RefCell::new(Vec::new()));
    let relists: Rc<RefCell<Vec<LoadHandle>>> = Rc::default();
    let (events, handles, folder) = (relisted.clone(), relists.clone(), changed.clone());
    let _check = LocalFileSource.check_stamps(
        vec![Location::local(&changed), Location::local(&slow)],
        Rc::new(move |location, _| {
            if location == Location::local(&folder) {
                let events = events.clone();
                handles.borrow_mut().push(LocalFileSource.enumerate(
                    DirectoryRequest {
                        background: true,
                        ..listing_request(&folder, 100)
                    },
                    Rc::new(move |event| events.borrow_mut().push(event)),
                ));
            }
        }),
    );

    drain_until(|| gate.reached());
    let context = glib::MainContext::default();
    let settle = Instant::now() + Duration::from_millis(200);
    while outcome_of(&relisted, 1).is_none() && Instant::now() < settle {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    gate.release();
    drain_until(|| outcome_of(&relisted, 1).is_some());

    assert_eq!(outcome_of(&relisted, 1), Some(Ok(1)));
    Ok(())
}

#[test]
fn a_renamed_folder_is_followed_where_its_mount_proves_it_is_the_same() -> Result<(), Box<dyn Error>>
{
    use crate::adapters::{Births, FsQuirks, Quirks, evict};

    type Row = (&'static str, fn() -> Quirks, bool, bool);
    let rows: [Row; 4] = [
        ("numbers kept", Quirks::ext4, false, true),
        (
            "renumbered, birth kept",
            || Quirks {
                births: Births::MadeUp,
                ..Quirks::vfat()
            },
            true,
            true,
        ),
        ("renumbered, no birth", Quirks::sshfs, true, false),
        (
            "renumbered, folder births unreliable",
            Quirks::exfat,
            true,
            false,
        ),
    ];
    for (row, quirks, evicted, followed) in rows {
        let root = tempfile::tempdir()?;
        let parent = root.path().join("parent");
        let (old, new) = (parent.join("old"), parent.join("new"));
        fs::create_dir_all(&old)?;
        let _scope = FsQuirks::install(&parent, quirks());
        let identity = directory_stamp(
            &old,
            &super::super::local_operations::folder_stamp(&old, false)
                .map_err(|error| error.to_string())?,
            SystemTime::now(),
        )
        .identity
        .ok_or("no identity")?;
        let probe = root.path().join("probe");
        loop {
            fs::write(&probe, b"probe")?;
            let stamped = fs::metadata(&probe)?;
            fs::remove_file(&probe)?;
            if stamped.modified()? > fs::metadata(&old)?.modified()? {
                break;
            }
        }
        fs::create_dir(parent.join("sibling"))?;
        fs::rename(&old, &new)?;
        if evicted {
            assert!(evict(&new), "{row}");
        }

        let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .map_err(|error| error.to_string())?;
        let found = Rc::new(RefCell::new(None));
        let slot = found.clone();
        let _find = LocalFileSource.find_by_identity(
            Location::local(&parent),
            identity,
            Rc::new(move |location| {
                slot.replace(Some(location));
            }),
        );
        drain_until(|| found.borrow().is_some());

        assert_eq!(
            found.take().flatten(),
            followed.then(|| Location::local(&new)),
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn enumerate_resolves_can_trash_from_a_child_entry() {
    let root = unique_fixture_root("can-trash");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let child = root.join("child.txt");
    fs::write(&child, b"content").expect("the fixture file should be written");
    let expected = gio::File::for_path(&child)
        .query_info(
            gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )
        .expect("the child capability query should succeed")
        .boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH);

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 10,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    assert_eq!(finished_can_trash(&events), Some(Some(expected)));
}

#[test]
fn enumerate_resolves_can_delete_from_a_child_entry() {
    let root = unique_fixture_root("can-delete");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let child = root.join("child.txt");
    fs::write(&child, b"content").expect("the fixture file should be written");
    let expected = gio::File::for_path(&child)
        .query_info(
            gio::FILE_ATTRIBUTE_ACCESS_CAN_DELETE,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )
        .expect("the child capability query should succeed")
        .boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_DELETE);

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 10,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    assert_eq!(finished_can_delete(&events), Some(Some(expected)));
}

#[test]
fn entry_limited_metadata_load_fills_every_retained_entry() {
    let root = unique_fixture_root("entry-budget-metadata");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    for index in 0..5 {
        fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the fixture file should be written");
    }

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 2,
        include_metadata: true,
        max_entries: 3,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    assert_eq!(finished_truncated(&events), Some(true));
    let entries = batched_entries(&events);
    assert_eq!(entries.len(), 3);
    assert!(
        entries
            .iter()
            .all(|entry| matches!(entry.size, MetadataValue::Known(7)))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DirectoryEvent::MetadataIncomplete { .. }))
    );
}

#[test]
fn enumerate_completes_untruncated_at_the_exact_entry_budget() {
    let root = unique_fixture_root("exact-entry-budget");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    for index in 0..4 {
        fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the fixture file should be written");
    }

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 2,
        include_metadata: false,
        max_entries: 4,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    assert_eq!(
        finished_truncated(&events),
        Some(false),
        "reaching the entry budget is not truncation when the directory is complete"
    );
    assert_eq!(batched_entry_count(&events), 4);
}

#[test]
fn enumerate_reports_truncated_once_the_time_budget_is_exceeded() {
    let root = unique_fixture_root("time-budget");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    fs::write(root.join("needle.txt"), b"content").expect("the fixture file should be written");
    fs::write(root.join("second.txt"), b"content").expect("the fixture file should be written");

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 1,
        include_metadata: false,
        max_entries: usize::MAX,
        time_budget: Duration::from_nanos(1),
        background: false,
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    assert_eq!(
        finished_truncated(&events),
        Some(true),
        "an exhausted time budget should stop the load and report truncation"
    );
}

#[test]
fn enumerate_completes_untruncated_within_budget() {
    let root = unique_fixture_root("within-budget");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    for index in 0..5 {
        fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the fixture file should be written");
    }

    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    assert_eq!(
        finished_truncated(&events),
        Some(false),
        "a directory well within budget should not be reported as truncated"
    );
    assert_eq!(batched_entry_count(&events), 5);
}

#[test]
fn native_enumeration_marks_hidden_files() {
    let root = unique_fixture_root("hidden");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    fs::write(root.join("visible.txt"), b"content").expect("the visible file should be written");
    fs::write(root.join(".hidden.txt"), b"content").expect("the hidden file should be written");
    fs::write(root.join("listed-hidden.txt"), b"content")
        .expect("the listed hidden file should be written");
    fs::write(root.join(".hidden"), b"listed-hidden.txt\n")
        .expect("the hidden-name list should be written");

    let entries = batched_entries(&run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    }));
    assert_eq!(entries.len(), 4);
    assert!(
        entries
            .iter()
            .find(|entry| entry.native_name == "visible.txt")
            .is_some_and(|entry| !entry.is_hidden)
    );
    for name in [".hidden.txt", "listed-hidden.txt", ".hidden"] {
        assert!(
            entries
                .iter()
                .find(|entry| entry.native_name == name)
                .is_some_and(|entry| entry.is_hidden),
            "{name} should be marked hidden"
        );
    }

    fs::remove_dir_all(&root).expect("the fixture directory should be removed");
}

#[test]
fn native_enumeration_reports_an_unreadable_root() {
    let root = unique_fixture_root("missing-root");
    let events = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    });

    assert!(matches!(events.as_slice(), [DirectoryEvent::Failed { .. }]));
}

#[test]
fn cancelled_native_scan_returns_no_partial_result() {
    let root = unique_fixture_root("cancelled");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    fs::write(root.join("file.txt"), b"content").expect("the fixture file should be written");
    let request = DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    };
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();

    assert!(matches!(
        scan_native_directory(
            &root,
            &request,
            &cancellable,
            Instant::now() + Duration::from_secs(10),
            &Progress::unlaned(),
            oneshot::channel().0,
        ),
        NativeEnumeration::Cancelled
    ));

    fs::remove_dir_all(&root).expect("the fixture directory should be removed");
}

#[test]
fn enumerate_with_metadata_fills_sizes_up_front() -> Result<(), Box<dyn Error>> {
    let root = unique_fixture_root("with-metadata");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let path = root.join("file-0.txt");
    fs::write(&path, b"content").expect("the fixture file should be written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
        .expect("the fixture permissions should be set");

    let with_metadata = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: true,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    let streaming = run_enumerate(DirectoryRequest {
        id: RequestId(2),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    let metadata = |events: &[DirectoryEvent]| {
        events
            .iter()
            .filter_map(|event| match event {
                DirectoryEvent::Batch { entries, .. } => Some(
                    entries
                        .iter()
                        .map(|entry| (entry.size.clone(), entry.mode.clone()))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        metadata(&with_metadata),
        vec![(MetadataValue::Known(7), MetadataValue::Known(0o100640))],
        "a metadata load should stat every entry up front"
    );
    assert_eq!(
        metadata(&streaming),
        vec![(MetadataValue::Unknown, MetadataValue::Unknown)],
        "a streaming load should leave sizes for the window fill"
    );
    Ok(())
}

#[test]
fn metadata_marks_folders_the_user_cannot_open() {
    let root = unique_fixture_root("folder-access");
    let locked = root.join("locked");
    let open = root.join("open");
    let file = root.join("file.txt");
    fs::create_dir_all(&locked).expect("the locked folder should be created");
    fs::create_dir_all(&open).expect("the open folder should be created");
    fs::write(&file, b"content").expect("the fixture file should be written");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))
        .expect("the locked folder's permissions should be set");

    let scanned = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: true,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    });
    let filled = run_fill(MetadataRequest {
        id: RequestId(2),
        entries: [&locked, &open, &file].map(Location::local).to_vec(),
        full: false,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    });
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))
        .expect("the locked folder's permissions should be restored");
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    let scanned: HashMap<_, _> = scanned
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::Batch { entries, .. } => Some(entries.iter()),
            _ => None,
        })
        .flatten()
        .map(|entry| (entry.location.clone(), entry.listable.clone()))
        .collect();
    let filled: HashMap<_, _> = filled
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::MetadataFilled { updates, .. } => Some(updates.iter()),
            _ => None,
        })
        .flatten()
        .map(|update| (update.location.clone(), update.listable.clone()))
        .collect();
    // Root reads every folder, so nothing is locked for it.
    let locked_access = MetadataValue::Known(rustix::process::geteuid().is_root());
    for listable in [scanned, filled] {
        assert_eq!(listable[&Location::local(&locked)], locked_access);
        assert_eq!(
            listable[&Location::local(&open)],
            MetadataValue::Known(true)
        );
        assert_eq!(
            listable[&Location::local(&file)],
            MetadataValue::Unavailable
        );
    }
}

fn run_fill(request: MetadataRequest) -> Vec<DirectoryEvent> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    glib::MainContext::default().block_on(async move {
        let events: Rc<RefCell<Vec<DirectoryEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let waker: Rc<RefCell<Option<std::task::Waker>>> = Rc::new(RefCell::new(None));
        let collected = events.clone();
        let collected_waker = waker.clone();
        let emit: Rc<dyn Fn(DirectoryEvent)> = Rc::new(move |event| {
            let is_terminal = matches!(event, DirectoryEvent::MetadataFinished { .. });
            collected.borrow_mut().push(event);
            if is_terminal && let Some(waker) = collected_waker.borrow_mut().take() {
                waker.wake();
            }
        });
        let handle = LocalFileSource.fill_metadata(request, emit);
        std::future::poll_fn(|cx| {
            let has_terminal_event = events
                .borrow()
                .iter()
                .any(|event| matches!(event, DirectoryEvent::MetadataFinished { .. }));
            if has_terminal_event {
                std::task::Poll::Ready(())
            } else {
                *waker.borrow_mut() = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
        .await;
        drop(handle);
        events.borrow().clone()
    })
}

fn fill_outcome(events: &[DirectoryEvent]) -> Option<MetadataOutcome> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::MetadataFinished { outcome, .. } => Some(*outcome),
        _ => None,
    })
}

fn fill_chunk_count(events: &[DirectoryEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, DirectoryEvent::MetadataFilled { .. }))
        .count()
}

#[test]
fn fill_empty_entries_completes_without_chunks() {
    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: Vec::new(),
        full: true,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    assert_eq!(fill_chunk_count(&events), 0);
}

#[test]
fn sequential_fill_with_no_time_remaining_is_truncated() {
    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![Location::local("/fixture/file.txt")],
        full: false,
        include_icon_details: false,
        time_budget: Duration::ZERO,
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Truncated));
    assert_eq!(fill_chunk_count(&events), 0);
}

#[test]
fn hostile_hidden_files_are_ignored_without_blocking_or_following() {
    use std::os::unix::fs::symlink;

    let root = unique_fixture_root("hidden-hostile");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let hidden = root.join(".hidden");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &hidden,
        rustix::fs::Mode::from_bits_truncate(0o600),
    )
    .expect("the fifo should be created");
    assert!(native_hidden_names(&root).is_empty());

    fs::remove_file(&hidden).expect("the fifo should be removed");
    let target = root.join("target");
    fs::write(&target, b"secret\n").expect("the target should be written");
    symlink(&target, &hidden).expect("the symlink should be created");
    assert!(native_hidden_names(&root).is_empty());
    fs::remove_dir_all(&root).expect("the fixture should be removed");
}

#[test]
fn fill_all_vanished_entries_reports_failed() {
    let root = unique_fixture_root("fill-vanished");
    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![
            Location::local(root.join("gone-0.txt")),
            Location::local(root.join("gone-1.txt")),
        ],
        full: true,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Failed));
}

#[test]
fn fill_unreachable_remote_reports_failed() {
    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![Location::uri("sftp://host/share/photo.jpg")],
        full: false,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Failed));
    assert_eq!(fill_chunk_count(&events), 1);
}

#[test]
fn fill_file_uri_stats_through_the_uri_form() -> Result<(), Box<dyn Error>> {
    let root = unique_fixture_root("fill-uri");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let path = root.join("photo.jpg");
    fs::write(&path, b"content").expect("the fixture file should be written");

    let uri = format!("file://{}", path.display());
    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![Location::uri(uri)],
        full: false,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    assert_eq!(fill_chunk_count(&events), 1);
    Ok(())
}

#[test]
fn fill_live_file_completes_with_a_chunk() -> Result<(), Box<dyn Error>> {
    let root = unique_fixture_root("fill-live");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let path = root.join("photo.jpg");
    fs::write(&path, b"content").expect("the fixture file should be written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
        .expect("the fixture permissions should be set");

    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![Location::local(&path)],
        full: false,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    assert_eq!(fill_chunk_count(&events), 1);
    let metadata = events.iter().find_map(|event| match event {
        DirectoryEvent::MetadataFilled { updates, .. } => updates
            .first()
            .map(|update| (update.size.clone(), update.mode.clone())),
        _ => None,
    });
    assert_eq!(
        metadata,
        Some((MetadataValue::Known(7), MetadataValue::Known(0o100640)))
    );
    Ok(())
}

#[test]
fn native_viewport_metadata_streams_multiple_chunks() -> Result<(), Box<dyn Error>> {
    let root = unique_fixture_root("fill-streaming");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let entries: Vec<_> = (0..20)
        .map(|index| {
            let path = root.join(format!("file-{index}.txt"));
            fs::write(&path, b"content").expect("the fixture file should be written");
            Location::local(path)
        })
        .collect();

    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries,
        full: false,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    assert!(fill_chunk_count(&events) > 1);
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");
    Ok(())
}

#[test]
fn icon_details_cache_bounds_revisit_history_and_preserves_lru() {
    let mut cache = IconDetailsCache::default();
    let fingerprint = IconDetailsFingerprint {
        size: 1,
        modified_seconds: 1,
        modified_nanoseconds: 0,
        changed_seconds: 1,
        changed_nanoseconds: 0,
    };
    let details = IconDetails {
        image_dimensions: MetadataValue::Known((1, 1)),
        child_count: MetadataValue::Unavailable,
        duration_seconds: MetadataValue::Unavailable,
    };
    for index in 0..MAX_ICON_DETAILS_CACHE_ENTRIES {
        cache.insert(
            PathBuf::from(index.to_string()),
            fingerprint,
            details.clone(),
        );
    }
    let revisited = Path::new("0");
    for _ in 0..MAX_ICON_DETAILS_CACHE_ENTRIES * 5 {
        assert!(cache.get(revisited, fingerprint).is_some());
    }
    assert!(cache.recent.len() <= MAX_ICON_DETAILS_CACHE_ENTRIES * 4);
    cache.insert(PathBuf::from("new"), fingerprint, details);
    assert!(cache.get(revisited, fingerprint).is_some());
    assert!(cache.get(Path::new("1"), fingerprint).is_none());
    assert_eq!(cache.entries.len(), MAX_ICON_DETAILS_CACHE_ENTRIES);
}

#[test]
fn fill_image_file_extracts_dimensions() -> Result<(), Box<dyn Error>> {
    let root = unique_fixture_root("fill-image-dimensions");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let path = root.join("pixel.png");
    let png_bytes = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0xdc, 0xcc, 0x59, 0xe7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    fs::write(&path, png_bytes).expect("the fixture file should be written");

    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![Location::local(&path)],
        full: false,
        include_icon_details: true,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    let dimensions = events.iter().rev().find_map(|event| match event {
        DirectoryEvent::MetadataFilled { updates, .. } => updates
            .first()
            .map(|update| update.image_dimensions.clone()),
        _ => None,
    });
    assert_eq!(dimensions, Some(MetadataValue::Known((1, 1))));
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");
    Ok(())
}

#[test]
fn fill_skips_media_probes_for_text_named_like_media() {
    let root = unique_fixture_root("fill-text-named-like-media");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let path = root.join("clip.mp4");
    fs::write(&path, "export const clip = 1;\n").expect("the fixture file should be written");

    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![Location::local(&path)],
        full: false,
        include_icon_details: true,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    let duration = events.iter().rev().find_map(|event| match event {
        DirectoryEvent::MetadataFilled { updates, .. } => updates
            .first()
            .map(|update| update.duration_seconds.clone()),
        _ => None,
    });
    assert_eq!(duration, Some(MetadataValue::Unavailable));
    assert_eq!(media_metadata_probe_count(&path), 0);
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");
}

#[test]
fn fill_media_file_caches_duration_for_revisits() -> Result<(), Box<dyn Error>> {
    let root = unique_fixture_root("fill-media-duration");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let path = root.join("clip.mkv");
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=16x16:rate=1:duration=2",
            "-c:v",
            "ffv1",
            "-threads",
            "1",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg should create the media fixture");
    assert!(status.success());

    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![Location::local(&path)],
        full: false,
        include_icon_details: true,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    let duration = events.iter().rev().find_map(|event| match event {
        DirectoryEvent::MetadataFilled { updates, .. } => updates
            .first()
            .map(|update| update.duration_seconds.clone()),
        _ => None,
    });
    assert_eq!(duration, Some(MetadataValue::Known(2)));
    assert_eq!(media_metadata_probe_count(&path), 1);

    let revisited_entries = batched_entries(&run_enumerate(DirectoryRequest {
        id: RequestId(2),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    }));
    assert_eq!(revisited_entries.len(), 1);
    assert_eq!(
        revisited_entries[0].duration_seconds,
        MetadataValue::Known(2)
    );

    let revisit = run_fill(MetadataRequest {
        id: RequestId(2),
        entries: vec![Location::local(&path)],
        full: false,
        include_icon_details: true,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&revisit), Some(MetadataOutcome::Complete));
    let revisited_duration = revisit.iter().rev().find_map(|event| match event {
        DirectoryEvent::MetadataFilled { updates, .. } => updates
            .first()
            .map(|update| update.duration_seconds.clone()),
        _ => None,
    });
    assert_eq!(revisited_duration, Some(MetadataValue::Known(2)));
    assert_eq!(media_metadata_probe_count(&path), 1);

    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=16x16:rate=1:duration=3",
            "-c:v",
            "ffv1",
            "-threads",
            "1",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg should replace the media fixture");
    assert!(status.success());
    let changed = run_fill(MetadataRequest {
        id: RequestId(3),
        entries: vec![Location::local(&path)],
        full: false,
        include_icon_details: true,
        time_budget: Duration::from_secs(10),
    });
    let changed_duration = changed.iter().rev().find_map(|event| match event {
        DirectoryEvent::MetadataFilled { updates, .. } => updates
            .first()
            .map(|update| update.duration_seconds.clone()),
        _ => None,
    });
    assert_eq!(changed_duration, Some(MetadataValue::Known(3)));
    assert_eq!(media_metadata_probe_count(&path), 2);
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");
    Ok(())
}

#[test]
fn fill_directory_child_count_extracts_item_count() -> Result<(), Box<dyn Error>> {
    let root = unique_fixture_root("fill-dir-count");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    fs::write(root.join("file-1.txt"), b"1").expect("file 1");
    fs::write(root.join("file-2.txt"), b"2").expect("file 2");
    fs::create_dir_all(root.join("subdir")).expect("subdir");
    let alias = root.with_extension("alias");
    std::os::unix::fs::symlink(&root, &alias).expect("alias");
    let fill = |id, folder: &Path| {
        let events = run_fill(MetadataRequest {
            id: RequestId(id),
            entries: vec![Location::local(folder)],
            full: false,
            include_icon_details: true,
            time_budget: Duration::from_secs(10),
        });
        assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
        events.iter().rev().find_map(|event| match event {
            DirectoryEvent::MetadataFilled { updates, .. } => {
                updates.first().map(|update| update.child_count.clone())
            }
            _ => None,
        })
    };
    let activity = crate::services::file_activity::begin(
        crate::services::file_activity::OperationScope::default(),
    );

    let real = root.canonicalize()?;

    for (row, folder) in [("as named", &root), ("through a symbolic link", &alias)] {
        let fence = activity.fence([real.clone()]);
        assert_eq!(
            fill(1, folder),
            Some(MetadataValue::Unknown),
            "{row}: a folder an operation fences is not listed"
        );
        drop(fence);
        assert_eq!(fill(2, folder), Some(MetadataValue::Known(3)), "{row}");
    }
    fs::remove_file(&alias).expect("the alias should be removed");
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");
    Ok(())
}

#[test]
fn background_folder_listings_skip_a_folder_an_operation_fences() {
    let root = tempfile::tempdir().expect("fixture directory");
    let folder = root.path().join("folder");
    fs::create_dir(&folder).expect("watched folder");
    fs::write(folder.join("#1234"), b"").expect("numbered name");
    let identity = directory_stamp(
        &folder,
        &super::super::local_operations::folder_stamp(&folder, false).expect("folder stamp"),
        SystemTime::now(),
    )
    .identity
    .expect("folder identity");
    let listings: [(&str, &dyn Fn() -> bool); 3] = [
        ("numbered names", &|| {
            numbered_names_in(&folder)
                .is_ok_and(|names| names.contains(std::ffi::OsStr::new("#1234")))
        }),
        ("departure probe", &|| {
            native_entry_with_identity(root.path(), identity).as_ref() == Some(&folder)
        }),
        ("background re-list", &|| {
            let events = run_enumerate(DirectoryRequest {
                background: true,
                ..listing_request(&folder, 100)
            });
            batched_entry_count(&events) == 1 && finished_truncated(&events) == Some(false)
        }),
    ];
    let activity = crate::services::file_activity::begin(
        crate::services::file_activity::OperationScope::default(),
    );

    for (listing, lists) in &listings {
        assert!(lists(), "{listing}");
    }
    let _fence = activity.fence([root.path().to_owned()]);
    for (listing, lists) in &listings {
        assert!(!lists(), "{listing} under a fence");
    }
}

#[test]
fn a_background_relist_stops_for_an_operation_that_changes_its_folder() -> Result<(), Box<dyn Error>>
{
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    fs::write(folder.join("file"), b"content")?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        &folder,
        Quirks {
            before: vec![(FsOp::ListChunks, "*", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    let _load = LocalFileSource.enumerate(
        DirectoryRequest {
            background: true,
            ..listing_request(&folder, 100)
        },
        Rc::new(move |event| recorded.borrow_mut().push(event)),
    );

    drain_until(|| gate.reached());
    let activity = file_activity::begin(file_activity::OperationScope::default());
    let fence = activity.fence([folder.clone()]);
    fence.stop_reads();
    gate.release();
    drain_until(|| outcome_of(&events, 1).is_some());

    assert_eq!(
        outcome_of(&events, 1),
        Some(Err(file_activity::HELD_FOR_OPERATION.to_owned()))
    );
    Ok(())
}

#[test]
fn parallel_fill_follows_symlinks_like_enumeration() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let root = unique_fixture_root("fill-symlink");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    let target = root.join("target.txt");
    fs::write(&target, b"0123456789").expect("the target should be written");
    let link = root.join("link.txt");
    symlink(&target, &link).expect("the symlink should be created");
    let subdir = root.join("sub");
    fs::create_dir_all(&subdir).expect("the subdir should be created");
    let dir_link = root.join("dir-link");
    symlink(&subdir, &dir_link).expect("the dir symlink should be created");

    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![
            Location::local(&link),
            Location::local(&dir_link),
            Location::local(&target),
        ],
        full: true,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    let by_location = |wanted: &Location| {
        events.iter().find_map(|event| match event {
            DirectoryEvent::MetadataFilled { updates, .. } => updates
                .iter()
                .find(|update| &update.location == wanted)
                .cloned(),
            _ => None,
        })
    };
    let link_update = by_location(&Location::local(&link)).expect("the link should fill");
    let target_update = by_location(&Location::local(&target)).expect("the target should fill");
    assert_eq!(link_update.size, MetadataValue::Known(10));
    assert_eq!(link_update.size, target_update.size);
    assert_eq!(
        link_update.modified_unix_seconds,
        target_update.modified_unix_seconds
    );
    let dir_update = by_location(&Location::local(&dir_link)).expect("the dir link should fill");
    assert_eq!(dir_update.size, MetadataValue::Unknown);
    assert_eq!(dir_update.child_count, MetadataValue::Unknown);
    assert!(matches!(
        dir_update.modified_unix_seconds,
        MetadataValue::Known(_)
    ));
    Ok(())
}

#[test]
fn parallel_fill_cancellation_reports_cancelled_without_chunks() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let root = unique_fixture_root("fill-cancel");
    let count = 20_000;
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    for index in 0..100 {
        fs::write(root.join(format!("file-{index:04}.txt")), b"content")
            .expect("the fixture file should be written");
    }
    let entries: Vec<Location> = (0..count)
        .map(|index| Location::local(root.join(format!("file-{index:05}.txt"))))
        .collect();
    let events: Rc<RefCell<Vec<DirectoryEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let collected = events.clone();
    let emit: Rc<dyn Fn(DirectoryEvent)> = Rc::new(move |event| {
        collected.borrow_mut().push(event);
    });
    glib::MainContext::default().block_on(async {
        let handle = super::fill_parallel_with(
            8,
            RequestId(1),
            entries,
            false,
            Duration::from_secs(60),
            emit,
        );
        let mut yielded = false;
        std::future::poll_fn(|cx| {
            if yielded {
                std::task::Poll::Ready(())
            } else {
                yielded = true;
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        })
        .await;
        drop(handle);
        let waker: Rc<RefCell<Option<std::task::Waker>>> = Rc::new(RefCell::new(None));
        let wake = waker.clone();
        let ticker = glib::timeout_add_local(Duration::from_millis(20), move || {
            if let Some(waker) = wake.borrow_mut().take() {
                waker.wake();
            }
            glib::ControlFlow::Continue
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        std::future::poll_fn(|cx| {
            let done = events
                .borrow()
                .iter()
                .any(|event| matches!(event, DirectoryEvent::MetadataFinished { .. }));
            if done || Instant::now() >= deadline {
                std::task::Poll::Ready(())
            } else {
                *waker.borrow_mut() = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
        .await;
        ticker.remove();
    });
}

#[test]
fn cheap_metadata_is_published_while_details_are_waiting() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main-context lock");
    let root = tempfile::tempdir().expect("fixture directory");
    let path = root.path().join("photo.jpg");
    fs::write(&path, b"content").expect("first fixture");
    let second = root.path().join("second.jpg");
    fs::write(&second, b"content").expect("second fixture");
    let (release, wait) = std::sync::mpsc::channel();
    let wait = Mutex::new(wait);
    glib::MainContext::default().block_on(async move {
        let (finished, done) = futures_channel::oneshot::channel();
        let finished = RefCell::new(Some(finished));
        let saw_cheap = Rc::new(Cell::new(0));
        let saw_details = Rc::new(Cell::new(0));
        let cheap = saw_cheap.clone();
        let details = saw_details.clone();
        let emit = Rc::new(move |event| match event {
            DirectoryEvent::MetadataFilled { updates, .. } => {
                for update in updates {
                    assert_eq!(update.size, MetadataValue::Known(7));
                    assert!(matches!(
                        update.modified_unix_seconds,
                        MetadataValue::Known(_)
                    ));
                    assert!(matches!(update.mode, MetadataValue::Known(_)));
                    if update.image_dimensions == MetadataValue::Unknown {
                        cheap.set(cheap.get() + 1);
                        if cheap.get() == 2 {
                            release.send(()).expect("release first probe");
                        }
                    } else {
                        assert_eq!(cheap.get(), 2);
                        assert_eq!(update.image_dimensions, MetadataValue::Known((20, 30)));
                        details.set(details.get() + 1);
                        if details.get() == 1 {
                            release.send(()).expect("release second probe");
                        }
                    }
                }
            }
            DirectoryEvent::MetadataFinished { outcome, .. } => {
                assert_eq!(outcome, MetadataOutcome::Complete);
                finished
                    .borrow_mut()
                    .take()
                    .expect("one completion")
                    .send(())
                    .expect("completion receiver");
            }
            _ => {}
        });
        let handle = fill_parallel_with_details(
            1,
            RequestId(1),
            vec![Location::local(path), Location::local(second)],
            true,
            Duration::from_secs(10),
            emit,
            move |update, _, _, _, _| {
                wait.lock()
                    .expect("probe receiver")
                    .recv_timeout(Duration::from_secs(5))
                    .expect("preceding metadata published before probe finishes");
                update.image_dimensions = MetadataValue::Known((20, 30));
                true
            },
        );
        done.await.expect("metadata completion");
        assert_eq!(saw_cheap.get(), 2);
        assert_eq!(saw_details.get(), 2);
        drop(handle);
    });
}

#[test]
fn cancelled_or_withheld_media_details_are_not_cached_as_unavailable() {
    let root = tempfile::tempdir().expect("fixture directory");
    let path = root.path().join("photo.jpg");
    fs::write(&path, b"\xff\xd8\xff\xe0\0\x10JFIF\0").expect("fixture file");
    let location = Location::local(&path);
    let info = gio::File::for_path(&path)
        .query_info(
            METADATA_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )
        .expect("source metadata");
    for (row, cancels, error, filled) in [
        ("cancelled", true, "cancelled", false),
        (
            "withheld by an operation",
            false,
            crate::services::file_activity::HELD_FOR_OPERATION,
            true,
        ),
    ] {
        let cancellable = gio::Cancellable::new();
        let (mut update, _) = update_from_info(&info, &location);
        assert_eq!(
            fill_icon_details_with_probe(
                &mut update,
                &info,
                &location,
                &cancellable,
                Instant::now() + Duration::from_secs(10),
                |_, _, cancellation| {
                    if cancels {
                        cancellation.cancel();
                    }
                    Err(error.to_owned())
                },
            ),
            filled,
            "{row}"
        );
        assert_eq!(update.image_dimensions, MetadataValue::Unknown, "{row}");
        assert_eq!(update.duration_seconds, MetadataValue::Unknown, "{row}");
        assert!(cached_icon_details_for_revisit(&path).is_none(), "{row}");
    }
}

#[derive(Clone, Default)]
struct FakeRemovalBackend {
    calls: Rc<RefCell<Vec<String>>>,
    outcomes: Rc<RefCell<HashMap<String, Result<(), String>>>>,
    hold: Rc<Cell<bool>>,
}

impl FakeRemovalBackend {
    fn new() -> Self {
        Self::default()
    }

    fn fail(&self, uri: &str, error: &str) {
        self.outcomes
            .borrow_mut()
            .insert(uri.to_owned(), Err(error.to_owned()));
    }
}

impl RecentRemovalBackend for FakeRemovalBackend {
    fn delete(&self, uri: &str) -> RecentEnumerationFuture<Result<(), String>> {
        self.calls.borrow_mut().push(uri.to_owned());
        let hold = self.hold.clone();
        let outcome = self.outcomes.borrow().get(uri).cloned().unwrap_or(Ok(()));
        Box::pin(async move {
            while hold.get() {
                glib::timeout_future(Duration::from_millis(1)).await;
            }
            outcome
        })
    }
}

fn drain_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() && Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        done(),
        "the awaited event did not arrive within the deadline"
    );
}

#[test]
fn recent_removal_calls_delete_and_logs_success() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let state = RecentRemovalState::new();
    let backend = Rc::new(FakeRemovalBackend::new());
    let backend_dyn: Rc<dyn RecentRemovalBackend> = backend.clone();
    let uri = "recent:///success".to_owned();

    let output = capture_logs(|| {
        recent_remove_entries_with_backend(&state, &backend_dyn, [uri.clone()]);
        drain_until(|| state.in_flight.borrow().is_empty());
    });

    assert_eq!(*backend.calls.borrow(), vec![uri.clone()]);
    assert!(state.in_flight.borrow().is_empty());
    assert!(output.contains("removed recent entry"));
    assert!(output.contains(&uri));
}

#[test]
fn recent_removal_failure_logs_a_warning_and_allows_retry() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let state = RecentRemovalState::new();
    let backend = Rc::new(FakeRemovalBackend::new());
    let uri = "recent:///locked".to_owned();
    backend.fail(&uri, "xbel is locked");
    let backend_dyn: Rc<dyn RecentRemovalBackend> = backend.clone();

    let output = capture_logs(|| {
        recent_remove_entries_with_backend(&state, &backend_dyn, [uri.clone()]);
        drain_until(|| state.in_flight.borrow().is_empty());
    });

    assert_eq!(*backend.calls.borrow(), vec![uri.clone()]);
    assert!(output.contains("failed to remove recent entry"));
    assert!(output.contains(&uri));
    assert!(output.contains("xbel is locked"));
    backend.outcomes.borrow_mut().remove(&uri);
    recent_remove_entries_with_backend(&state, &backend_dyn, [uri.clone()]);
    drain_until(|| state.in_flight.borrow().is_empty());
    assert_eq!(*backend.calls.borrow(), vec![uri.clone(), uri]);
}

#[test]
fn recent_removal_logs_keep_credentials_out_of_the_default_log() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let state = RecentRemovalState::new();
    let backend = Rc::new(FakeRemovalBackend::new());
    let uri = "recent://alice:s3cret;domain=CORP@host.example/item?token=abc#frag".to_owned();
    backend.fail(
        &uri,
        "cannot reach sftp://alice:s3cret@host.example/item?token=abc#frag",
    );
    let backend_dyn: Rc<dyn RecentRemovalBackend> = backend.clone();

    let output = capture_logs(|| {
        recent_remove_entries_with_backend(&state, &backend_dyn, [uri.clone()]);
        drain_until(|| state.in_flight.borrow().is_empty());
    });

    let warning = crate::test_support::captured_event(&output, "failed to remove recent entry");
    assert_eq!(warning.split_whitespace().next(), Some("WARN"));
    assert!(warning.contains("recent://host.example/item"), "{warning}");
    for secret in ["alice", "s3cret", "CORP", "token", "frag"] {
        assert!(!output.contains(secret), "{secret} leaked:\n{output}");
    }
}

#[test]
fn rapid_double_invocation_on_the_same_uri_does_not_issue_two_concurrent_deletes() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let state = RecentRemovalState::new();
    let backend = Rc::new(FakeRemovalBackend::new());
    backend.hold.set(true);
    let backend_dyn: Rc<dyn RecentRemovalBackend> = backend.clone();
    let uri = "recent:///double-click".to_owned();

    recent_remove_entries_with_backend(&state, &backend_dyn, [uri.clone()]);
    drain_until(|| !backend.calls.borrow().is_empty());
    assert_eq!(
        backend.calls.borrow().len(),
        1,
        "the first removal should have called delete() once"
    );

    recent_remove_entries_with_backend(&state, &backend_dyn, [uri.clone()]);
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(10)));

    assert_eq!(
        backend.calls.borrow().len(),
        1,
        "a second removal for an in-flight URI must not call delete() again"
    );
    assert!(state.in_flight.borrow().contains(&uri));
    backend.hold.set(false);
    drain_until(|| state.in_flight.borrow().is_empty());
}

#[test]
fn multi_selection_removal_issues_one_delete_per_uri() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let state = RecentRemovalState::new();
    let backend = Rc::new(FakeRemovalBackend::new());
    let backend_dyn: Rc<dyn RecentRemovalBackend> = backend.clone();
    let uris = vec![
        "recent:///one".to_owned(),
        "recent:///two".to_owned(),
        "recent:///three".to_owned(),
    ];

    recent_remove_entries_with_backend(&state, &backend_dyn, uris.clone());
    drain_until(|| state.in_flight.borrow().is_empty());

    let mut called = backend.calls.borrow().clone();
    called.sort();
    let mut expected = uris;
    expected.sort();
    assert_eq!(called, expected);
}

#[test]
fn recent_removal_rejects_real_file_uris() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let fixture = tempfile::NamedTempFile::new().expect("target file");
    let state = RecentRemovalState::new();
    let backend = Rc::new(FakeRemovalBackend::new());
    let backend_dyn: Rc<dyn RecentRemovalBackend> = backend.clone();
    recent_remove_entries_with_backend(
        &state,
        &backend_dyn,
        [
            gio::File::for_path(fixture.path()).uri().to_string(),
            "smb://server/share/file".to_owned(),
        ],
    );
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(10)));
    assert!(backend.calls.borrow().is_empty());
    assert!(fixture.path().exists());
    assert!(state.in_flight.borrow().is_empty());
}
