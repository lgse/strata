// SPDX-License-Identifier: GPL-3.0-or-later

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
        MemberContent::File(&mut &b"contents"[..]),
    )?;
    session.extract_member("folder/empty", MemberContent::Directory)?;
    session.extract_member("report.txt", MemberContent::File(&mut io::empty()))?;

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
fn cancellation_before_enumeration_uses_only_supplied_remaining_locations()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(true);
    let session = ExtractionSession::open(root.path(), &progress, &cancelled)?;
    let result = session.check_cancelled();
    let remaining = Location::local(root.path().join("known.txt"));

    assert!(matches!(
        session.finish(result, || vec![remaining.clone()])?,
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
            MemberContent::File(&mut reader)
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
    session.extract_member("done.txt", MemberContent::File(&mut &b"done"[..]))?;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        buffer[..7].copy_from_slice(b"partial");
        cancelled.store(true, Ordering::Relaxed);
        Ok(7)
    });
    let result = session.extract_member("partial.txt", MemberContent::File(&mut reader));
    assert_eq!(result, Err(ArchiveError::Cancelled));
    let later = Location::local(root.path().join("later.txt"));
    assert!(matches!(
        session.finish(result, || vec![later.clone()])?,
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
    let result = session.extract_member("partial.txt", MemberContent::File(&mut reader));
    assert_eq!(result, Err(ArchiveError::Cancelled));
    let later = Location::local(root.path().join("later.txt"));
    assert!(matches!(
        session.finish(result, || vec![later.clone()])?,
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
    session.extract_member("done.txt", MemberContent::File(&mut &b"done"[..]))?;
    let mut first_read = true;
    let mut reader = TestReader(|buffer: &mut [u8]| {
        if !first_read {
            return Err(io::Error::other("broken stream"));
        }
        first_read = false;
        buffer[..7].copy_from_slice(b"partial");
        Ok(7)
    });
    let result = session.extract_member("partial.txt", MemberContent::File(&mut reader));
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
    session.extract_member("empty.txt", MemberContent::File(&mut io::empty()))?;
    cancelled.store(true, Ordering::Relaxed);
    assert!(
        matches!(session.finish(Ok(()), Vec::new)?, ArchiveOutcome::Completed(Some(name)) if name == "empty.txt")
    );
    assert_eq!(progress.load(Ordering::Relaxed), 1);
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
