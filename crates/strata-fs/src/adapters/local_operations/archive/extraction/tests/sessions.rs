// SPDX-License-Identifier: MIT

use std::{collections::HashSet, error::Error, fs, path::Path};

use super::super::{ArchiveError, ArchiveOutcome, MemberContent, MemberMetadata};
use super::harness::{Harness, Node, Outcome, file};
use crate::{
    adapters::local_operations::archive::fixtures::{EXTRACTION_STAGE, stages},
    model::Location,
};

#[test]
fn multi_root_members_keep_their_names_beside_existing_entries() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    fs::create_dir(harness.path().join("folder"))?;
    fs::write(harness.path().join("folder/keep.txt"), b"original")?;
    fs::write(harness.path().join("report.txt"), b"original")?;
    let mut session = harness.open()?;

    session.extract_dir("folder")?;
    session.extract_file("folder/nested/file.txt", b"contents")?;
    session.extract_dir("folder/empty")?;
    session.extract_file("report.txt", b"")?;

    let published = session.finish_completed(Ok(()), || {
        panic!("completion must not enumerate remaining members")
    })?;
    assert_eq!(published.as_deref(), Some("archive"));
    session.assert_extracted(
        4,
        [
            ("archive/folder/nested/file.txt", file(b"contents")),
            ("archive/folder/empty", Node::Dir),
            ("archive/report.txt", file(b"")),
            ("folder/keep.txt", file(b"original")),
            ("report.txt", file(b"original")),
        ],
    )
}

#[test]
fn unsafe_member_paths_are_sanitized_without_stopping_extraction() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::in_folder("destination")?;
    let mut session = harness.open_named("bundle.zip")?;

    session.extract_file("../escaped.txt", b"escaped")?;
    session.extract_file("escaped.txt", b"duplicate")?;
    session.extract_file("after.txt", b"after")?;

    let published = session.finish_completed(Ok(()), Vec::new)?;
    assert_eq!(published.as_deref(), Some("bundle"));
    assert_eq!(session.progress(), 3);
    super::harness::assert_tree(
        session.root(),
        [
            ("destination/bundle/escaped.txt", file(b"escaped")),
            ("destination/bundle/escaped (2).txt", file(b"duplicate")),
            ("destination/bundle/after.txt", file(b"after")),
        ],
    )
}

#[test]
fn completed_worker_is_not_reclassified_by_late_cancellation() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open()?;
    session.extract_file("empty.txt", b"")?;
    session.cancel();

    let published = session.finish_completed(Ok(()), Vec::new)?;

    assert_eq!(published.as_deref(), Some("empty.txt"));
    assert_eq!(session.progress(), 1);
    Ok(())
}

#[test]
fn empty_session_completes_without_a_first_name() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open()?;

    assert_eq!(session.finish_completed(Ok(()), Vec::new)?, None);
    session.assert_extracted(0, [])
}

#[test]
fn single_root_colliding_with_a_destination_entry_is_suffixed_at_publish()
-> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    fs::write(harness.path().join("readme.txt"), b"existing")?;
    let mut session = harness.open()?;
    session.extract_file("readme.txt", b"archived")?;

    let published = session.finish_completed(Ok(()), Vec::new)?;

    assert_eq!(published.as_deref(), Some("readme (2).txt"));
    session.assert_extracted(
        1,
        [
            ("readme.txt", file(b"existing")),
            ("readme (2).txt", file(b"archived")),
        ],
    )
}

#[test]
fn a_session_dropped_before_finish_publishes_its_output() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open()?;
    session.extract_file("done.txt", b"done")?;

    drop(session);

    assert!(stages(harness.path(), EXTRACTION_STAGE)?.is_empty());
    super::harness::assert_tree(harness.path(), [("archive/done.txt", file(b"done"))])
}

enum Expected {
    Completed(Option<&'static str>),
    Failed(&'static str),
    Cancelled(&'static [&'static str]),
}

struct OutcomeCase {
    members: &'static [&'static str],
    result: Result<(), ArchiveError>,
    expected: Expected,
    files: &'static [&'static str],
}

