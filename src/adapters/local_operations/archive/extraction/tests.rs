// SPDX-License-Identifier: MIT

use std::{
    error::Error,
    ffi::OsStr,
    fs,
    io::{self, Read},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    },
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, UNIX_EPOCH},
};

use rustix::io::Errno;

use super::{ArchiveError, ArchiveOutcome, ExtractionSession, MemberContent, MemberMetadata};
use crate::{
    adapters::local_operations::{
        archive::fixtures::{EXTRACTION_STAGE, expected_mode, stages},
        tests::fs_quirks::{FsQuirks, Op, Preset, Quirks, Support, skips_malformed_name_on_fuse},
    },
    model::Location,
    services::{RenameCause, RenamedItem, SkippedItem, SpecialKind, TransferReport},
};

const ARCHIVE: &str = "archive.zip";

struct TestReader<F>(F);

impl<F: FnMut(&mut [u8]) -> io::Result<usize>> Read for TestReader<F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        (self.0)(buffer)
    }
}

#[test]
fn multi_root_members_keep_their_names_beside_existing_entries() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    fs::write(root.path().join("folder/keep.txt"), b"original")?;
    fs::write(root.path().join("report.txt"), b"original")?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;

    session.extract_member("folder", MemberContent::Directory, MemberMetadata::NONE)?;
    session.extract_member(
        "folder/nested/file.txt",
        MemberContent::File(&mut &b"contents"[..], Some(8)),
        MemberMetadata::NONE,
    )?;
    session.extract_member(
        "folder/empty",
        MemberContent::Directory,
        MemberMetadata::NONE,
    )?;
    session.extract_member(
        "report.txt",
        MemberContent::File(&mut io::empty(), Some(0)),
        MemberMetadata::NONE,
    )?;

    assert!(matches!(
        session.finish(Ok(()), || panic!("completion must not enumerate remaining members"))?,
        ArchiveOutcome::Completed(Some(name)) if name == "archive"
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 4);
    assert_eq!(
        fs::read(root.path().join("archive/folder/nested/file.txt"))?,
        b"contents"
    );
    assert!(root.path().join("archive/folder/empty").is_dir());
    assert!(fs::read(root.path().join("archive/report.txt"))?.is_empty());
    assert_eq!(fs::read(root.path().join("folder/keep.txt"))?, b"original");
    assert_eq!(root.path().join("folder").read_dir()?.count(), 1);
    assert_eq!(fs::read(root.path().join("report.txt"))?, b"original");
    assert_eq!(root.path().read_dir()?.count(), 3);
    Ok(())
}

#[test]
fn unsafe_member_paths_are_sanitized_without_stopping_extraction() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open(
        &destination,
        "bundle.zip",
        &progress,
        &cancelled,
        &mut report,
    )?;

    session.extract_member(
        "../escaped.txt",
        MemberContent::File(&mut &b"escaped"[..], Some(7)),
        MemberMetadata::NONE,
    )?;
    session.extract_member(
        "escaped.txt",
        MemberContent::File(&mut &b"duplicate"[..], Some(9)),
        MemberMetadata::NONE,
    )?;
    session.extract_member(
        "after.txt",
        MemberContent::File(&mut &b"after"[..], Some(5)),
        MemberMetadata::NONE,
    )?;

    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(Some(name)) if name == "bundle"
    ));
    assert!(!root.path().join("escaped.txt").exists());
    assert_eq!(progress.load(Ordering::Relaxed), 3);
    assert_eq!(
        fs::read(destination.join("bundle/escaped.txt"))?,
        b"escaped"
    );
    assert_eq!(
        fs::read(destination.join("bundle/escaped (2).txt"))?,
        b"duplicate"
    );
    assert_eq!(fs::read(destination.join("bundle/after.txt"))?, b"after");
    assert_eq!(destination.read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn cancellation_reports_actual_destinations_for_duplicate_members() -> Result<(), Box<dyn Error>> {
    for (name, first, second) in [
        ("same.txt", "archive/same.txt", "archive/same (2).txt"),
        (
            "folder/same.txt",
            "archive/folder/same.txt",
            "archive/folder/same (2).txt",
        ),
    ] {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("folder"))?;
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let mut report = TransferReport::default();
        let mut session =
            ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
        session.extract_member(
            name,
            MemberContent::File(&mut &b"one"[..], Some(3)),
            MemberMetadata::NONE,
        )?;
        session.extract_member(
            name,
            MemberContent::File(&mut &b"two"[..], Some(3)),
            MemberMetadata::NONE,
        )?;
        cancelled.store(true, Ordering::Relaxed);
        let result = session.check_cancelled();
        let ArchiveOutcome::Cancelled {
            completed,
            failed,
            not_attempted,
        } = session.finish(result, Vec::new)?
        else {
            panic!("expected cancellation after both members completed");
        };
        assert_eq!(
            completed,
            [
                Location::local(root.path().join(first)),
                Location::local(root.path().join(second))
            ]
        );
        assert!(failed.is_empty());
        assert!(not_attempted.is_empty());
        assert_eq!(progress.load(Ordering::Relaxed), 2);
        assert_eq!(fs::read(root.path().join(first))?, b"one");
        assert_eq!(fs::read(root.path().join(second))?, b"two");
        assert!(root.path().join("folder").read_dir()?.next().is_none());
    }
    Ok(())
}

#[test]
fn cancellation_before_enumeration_uses_only_supplied_pending_names() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(true);
    let mut report = TransferReport::default();
    let session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    let result = session.check_cancelled();
    let remaining = Location::local(root.path().join("known.txt"));

    assert!(matches!(
        session.finish(result, || vec!["known.txt".to_owned()])?,
        ArchiveOutcome::Cancelled { completed, failed, not_attempted }
            if completed.is_empty() && failed.is_empty() && not_attempted == [remaining]
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    assert!(root.path().read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn cancellation_before_member_processing_never_reads_or_creates_it() -> Result<(), Box<dyn Error>> {
    for directory in [false, true] {
        let root = tempfile::tempdir()?;
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(true);
        let mut report = TransferReport::default();
        let mut session =
            ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
        let mut reader = TestReader(|_: &mut [u8]| panic!("cancelled member must not be read"));
        let content = if directory {
            MemberContent::Directory
        } else {
            MemberContent::File(&mut reader, None)
        };
        let result = session.extract_member("folder/member", content, MemberMetadata::NONE);
        assert_eq!(result, Err(ArchiveError::Cancelled));
        assert!(matches!(
            session.finish(result, Vec::new)?,
            ArchiveOutcome::Cancelled { completed, failed, not_attempted }
                if completed.is_empty() && failed.is_empty()
                    && not_attempted == [Location::local(root.path().join("folder/member"))]
        ));
        assert_eq!(progress.load(Ordering::Relaxed), 0);
        assert!(root.path().read_dir()?.next().is_none());
    }
    Ok(())
}

#[test]
fn mid_copy_cancellation_removes_only_the_partial_file_and_preserves_results()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("partial.txt"), b"original")?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    session.extract_member(
        "done.txt",
        MemberContent::File(&mut &b"done"[..], Some(4)),
        MemberMetadata::NONE,
    )?;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        buffer[..7].copy_from_slice(b"partial");
        cancelled.store(true, Ordering::Relaxed);
        Ok(7)
    });
    let result = session.extract_member(
        "partial.txt",
        MemberContent::File(&mut reader, None),
        MemberMetadata::NONE,
    );
    assert_eq!(result, Err(ArchiveError::Cancelled));
    let kept = |name: &str| Location::local(root.path().join("archive").join(name));
    assert!(matches!(
        session.finish(result, || vec!["later.txt".to_owned()])?,
        ArchiveOutcome::Cancelled { completed, failed, not_attempted }
            if completed == [kept("done.txt")]
                && failed.is_empty()
                && not_attempted == [kept("partial.txt"), kept("later.txt")]
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(root.path().join("archive/done.txt"))?, b"done");
    assert_eq!(fs::read(root.path().join("partial.txt"))?, b"original");
    assert!(!root.path().join("archive/partial.txt").exists());
    assert!(!root.path().join("archive/later.txt").exists());
    assert!(!root.path().join("done.txt").exists());
    Ok(())
}

