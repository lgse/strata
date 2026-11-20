// SPDX-License-Identifier: MIT

use std::{
    error::Error,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    time::{Duration, UNIX_EPOCH},
};

use rustix::io::Errno;

use super::super::{ArchiveError, MemberContent, MemberMetadata};
use super::harness::{Harness, assert_tree};
use crate::adapters::local_operations::{
    archive::fixtures::expected_mode,
    tests::fs_quirks::{FsQuirks, Op, Quirks},
};

fn read_only() -> MemberMetadata {
    MemberMetadata {
        mode: Some(0o555),
        modified: Some(UNIX_EPOCH + Duration::from_secs(1_000_000_000)),
    }
}

#[test]
fn directory_metadata_is_restored_deepest_first_only_on_completion() -> Result<(), Box<dyn Error>> {
    for complete in [false, true] {
        let mut harness = Harness::new()?;
        let mut session = harness.open()?;
        session.extract_member("ro", MemberContent::Directory, read_only())?;
        session.extract_member("ro/sub", MemberContent::Directory, read_only())?;
        session.extract_file("ro/sub/child.txt", b"child")?;

        let (output, mode) = if complete {
            let published = session.finish_completed(Ok(()), Vec::new)?;
            assert_eq!(published.as_deref(), Some("ro"));
            (session.path().join("ro"), 0o555)
        } else {
            session.cancel();
            let result = session.extract_member(
                "ro/later.txt",
                MemberContent::File(&mut &b""[..], Some(0)),
                MemberMetadata::NONE,
            );
            session.finish_cancelled(result, Vec::new)?;
            (session.path().join("archive/ro"), 0o777)
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
    let mut harness = Harness::new()?;
    let _quirks = FsQuirks::install(
        harness.path(),
        Quirks {
            fail: vec![(Op::SetTimes, "*", 0, Errno::IO)],
            ..Quirks::default()
        },
    );
    let mut session = harness.open()?;
    session.extract_member("top/ro", MemberContent::Directory, read_only())?;
    session.extract_dir("top/ro/child")?;

    let message = match session.finish(Ok(()), Vec::new) {
        Err(ArchiveError::Failed(message)) => message,
        other => panic!("expected a failed restore, got {other:?}"),
    };

    assert_eq!(
        message,
        format!(
            "Could not restore the modification time of “top/ro”: {}",
            crate::services::io_error_message(&Errno::IO.into())
        )
    );
    assert_tree(session.path(), [])
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
        let mut harness = Harness::new()?;
        let mut session = harness.open()?;
        session.extract_member(directory, MemberContent::Directory, earlier)?;
        session.extract_file(&format!("{directory}/file.txt"), b"file")?;
        session.extract_member(directory, MemberContent::Directory, later)?;

        let outcome = session.finish_completed(Ok(()), Vec::new);

        assert!(
            matches!(&outcome, Ok(Some(name)) if name == published),
            "{label}: {outcome:?}"
        );
        let output = session.path().join(directory);
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
    let mut harness = Harness::new()?;
    let mut session = harness.open()?;
    session.extract_file("locked/file.txt", b"file")?;
    let staging = fs::read_dir(session.path())?
        .next()
        .ok_or("no staging folder")??
        .path();
    fs::set_permissions(staging.join("locked"), fs::Permissions::from_mode(0o500))?;

    let outcome = session.finish(
        Err(ArchiveError::PasswordRequired(
            crate::sandbox::archive_listing::PASSWORD_REQUIRED.to_owned(),
        )),
        Vec::new,
    );

    let kept = session.path().join("archive/locked");
    let _ = fs::set_permissions(&kept, fs::Permissions::from_mode(0o700));
    let _ = fs::set_permissions(staging.join("locked"), fs::Permissions::from_mode(0o700));
    assert!(
        matches!(&outcome, Err(ArchiveError::Failed(message))
            if message.starts_with(&format!(
                "{} Could not remove the extraction staging folder: Permission denied",
                crate::sandbox::archive_listing::PASSWORD_REQUIRED
            )) && message.ends_with(" Extracted entries remain in “archive”.")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(kept.join("file.txt"))?, b"file");
    Ok(())
}
