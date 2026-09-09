// SPDX-License-Identifier: MIT

use std::{
    error::Error,
    fs,
    io::{self, Read},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::{ArchiveError, ArchiveOutcome, ExtractionSession, MemberContent};
use crate::model::Location;

struct TestReader<F>(F);

impl<F: FnMut(&mut [u8]) -> io::Result<usize>> Read for TestReader<F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        (self.0)(buffer)
    }
}

#[test]
fn members_share_conflict_names_and_count_only_completed_work() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    fs::write(root.path().join("folder/keep.txt"), b"original")?;
    fs::write(root.path().join("report.txt"), b"original")?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;

    session.extract_member("folder", MemberContent::Directory)?;
    session.extract_member(
        "folder/nested/file.txt",
        MemberContent::File(&mut &b"contents"[..], Some(8)),
    )?;
    session.extract_member("folder/empty", MemberContent::Directory)?;
    session.extract_member("report.txt", MemberContent::File(&mut io::empty(), Some(0)))?;

    assert_eq!(progress.load(Ordering::Relaxed), 4);
    assert_eq!(fs::read(root.path().join("folder/keep.txt"))?, b"original");
    assert_eq!(
        fs::read(root.path().join("folder (2)/nested/file.txt"))?,
        b"contents"
    );
    assert!(root.path().join("folder (2)/empty").is_dir());
    assert_eq!(fs::read(root.path().join("report.txt"))?, b"original");
    assert!(fs::read(root.path().join("report (2).txt"))?.is_empty());
    assert!(matches!(
        session.finish(Ok(()), || panic!("completion must not enumerate remaining members"))?,
        ArchiveOutcome::Completed(Some(name)) if name == "folder (2)"
    ));
    Ok(())
}

#[test]
fn cancellation_reports_actual_destinations_for_duplicate_members() -> Result<(), Box<dyn Error>> {
    for (name, first, second) in [
        ("same.txt", "same.txt", "same (2).txt"),
        (
            "folder/same.txt",
            "folder (2)/same.txt",
            "folder (2)/same (2).txt",
        ),
    ] {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("folder"))?;
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
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
    }
    Ok(())
}

#[test]
fn cancellation_before_enumeration_uses_only_supplied_pending_names() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(true);
    let session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
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
        let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
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
    let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
    session.extract_member("done.txt", MemberContent::File(&mut &b"done"[..], Some(4)))?;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        buffer[..7].copy_from_slice(b"partial");
        cancelled.store(true, Ordering::Relaxed);
        Ok(7)
    });
    let result = session.extract_member("partial.txt", MemberContent::File(&mut reader, None));
    assert_eq!(result, Err(ArchiveError::Cancelled));
    let later = Location::local(root.path().join("later.txt"));
    assert!(matches!(
        session.finish(result, || vec!["later.txt".to_owned()])?,
        ArchiveOutcome::Cancelled { completed, failed, not_attempted }
            if completed == [Location::local(root.path().join("done.txt"))]
                && failed.is_empty()
                && not_attempted == [Location::local(root.path().join("partial (2).txt")), later]
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(root.path().join("done.txt"))?, b"done");
    assert_eq!(fs::read(root.path().join("partial.txt"))?, b"original");
    assert!(!root.path().join("partial (2).txt").exists());
    assert!(!root.path().join("later.txt").exists());
    Ok(())
}

#[test]
fn cleanup_failure_marks_the_interrupted_location_failed() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("partial.txt");
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        // A directory replacement makes unlink fail even when tests run as root.
        fs::remove_file(&path)?;
        fs::create_dir(&path)?;
        buffer[0] = b'x';
        cancelled.store(true, Ordering::Relaxed);
        Ok(1)
    });
    let result = session.extract_member("partial.txt", MemberContent::File(&mut reader, None));
    assert_eq!(result, Err(ArchiveError::Cancelled));
    let later = Location::local(root.path().join("later.txt"));
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
    let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
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
        Err(ArchiveError::Failed(message)) if message == "broken stream"
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(root.path().join("done.txt"))?, b"done");
    assert!(!root.path().join("partial.txt").exists());
    Ok(())
}