#[test]
fn cleanup_failure_marks_the_interrupted_location_failed() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        let staging = stages(root.path(), EXTRACTION_STAGE)
            .map_err(|error| io::Error::other(error.to_string()))?
            .pop()
            .ok_or_else(|| io::Error::other("no extraction staging folder"))?;
        let staged = root.path().join(staging).join("partial.txt");
        // A directory replacement makes unlink fail even when tests run as root.
        fs::remove_file(&staged)?;
        fs::create_dir(&staged)?;
        buffer[0] = b'x';
        cancelled.store(true, Ordering::Relaxed);
        Ok(1)
    });
    let result = session.extract_member(
        "partial.txt",
        MemberContent::File(&mut reader, None),
        MemberMetadata::NONE,
    );
    assert_eq!(result, Err(ArchiveError::Cancelled));
    let path = root.path().join("archive/partial.txt");
    let later = Location::local(root.path().join("archive/later.txt"));
    assert!(matches!(
        session.finish(result, || vec!["later.txt".to_owned()])?,
        ArchiveOutcome::Cancelled { completed, failed, not_attempted }
            if completed.is_empty() && failed == [Location::local(&path)]
                && not_attempted == [later]
    ));
    assert!(path.is_dir());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn read_failure_removes_partial_output_without_reporting_cancellation() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    session.extract_member(
        "done.txt",
        MemberContent::File(&mut &b"done"[..], Some(4)),
        MemberMetadata::NONE,
    )?;
    let mut first_read = true;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        if !first_read {
            return Err(io::Error::other("broken stream"));
        }
        first_read = false;
        buffer[..7].copy_from_slice(b"partial");
        Ok(7)
    });
    let result = session.extract_member(
        "partial.txt",
        MemberContent::File(&mut reader, None),
        MemberMetadata::NONE,
    );
    assert!(matches!(
        session.finish(result, || panic!("failure must not enumerate remaining members")),
        Err(ArchiveError::Failed(message))
            if message == "broken stream. Extracted entries remain in “archive”."
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(root.path().join("archive/done.txt"))?, b"done");
    assert!(!root.path().join("archive/partial.txt").exists());
    assert_eq!(root.path().read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn completed_worker_is_not_reclassified_by_late_cancellation() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    session.extract_member(
        "empty.txt",
        MemberContent::File(&mut io::empty(), Some(0)),
        MemberMetadata::NONE,
    )?;
    cancelled.store(true, Ordering::Relaxed);
    assert!(
        matches!(session.finish(Ok(()), Vec::new)?, ArchiveOutcome::Completed(Some(name)) if name == "empty.txt")
    );
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn pending_names_are_sanitized_and_kept_under_the_archive_folder() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    fs::write(root.path().join("unvisited.txt"), b"original")?;
    std::os::unix::fs::symlink("missing", root.path().join("redirect"))?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    session.extract_member(
        "folder/first.txt",
        MemberContent::File(&mut &b"done"[..], Some(4)),
        MemberMetadata::NONE,
    )?;
    cancelled.store(true, Ordering::Relaxed);
    let result = session.check_cancelled();
    let outcome = session.finish(result, || {
        [
            "folder/./next.txt",
            r"folder\nested\later.txt",
            "unvisited.txt",
            "unvisited.txt",
            "missing/new.txt",
            "redirect/child",
            "../outside",
            "/outside",
            "C:drive",
            "",
            ".",
        ]
        .map(str::to_owned)
        .to_vec()
    })?;
    let ArchiveOutcome::Cancelled {
        completed,
        failed,
        not_attempted,
    } = outcome
    else {
        panic!("expected cancellation after one member");
    };
    let kept = |name: &str| Location::local(root.path().join("archive").join(name));
    assert_eq!(completed, [kept("folder/first.txt")]);
    assert!(failed.is_empty());
    assert_eq!(
        not_attempted,
        [
            "folder/next.txt",
            "folder/nested/later.txt",
            "unvisited.txt",
            "unvisited.txt",
            "missing/new.txt",
            "redirect/child",
            "outside"
        ]
        .map(kept)
    );
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(root.path().join("unvisited.txt"))?, b"original");
    assert_eq!(root.path().read_dir()?.count(), 4);
    assert_eq!(root.path().join("archive/folder").read_dir()?.count(), 1);
    assert!(root.path().join("folder").read_dir()?.next().is_none());
    assert_eq!(
        fs::read_link(root.path().join("redirect"))?,
        Path::new("missing")
    );
    Ok(())
}

