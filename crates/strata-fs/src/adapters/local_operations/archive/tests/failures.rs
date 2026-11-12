// SPDX-License-Identifier: MIT

//! Extractions that are cancelled or fail, and what they leave behind.

use super::*;
use crate::services::CancelledOperation;
use std::time::{Duration, Instant};

type Events = Rc<RefCell<Vec<OperationEvent>>>;
type Operation = Rc<RefCell<Option<LoadHandle>>>;

fn ended(event: &OperationEvent) -> bool {
    matches!(
        event,
        OperationEvent::Cancelled { .. }
            | OperationEvent::Extracted { .. }
            | OperationEvent::Failed { .. }
    )
}

/// Extracts, dropping the extraction's handle as soon as its archive starts.
fn extract_cancelled_on_start(request: ExtractRequest) -> (Events, Operation) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = Rc::new(RefCell::new(None::<LoadHandle>));
    let cancel_on_started = operation.clone();
    let handle = LocalOperationProvider.extract(
        request,
        Rc::new(move |event| {
            if matches!(event, OperationEvent::ArchiveStarted { .. }) {
                // Cancel before dispatching the worker, not after a main-context
                // iteration that may also finish extracting a small archive.
                drop(
                    cancel_on_started
                        .borrow_mut()
                        .take()
                        .expect("extraction handle"),
                );
            }
            emitted.borrow_mut().push(event);
        }),
    );
    operation.replace(Some(handle));
    while !events.borrow().iter().any(ended) {
        glib::MainContext::default().iteration(true);
    }
    (events, operation)
}

fn cancelled_result(events: &[OperationEvent]) -> CancelledOperation {
    events
        .iter()
        .find_map(|event| match event {
            OperationEvent::Cancelled { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("terminal cancellation result")
}

fn expect_only_not_attempted(result: &CancelledOperation, not_attempted: &[Location]) {
    assert!(result.completed.is_empty());
    assert!(result.failed.is_empty());
    assert_eq!(result.not_attempted, not_attempted);
}

#[test]
fn cancelling_extraction_from_started_waits_for_the_worker_and_reports_pending_output()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive_path = root.path().join("content.zip");
    write_zip_stored(
        &archive_path,
        &[("first.bin", b"early"), ("second.txt", b"late")],
    )?;

    let (events, operation) =
        extract_cancelled_on_start(extract_request(11, &archive_path, &destination));

    let events = events.borrow();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, OperationEvent::Cancelled { .. })),
        "expected cancellation after the worker stopped: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, OperationEvent::Extracted { .. }))
    );
    let result = cancelled_result(&events);
    assert!(
        result
            .affected_locations
            .contains(&Location::local(&destination))
    );
    expect_only_not_attempted(
        &result,
        &[
            Location::local(destination.join("first.bin")),
            Location::local(destination.join("second.txt")),
        ],
    );
    assert!(destination.read_dir()?.next().is_none());
    assert!(operation.borrow().is_none());
    Ok(())
}

const DAMAGED: &str = "This file is not a valid archive or is damaged.";

/// Each archive that cannot be extracted, and what its failure says.
const UNEXTRACTABLE: [(&str, &str); 10] = [
    ("fake.zip", DAMAGED),
    ("fake.7z", DAMAGED),
    ("fake.tar", DAMAGED),
    ("fake.tar.gz", DAMAGED),
    ("missing.zip", "No such file"),
    ("unreadable.zip", "Permission denied"),
    ("destination.zip", "Not a folder"),
    ("unknown.iso", "Unsupported archive format"),
    ("folder.zip", "Not an archive: “folder.zip”"),
    ("passwords.zip", "Not an archive: “passwords.zip”"),
];

fn write_unextractable(archive: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    if matches!(name, "folder.zip" | "passwords.zip") {
        fs::create_dir(archive)?;
    } else if name != "missing.zip" {
        fs::write(archive, b"not an archive")?;
    }
    if name == "unreadable.zip" {
        fs::set_permissions(archive, fs::Permissions::from_mode(0o000))?;
    }
    if name == "destination.zip" {
        write_zip_stored(archive, &[("file.txt", b"contents")])?;
    }
    Ok(())
}

