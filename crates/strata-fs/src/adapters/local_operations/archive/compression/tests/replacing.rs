// SPDX-License-Identifier: MIT

//! Replacing an existing archive: where its original goes, and what stops a replacement.

use super::*;
use crate::adapters::local_operations::tests::fs_quirks::Racer;
use std::sync::Mutex;

#[test]
fn an_archive_replaced_immediately_after_publication_is_not_owned_for_undo()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for edited in [false, true] {
        let root = tempdir_on_home_device()?;
        let archive = root.path().join("existing.zip");
        let _trash = HomeTrashGuard::new(&archive);
        fs::write(&archive, b"original")?;
        let arrival = root.path().join("arrival");
        fs::write(&arrival, b"other program")?;
        let over = archive.clone();
        let racer = Box::new(move || {
            if edited {
                fs::write(&over, b"other program").expect("edit published archive");
            } else {
                fs::rename(&arrival, &over).expect("replace published archive");
            }
        });
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                after: vec![(Op::Rename, "existing.zip", 0, racer)],
                ..Quirks::default()
            },
        );
        let result = stage_archive(
            root.path(),
            &archive,
            TransferConflict::ReplaceExisting,
            b"archive",
        );
        let (_, replaced, placed) = result?;
        assert!(
            matches!(replaced, Some(ReplacedOriginal::Trashed(_))),
            "{replaced:?}"
        );
        assert!(placed.is_none());
        assert_eq!(fs::read(&archive)?, b"other program");
    }
    Ok(())
}

#[test]
fn compression_replacement_preserves_directory_destinations() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for populated in [false, true] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::create_dir(&archive)?;
        if populated {
            fs::write(archive.join("contents"), b"original")?;
        }
        let result = stage_archive(
            root.path(),
            &archive,
            TransferConflict::ReplaceExisting,
            b"replacement",
        );

        assert!(matches!(result, Err(ArchiveError::Failed(_))));
        assert!(archive.is_dir());
        if populated {
            assert_eq!(fs::read(archive.join("contents"))?, b"original");
        }
        assert!(stages(root.path(), COMPRESSION_STAGE)?.is_empty());
    }
    Ok(())
}

#[test]
fn an_archive_replaced_without_trash_deletes_its_original() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let cases: [(Trash, rustix::fs::FsWord, Preset, bool); 5] = [
        (Trash::Unsupported, NFS_SUPER_MAGIC, Quirks::default, true),
        (Trash::Unsupported, TMPFS_MAGIC, Quirks::default, false),
        (
            Trash::SucceedsWithoutMoving,
            NFS_SUPER_MAGIC,
            Quirks::default,
            true,
        ),
        (Trash::Unsupported, FUSE_SUPER_MAGIC, Quirks::sshfs, true),
        (Trash::Unsupported, FUSE_SUPER_MAGIC, Quirks::vboxsf, true),
    ];
    for (trash, fs_type, quirks, sets_aside) in cases {
        let context = format!("{trash:?} {fs_type:#x}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::write(&archive, b"original")?;
        fs::set_permissions(&archive, fs::Permissions::from_mode(0o640))?;
        let quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash,
                fs_type: Some(fs_type),
                ..quirks()
            },
        );

        let (result, _) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

        assert_eq!(
            result,
            Ok(("existing.zip".to_owned(), Some(ReplacedOriginal::Deleted))),
            "{context}"
        );
        assert_eq!(fs::metadata(&archive)?.permissions().mode() & 0o777, 0o640);
        expect_only_existing(root.path(), b"replacement", &context)?;
        assert!(quirks.used_name(".strata-originals-*"));
        assert_eq!(
            quirks.count(Op::Exchange),
            usize::from(!sets_aside),
            "{context}"
        );
    }
    Ok(())
}

#[test]
fn an_unplug_after_an_exchange_leaves_no_original_at_a_stage() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("existing.zip");
    fs::write(&archive, b"original")?;
    let quirks = FsQuirks::install(
        root.path(),
        Quirks {
            trash: Trash::Unsupported,
            fs_type: Some(TMPFS_MAGIC),
            unplugged_after: Some((Op::Exchange, "existing.zip", 0)),
            ..Quirks::default()
        },
    );

    let (result, _) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

    assert!(result.is_err());
    quirks.replug();
    let names = names(root.path())?;
    let [originals, published] = names.as_slice() else {
        return Err(format!("the original and the archive: {names:?}").into());
    };
    assert!(originals.starts_with(".strata-originals-"), "{names:?}");
    assert_eq!(published, "existing.zip");
    expect_contents(
        &[
            (
                root.path().join(originals).join("existing.zip"),
                b"original",
            ),
            (archive, b"replacement"),
        ],
        "unplugged",
    )
}