#[test]
fn member_cancelled_before_creation_is_reported_inside_the_archive_folder()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    session.extract_member(
        "folder/first.txt",
        MemberContent::File(&mut io::empty(), Some(0)),
        MemberMetadata::NONE,
    )?;
    cancelled.store(true, Ordering::Relaxed);
    let mut reader = TestReader(|_: &mut [u8]| panic!("cancelled member must not be read"));
    let result = session.extract_member(
        "folder/next.txt",
        MemberContent::File(&mut reader, None),
        MemberMetadata::NONE,
    );
    assert!(matches!(session.finish(result, Vec::new)?,
        ArchiveOutcome::Cancelled { not_attempted, .. }
            if not_attempted == [Location::local(root.path().join("archive/folder/next.txt"))]
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert!(!root.path().join("archive/folder/next.txt").exists());
    assert!(root.path().join("folder").read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn empty_session_completes_without_a_first_name() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(None)
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    assert!(root.path().read_dir()?.next().is_none());
    Ok(())
}

fn failed_extract(session: ExtractionSession<'_>, result: Result<(), ArchiveError>) -> String {
    assert!(
        matches!(result, Err(ArchiveError::Failed(_))),
        "expected a failed member, got {result:?}"
    );
    match session.finish(result, || {
        panic!("failure must not enumerate remaining members")
    }) {
        Err(ArchiveError::Failed(message)) => message,
        other => panic!("expected a failed extraction, got {other:?}"),
    }
}

#[test]
fn declared_size_overflow_removes_partial_output() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(1024),
        &mut report,
    )?;

    let result = session.extract_member(
        "overflow.txt",
        MemberContent::File(&mut &b"abcdefgh"[..], Some(4)),
        MemberMetadata::NONE,
    );
    let message = failed_extract(session, result);

    assert_eq!(
        message,
        "Archive member “overflow.txt” declared 4 bytes but produced more"
    );
    assert!(root.path().read_dir()?.next().is_none());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn declared_size_shortfall_removes_partial_output() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;

    let result = session.extract_member(
        "short.txt",
        MemberContent::File(&mut &b"four"[..], Some(8)),
        MemberMetadata::NONE,
    );
    let message = failed_extract(session, result);

    assert_eq!(
        message,
        "Archive member “short.txt” declared 8 bytes but produced 4 bytes"
    );
    assert!(root.path().read_dir()?.next().is_none());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn member_preflight_refuses_when_destination_lacks_space() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(4),
        &mut report,
    )?;
    let mut reader = TestReader(|_: &mut [u8]| panic!("member that cannot fit must not be read"));

    let result = session.extract_member(
        "huge.txt",
        MemberContent::File(&mut reader, Some(8)),
        MemberMetadata::NONE,
    );
    let message = failed_extract(session, result);

    assert!(
        message.contains("declared 8 B, but only 4 B is free"),
        "{message}"
    );
    assert!(!message.contains("remain in"), "{message}");
    assert!(root.path().read_dir()?.next().is_none());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn copy_without_declared_size_stops_at_free_space() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(4),
        &mut report,
    )?;

    let result = session.extract_member(
        "payload.txt",
        MemberContent::File(&mut &b"12345678"[..], None),
        MemberMetadata::NONE,
    );
    let message = failed_extract(session, result);

    assert_eq!(
        message,
        "Not enough free space at the destination to extract “payload.txt” (4 B available)"
    );
    assert!(root.path().read_dir()?.next().is_none());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn claimed_total_preflight_refuses_before_any_member() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(10),
        &mut report,
    )?;

    let result = session.preflight_claimed_size(100);
    let message = failed_extract(session, result);

    assert!(
        message.contains(
            "Archive declared size (100 B) exceeds the free space at the destination (10 B)"
        ),
        "{message}"
    );
    assert!(!message.contains("remain in"), "{message}");
    assert!(root.path().read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn second_member_preflight_uses_remaining_space() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(10),
        &mut report,
    )?;
    session.extract_member(
        "first.txt",
        MemberContent::File(&mut &b"12345678"[..], Some(8)),
        MemberMetadata::NONE,
    )?;
    let mut reader =
        TestReader(|_: &mut [u8]| panic!("second member that cannot fit must not be read"));

    let result = session.extract_member(
        "second.txt",
        MemberContent::File(&mut reader, Some(4)),
        MemberMetadata::NONE,
    );
    let message = failed_extract(session, result);

    assert!(
        message.contains("declared 4 B, but only 2 B is free"),
        "{message}"
    );
    assert!(
        message.ends_with("free at the destination. Extracted entries remain in “archive”."),
        "{message}"
    );
    assert_eq!(
        fs::read(root.path().join("archive/first.txt"))?,
        b"12345678"
    );
    assert!(!root.path().join("archive/second.txt").exists());
    assert_eq!(root.path().read_dir()?.count(), 1);
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn matching_declared_size_completes_under_an_injected_quota() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(16),
        &mut report,
    )?;
    session.extract_member(
        "ok.txt",
        MemberContent::File(&mut &b"contents"[..], Some(8)),
        MemberMetadata::NONE,
    )?;
    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(Some(name)) if name == "ok.txt"
    ));
    assert_eq!(fs::read(root.path().join("ok.txt"))?, b"contents");
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn unreported_free_space_skips_capacity_checks() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        None,
        &mut report,
    )?;

    session.preflight_claimed_size(u128::MAX)?;
    session.extract_member(
        "declared.txt",
        MemberContent::File(&mut &b"12345678"[..], Some(8)),
        MemberMetadata::NONE,
    )?;
    session.extract_member(
        "undeclared.txt",
        MemberContent::File(&mut &b"12345678"[..], None),
        MemberMetadata::NONE,
    )?;

    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(Some(name)) if name == "archive"
    ));
    assert_eq!(
        fs::read(root.path().join("archive/declared.txt"))?,
        b"12345678"
    );
    assert_eq!(
        fs::read(root.path().join("archive/undeclared.txt"))?,
        b"12345678"
    );
    assert_eq!(progress.load(Ordering::Relaxed), 2);
    Ok(())
}

#[test]
fn unreported_free_space_still_enforces_declared_size() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        None,
        &mut report,
    )?;

    let result = session.extract_member(
        "overflow.txt",
        MemberContent::File(&mut &b"abcdefgh"[..], Some(4)),
        MemberMetadata::NONE,
    );
    let message = failed_extract(session, result);

    assert!(
        message.contains("declared 4 bytes but produced more"),
        "{message}"
    );
    assert!(!message.contains("remain in"), "{message}");
    assert!(root.path().read_dir()?.next().is_none());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

enum Expected {
    Completed(Option<&'static str>),
    Failed(&'static str),
    Cancelled(&'static [&'static str]),
}

type OutcomeCase = (
    &'static [&'static str],
    Result<(), ArchiveError>,
    Expected,
    &'static [&'static str],
);

#[test]
fn finish_publishes_every_outcome_shape() -> Result<(), Box<dyn Error>> {
    let cases: [OutcomeCase; 7] = [
        (&[], Ok(()), Expected::Completed(None), &[]),
        (
            &["only.txt"],
            Ok(()),
            Expected::Completed(Some("only.txt")),
            &["only.txt"],
        ),
        (
            &["a.txt", "b/c.txt"],
            Ok(()),
            Expected::Completed(Some("archive")),
            &["archive/a.txt", "archive/b/c.txt"],
        ),
        (
            &["a.txt"],
            Err(ArchiveError::Failed("boom".to_owned())),
            Expected::Failed("boom. Extracted entries remain in “archive”."),
            &["archive/a.txt"],
        ),
        (
            &["a.txt"],
            Err(ArchiveError::Failed("Damaged.".to_owned())),
            Expected::Failed("Damaged. Extracted entries remain in “archive”."),
            &["archive/a.txt"],
        ),
        (
            &["a.txt"],
            Err(ArchiveError::Cancelled),
            Expected::Cancelled(&["archive/a.txt"]),
            &["archive/a.txt"],
        ),
        (
            &[],
            Err(ArchiveError::Failed("boom".to_owned())),
            Expected::Failed("boom"),
            &[],
        ),
    ];
    for (members, result, expected, files) in cases {
        let root = tempfile::tempdir()?;
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let mut report = TransferReport::default();
        let mut session =
            ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
        for member in members {
            session.extract_member(
                member,
                MemberContent::File(&mut member.as_bytes(), Some(member.len() as u64)),
                MemberMetadata::NONE,
            )?;
        }

        let outcome = session.finish(result, Vec::new);

        let label = format!("{members:?}");
        match expected {
            Expected::Completed(first_name) => assert!(
                matches!(&outcome, Ok(ArchiveOutcome::Completed(name)) if name.as_deref() == first_name),
                "{label}: {outcome:?}"
            ),
            Expected::Failed(message) => assert!(
                matches!(&outcome, Err(ArchiveError::Failed(actual)) if actual == message),
                "{label}: {outcome:?}"
            ),
            Expected::Cancelled(paths) => {
                let paths: Vec<_> = paths
                    .iter()
                    .map(|path| Location::local(root.path().join(path)))
                    .collect();
                assert!(
                    matches!(&outcome, Ok(ArchiveOutcome::Cancelled { completed, failed, not_attempted })
                        if *completed == paths && failed.is_empty() && not_attempted.is_empty()),
                    "{label}: {outcome:?}"
                );
            }
        }
        for file in files {
            let member = Path::new(file)
                .strip_prefix("archive")
                .unwrap_or(Path::new(file));
            assert_eq!(
                fs::read(root.path().join(file))?,
                member.as_os_str().as_encoded_bytes(),
                "{label}"
            );
        }
        let top_level: std::collections::HashSet<_> = files
            .iter()
            .filter_map(|file| Path::new(file).iter().next())
            .collect();
        assert_eq!(root.path().read_dir()?.count(), top_level.len(), "{label}");
    }
    Ok(())
}

#[test]
fn single_root_colliding_with_a_destination_entry_is_suffixed_at_publish()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("readme.txt"), b"existing")?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    session.extract_member(
        "readme.txt",
        MemberContent::File(&mut &b"archived"[..], Some(8)),
        MemberMetadata::NONE,
    )?;

    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(Some(name)) if name == "readme (2).txt"
    ));
    assert_eq!(fs::read(root.path().join("readme.txt"))?, b"existing");
    assert_eq!(fs::read(root.path().join("readme (2).txt"))?, b"archived");
    assert_eq!(root.path().read_dir()?.count(), 2);
    Ok(())
}