/// Iterates the main context until the extraction fails or ends, for at most ten seconds.
fn wait_until_finished(events: &Events, name: &str) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Failed { .. } | OperationEvent::Extracted { .. }
        )
    }) {
        assert!(
            Instant::now() < deadline,
            "extraction did not terminate: {name}"
        );
        context.iteration(false);
        std::thread::yield_now();
    }
}

fn expect_no_more_events(events: &Events, name: &str) {
    let context = glib::MainContext::default();
    let count = events.borrow().len();
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        events.borrow().len(),
        count,
        "progress continued after {name} failed"
    );
}

fn expect_failure_stops(root: &Path, name: &str, expected: &str) -> Result<(), Box<dyn Error>> {
    let destination = root.join("destination");
    let unmade_destination = root.join("unmade");
    let archive = root.join(name);
    write_unextractable(&archive, name)?;
    let target = match name {
        "destination.zip" => &archive,
        "folder.zip" => &unmade_destination,
        _ => &destination,
    };
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let handle = LocalOperationProvider.extract(
        extract_request(434, &archive, target),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_until_finished(&events, name);
    assert!(
        matches!(events.borrow().last(), Some(OperationEvent::Failed { message, password_failure: None, .. }) if message.contains(expected)),
        "{name}: {:?}",
        events.borrow()
    );
    expect_no_more_events(&events, name);
    drop(handle);
    assert!(destination.read_dir()?.next().is_none());
    assert!(!unmade_destination.exists());
    assert!(!root.join("outside").exists());
    Ok(())
}

#[test]
fn extraction_failures_stop_progress_and_preserve_error_distinctions() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("destination"))?;
    for (name, expected) in UNEXTRACTABLE {
        expect_failure_stops(root.path(), name, expected)?;
    }
    Ok(())
}

#[test]
fn extraction_provider_sanitizes_parent_paths_without_failure() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive = root.path().join("unsafe.zip");
    write_zip_stored(
        &archive,
        &[("../escaped.txt", b"escaped"), ("after.txt", b"after")],
    )?;

    let events = run_extraction(extract_request(435, &archive, &destination));

    assert!(events.iter().any(|event| matches!(
        event,
        OperationEvent::Extracted { first_name: Some(name), .. } if name == "unsafe"
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, OperationEvent::Failed { .. }))
    );
    expect_contents(
        &[
            (destination.join("unsafe/escaped.txt"), b"escaped"),
            (destination.join("unsafe/after.txt"), b"after"),
        ],
        "sanitized",
    )?;
    assert!(!root.path().join("escaped.txt").exists());
    Ok(())
}

#[test]
fn failed_extraction_removes_a_newly_created_empty_destination() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("fake.zip");
    fs::write(&archive, b"not an archive")?;
    let destination = root.path().join("leftover");
    let on_ui_thread = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = |step: &'static str| {
        let seen = on_ui_thread.clone();
        Box::new(move || {
            seen.lock()
                .expect("steps")
                .push((step, crate::util::ui_thread::is_current()));
        }) as Box<dyn FnMut() + Send>
    };
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![
                (Op::FolderStamp, "fake.zip", 0, seen("archive check")),
                (Op::Unlink, "leftover", 0, seen("cleanup")),
            ],
            ..Quirks::default()
        },
    );
    let events = run_extraction(extract_request(908, &archive, &destination));
    expect_failed(&events);
    assert!(
        !destination.exists(),
        "leftover destination was not cleaned up"
    );
    assert_eq!(
        *on_ui_thread.lock().expect("steps"),
        [("archive check", false), ("cleanup", false)],
        "the archive is checked and the folder removed off the UI thread"
    );
    Ok(())
}