type Listings = Arc<Mutex<Vec<usize>>>;

fn lock(during: &Listings) -> std::sync::MutexGuard<'_, Vec<usize>> {
    during.lock().unwrap_or_else(|error| error.into_inner())
}

/// Records how many entries `watched` holds each time it runs.
fn records_listing(during: &Listings, watched: &Path) -> Racer {
    let (record, watched) = (during.clone(), watched.to_path_buf());
    Box::new(move || {
        lock(&record).push(names(&watched).map_or(0, |names| names.len()));
    })
}

fn barrier_quirks(fs_type: &'static str, fails: bool, during: &Listings, folder: &Path) -> Quirks {
    Quirks {
        trash: Trash::Unsupported,
        fs_type: Some(TMPFS_MAGIC),
        mount_type: Some((fs_type, "rw")),
        before: vec![(Op::SyncFs, "*", 0, records_listing(during, folder))],
        fail: if fails {
            vec![(Op::SyncFs, "*", 0, Errno::IO)]
        } else {
            Vec::new()
        },
        ..Quirks::default()
    }
}

/// Checks a failed barrier keeps the original, and the failure says where.
fn expect_original_kept(
    folder: &Path,
    result: Compressed,
    row: &str,
) -> Result<(), Box<dyn Error>> {
    let kept = names(folder)?
        .into_iter()
        .find(|name| name != "existing.zip")
        .ok_or("the original is kept")?;
    assert_eq!(
        fs::read(folder.join(&kept).join("existing.zip"))?,
        b"original"
    );
    let Err(ArchiveError::Failed(message)) = result else {
        return Err(format!("{row}: {result:?}").into());
    };
    assert!(
        message.contains(&kept) && message.contains("writes may still be pending"),
        "{message}"
    );
    Ok(())
}

fn expect_barrier_outcome(
    folder: &Path,
    result: Compressed,
    fails: bool,
    row: &str,
) -> Result<(), Box<dyn Error>> {
    if fails {
        return expect_original_kept(folder, result, row);
    }
    assert_eq!(
        result,
        Ok(("existing.zip".to_owned(), Some(ReplacedOriginal::Deleted))),
        "{row}"
    );
    assert_eq!(names(folder)?, ["existing.zip"], "{row}");
    Ok(())
}

fn expect_listed_at_barrier(during: &Listings, each_file: bool, row: &str) {
    if each_file {
        assert!(lock(during).is_empty());
    } else {
        assert_eq!(
            *lock(during),
            [2],
            "{row}: the original stays until the barrier"
        );
    }
}

#[test]
fn an_archive_replaced_without_trash_is_on_disk_before_its_original_is_deleted()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The family, whether the barrier fails, and whether the archive is
    // synced itself, as the barrier cannot reach it.
    for (fs_type, fails, each_file) in [
        ("ext4", false, false),
        ("ext4", true, false),
        ("cifs", false, true),
    ] {
        let row = format!("{fs_type} fails: {fails}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::write(&archive, b"original")?;
        let during = Listings::default();
        let quirks = FsQuirks::install(
            root.path(),
            barrier_quirks(fs_type, fails, &during, root.path()),
        );

        let (result, _) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

        assert_eq!(fs::read(&archive)?, b"replacement", "{row}");
        assert_eq!(quirks.count(Op::SyncData) == 1, each_file, "{row}");
        expect_listed_at_barrier(&during, each_file, &row);
        expect_barrier_outcome(root.path(), result, fails, &row)?;
    }
    Ok(())
}

#[test]
fn archives_are_created_without_noreplace_or_usable_links() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for quirks in [Quirks::sshfs, Quirks::vboxsf] {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), quirks());

        let (result, encoded) = publish_replacement(root.path(), TransferConflict::FailIfExists);

        assert_eq!(result, Ok(("existing.zip".to_owned(), None)));
        assert!(encoded);
        expect_only_existing(root.path(), b"replacement", "created")?;
    }
    Ok(())
}