#[test]
fn a_failure_that_left_only_directories_leaves_nothing_behind() -> Result<(), Box<dyn Error>> {
    for cancel in [false, true] {
        let root = tempfile::tempdir()?;
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let mut report = TransferReport::default();
        let mut session =
            ExtractionSession::open(root.path(), "secret.7z", &progress, &cancelled, &mut report)?;
        session.extract_member("secret", MemberContent::Directory, MemberMetadata::NONE)?;
        let mut reader = TestReader(|buffer: &mut [u8]| {
            if cancel {
                buffer[0] = b'x';
                cancelled.store(true, Ordering::Relaxed);
                Ok(1)
            } else {
                Err(io::Error::other("The password may be incorrect."))
            }
        });
        let result = session.extract_member(
            "secret/protected.txt",
            MemberContent::File(&mut reader, None),
            MemberMetadata::NONE,
        );

        let outcome = session.finish(result, Vec::new);

        let pending = |name: &str| Location::local(root.path().join(name));
        if cancel {
            assert!(
                matches!(&outcome, Ok(ArchiveOutcome::Cancelled { completed, failed, not_attempted })
                    if completed.is_empty() && failed.is_empty()
                        && *not_attempted == [pending("secret"), pending("secret/protected.txt")]),
                "{outcome:?}"
            );
        } else {
            assert!(
                matches!(&outcome, Err(ArchiveError::Failed(message))
                    if message == "The password may be incorrect."),
                "{outcome:?}"
            );
        }
        assert!(root.path().read_dir()?.next().is_none(), "cancel={cancel}");
    }
    Ok(())
}

#[test]
fn a_session_dropped_before_finish_publishes_its_output() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    session.extract_member(
        "done.txt",
        MemberContent::File(&mut &b"done"[..], Some(4)),
        MemberMetadata::NONE,
    )?;

    drop(session);

    assert_eq!(fs::read(root.path().join("archive/done.txt"))?, b"done");
    assert!(stages(root.path(), EXTRACTION_STAGE)?.is_empty());
    assert_eq!(root.path().read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn links_consume_no_space_and_hard_links_follow_renamed_targets() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(12),
        &mut report,
    )?;
    session.record_hard_link_targets();
    for contents in [b"first!", b"second"] {
        session.extract_member(
            "data.txt",
            MemberContent::File(&mut &contents[..], Some(6)),
            MemberMetadata::NONE,
        )?;
    }
    session.extract_member(
        "lnk",
        MemberContent::Symlink(b"data.txt"),
        MemberMetadata::NONE,
    )?;
    session.extract_member(
        "hard",
        MemberContent::HardLink(Path::new("data.txt")),
        MemberMetadata::NONE,
    )?;

    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(Some(name)) if name == "archive"
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 4);
    let output = root.path().join("archive");
    assert_eq!(fs::read_link(output.join("lnk"))?, Path::new("data.txt"));
    assert_eq!(fs::read(output.join("lnk"))?, b"first!");
    assert_eq!(
        fs::metadata(output.join("hard"))?.ino(),
        fs::metadata(output.join("data (2).txt"))?.ino()
    );
    Ok(())
}

#[test]
fn refused_link_members_leave_no_staging_behind() -> Result<(), Box<dyn Error>> {
    let too_long = [b'a'; 4096];
    for (index, expected) in [
        "is a hard link to “later.txt”, which was not extracted",
        "has an invalid symbolic link target",
        "has an invalid symbolic link target",
        "has an invalid symbolic link target",
    ]
    .into_iter()
    .enumerate()
    {
        let content = match index {
            0 => MemberContent::HardLink(Path::new("later.txt")),
            1 => MemberContent::Symlink(b""),
            2 => MemberContent::Symlink(b"a\0b"),
            _ => MemberContent::Symlink(&too_long),
        };
        let root = tempfile::tempdir()?;
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let mut report = TransferReport::default();
        let mut session =
            ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
        session.record_hard_link_targets();

        let result = session.extract_member("link", content, MemberMetadata::NONE);

        assert!(root.path().read_dir()?.next().is_none(), "case {index}");
        let message = failed_extract(session, result);
        assert_eq!(message, format!("Archive member “link” {expected}"));
        assert!(root.path().read_dir()?.next().is_none(), "case {index}");
        assert_eq!(progress.load(Ordering::Relaxed), 0);
    }
    Ok(())
}

#[test]
fn directory_metadata_is_restored_deepest_first_only_on_completion() -> Result<(), Box<dyn Error>> {
    let read_only = MemberMetadata {
        mode: Some(0o555),
        modified: Some(UNIX_EPOCH + Duration::from_secs(1_000_000_000)),
    };
    for complete in [false, true] {
        let root = tempfile::tempdir()?;
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let mut report = TransferReport::default();
        let mut session =
            ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
        session.extract_member("ro", MemberContent::Directory, read_only)?;
        session.extract_member("ro/sub", MemberContent::Directory, read_only)?;
        session.extract_member(
            "ro/sub/child.txt",
            MemberContent::File(&mut &b"child"[..], Some(5)),
            MemberMetadata::NONE,
        )?;

        let (output, mode) = if complete {
            assert!(matches!(
                session.finish(Ok(()), Vec::new)?,
                ArchiveOutcome::Completed(Some(name)) if name == "ro"
            ));
            (root.path().join("ro"), 0o555)
        } else {
            cancelled.store(true, Ordering::Relaxed);
            let result = session.extract_member(
                "ro/later.txt",
                MemberContent::File(&mut io::empty(), Some(0)),
                MemberMetadata::NONE,
            );
            assert!(matches!(
                session.finish(result, Vec::new)?,
                ArchiveOutcome::Cancelled { .. }
            ));
            (root.path().join("archive/ro"), 0o777)
        };

        assert_eq!(fs::read(output.join("sub/child.txt"))?, b"child");
        for directory in [output.join("sub"), output.clone()] {
            let metadata = fs::metadata(&directory)?;
            assert_eq!(metadata.permissions().mode() & 0o7777, expected_mode(mode));
            assert_eq!(metadata.mtime() == 1_000_000_000, complete);
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

#[test]
fn a_failed_directory_restore_leaves_restored_directories_writable() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _quirks = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::SetTimes, "*", 0, rustix::io::Errno::IO)],
            ..Quirks::default()
        },
    );
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    let read_only = MemberMetadata {
        mode: Some(0o555),
        modified: Some(UNIX_EPOCH + Duration::from_secs(1_000_000_000)),
    };
    session.extract_member("top/ro", MemberContent::Directory, read_only)?;
    session.extract_member(
        "top/ro/child",
        MemberContent::Directory,
        MemberMetadata::NONE,
    )?;

    let message = match session.finish(Ok(()), Vec::new) {
        Err(ArchiveError::Failed(message)) => message,
        other => panic!("expected a failed restore, got {other:?}"),
    };

    assert_eq!(
        message,
        format!(
            "Could not restore the modification time of “top/ro”: {}",
            crate::services::io_error_message(&rustix::io::Errno::IO.into())
        )
    );
    assert!(root.path().read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn a_repeated_directory_member_merges_its_stored_metadata() -> Result<(), Box<dyn Error>> {
    let earlier_time = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let later_time = UNIX_EPOCH + Duration::from_secs(1_100_000_000);
    let unreadable = MemberMetadata {
        mode: Some(0o000),
        modified: None,
    };
    let full = |modified| MemberMetadata {
        mode: Some(0o750),
        modified: Some(modified),
    };
    let time_only = MemberMetadata {
        mode: None,
        modified: Some(later_time),
    };
    // A top-level directory is restored after publication, a nested one in staging.
    for (directory, published, earlier, later, mtime) in [
        ("dir", "dir", unreadable, full(earlier_time), earlier_time),
        (
            "top/dir",
            "top",
            unreadable,
            full(earlier_time),
            earlier_time,
        ),
        (
            "dir",
            "dir",
            full(earlier_time),
            MemberMetadata::NONE,
            earlier_time,
        ),
        ("dir", "dir", full(earlier_time), time_only, later_time),
    ] {
        let label = format!("{directory}: {earlier:?} then {later:?}");
        let root = tempfile::tempdir()?;
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let mut report = TransferReport::default();
        let mut session =
            ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
        session.extract_member(directory, MemberContent::Directory, earlier)?;
        session.extract_member(
            format!("{directory}/file.txt"),
            MemberContent::File(&mut &b"file"[..], Some(4)),
            MemberMetadata::NONE,
        )?;
        session.extract_member(directory, MemberContent::Directory, later)?;

        let outcome = session.finish(Ok(()), Vec::new);

        assert!(
            matches!(&outcome, Ok(ArchiveOutcome::Completed(Some(name))) if name == published),
            "{label}: {outcome:?}"
        );
        let output = root.path().join(directory);
        let metadata = fs::metadata(&output)?;
        assert_eq!(
            metadata.permissions().mode() & 0o7777,
            expected_mode(0o750),
            "{label}"
        );
        assert_eq!(metadata.modified()?, mtime, "{label}");
        assert_eq!(fs::read(output.join("file.txt"))?, b"file", "{label}");
    }
    Ok(())
}

#[test]
fn a_password_failure_that_cannot_discard_its_output_offers_no_retry() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let mut session =
        ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled, &mut report)?;
    session.extract_member(
        "locked/file.txt",
        MemberContent::File(&mut &b"file"[..], Some(4)),
        MemberMetadata::NONE,
    )?;
    let staging = fs::read_dir(root.path())?
        .next()
        .ok_or("no staging folder")??
        .path();
    fs::set_permissions(staging.join("locked"), fs::Permissions::from_mode(0o500))?;

    let outcome = session.finish(
        Err(ArchiveError::PasswordRequired(
            crate::adapters::PASSWORD_REQUIRED.to_owned(),
        )),
        Vec::new,
    );

    let kept = root.path().join("archive/locked");
    let _ = fs::set_permissions(&kept, fs::Permissions::from_mode(0o700));
    let _ = fs::set_permissions(staging.join("locked"), fs::Permissions::from_mode(0o700));
    assert!(
        matches!(&outcome, Err(ArchiveError::Failed(message))
            if message.starts_with(&format!(
                "{} Could not remove the extraction staging folder: Permission denied",
                crate::adapters::PASSWORD_REQUIRED
            )) && message.ends_with(" Extracted entries remain in “archive”.")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(kept.join("file.txt"))?, b"file");
    Ok(())
}

