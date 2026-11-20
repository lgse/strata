// SPDX-License-Identifier: MIT

use std::{error::Error, fs, io, path::Path, sync::atomic::Ordering};

use super::super::{ArchiveError, ArchiveOutcome, MemberContent, MemberMetadata};
use super::harness::{Cancelled, Harness, Node, TestReader, file, unread};
use crate::{
    adapters::local_operations::archive::fixtures::{EXTRACTION_STAGE, stages},
    model::Location,
};

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
        let mut harness = Harness::new()?;
        fs::create_dir(harness.path().join("folder"))?;
        let mut session = harness.open()?;
        session.extract_file(name, b"one")?;
        session.extract_file(name, b"two")?;
        session.cancel();
        let result = session.check_cancelled();

        let outcome = session.finish_cancelled(result, Vec::new)?;

        assert_eq!(
            outcome,
            Cancelled {
                completed: session.locations(&[first, second]),
                failed: vec![],
                not_attempted: vec![],
            }
        );
        session.assert_extracted(
            2,
            [
                ("folder", Node::Dir),
                (first, file(b"one")),
                (second, file(b"two")),
            ],
        )?;
    }
    Ok(())
}

#[test]
fn cancellation_before_enumeration_uses_only_supplied_pending_names() -> Result<(), Box<dyn Error>>
{
    let mut harness = Harness::new()?;
    harness.cancel();
    let mut session = harness.open()?;
    let result = session.check_cancelled();

    let outcome = session.finish_cancelled(result, || vec!["known.txt".to_owned()])?;

    assert_eq!(
        outcome,
        Cancelled {
            completed: vec![],
            failed: vec![],
            not_attempted: session.locations(&["known.txt"]),
        }
    );
    session.assert_extracted(0, [])
}

#[test]
fn cancellation_before_member_processing_never_reads_or_creates_it() -> Result<(), Box<dyn Error>> {
    for directory in [false, true] {
        let mut harness = Harness::new()?;
        harness.cancel();
        let mut session = harness.open()?;
        let mut reader = unread("cancelled member must not be read");
        let content = if directory {
            MemberContent::Directory
        } else {
            MemberContent::File(&mut reader, None)
        };

        let result = session.extract_cancelled("folder/member", content);

        let outcome = session.finish_cancelled(result, Vec::new)?;
        assert_eq!(
            outcome,
            Cancelled {
                completed: vec![],
                failed: vec![],
                not_attempted: session.locations(&["folder/member"]),
            }
        );
        session.assert_extracted(0, [])?;
    }
    Ok(())
}

#[test]
fn mid_copy_cancellation_removes_only_the_partial_file_and_preserves_results()
-> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    fs::write(harness.path().join("partial.txt"), b"original")?;
    let mut session = harness.open()?;
    session.extract_file("done.txt", b"done")?;
    let cancelled = session.cancelled();
    let mut reader = TestReader(|buffer: &mut [u8]| {
        buffer[..7].copy_from_slice(b"partial");
        cancelled.store(true, Ordering::Relaxed);
        Ok(7)
    });

    let result = session.extract_cancelled("partial.txt", MemberContent::File(&mut reader, None));

    let outcome = session.finish_cancelled(result, || vec!["later.txt".to_owned()])?;
    assert_eq!(
        outcome,
        Cancelled {
            completed: session.locations(&["archive/done.txt"]),
            failed: vec![],
            not_attempted: session.locations(&["archive/partial.txt", "archive/later.txt"]),
        }
    );
    session.assert_extracted(
        1,
        [
            ("archive/done.txt", file(b"done")),
            ("partial.txt", file(b"original")),
        ],
    )
}