#[test]
fn replacement_fails_before_encoding_when_no_strategy_is_safe() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("existing.zip"), b"original")?;
    let _quirks = FsQuirks::install(
        root.path(),
        Quirks {
            trash: Trash::Unsupported,
            fs_type: Some(NFS_SUPER_MAGIC),
            ..Quirks::without_placement()
        },
    );

    let (result, encoded) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

    let Err(ArchiveError::Failed(message)) = result else {
        return Err(format!("expected a refusal, got {result:?}").into());
    };
    assert!(message.contains("replaced safely"), "{message}");
    assert!(!encoded);
    expect_only_existing(root.path(), b"original", "refused")
}

#[test]
fn an_archive_replaces_only_the_item_its_mount_still_knows() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let reused = |fs_type| {
        move || Quirks {
            fs_type: Some(fs_type),
            reused_inodes: Some("existing.zip"),
            ..Quirks::default()
        }
    };
    // NFS sets the original aside; tmpfs exchanges it.
    type MakeQuirks = Box<dyn Fn() -> Quirks>;
    let rows: [(MakeQuirks, bool); 3] = [
        (Box::new(reused(NFS_SUPER_MAGIC)), true),
        (Box::new(reused(TMPFS_MAGIC)), true),
        (Box::new(Quirks::vfat), false),
    ];
    for (quirks, swapped) in rows {
        let row = format!("{:?}, swapped: {swapped}", quirks().fs_type);
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::write(&archive, b"original")?;
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash: Trash::Unsupported,
                ..quirks()
            },
        );
        let changed = archive.clone();

        let result = glib::MainContext::default().block_on(write_staged_archive(
            root.path(),
            &archive,
            TransferConflict::ReplaceExisting,
            never_cancelled(),
            None,
            move |mut file| {
                if swapped {
                    fs::remove_file(&changed).map_err(|error| error.to_string())?;
                    fs::write(&changed, b"theirs").map_err(|error| error.to_string())?;
                    recreated(&changed);
                } else if !evict(&changed) {
                    return Err("the original was not evicted".to_owned().into());
                }
                file.write_all(b"replacement")
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        ));

        if swapped {
            let Err(ArchiveError::Failed(message)) = result else {
                return Err(format!("{row}: expected a refusal, got {result:?}").into());
            };
            assert!(message.contains("changed"), "{row}: {message}");
            assert_eq!(fs::read(&archive)?, b"theirs", "{row}");
        } else {
            assert!(result.is_ok(), "{row}: {result:?}");
            assert_eq!(fs::read(&archive)?, b"replacement", "{row}");
        }
        assert_eq!(names(root.path())?, ["existing.zip"], "{row}");
    }
    Ok(())
}

#[test]
fn a_name_taken_during_replacement_keeps_the_original_aside() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The first rename of the name sets the original aside; the second publishes.
    for (skip, saves_over) in [(1, false), (0, true)] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::write(&archive, b"original")?;
        let (racer_archive, saved) = (archive.clone(), root.path().join("saved"));
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash: Trash::Unsupported,
                fs_type: Some(NFS_SUPER_MAGIC),
                before: vec![(
                    Op::Rename,
                    "existing.zip",
                    skip,
                    Box::new(move || {
                        if saves_over {
                            let _ = fs::write(&saved, b"racer");
                            let _ = fs::rename(&saved, &racer_archive);
                        } else {
                            let _ = fs::write(&racer_archive, b"racer");
                        }
                    }),
                )],
                ..Quirks::default()
            },
        );

        let (result, _) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

        let Err(ArchiveError::Failed(message)) = result else {
            return Err(format!("expected the original to be kept aside, got {result:?}").into());
        };
        let names = names(root.path())?;
        let [kept] = names
            .iter()
            .filter(|name| name.starts_with(".strata-originals-"))
            .collect::<Vec<_>>()[..]
        else {
            return Err(format!("expected one kept original, found {names:?}").into());
        };
        assert!(
            message.contains(&format!("{kept}/existing.zip")),
            "{saves_over}: {message}"
        );
        let original = root.path().join(kept).join("existing.zip");
        if saves_over {
            assert_eq!(fs::read(original)?, b"racer");
            assert_eq!(names, [kept.as_str()]);
        } else {
            assert_eq!(fs::read(original)?, b"original");
            assert_eq!(fs::read(&archive)?, b"racer");
        }
        assert!(stages(root.path(), COMPRESSION_STAGE)?.is_empty());
    }
    Ok(())
}