fn outcome_cases() -> [OutcomeCase; 7] {
    [
        OutcomeCase {
            members: &[],
            result: Ok(()),
            expected: Expected::Completed(None),
            files: &[],
        },
        OutcomeCase {
            members: &["only.txt"],
            result: Ok(()),
            expected: Expected::Completed(Some("only.txt")),
            files: &["only.txt"],
        },
        OutcomeCase {
            members: &["a.txt", "b/c.txt"],
            result: Ok(()),
            expected: Expected::Completed(Some("archive")),
            files: &["archive/a.txt", "archive/b/c.txt"],
        },
        OutcomeCase {
            members: &["a.txt"],
            result: Err(ArchiveError::Failed("boom".to_owned())),
            expected: Expected::Failed("boom. Extracted entries remain in “archive”."),
            files: &["archive/a.txt"],
        },
        OutcomeCase {
            members: &["a.txt"],
            result: Err(ArchiveError::Failed("Damaged.".to_owned())),
            expected: Expected::Failed("Damaged. Extracted entries remain in “archive”."),
            files: &["archive/a.txt"],
        },
        OutcomeCase {
            members: &["a.txt"],
            result: Err(ArchiveError::Cancelled),
            expected: Expected::Cancelled(&["archive/a.txt"]),
            files: &["archive/a.txt"],
        },
        OutcomeCase {
            members: &[],
            result: Err(ArchiveError::Failed("boom".to_owned())),
            expected: Expected::Failed("boom"),
            files: &[],
        },
    ]
}

fn assert_outcome(root: &Path, expected: &Expected, outcome: &Outcome, label: &str) {
    match expected {
        Expected::Completed(first_name) => assert!(
            matches!(outcome, Ok(ArchiveOutcome::Completed(name)) if name.as_deref() == *first_name),
            "{label}: {outcome:?}"
        ),
        Expected::Failed(message) => assert!(
            matches!(outcome, Err(ArchiveError::Failed(actual)) if actual == message),
            "{label}: {outcome:?}"
        ),
        Expected::Cancelled(paths) => {
            let paths: Vec<_> = paths
                .iter()
                .map(|path| Location::local(root.join(path)))
                .collect();
            assert!(
                matches!(outcome, Ok(ArchiveOutcome::Cancelled { completed, failed, not_attempted })
                    if *completed == paths && failed.is_empty() && not_attempted.is_empty()),
                "{label}: {outcome:?}"
            );
        }
    }
}

/// Each published file holds its member name, and nothing else reached the root.
fn assert_published_files(root: &Path, files: &[&str], label: &str) -> Result<(), Box<dyn Error>> {
    for file in files {
        let member = Path::new(file)
            .strip_prefix("archive")
            .unwrap_or(Path::new(file));
        assert_eq!(
            fs::read(root.join(file))?,
            member.as_os_str().as_encoded_bytes(),
            "{label}"
        );
    }
    let top_level: HashSet<_> = files
        .iter()
        .filter_map(|file| Path::new(file).iter().next())
        .collect();
    assert_eq!(root.read_dir()?.count(), top_level.len(), "{label}");
    Ok(())
}

#[test]
fn finish_publishes_every_outcome_shape() -> Result<(), Box<dyn Error>> {
    for case in outcome_cases() {
        let mut harness = Harness::new()?;
        let mut session = harness.open()?;
        for member in case.members {
            session.extract_member(
                member,
                MemberContent::File(&mut member.as_bytes(), Some(member.len() as u64)),
                MemberMetadata::NONE,
            )?;
        }

        let outcome = session.finish(case.result, Vec::new);

        let label = format!("{:?}", case.members);
        assert_outcome(session.path(), &case.expected, &outcome, &label);
        assert_published_files(session.path(), case.files, &label)?;
    }
    Ok(())
}