#[test]
fn cleanup_failure_marks_the_interrupted_location_failed() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open()?;
    let (root, cancelled) = (session.path(), session.cancelled());
    let mut reader = TestReader(|buffer: &mut [u8]| {
        let staging = stages(root, EXTRACTION_STAGE)
            .map_err(|error| io::Error::other(error.to_string()))?
            .pop()
            .ok_or_else(|| io::Error::other("no extraction staging folder"))?;
        let staged = root.join(staging).join("partial.txt");
        // A directory replacement makes unlink fail even when tests run as root.
        fs::remove_file(&staged)?;
        fs::create_dir(&staged)?;
        buffer[0] = b'x';
        cancelled.store(true, Ordering::Relaxed);
        Ok(1)
    });

    let result = session.extract_cancelled("partial.txt", MemberContent::File(&mut reader, None));

    let outcome = session.finish_cancelled(result, || vec!["later.txt".to_owned()])?;
    assert_eq!(
        outcome,
        Cancelled {
            completed: vec![],
            failed: session.locations(&["archive/partial.txt"]),
            not_attempted: session.locations(&["archive/later.txt"]),
        }
    );
    assert!(session.path().join("archive/partial.txt").is_dir());
    assert_eq!(session.progress(), 0);
    Ok(())
}

#[test]
fn read_failure_removes_partial_output_without_reporting_cancellation() -> Result<(), Box<dyn Error>>
{
    let mut harness = Harness::new()?;
    let mut session = harness.open()?;
    session.extract_file("done.txt", b"done")?;
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

    assert_eq!(
        session.finish_failed(result),
        "broken stream. Extracted entries remain in “archive”."
    );
    session.assert_extracted(1, [("archive/done.txt", file(b"done"))])
}

#[test]
fn pending_names_are_sanitized_and_kept_under_the_archive_folder() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    fs::create_dir(harness.path().join("folder"))?;
    fs::write(harness.path().join("unvisited.txt"), b"original")?;
    std::os::unix::fs::symlink("missing", harness.path().join("redirect"))?;
    let mut session = harness.open()?;
    session.extract_file("folder/first.txt", b"done")?;
    session.cancel();
    let result = session.check_cancelled();

    let outcome = session.finish_cancelled(result, || {
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

    assert_eq!(
        outcome,
        Cancelled {
            completed: session.locations(&["archive/folder/first.txt"]),
            failed: vec![],
            not_attempted: session.locations(&[
                "archive/folder/next.txt",
                "archive/folder/nested/later.txt",
                "archive/unvisited.txt",
                "archive/unvisited.txt",
                "archive/missing/new.txt",
                "archive/redirect/child",
                "archive/outside",
            ]),
        }
    );
    session.assert_extracted(
        1,
        [
            ("archive/folder/first.txt", file(b"done")),
            ("folder", Node::Dir),
            ("unvisited.txt", file(b"original")),
            ("redirect", Node::Link(Path::new("missing").to_path_buf())),
        ],
    )
}

#[test]
fn member_cancelled_before_creation_is_reported_inside_the_archive_folder()
-> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    fs::create_dir(harness.path().join("folder"))?;
    let mut session = harness.open()?;
    session.extract_file("folder/first.txt", b"")?;
    session.cancel();
    let mut reader = unread("cancelled member must not be read");

    let result = session.extract_member(
        "folder/next.txt",
        MemberContent::File(&mut reader, None),
        MemberMetadata::NONE,
    );

    let outcome = session.finish_cancelled(result, Vec::new)?;
    assert_eq!(
        outcome.not_attempted,
        session.locations(&["archive/folder/next.txt"])
    );
    session.assert_extracted(
        1,
        [
            ("archive/folder/first.txt", file(b"")),
            ("folder", Node::Dir),
        ],
    )
}

#[test]
fn a_failure_that_left_only_directories_leaves_nothing_behind() -> Result<(), Box<dyn Error>> {
    for cancel in [false, true] {
        let mut harness = Harness::new()?;
        let mut session = harness.open_named("secret.7z")?;
        session.extract_dir("secret")?;
        let cancelled = session.cancelled();
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

        let pending = |name: &str| Location::local(session.path().join(name));
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
        assert!(
            session.path().read_dir()?.next().is_none(),
            "cancel={cancel}"
        );
    }
    Ok(())
}
