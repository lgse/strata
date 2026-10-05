// SPDX-License-Identifier: MIT

use std::{
    error::Error,
    fs,
    io::{self, Read},
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::{ArchiveError, ArchiveOutcome, ExtractionSession, MemberContent};
use crate::{
    adapters::local_operations::archive::fixtures::{EXTRACTION_STAGE, stages},
    model::Location,
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;

    session.extract_member("folder", MemberContent::Directory)?;
    session.extract_member(
        "folder/nested/file.txt",
        MemberContent::File(&mut &b"contents"[..], Some(8)),
    )?;
    session.extract_member("folder/empty", MemberContent::Directory)?;
    session.extract_member("report.txt", MemberContent::File(&mut io::empty(), Some(0)))?;

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
    let mut session = ExtractionSession::open(&destination, "bundle.zip", &progress, &cancelled)?;

    session.extract_member(
        "../escaped.txt",
        MemberContent::File(&mut &b"escaped"[..], Some(7)),
    )?;
    session.extract_member(
        "escaped.txt",
        MemberContent::File(&mut &b"duplicate"[..], Some(9)),
    )?;
    session.extract_member(
        "after.txt",
        MemberContent::File(&mut &b"after"[..], Some(5)),
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
        let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
        session.extract_member(name, MemberContent::File(&mut &b"one"[..], Some(3)))?;
        session.extract_member(name, MemberContent::File(&mut &b"two"[..], Some(3)))?;
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
    let session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
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
        let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
        let mut reader = TestReader(|_: &mut [u8]| panic!("cancelled member must not be read"));
        let content = if directory {
            MemberContent::Directory
        } else {
            MemberContent::File(&mut reader, None)
        };
        let result = session.extract_member("folder/member", content);
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
    session.extract_member("done.txt", MemberContent::File(&mut &b"done"[..], Some(4)))?;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        buffer[..7].copy_from_slice(b"partial");
        cancelled.store(true, Ordering::Relaxed);
        Ok(7)
    });
    let result = session.extract_member("partial.txt", MemberContent::File(&mut reader, None));
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
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
    let result = session.extract_member("partial.txt", MemberContent::File(&mut reader, None));
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
    session.extract_member("done.txt", MemberContent::File(&mut &b"done"[..], Some(4)))?;
    let mut first_read = true;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        if !first_read {
            return Err(io::Error::other("broken stream"));
        }
        first_read = false;
        buffer[..7].copy_from_slice(b"partial");
        Ok(7)
    });
    let result = session.extract_member("partial.txt", MemberContent::File(&mut reader, None));
    assert!(matches!(
        session.finish(result, || panic!("failure must not enumerate remaining members")),
        Err(ArchiveError::Failed(message))
            if message == "broken stream. Extracted entries remain in `archive`."
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
    session.extract_member("empty.txt", MemberContent::File(&mut io::empty(), Some(0)))?;
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
    session.extract_member(
        "folder/first.txt",
        MemberContent::File(&mut &b"done"[..], Some(4)),
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
    assert_eq!(fs::read_link(root.path().join("redirect"))?, Path::new("missing"));
    Ok(())
}

#[test]
fn member_cancelled_before_creation_is_reported_inside_the_archive_folder()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
    session.extract_member(
        "folder/first.txt",
        MemberContent::File(&mut io::empty(), Some(0)),
    )?;
    cancelled.store(true, Ordering::Relaxed);
    let mut reader = TestReader(|_: &mut [u8]| panic!("cancelled member must not be read"));
    let result = session.extract_member("folder/next.txt", MemberContent::File(&mut reader, None));
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
    let session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(None)
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    assert!(root.path().read_dir()?.next().is_none());
    Ok(())
}

/// Finishes the session with `result`, which must be a failure, and returns the reported message.
fn failed_extract(session: ExtractionSession<'_>, result: Result<(), ArchiveError>) -> String {
    assert!(
        matches!(result, Err(ArchiveError::Failed(_))),
        "expected a failed member, got {result:?}"
    );
    match session.finish(result, || panic!("failure must not enumerate remaining members")) {
        Err(ArchiveError::Failed(message)) => message,
        other => panic!("expected a failed extraction, got {other:?}"),
    }
}

#[test]
fn declared_size_overflow_removes_partial_output() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(1024),
    )?;

    let result = session.extract_member(
        "overflow.txt",
        MemberContent::File(&mut &b"abcdefgh"[..], Some(4)),
    );
    let message = failed_extract(session, result);

    assert_eq!(
        message,
        "Archive member `overflow.txt` declared 4 bytes but produced more"
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;

    let result =
        session.extract_member("short.txt", MemberContent::File(&mut &b"four"[..], Some(8)));
    let message = failed_extract(session, result);

    assert_eq!(
        message,
        "Archive member `short.txt` declared 8 bytes but produced 4 bytes"
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
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(4),
    )?;
    let mut reader = TestReader(|_: &mut [u8]| panic!("member that cannot fit must not be read"));

    let result = session.extract_member("huge.txt", MemberContent::File(&mut reader, Some(8)));
    let message = failed_extract(session, result);

    assert!(
        message.contains("declared 8 bytes, but only 4 bytes are free"),
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
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(4),
    )?;

    let result = session.extract_member(
        "payload.txt",
        MemberContent::File(&mut &b"12345678"[..], None),
    );
    let message = failed_extract(session, result);

    assert_eq!(
        message,
        "Not enough free space at the destination to extract `payload.txt` (4 bytes available)"
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
    let session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(10),
    )?;

    let result = session.preflight_claimed_size(100);
    let message = failed_extract(session, result);

    assert!(
        message.contains("Archive declared size (100 bytes) exceeds the 10 bytes of free space"),
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
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(10),
    )?;
    session.extract_member(
        "first.txt",
        MemberContent::File(&mut &b"12345678"[..], Some(8)),
    )?;
    let mut reader =
        TestReader(|_: &mut [u8]| panic!("second member that cannot fit must not be read"));

    let result = session.extract_member("second.txt", MemberContent::File(&mut reader, Some(4)));
    let message = failed_extract(session, result);

    assert!(
        message.contains("declared 4 bytes, but only 2 bytes are free"),
        "{message}"
    );
    assert!(
        message.ends_with("free at the destination. Extracted entries remain in `archive`."),
        "{message}"
    );
    assert_eq!(fs::read(root.path().join("archive/first.txt"))?, b"12345678");
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
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        Some(16),
    )?;
    session.extract_member(
        "ok.txt",
        MemberContent::File(&mut &b"contents"[..], Some(8)),
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
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        None,
    )?;

    session.preflight_claimed_size(u128::MAX)?;
    session.extract_member(
        "declared.txt",
        MemberContent::File(&mut &b"12345678"[..], Some(8)),
    )?;
    session.extract_member(
        "undeclared.txt",
        MemberContent::File(&mut &b"12345678"[..], None),
    )?;

    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(Some(name)) if name == "archive"
    ));
    assert_eq!(fs::read(root.path().join("archive/declared.txt"))?, b"12345678");
    assert_eq!(fs::read(root.path().join("archive/undeclared.txt"))?, b"12345678");
    assert_eq!(progress.load(Ordering::Relaxed), 2);
    Ok(())
}