#[derive(Clone, Copy)]
enum Member<'a> {
    Folder,
    File,
    Link(&'a [u8]),
    Hard(&'a str),
}

type Entry<'a> = (&'a [u8], Member<'a>);

fn extract_members(
    destination: &Path,
    archive: &str,
    members: &[(&[u8], Member<'_>)],
    ending: Result<(), ArchiveError>,
    report: &mut TransferReport,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(destination, archive, &progress, &cancelled, report)?;
    session.record_hard_link_targets();
    let result = members.iter().try_for_each(|(name, member)| {
        let content = match member {
            Member::Folder => MemberContent::Directory,
            Member::File => MemberContent::File(&mut &b"data"[..], Some(4)),
            Member::Link(target) => MemberContent::Symlink(target),
            Member::Hard(target) => MemberContent::HardLink(Path::new(target)),
        };
        session.extract_member(OsStr::from_bytes(name), content, MemberMetadata::NONE)
    });
    session.finish(result.and(ending), || {
        vec![format!("10:00/later:x{}.d/next.txt", "a".repeat(300))]
    })
}

fn entries(folder: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut entries = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            let mut name = path.strip_prefix(folder)?.to_string_lossy().into_owned();
            if fs::symlink_metadata(&path)?.is_dir() {
                name.push('/');
                pending.push(path);
            }
            entries.push(name);
        }
    }
    entries.sort();
    Ok(entries)
}

fn renamed(root: &Path, entries: &[(&[u8], &str, RenameCause)]) -> Vec<RenamedItem> {
    entries
        .iter()
        .map(|(requested, saved_as, cause)| RenamedItem {
            requested: Location::local(root.join(OsStr::from_bytes(requested))),
            saved_as: Location::local(root.join(saved_as)),
            cause: *cause,
        })
        .collect()
}

#[test]
fn members_take_the_names_the_destination_stores_and_distinct_folders_stay_apart()
-> Result<(), Box<dyn Error>> {
    use Member::{File, Folder};
    use RenameCause::{Alias, Invalid, Taken};
    let members: &[(&[u8], Member<'_>)] = &[
        (b"10:00", Folder),
        (b"10:00/c?d.txt", File),
        (b"10_00/x.txt", File),
        (b"CON", File),
        (b"aux.c", File),
        (b"aux.c", File),
        (b"Readme.txt", File),
        (b"README.TXT", File),
        (b"dot.", File),
        (b"x\xffy", File),
        (b"x?y", File),
        (b"\xc0\xaf", File),
    ];
    let windows = [
        "CON_",
        "README (2).TXT",
        "Readme.txt",
        "__",
        "10_00 (2)/",
        "10_00 (2)/x.txt",
        "10_00/",
        "10_00/c_d.txt",
        "aux (2).c",
        "aux.c",
        "dot",
        "x_y",
        "x_y (2)",
    ];
    let fixed: &[(&[u8], &str, RenameCause)] = &[
        (b"10:00", "10_00", Invalid),
        (b"10_00/c?d.txt", "10_00/c_d.txt", Invalid),
        (b"10_00", "10_00 (2)", Taken),
        (b"CON", "CON_", Invalid),
        (b"README.TXT", "README (2).TXT", Alias),
        (b"dot.", "dot", Invalid),
        (b"x\xffy", "x_y", Invalid),
        (b"x?y", "x_y (2)", Invalid),
        (b"\xc0\xaf", "__", Invalid),
    ];
    let posix = [
        "CON",
        "README.TXT",
        "Readme.txt",
        "10:00/",
        "10:00/c?d.txt",
        "10_00/",
        "10_00/x.txt",
        "aux (2).c",
        "aux.c",
        "dot.",
        "x?y",
        "x\u{fffd}y",
        "\u{fffd}\u{fffd}",
    ];
    for (preset, listed, reported) in [
        (Quirks::ext4 as Preset, &posix[..], &[][..]),
        (Quirks::vfat, &windows, fixed),
        (Quirks::exfat_fuse, &windows, fixed),
    ] {
        let utf8_only = !reported.is_empty();
        let root = tempfile::tempdir()?;
        if !utf8_only
            && members
                .iter()
                .any(|(name, _)| skips_malformed_name_on_fuse(root.path(), OsStr::from_bytes(name)))
        {
            continue;
        }
        let quirks = FsQuirks::install(root.path(), preset());
        let mut report = TransferReport::default();

        let outcome = extract_members(root.path(), ARCHIVE, members, Ok(()), &mut report)?;

        let mut expected = listed.to_vec();
        expected.sort_unstable();
        assert!(
            matches!(&outcome, ArchiveOutcome::Completed(Some(name)) if name == "archive"),
            "{outcome:?}"
        );
        assert_eq!(entries(&root.path().join("archive"))?, expected);
        assert_eq!(
            report.renamed,
            renamed(&root.path().join("archive"), reported)
        );
        assert_eq!(
            quirks
                .calls()
                .iter()
                .all(|call| call.names.iter().all(|name| name.to_str().is_some())),
            utf8_only,
            "malformed names were sent only where they are stored"
        );
    }
    Ok(())
}

#[test]
fn links_the_destination_cannot_hold_are_left_out_with_hard_links_to_them()
-> Result<(), Box<dyn Error>> {
    use Member::{File, Folder, Hard, Link};
    let venv: &[(&[u8], Member<'_>)] = &[
        (b"bin", Folder),
        (b"bin/python", File),
        (b"bin/python3", Link(b"python")),
        (b"bin/py", Hard("bin/python3")),
        (b"bin/p2", Hard("bin/py")),
        (b"bin/py:3", Link(b"python")),
    ];
    let learned: &[(&[u8], Member<'_>)] = &[
        (b"bin", Folder),
        (b"bin/python", File),
        (b"bin/python3", Link(b"python")),
        (b"bin/pip", Link(b"python")),
    ];
    let per_link: &[(&[u8], Member<'_>)] = &[
        (b"bin", Folder),
        (b"bin/python", File),
        (b"bin/bad", Link(b"far")),
        (b"bin/python3", Link(b"python")),
        (b"bin/10:00", File),
        (b"bin/now", Hard("bin/10:00")),
    ];
    let malformed: &[(&[u8], Member<'_>)] = &[
        (b"bin", Folder),
        (b"bin/python", File),
        (b"bin/bad", Link(b"x\xffy")),
        (b"bin/python3", Link(b"python")),
    ];
    let refusing_one_target = |errno| Quirks {
        fail: vec![(Op::Symlink, "bad", 0, errno)],
        ..Quirks::ntfs3()
    };
    // A stricter name would be `dot`, which a link the mount cannot hold never tries.
    let unheld: &[(&[u8], Member<'_>)] = &[
        (b"bin", Folder),
        (b"bin/python", File),
        (b"bin/dot.", Link(b"python")),
    ];
    let refusing_one_link = Quirks {
        fail: vec![(Op::Symlink, "dot.", 0, Errno::PERM)],
        ..Quirks::ext4()
    };
    let per_link_listed = &["bin/", "bin/10_00", "bin/now", "bin/python", "bin/python3"][..];
    for (quirks, members, listed, skipped, calls) in [
        (
            Quirks::ext4(),
            venv,
            &[
                "bin/",
                "bin/p2",
                "bin/py",
                "bin/py:3",
                "bin/python",
                "bin/python3",
            ][..],
            &[][..],
            2,
        ),
        (
            Quirks::vfat(),
            venv,
            &["bin/", "bin/python"],
            &["python3", "py", "p2", "py:3"],
            0,
        ),
        (
            Quirks::cifs(),
            learned,
            &["bin/", "bin/python"],
            &["python3", "pip"],
            1,
        ),
        (
            refusing_one_target(rustix::io::Errno::INVAL),
            per_link,
            per_link_listed,
            &["bad"],
            2,
        ),
        (
            refusing_one_target(rustix::io::Errno::ILSEQ),
            per_link,
            per_link_listed,
            &["bad"],
            2,
        ),
        (
            Quirks::cifs_posix(),
            malformed,
            &["bin/", "bin/python", "bin/python3"],
            &["bad"],
            1,
        ),
        (
            refusing_one_link,
            unheld,
            &["bin/", "bin/python"],
            &["dot."],
            1,
        ),
    ] {
        let root = tempfile::tempdir()?;
        let quirks = FsQuirks::install(root.path(), quirks);
        let mut report = TransferReport::default();

        let outcome = extract_members(root.path(), ARCHIVE, members, Ok(()), &mut report)?;

        assert!(
            matches!(&outcome, ArchiveOutcome::Completed(Some(name)) if name == "bin"),
            "{outcome:?}"
        );
        assert_eq!(entries(root.path())?, listed);
        assert_eq!(
            report.skipped,
            skipped
                .iter()
                .map(|name| SkippedItem {
                    location: Location::local(root.path().join("bin").join(name)),
                    kind: SpecialKind::Symlink,
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(quirks.count(Op::Symlink), calls, "{listed:?}");
        if listed.contains(&"bin/now") {
            assert_eq!(
                fs::metadata(root.path().join("bin/now"))?.ino(),
                fs::metadata(root.path().join("bin/10_00"))?.ino()
            );
        }
    }
    Ok(())
}

#[test]
fn a_hard_link_the_destination_cannot_hold_is_copied_and_the_rest_still_extracts()
-> Result<(), Box<dyn Error>> {
    use Member::{File, Folder, Hard};
    let members: &[(&[u8], Member<'_>)] = &[
        (b"d", Folder),
        (b"d/a.txt", File),
        (b"d/b.txt", Hard("d/a.txt")),
        (b"d/c.txt", Hard("d/b.txt")),
        (b"d/e.txt", File),
    ];
    let refusing = |errno| Quirks {
        links: Support::Rejected(errno),
        ..Quirks::ext4()
    };
    for (quirks, shares_inode) in [
        (Quirks::ext4(), true),
        (Quirks::vfat(), false),
        (Quirks::exfat_fuse(), false),
        (refusing(Errno::OPNOTSUPP), false),
    ] {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), quirks);
        let mut report = TransferReport::default();

        let outcome = extract_members(root.path(), ARCHIVE, members, Ok(()), &mut report)?;

        assert!(
            matches!(&outcome, ArchiveOutcome::Completed(Some(name)) if name == "d"),
            "{outcome:?}"
        );
        assert_eq!(
            entries(root.path())?,
            ["d/", "d/a.txt", "d/b.txt", "d/c.txt", "d/e.txt"]
        );
        for name in ["a.txt", "b.txt", "c.txt", "e.txt"] {
            assert_eq!(fs::read(root.path().join("d").join(name))?, b"data");
        }
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        let inode = |name| fs::metadata(root.path().join("d").join(name)).map(|meta| meta.ino());
        assert_eq!(inode("a.txt")? == inode("b.txt")?, shares_inode);
        assert_eq!(inode("a.txt")? == inode("c.txt")?, shares_inode);
    }
    Ok(())
}

#[test]
fn a_colon_after_the_first_component_extracts_as_a_name() -> Result<(), Box<dyn Error>> {
    use Member::{File, Folder, Hard};
    let root = tempfile::tempdir()?;
    let mut report = TransferReport::default();
    let members: &[(&[u8], Member<'_>)] = &[
        (b"before.txt", File),
        (b"names", Folder),
        (b"names/c:notes", File),
        (b"names/linked", Hard("names/c:notes")),
        (b"after.txt", File),
    ];

    extract_members(root.path(), ARCHIVE, members, Ok(()), &mut report)?;

    assert_eq!(
        entries(root.path())?,
        [
            "archive/",
            "archive/after.txt",
            "archive/before.txt",
            "archive/names/",
            "archive/names/c:notes",
            "archive/names/linked"
        ]
    );
    assert_eq!(fs::read(root.path().join("archive/names/linked"))?, b"data");
    Ok(())
}

fn refuses_colons(name: &OsStr) -> Option<Errno> {
    name.as_bytes().contains(&b':').then_some(Errno::INVAL)
}

fn refuses_colons_as_bad_messages(name: &OsStr) -> Option<Errno> {
    name.as_bytes().contains(&b':').then_some(Errno::BADMSG)
}

fn refuses_trailing_dots(name: &OsStr) -> Option<Errno> {
    name.as_bytes().ends_with(b".").then_some(Errno::BADMSG)
}

#[test]
fn a_refused_member_name_takes_the_first_stricter_rules_it_passes_and_teaches_only_a_mount_that_learns()
-> Result<(), Box<dyn Error>> {
    use Member::{File, Folder, Hard, Link};
    use RenameCause::Invalid;
    let first: &[(&[u8], Member<'_>)] = &[
        (b"10:00", Folder),
        (b"10:00/note:1.txt", File),
        (b"10:00/py:3", Link(b"note:1.txt")),
        (b"10:00/hard:1", Hard("10:00/note:1.txt")),
        (b"day_1.txt", File),
        (b"day:1.txt", File),
    ];
    let later: &[(&[u8], Member<'_>)] = &[
        (b"dir:1", Folder),
        (b"dir:1/t", File),
        (b"file:1", File),
        (b"dot.", File),
    ];
    let unknown_daemon = || Quirks {
        mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
        invalid_name: Some(refuses_colons_as_bad_messages),
        ..Quirks::default()
    };
    // Known rules never learn. This one also refuses a hard link's name,
    // and the lookup of a name, as sshfs does.
    let strict_ext4 = || Quirks {
        invalid_name: Some(refuses_colons),
        fail: vec![
            (Op::Link, "hard:1", 0, Errno::INVAL),
            (Op::Stat, "day:1.txt", 0, Errno::BADMSG),
        ],
        ..Quirks::ext4()
    };
    type Refused<'a> = &'a [(Op, &'a str)];
    let rows: [(Preset, Refused<'_>, Refused<'_>); 2] = [
        (unknown_daemon, &[(Op::Mkdir, "10:00")], &[]),
        (
            strict_ext4,
            &[
                (Op::Mkdir, "10:00"),
                (Op::Stat, "note:1.txt"),
                (Op::Create, "note:1.txt"),
                (Op::Stat, "py:3"),
                (Op::Symlink, "py:3"),
                (Op::Stat, "hard:1"),
                (Op::Link, "hard:1"),
                (Op::Stat, "day:1.txt"),
            ],
            &[
                (Op::Mkdir, "dir:1"),
                (Op::Stat, "file:1"),
                (Op::Create, "file:1"),
            ],
        ),
    ];
    for (quirks, refused_first, refused_later) in rows {
        let root = tempfile::tempdir()?;
        let quirks = FsQuirks::install(root.path(), quirks());
        let refused_calls = || {
            quirks
                .calls()
                .into_iter()
                .filter_map(|call| {
                    let name = call
                        .names
                        .iter()
                        .find(|name| name.as_bytes().contains(&b':'))?;
                    Some((call.op, name.to_string_lossy().into_owned()))
                })
                .collect::<Vec<_>>()
        };
        let owned = |calls: Refused<'_>| {
            calls
                .iter()
                .map(|(op, name)| (*op, (*name).to_owned()))
                .collect::<Vec<_>>()
        };
        let mut report = TransferReport::default();

        let outcome = extract_members(root.path(), ARCHIVE, first, Ok(()), &mut report)?;

        assert!(
            matches!(&outcome, ArchiveOutcome::Completed(Some(name)) if name == "archive"),
            "{outcome:?}"
        );
        let archive = root.path().join("archive");
        assert_eq!(
            entries(&archive)?,
            [
                "10_00/",
                "10_00/hard_1",
                "10_00/note_1.txt",
                "10_00/py_3",
                "day_1 (2).txt",
                "day_1.txt"
            ]
        );
        assert_eq!(
            report.renamed,
            renamed(
                &archive,
                &[
                    (b"10:00", "10_00", Invalid),
                    (b"10_00/note:1.txt", "10_00/note_1.txt", Invalid),
                    (b"10_00/py:3", "10_00/py_3", Invalid),
                    (b"10_00/hard:1", "10_00/hard_1", Invalid),
                    (b"day:1.txt", "day_1 (2).txt", Invalid),
                ]
            )
        );
        assert_eq!(
            fs::read_link(archive.join("10_00/py_3"))?,
            Path::new("note:1.txt")
        );
        assert_eq!(
            fs::metadata(archive.join("10_00/hard_1"))?.ino(),
            fs::metadata(archive.join("10_00/note_1.txt"))?.ino()
        );
        assert_eq!(refused_calls(), owned(refused_first));
        quirks.clear_calls();
        let mut report = TransferReport::default();

        let outcome = extract_members(root.path(), "later.zip", later, Ok(()), &mut report)?;

        assert!(
            matches!(&outcome, ArchiveOutcome::Completed(Some(name)) if name == "later"),
            "{outcome:?}"
        );
        let archive = root.path().join("later");
        assert_eq!(entries(&archive)?, ["dir_1/", "dir_1/t", "dot.", "file_1"]);
        assert_eq!(
            report.renamed,
            renamed(
                &archive,
                &[(b"dir:1", "dir_1", Invalid), (b"file:1", "file_1", Invalid)]
            )
        );
        assert_eq!(refused_calls(), owned(refused_later));
    }
    Ok(())
}

#[test]
fn a_mount_that_learns_keeps_only_the_rules_its_refused_member_names_needed()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let quirks = FsQuirks::install(
        root.path(),
        Quirks {
            mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
            invalid_name: Some(refuses_trailing_dots),
            ..Quirks::default()
        },
    );
    let refused_creates = || {
        quirks
            .calls()
            .into_iter()
            .filter(|call| call.op == Op::Create)
            .flat_map(|call| call.names)
            .filter(|name| name.as_bytes().ends_with(b"."))
            .map(|name| name.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    let mut report = TransferReport::default();

    extract_members(
        root.path(),
        ARCHIVE,
        &[(b"note:1.", Member::File), (b"x", Member::File)],
        Ok(()),
        &mut report,
    )?;

    assert_eq!(entries(&root.path().join("archive"))?, ["note:1", "x"]);
    assert_eq!(refused_creates(), ["note:1.", "note_1."]);
    quirks.clear_calls();
    let mut report = TransferReport::default();

    extract_members(
        root.path(),
        "later.zip",
        &[(b"text:1", Member::File), (b"dot.", Member::File)],
        Ok(()),
        &mut report,
    )?;

    let later = root.path().join("later");
    assert_eq!(entries(&later)?, ["dot", "text:1"]);
    assert_eq!(
        report.renamed,
        renamed(&later, &[(b"dot.", "dot", RenameCause::Invalid)])
    );
    assert!(refused_creates().is_empty());
    Ok(())
}

#[test]
fn members_the_destination_will_not_make_fail_the_extraction_naming_them_and_why()
-> Result<(), Box<dyn Error>> {
    use Member::{File, Folder, Hard, Link};
    let invalid = "The name isn’t valid at this location";
    let io_failed = crate::services::io_error_message(&Errno::IO.into());
    // Only `py:3` has a stricter name; the destination refuses it too, and
    // the first refusal is the one reported.
    type Fails<'a> = &'a [(Op, &'a str, Errno)];
    let rows: [(Entry<'_>, Fails<'_>, String); 10] = [
        (
            (b"bin/python3", Link(b"python")),
            &[(Op::Symlink, "python3", Errno::NOSPC)],
            "Could not create symbolic link “bin/python3”: There is not enough space on the device"
                .to_owned(),
        ),
        (
            (b"bin/python3", Link(b"python")),
            &[(Op::Symlink, "python3", Errno::BADMSG)],
            format!("Could not create symbolic link “bin/python3”: {invalid}"),
        ),
        (
            (b"bin/py:3", Link(b"python")),
            &[
                (Op::Symlink, "py:3", Errno::BADMSG),
                (Op::Symlink, "py_3", Errno::NAMETOOLONG),
            ],
            format!("Could not create symbolic link “bin/py:3”: {invalid}"),
        ),
        (
            (b"bin/lib", Folder),
            &[(Op::Mkdir, "lib", Errno::INVAL)],
            format!("Could not create “bin/lib”: {invalid}"),
        ),
        (
            (b"lib/x", File),
            &[(Op::Mkdir, "lib", Errno::BADMSG)],
            format!("Could not create “lib”: {invalid}"),
        ),
        (
            (b"bin/plain", File),
            &[(Op::Create, "plain", Errno::BADMSG)],
            format!("Could not create “bin/plain”: {invalid}"),
        ),
        (
            (b"bin/plain", File),
            &[(Op::Stat, "plain", Errno::ILSEQ)],
            format!("Could not create “bin/plain”: {invalid}"),
        ),
        (
            (b"bin/plain", File),
            &[(Op::Create, "plain", Errno::NAMETOOLONG)],
            "Could not create “bin/plain”: The name is too long".to_owned(),
        ),
        (
            (b"bin/hard", Hard("bin/python")),
            &[(Op::Link, "hard", Errno::INVAL)],
            format!("Could not link “bin/hard” to “bin/python”: {invalid}"),
        ),
        // The copy that stands in for a hard link the mount cannot hold
        // reports why its target could not be read, not the refused link.
        (
            (b"bin/hard", Hard("bin/python")),
            &[
                (Op::Link, "hard", Errno::PERM),
                (Op::OpenSource, "python", Errno::IO),
            ],
            format!("Could not link “bin/hard” to “bin/python”: {io_failed}"),
        ),
    ];
    for (member, fails, failure) in rows {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                fail: fails
                    .iter()
                    .map(|(op, name, errno)| (*op, *name, 0, *errno))
                    .collect(),
                ..Quirks::ext4()
            },
        );
        let mut report = TransferReport::default();

        let outcome = extract_members(
            root.path(),
            ARCHIVE,
            &[(b"bin/python", File), member],
            Ok(()),
            &mut report,
        );

        let Err(ArchiveError::Failed(message)) = outcome else {
            panic!("{fails:?} did not fail: {outcome:?}");
        };
        assert_eq!(
            message,
            format!("{failure}. Extracted entries remain in “archive”.")
        );
        assert!(report.skipped.is_empty());
    }
    Ok(())
}

#[test]
fn the_report_names_entries_where_the_extraction_left_them() -> Result<(), Box<dyn Error>> {
    use Member::{File, Folder, Link};
    use RenameCause::{Invalid, TooLong};
    type Outcome = Result<ArchiveOutcome<Option<String>>, ArchiveError>;
    let nested: &[(&[u8], Member<'_>)] = &[(b"10:00", Folder), (b"10:00/c?d", File)];
    let nested_fixed: &[(&[u8], &str, RenameCause)] = &[
        (b"archive/10:00", "archive/10_00", Invalid),
        (b"archive/10_00/c?d", "archive/10_00/c_d", Invalid),
    ];
    let long = format!("{}.zip", "a".repeat(256));
    let cut = "a".repeat(255);
    type Row<'a> = (
        Preset,
        &'a str,
        Option<&'a str>,
        &'a [Entry<'a>],
        Result<(), ArchiveError>,
        fn(&Outcome) -> bool,
        &'a [(&'a [u8], &'a str, RenameCause)],
        &'a [&'a str],
    );
    let rows: [Row<'_>; 9] = [
        (
            Quirks::vfat,
            ARCHIVE,
            Some("10_00"),
            nested,
            Ok(()),
            |outcome| matches!(outcome, Ok(ArchiveOutcome::Completed(Some(name))) if name == "10_00 (2)"),
            &[
                (b"10:00".as_slice(), "10_00 (2)", Invalid),
                (b"10_00 (2)/c?d", "10_00 (2)/c_d", Invalid),
            ],
            &[][..],
        ),
        (
            Quirks::vfat,
            "10:00.zip",
            None,
            &[(b"a.txt", File), (b"b?.txt", File)],
            Ok(()),
            |outcome| matches!(outcome, Ok(ArchiveOutcome::Completed(Some(name))) if name == "10_00"),
            &[
                (b"10:00".as_slice(), "10_00", Invalid),
                (b"10_00/b?.txt", "10_00/b_.txt", Invalid),
            ],
            &[],
        ),
        (
            Quirks::vfat,
            "10:00.zip",
            Some("10_00"),
            &[(b"a.txt", File), (b"b?.txt", File)],
            Ok(()),
            |outcome| matches!(outcome, Ok(ArchiveOutcome::Completed(Some(name))) if name == "10_00 (1)"),
            &[
                (b"10:00".as_slice(), "10_00 (1)", Invalid),
                (b"10_00 (1)/b?.txt", "10_00 (1)/b_.txt", Invalid),
            ],
            &[],
        ),
        (
            Quirks::ext4,
            &long,
            None,
            &[(b"a.txt", File), (b"b.txt", File)],
            Ok(()),
            |outcome| matches!(outcome, Ok(ArchiveOutcome::Completed(Some(name))) if name.len() == 255),
            &[(&long.as_bytes()[..256], &cut, TooLong)],
            &[],
        ),
        (
            Quirks::vfat,
            ARCHIVE,
            None,
            nested,
            Err(ArchiveError::Failed("Damaged.".to_owned())),
            |outcome| matches!(outcome, Err(ArchiveError::Failed(message)) if message.ends_with("remain in “archive”.")),
            nested_fixed,
            &[],
        ),
        (
            Quirks::vfat,
            ARCHIVE,
            None,
            nested,
            Err(ArchiveError::Cancelled),
            |outcome| matches!(outcome, Ok(ArchiveOutcome::Cancelled { .. })),
            nested_fixed,
            &[],
        ),
        (
            Quirks::vfat,
            ARCHIVE,
            None,
            nested,
            Err(ArchiveError::PasswordRequired(String::new())),
            |outcome| matches!(outcome, Err(ArchiveError::PasswordRequired(_))),
            &[],
            &[],
        ),
        (
            Quirks::vfat,
            ARCHIVE,
            None,
            &[(b"lnk", Link(b"target"))],
            Ok(()),
            |outcome| matches!(outcome, Ok(ArchiveOutcome::Completed(None))),
            &[],
            &["lnk"],
        ),
        (
            Quirks::vfat,
            ARCHIVE,
            None,
            &[(b"bin/lnk", Link(b"target"))],
            Ok(()),
            |outcome| matches!(outcome, Ok(ArchiveOutcome::Completed(Some(name))) if name == "bin"),
            &[],
            &["bin/lnk"],
        ),
    ];
    for (preset, archive, existing, members, ending, finished, renamed_as, skipped) in rows {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), preset());
        if let Some(existing) = existing {
            fs::create_dir(root.path().join(existing))?;
        }
        let mut report = TransferReport::default();

        let outcome = extract_members(root.path(), archive, members, ending, &mut report);

        assert!(finished(&outcome), "{archive}: {outcome:?}");
        assert_eq!(
            report.renamed,
            renamed(root.path(), renamed_as),
            "{outcome:?}"
        );
        assert_eq!(
            report.skipped,
            skipped
                .iter()
                .map(|name| SkippedItem {
                    location: Location::local(root.path().join(name)),
                    kind: SpecialKind::Symlink,
                })
                .collect::<Vec<_>>()
        );
        if let Ok(ArchiveOutcome::Cancelled { not_attempted, .. }) = &outcome {
            let pending = format!("archive/10_00/later_x{}/next.txt", "a".repeat(248));
            assert_eq!(*not_attempted, [Location::local(root.path().join(pending))]);
        }
    }
    Ok(())
}
