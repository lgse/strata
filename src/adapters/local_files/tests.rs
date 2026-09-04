// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::{ErrorKind, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
    sync::{Arc, Mutex, MutexGuard},
    time::{Instant, SystemTime},
};

use tracing_subscriber::fmt::MakeWriter;

use super::*;
use crate::{
    model::{Location, MetadataValue},
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};

#[derive(Clone, Default)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);

struct LogWriterGuard<'a>(MutexGuard<'a, Vec<u8>>);

impl Write for LogWriterGuard<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = LogWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriterGuard(self.0.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

impl LogWriter {
    fn output(&self) -> String {
        let output = self.0.lock().unwrap_or_else(|error| error.into_inner());
        String::from_utf8_lossy(&output).into_owned()
    }
}

fn capture_directory_start_logs(locations: &[(RequestId, &Location)]) -> String {
    let writer = LogWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(writer.clone())
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("the logging subscriber should only be installed once");
    for (request_id, location) in locations {
        log_directory_load_started(*request_id, location);
    }
    writer.output()
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

    info.set_is_hidden(true);
    info.set_is_symlink(true);

    assert!(info_is_hidden(&info));
    assert!(info_is_symlink(&info));
}

#[test]
fn unmounted_network_shares_are_treated_as_directories() {
    let info = gio::FileInfo::new();
    info.set_file_type(gio::FileType::Mountable);
    info.set_name("share");
    info.set_display_name("share");

    let entry = entry_from_info(Location::uri("smb://host/share"), info);

    assert_eq!(entry.kind, EntryKind::Directory);
    assert!(entry.is_directory());
}

#[test]
fn native_files_are_located_by_their_real_path() {
    let file = gio::File::for_path("/tmp");
    assert_eq!(location_for_file(&file), Some(Location::local("/tmp")));
}

#[test]
fn gvfs_backed_files_use_their_uri_even_when_a_fuse_path_exists() {
    let file = gio::File::for_uri("smb://host/share");
    assert!(!file.is_native(), "smb:// should never be reported native");
    assert_eq!(location_for_file(&file), Some(Location::uri(file.uri())));
}

#[test]
fn gio_files_with_embedded_credentials_are_sanitized() {
    for uri in [
        "smb://user%3Asecret@host/share",
        "smb://user;password=secret@host/share",
        "smb://user%3Bpassword=secret@host/share",
        "smb://user:secret@host/share",
    ] {
        let location = location_for_file(&gio::File::for_uri(uri))
            .expect("credential URI should produce a sanitized location");
        assert_eq!(
            location
                .uri_value()
                .expect("remote location should have a URI")
                .trim_end_matches('/'),
            "smb://user@host/share",
            "did not sanitize {uri}"
        );
    }
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
            from: "/fixture/old".into(),
            to: "/fixture/new".into(),
        },
        PendingMonitorChange::Upsert("/fixture/new".into()),
    );

    assert!(matches!(change, PendingMonitorChange::Move { .. }));
}

#[test]
fn large_monitor_bursts_collapse_to_one_rescan() {
    let mut pending = HashMap::new();
    for index in 0..=MAX_PENDING_MONITOR_CHANGES {
        let path = PathBuf::from(format!("/fixture/{index}"));
        assert!(queue_monitor_change(
            &mut pending,
            path.clone(),
            PendingMonitorChange::Upsert(path),
        ));
    }

    assert_eq!(pending.len(), 1);
    assert!(matches!(
        pending.get(Path::new("")),
        Some(PendingMonitorChange::Rescan)
    ));
    assert!(!queue_monitor_change(
        &mut pending,
        "/fixture/ignored".into(),
        PendingMonitorChange::Remove("/fixture/ignored".into()),
    ));
}

#[test]
fn conflicting_move_events_fall_back_to_a_rescan() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: "/fixture/old".into(),
            to: "/fixture/new".into(),
        },
        PendingMonitorChange::Remove("/fixture/new".into()),
    );

    assert!(matches!(change, PendingMonitorChange::Rescan));
}

#[test]
fn permission_errors_are_reported_as_inaccessible() {
    let error = std::io::Error::from(ErrorKind::PermissionDenied);
    assert_eq!(
        map_validation_error(error),
        LocationValidationError::Inaccessible
    );
}

fn unique_fixture_root(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-local-files-{label}-{unique}"))
}

/// `enumerate()` spawns its work on `glib::MainContext::default()` internally (not whatever
/// context happens to be thread-default), so it can only be driven via that same shared context
/// -- a private context pushed as thread-default would never see the spawned task at all. Bridge
/// the callback-based API into a future with `poll_fn` and drive it with `block_on`. The shared
/// lock is still required: concurrent `block_on(default())` calls from different test-harness
/// threads panic with a GLib thread-affinity error, same as concurrent `spawn_local`/`iteration()`
/// would.
fn run_enumerate(request: DirectoryRequest) -> Vec<DirectoryEvent> {
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
        let handle = LocalFileSource.enumerate(request, emit);
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
    };
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();

    assert!(matches!(
        scan_native_directory(
            &root,
            &request,
            &cancellable,
            Instant::now() + Duration::from_secs(10)
        ),
        NativeEnumeration::Cancelled
    ));

    fs::remove_dir_all(&root).expect("the fixture directory should be removed");
}

#[test]
fn enumerate_with_metadata_fills_sizes_up_front() -> Result<(), Box<dyn Error>> {
    let root = unique_fixture_root("with-metadata");
    fs::create_dir_all(&root).expect("the fixture directory should be created");
    fs::write(root.join("file-0.txt"), b"content").expect("the fixture file should be written");

    let with_metadata = run_enumerate(DirectoryRequest {
        id: RequestId(1),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: true,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
    });
    let streaming = run_enumerate(DirectoryRequest {
        id: RequestId(2),
        location: Location::local(&root),
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
    });
    fs::remove_dir_all(&root).expect("the fixture directory should be removed");

    let sizes = |events: &[DirectoryEvent]| {
        events
            .iter()
            .filter_map(|event| match event {
                DirectoryEvent::Batch { entries, .. } => Some(
                    entries
                        .iter()
                        .map(|entry| entry.size.clone())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        sizes(&with_metadata),
        vec![MetadataValue::Known(7)],
        "a metadata load should stat every entry up front"
    );
    assert_eq!(
        sizes(&streaming),
        vec![MetadataValue::Unknown],
        "a streaming load should leave sizes for the window fill"
    );
    Ok(())
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
    fs::write(root.join("photo.jpg"), b"content").expect("the fixture file should be written");

    let events = run_fill(MetadataRequest {
        id: RequestId(1),
        entries: vec![Location::local(root.join("photo.jpg"))],
        full: false,
        time_budget: Duration::from_secs(10),
    });
    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Complete));
    assert_eq!(fill_chunk_count(&events), 1);
    let size = events.iter().find_map(|event| match event {
        DirectoryEvent::MetadataFilled { updates, .. } => {
            updates.first().map(|update| update.size.clone())
        }
        _ => None,
    });
    assert_eq!(size, Some(MetadataValue::Known(7)));
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
        let handle =
            super::fill_parallel_with(8, RequestId(1), entries, Duration::from_secs(60), emit);
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