#[test]
fn failed_extraction_preserves_a_pre_existing_destination() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("fake.zip");
    fs::write(&archive, b"not an archive")?;
    let destination = root.path().join("existing");
    fs::create_dir(&destination)?;
    fs::write(destination.join("kept.txt"), b"kept")?;
    let events = run_extraction(extract_request(909, &archive, &destination));
    expect_failed(&events);
    assert!(destination.exists(), "pre-existing destination was removed");
    assert!(
        destination.join("kept.txt").exists(),
        "user content was lost"
    );
    assert_eq!(destination.read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn failed_extraction_removes_a_caller_created_destination() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("broken.zip");
    fs::write(&archive, b"not an archive")?;
    let destination = root.path().join("broken");
    fs::create_dir(&destination)?;
    let events = run_extraction(ExtractRequest {
        created_destination: true,
        ..extract_request(910, &archive, &destination)
    });
    expect_failed(&events);
    assert!(
        !destination.exists(),
        "caller-created destination was not cleaned up"
    );
    Ok(())
}

/// An archive with two safe members and `unsafe_member`, which stops the extraction.
fn write_with_unsafe_member(
    archive: &Path,
    name: &str,
    unsafe_member: &str,
) -> Result<(), Box<dyn Error>> {
    let members: [(&str, &[u8]); 3] = [
        ("ok.txt", b"ok"),
        ("second.txt", b"2"),
        (unsafe_member, b"evil"),
    ];
    if name.ends_with(".zip") {
        write_zip_stored(archive, &members)?;
    } else {
        let entries = members.map(|(member, contents)| (tar::EntryType::Regular, member, contents));
        write_tar_entries(archive, &entries, name.ends_with(".gz"))?;
    }
    Ok(())
}

fn expect_partial_output_kept(
    root: &Path,
    name: &str,
    stem: &str,
    unsafe_member: &str,
) -> Result<(), Box<dyn Error>> {
    let archive = root.join(name);
    write_with_unsafe_member(&archive, name, unsafe_member)?;
    let destination = tempfile::tempdir()?;
    fs::write(destination.path().join("keep.txt"), b"keep")?;

    let events = run_extraction(extract_request(6, &archive, destination.path()));

    let Some(OperationEvent::Failed { message, .. }) = events.last() else {
        panic!("{name}: expected a failure, got {events:?}");
    };
    let mut expected = [stem, "keep.txt"];
    expected.sort_unstable();
    assert_eq!(
        entry_names(destination.path())?,
        expected,
        "{name}: {message}"
    );
    assert_eq!(
        *message,
        format!(
            "Refusing unsafe archive path: {unsafe_member}. Extracted entries remain in “{stem}”."
        ),
        "{name}"
    );
    assert!(
        stages(destination.path(), EXTRACTION_STAGE)?.is_empty(),
        "{name}"
    );
    let output = destination.path().join(stem);
    expect_contents(
        &[
            (output.join("ok.txt"), b"ok"),
            (output.join("second.txt"), b"2"),
            (destination.path().join("keep.txt"), b"keep"),
        ],
        name,
    )
}

#[test]
fn failed_multi_root_extraction_keeps_partial_output_in_the_archive_folder()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let cases = [
        ("abs.zip", "abs", "/etc/evil.txt"),
        ("drive.zip", "drive", "C:/evil.txt"),
        ("abs.tar", "abs", "/etc/evil.txt"),
        ("abs.tar.gz", "abs", "/etc/evil.txt"),
    ];
    for (name, stem, unsafe_member) in cases {
        expect_partial_output_kept(root.path(), name, stem, unsafe_member)?;
    }
    Ok(())
}

#[test]
fn gzip_trailer_failure_is_reported_through_the_provider() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("content.tar.gz");
    let large = vec![b'x'; 50_000];
    write_tar_entries(
        &archive,
        &[
            (tar::EntryType::Regular, "a.txt", &large),
            (tar::EntryType::Regular, "b.txt", b"b"),
        ],
        true,
    )?;
    corrupt_gzip_trailer(&archive, 8)?;
    let destination = tempfile::tempdir()?;

    let events = run_extraction(extract_request(7, &archive, destination.path()));

    let Some(OperationEvent::Failed {
        message,
        password_failure: None,
        ..
    }) = events.last()
    else {
        panic!("a damaged gzip trailer was accepted: {events:?}");
    };
    assert_eq!(
        *message,
        format!("{INVALID_ARCHIVE} Extracted entries remain in “content”.")
    );
    assert_eq!(entry_names(destination.path())?, ["content"]);
    assert_eq!(
        fs::metadata(destination.path().join("content/a.txt"))?.len(),
        50_000
    );
    Ok(())
}