#[test]
fn completed_worker_is_not_reclassified_by_late_cancellation() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
    session.extract_member("empty.txt", MemberContent::File(&mut io::empty(), Some(0)))?;
    cancelled.store(true, Ordering::Relaxed);
    assert!(
        matches!(session.finish(Ok(()), Vec::new)?, ArchiveOutcome::Completed(Some(name)) if name == "empty.txt")
    );
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn pending_names_are_validated_and_only_use_established_renames() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    fs::write(root.path().join("unvisited.txt"), b"original")?;
    std::os::unix::fs::symlink("missing", root.path().join("redirect"))?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
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
    assert_eq!(
        completed,
        [Location::local(root.path().join("folder (2)/first.txt"))]
    );
    assert!(failed.is_empty());
    assert_eq!(
        not_attempted,
        [
            "folder (2)/next.txt",
            "folder (2)/nested/later.txt",
            "unvisited.txt",
            "unvisited.txt",
            "missing/new.txt",
            "redirect/child"
        ]
        .map(|name| Location::local(root.path().join(name)))
    );
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(root.path().join("unvisited.txt"))?, b"original");
    assert_eq!(root.path().read_dir()?.count(), 4);
    assert_eq!(root.path().join("folder (2)").read_dir()?.count(), 1);
    assert!(root.path().join("folder").read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn member_cancelled_before_creation_uses_the_known_root_rename() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
    session.extract_member(
        "folder/first.txt",
        MemberContent::File(&mut io::empty(), Some(0)),
    )?;
    cancelled.store(true, Ordering::Relaxed);
    let mut reader = TestReader(|_: &mut [u8]| panic!("cancelled member must not be read"));
    let result = session.extract_member("folder/next.txt", MemberContent::File(&mut reader, None));
    assert!(matches!(session.finish(result, Vec::new)?,
        ArchiveOutcome::Cancelled { not_attempted, .. }
            if not_attempted == [Location::local(root.path().join("folder (2)/next.txt"))]
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert!(!root.path().join("folder (2)/next.txt").exists());
    Ok(())
}

#[test]
fn empty_session_completes_without_a_first_name() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
    assert!(matches!(
        session.finish(Ok(()), Vec::new)?,
        ArchiveOutcome::Completed(None)
    ));
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

fn failed_extract(result: Result<(), ArchiveError>) -> String {
    match result {
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
        &progress,
        &cancelled,
        Some(1024),
    )?;

    let message = failed_extract(session.extract_member(
        "overflow.txt",
        MemberContent::File(&mut &b"abcdefgh"[..], Some(4)),
    ));
    assert!(
        message.contains("declared 4 bytes but produced more"),
        "{message}"
    );
    assert!(!root.path().join("overflow.txt").exists());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn declared_size_shortfall_removes_partial_output() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(root.path(), &progress, &cancelled)?;

    let message = failed_extract(
        session.extract_member("short.txt", MemberContent::File(&mut &b"four"[..], Some(8))),
    );
    assert!(
        message.contains("declared 8 bytes but produced 4 bytes"),
        "{message}"
    );
    assert!(!root.path().join("short.txt").exists());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn member_preflight_refuses_when_destination_lacks_space() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session =
        ExtractionSession::open_with_available_bytes(root.path(), &progress, &cancelled, Some(4))?;
    let mut reader = TestReader(|_: &mut [u8]| panic!("member that cannot fit must not be read"));

    let message = failed_extract(
        session.extract_member("huge.txt", MemberContent::File(&mut reader, Some(8))),
    );
    assert!(
        message.contains("declared 8 bytes, but only 4 bytes are free"),
        "{message}"
    );
    assert!(root.path().read_dir()?.next().is_none());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn copy_without_declared_size_stops_at_free_space() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session =
        ExtractionSession::open_with_available_bytes(root.path(), &progress, &cancelled, Some(4))?;

    let message = failed_extract(session.extract_member(
        "payload.txt",
        MemberContent::File(&mut &b"12345678"[..], None),
    ));
    assert!(
        message.contains("Not enough free space at the destination to extract `payload.txt`"),
        "{message}"
    );
    assert!(!root.path().join("payload.txt").exists());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn claimed_total_preflight_refuses_before_any_member() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let session =
        ExtractionSession::open_with_available_bytes(root.path(), &progress, &cancelled, Some(10))?;

    let message = failed_extract(session.preflight_claimed_size(100));
    assert!(
        message.contains("Archive declared size (100 bytes) exceeds the 10 bytes of free space"),
        "{message}"
    );
    assert!(root.path().read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn second_member_preflight_uses_remaining_space() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session =
        ExtractionSession::open_with_available_bytes(root.path(), &progress, &cancelled, Some(10))?;
    session.extract_member(
        "first.txt",
        MemberContent::File(&mut &b"12345678"[..], Some(8)),
    )?;
    let mut reader =
        TestReader(|_: &mut [u8]| panic!("second member that cannot fit must not be read"));

    let message = failed_extract(
        session.extract_member("second.txt", MemberContent::File(&mut reader, Some(4))),
    );
    assert!(
        message.contains("declared 4 bytes, but only 2 bytes are free"),
        "{message}"
    );
    assert_eq!(fs::read(root.path().join("first.txt"))?, b"12345678");
    assert!(!root.path().join("second.txt").exists());
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn matching_declared_size_completes_under_an_injected_quota() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session =
        ExtractionSession::open_with_available_bytes(root.path(), &progress, &cancelled, Some(16))?;
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

/// A filesystem that does not report capacity must not be mistaken for a full
/// one; free-space checks are skipped while writes proceed normally.
#[test]
fn unreported_free_space_skips_capacity_checks() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session =
        ExtractionSession::open_with_available_bytes(root.path(), &progress, &cancelled, None)?;

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
        ArchiveOutcome::Completed(Some(name)) if name == "declared.txt"
    ));
    assert_eq!(fs::read(root.path().join("declared.txt"))?, b"12345678");
    assert_eq!(fs::read(root.path().join("undeclared.txt"))?, b"12345678");
    assert_eq!(progress.load(Ordering::Relaxed), 2);
    Ok(())
}

/// Unreported free space relaxes only the capacity checks; a member that
/// overruns its declared size is still refused.
#[test]
fn unreported_free_space_still_enforces_declared_size() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut session =
        ExtractionSession::open_with_available_bytes(root.path(), &progress, &cancelled, None)?;

    let message = failed_extract(session.extract_member(
        "overflow.txt",
        MemberContent::File(&mut &b"abcdefgh"[..], Some(4)),
    ));

    assert!(
        message.contains("declared 4 bytes but produced more"),
        "{message}"
    );
    assert!(!root.path().join("overflow.txt").exists());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}
