// SPDX-License-Identifier: MIT

//! Keeping both archives numbers the new one past every name already taken.

use super::*;

#[test]
fn keep_both_retries_publication_collisions_without_encoding_again() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    for extension in ["zip", "7z", "tar", "tar.gz"] {
        let destination = root.path().join(extension);
        fs::create_dir(&destination)?;
        let archive = destination.join(format!("archive.part.{extension}"));
        let first = destination.join(format!("archive.part (1).{extension}"));
        let directory = destination.join(format!("archive.part (2).{extension}"));
        let symlink = destination.join(format!("archive.part (3).{extension}"));
        fs::create_dir(&directory)?;
        std::os::unix::fs::symlink("missing", &symlink)?;
        let encoded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_encoded = encoded.clone();
        let late_archive = archive.clone();
        let late_first = first.clone();
        let (published, original, _) =
            glib::MainContext::default().block_on(write_staged_archive(
                &destination,
                &archive,
                TransferConflict::KeepBoth,
                never_cancelled(),
                None,
                move |mut file| {
                    worker_encoded.fetch_add(1, Ordering::Relaxed);
                    file.write_all(b"new archive")
                        .map_err(|error| error.to_string())?;
                    fs::write(late_archive, b"late original").map_err(|error| error.to_string())?;
                    fs::write(late_first, b"late numbered").map_err(|error| error.to_string())?;
                    Ok(())
                },
            ))?;
        assert_eq!(published, format!("archive.part (4).{extension}"));
        assert!(original.is_none());
        assert_eq!(encoded.load(Ordering::Relaxed), 1);
        expect_contents(
            &[
                (destination.join(&published), b"new archive"),
                (archive, b"late original"),
                (first, b"late numbered"),
            ],
            extension,
        )?;
        assert!(directory.is_dir());
        assert_eq!(fs::read_link(symlink)?, Path::new("missing"));
        assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    }
    Ok(())
}

#[test]
fn keep_both_numbers_a_long_archive_name_within_the_limit() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    for (name, numbered) in [
        (
            format!("{}.zip", "c".repeat(251)),
            format!("{} (1).zip", "c".repeat(247)),
        ),
        (
            format!("{}.tar.gz", "c".repeat(248)),
            format!("{} (1).tar.gz", "c".repeat(244)),
        ),
    ] {
        let archive = root.path().join(&name);
        fs::write(&archive, b"existing")?;

        let (published, _, _) = stage_archive(
            root.path(),
            &archive,
            TransferConflict::KeepBoth,
            b"new archive",
        )?;

        assert_eq!(published, numbered);
        assert_eq!(fs::read(root.path().join(&numbered))?, b"new archive");
        assert_eq!(fs::read(&archive)?, b"existing");
    }
    Ok(())
}

#[test]
fn keep_both_tries_every_numbered_name_up_to_its_bound() -> Result<(), Box<dyn Error>> {
    use crate::adapters::local_operations::placement::NUMBERED_NAME_ATTEMPTS;
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive.zip");
    fs::write(&archive, b"existing")?;
    let last = NUMBERED_NAME_ATTEMPTS - 1;
    let _taken = FsQuirks::install(
        root.path(),
        Quirks {
            invalid_name: Some(|name| {
                let number: u64 = name
                    .to_str()?
                    .strip_prefix("archive (")?
                    .strip_suffix(").zip")?
                    .parse()
                    .ok()?;
                (number < NUMBERED_NAME_ATTEMPTS - 1).then_some(Errno::EXIST)
            }),
            ..Quirks::default()
        },
    );

    let (published, _, _) = stage_archive(
        root.path(),
        &archive,
        TransferConflict::KeepBoth,
        b"new archive",
    )?;

    assert_eq!(published, format!("archive ({last}).zip"));
    assert_eq!(fs::read(root.path().join(&published))?, b"new archive");
    assert_eq!(fs::read(&archive)?, b"existing");
    Ok(())
}

#[test]
fn keep_both_numbering_works_with_network_collision_errnos() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for sshfs in [true, false] {
        let context = format!("sshfs: {sshfs}");
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("existing.zip"), b"original")?;
        fs::write(root.path().join("existing (1).zip"), b"numbered")?;
        let quirks = if sshfs {
            Quirks {
                collisions_as_eperm: true,
                ..Quirks::without_rename_flags()
            }
        } else {
            // SMB with POSIX extensions: the first stage name is held by a folder.
            Quirks {
                fail: vec![(Op::Create, ".strata-copy-*", 0, Errno::ISDIR)],
                ..Quirks::default()
            }
        };
        let _quirks = FsQuirks::install(root.path(), quirks);

        let (result, _) = publish_replacement(root.path(), TransferConflict::KeepBoth);

        assert_eq!(
            result,
            Ok(("existing (2).zip".to_owned(), None)),
            "{context}"
        );
        expect_contents(
            &[
                (root.path().join("existing (2).zip"), b"replacement"),
                (root.path().join("existing.zip"), b"original"),
                (root.path().join("existing (1).zip"), b"numbered"),
            ],
            &context,
        )?;
        assert!(stages(root.path(), COMPRESSION_STAGE)?.is_empty());
    }
    Ok(())
}