#[test]
fn unreported_free_space_still_enforces_declared_size() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open_with_available_bytes(
        root.path(),
        ARCHIVE,
        &progress,
        &cancelled,
        None,
    )?;

    let result = session.extract_member(
        "overflow.txt",
        MemberContent::File(&mut &b"abcdefgh"[..], Some(4)),
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

#[test]
fn finish_publishes_every_outcome_shape() -> Result<(), Box<dyn Error>> {
    let cases: [(&[&str], Result<(), ArchiveError>, Expected, &[&str]); 7] = [
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
            Expected::Failed("boom. Extracted entries remain in `archive`."),
            &["archive/a.txt"],
        ),
        (
            &["a.txt"],
            Err(ArchiveError::Failed("Damaged.".to_owned())),
            Expected::Failed("Damaged. Extracted entries remain in `archive`."),
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
        let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
        for member in members {
            session.extract_member(
                member,
                MemberContent::File(&mut member.as_bytes(), Some(member.len() as u64)),
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
            let member = Path::new(file).strip_prefix("archive").unwrap_or(Path::new(file));
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
    session.extract_member(
        "readme.txt",
        MemberContent::File(&mut &b"archived"[..], Some(8)),
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
        let mut session = ExtractionSession::open(root.path(), "secret.7z", &progress, &cancelled)?;
        session.extract_member("secret", MemberContent::Directory)?;
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
    let mut session = ExtractionSession::open(root.path(), ARCHIVE, &progress, &cancelled)?;
    session.extract_member("done.txt", MemberContent::File(&mut &b"done"[..], Some(4)))?;

    drop(session);

    assert_eq!(fs::read(root.path().join("archive/done.txt"))?, b"done");
    assert!(stages(root.path(), EXTRACTION_STAGE)?.is_empty());
    assert_eq!(root.path().read_dir()?.count(), 1);
    Ok(())
}
