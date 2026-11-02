// SPDX-License-Identifier: MIT

use std::os::unix::{ffi::OsStrExt, fs::symlink};

use super::*;

/// Lists a fixture folder of `count` numbered files with the request `shape` builds.
fn enumerate_numbered(
    label: &str,
    count: usize,
    shape: impl FnOnce(DirectoryRequest) -> DirectoryRequest,
) -> Vec<DirectoryEvent> {
    let root = FixtureDir::new(label);
    root.write_numbered(count);
    let events = run_enumerate(shape(directory_request(root.location())));
    root.remove();
    events
}

/// The capability `attribute` the folder's one file answers, and the load's finished flags.
fn listed_beside_child(label: &str, attribute: &str) -> (bool, Option<FinishedFlags>) {
    let root = FixtureDir::new(label);
    let child = root.write("child.txt", b"content");
    let expected = gio::File::for_path(&child)
        .query_info(
            attribute,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )
        .expect("the child capability query should succeed")
        .boolean(attribute);

    let events = run_enumerate(listing_request(root.path(), 10));
    root.remove();
    (expected, finished_flags(&events))
}

fn hidden_flag(entries: &[FileEntry], name: &str) -> Option<bool> {
    entries
        .iter()
        .find(|entry| entry.native_name == name)
        .map(|entry| entry.is_hidden)
}

#[test]
fn invalid_utf8_names_keep_their_native_bytes() {
    let directory = FixtureDir::new("native-name");
    let native_name = OsString::from_vec(b"invalid-\xff".to_vec());
    let path = directory.path().join(&native_name);
    fs::write(&path, b"fixture").expect("the fixture file should be written");

    let events = run_enumerate(directory_request(directory.location()));
    let entry = batched_entries(&events)
        .into_iter()
        .next()
        .expect("the invalid UTF-8 entry should be listed");

    assert_eq!(entry.native_name.as_bytes(), native_name.as_bytes());
    assert_eq!(entry.location.native_path(), Some(path.as_path()));
    assert!(!entry.display_name.is_empty());

    directory.remove();
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
    let directory = FixtureDir::new("symlink");
    fs::create_dir(directory.path().join("directory"))?;
    directory.write("file", b"fixture");
    for (target, link) in [
        ("directory", "directory-link"),
        ("file", "file-link"),
        ("missing", "broken-link"),
    ] {
        symlink(target, directory.path().join(link))?;
    }

    let events = run_enumerate(DirectoryRequest {
        include_metadata: true,
        ..directory_request(directory.location())
    });
    let entries = batched_entries(&events);
    let kind = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.native_name == name)
            .map(|entry| entry.kind)
            .expect("the symlink should be listed")
    };

    assert_eq!(
        ["directory-link", "file-link", "broken-link"].map(kind),
        [
            EntryKind::DirectorySymbolicLink,
            EntryKind::FileSymbolicLink,
            EntryKind::SymbolicLink
        ]
    );

    directory.remove();
    Ok(())
}

#[test]
fn enumerate_reports_truncated_once_the_entry_budget_is_exceeded() {
    let events = enumerate_numbered("entry-budget", 5, |request| DirectoryRequest {
        batch_size: 2,
        max_entries: 3,
        ..request
    });

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

        let events = run_enumerate(directory_request(Location::local(root.path())));

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
                failure_message(&events),
            ),
            (listed, 1, chunks, failure)
        );
    }
    Ok(())
}

#[test]
fn enumerate_resolves_can_trash_from_a_child_entry() {
    let (expected, finished) =
        listed_beside_child("can-trash", gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH);

    assert_eq!(
        finished.map(|(_, can_trash, _)| can_trash),
        Some(Some(expected))
    );
}

#[test]
fn enumerate_resolves_can_delete_from_a_child_entry() {
    let (expected, finished) =
        listed_beside_child("can-delete", gio::FILE_ATTRIBUTE_ACCESS_CAN_DELETE);

    assert_eq!(
        finished.map(|(_, _, can_delete)| can_delete),
        Some(Some(expected))
    );
}

#[test]
fn entry_limited_metadata_load_fills_every_retained_entry() {
    let events = enumerate_numbered("entry-budget-metadata", 5, |request| DirectoryRequest {
        batch_size: 2,
        include_metadata: true,
        max_entries: 3,
        ..request
    });

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
    let events = enumerate_numbered("exact-entry-budget", 4, |request| DirectoryRequest {
        batch_size: 2,
        max_entries: 4,
        ..request
    });

    assert_eq!(
        finished_truncated(&events),
        Some(false),
        "reaching the entry budget is not truncation when the directory is complete"
    );
    assert_eq!(batched_entry_count(&events), 4);
}

#[test]
fn enumerate_reports_truncated_once_the_time_budget_is_exceeded() {
    let root = FixtureDir::new("time-budget");
    root.write("needle.txt", b"content");
    root.write("second.txt", b"content");

    let events = run_enumerate(DirectoryRequest {
        batch_size: 1,
        max_entries: usize::MAX,
        time_budget: Duration::from_nanos(1),
        ..directory_request(root.location())
    });
    root.remove();

    assert_eq!(
        finished_truncated(&events),
        Some(true),
        "an exhausted time budget should stop the load and report truncation"
    );
}

#[test]
fn enumerate_completes_untruncated_within_budget() {
    let events = enumerate_numbered("within-budget", 5, |request| request);

    assert_eq!(
        finished_truncated(&events),
        Some(false),
        "a directory well within budget should not be reported as truncated"
    );
    assert_eq!(batched_entry_count(&events), 5);
}

#[test]
fn native_enumeration_marks_hidden_files() {
    let root = FixtureDir::new("hidden");
    root.write("visible.txt", b"content");
    root.write(".hidden.txt", b"content");
    root.write("listed-hidden.txt", b"content");
    root.write(".hidden", b"listed-hidden.txt\n");

    let entries = batched_entries(&run_enumerate(directory_request(root.location())));
    assert_eq!(entries.len(), 4);
    assert_eq!(hidden_flag(&entries, "visible.txt"), Some(false));
    for name in [".hidden.txt", "listed-hidden.txt", ".hidden"] {
        assert_eq!(
            hidden_flag(&entries, name),
            Some(true),
            "{name} should be marked hidden"
        );
    }

    root.remove();
}

#[test]
fn native_enumeration_reports_an_unreadable_root() {
    let root = unique_fixture_root("missing-root");
    let events = run_enumerate(directory_request(Location::local(root)));

    assert!(matches!(events.as_slice(), [DirectoryEvent::Failed { .. }]));
}

#[test]
fn cancelled_native_scan_returns_no_partial_result() {
    let root = FixtureDir::new("cancelled");
    root.write("file.txt", b"content");
    let request = directory_request(root.location());
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();

    assert!(matches!(
        scan_native_directory(
            root.path(),
            &request,
            &cancellable,
            Instant::now() + Duration::from_secs(10),
            &Progress::unlaned(),
            oneshot::channel().0,
        ),
        NativeEnumeration::Cancelled
    ));

    root.remove();
}

#[test]
fn enumerate_with_metadata_fills_sizes_up_front() {
    let root = FixtureDir::new("with-metadata");
    let path = root.write("file-0.txt", b"content");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
        .expect("the fixture permissions should be set");

    let with_metadata = run_enumerate(DirectoryRequest {
        include_metadata: true,
        ..directory_request(root.location())
    });
    let streaming = run_enumerate(DirectoryRequest {
        id: RequestId(2),
        ..directory_request(root.location())
    });
    root.remove();

    let metadata = |events: &[DirectoryEvent]| {
        batched_entries(events)
            .into_iter()
            .map(|entry| (entry.size, entry.mode))
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
}

#[test]
fn hostile_hidden_files_are_ignored_without_blocking_or_following() {
    let root = FixtureDir::new("hidden-hostile");
    let hidden = root.path().join(".hidden");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &hidden,
        rustix::fs::Mode::from_bits_truncate(0o600),
    )
    .expect("the fifo should be created");
    assert!(native_hidden_names(root.path()).is_empty());

    fs::remove_file(&hidden).expect("the fifo should be removed");
    let target = root.write("target", b"secret\n");
    symlink(&target, &hidden).expect("the symlink should be created");
    assert!(native_hidden_names(root.path()).is_empty());
    root.remove();
}
