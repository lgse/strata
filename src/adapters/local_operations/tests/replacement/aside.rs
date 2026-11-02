// SPDX-License-Identifier: MIT

//! Without Trash, a replaced original is set aside until its replacement is published.

use super::fs_quirks::evict;
use super::*;

fn mode(path: &Path) -> Result<u32, io::Error> {
    fs::metadata(path).map(|metadata| metadata.mode() & 0o7777)
}

fn set_mode(path: &Path, mode: u32) -> Result<(), io::Error> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

fn aside_without_trash(fs_type: FsWord, preset: Preset, item: Item) -> Result<(), Box<dyn Error>> {
    let directory = item.is_directory();
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    write_item(&source, directory, b"new")?;
    item.write_target(&target)?;
    let quirks = FsQuirks::install(root.path(), without_trash(fs_type, preset()));

    let (result, replaced) = replace_reporting(&source, &target, false);

    assert!(result.is_ok(), "{item:?}: {result:?}");
    assert_eq!(replaced, [ReplacedOriginal::Deleted]);
    expect_left(&target, directory, b"new", &["source", "target"]);
    assert_eq!(read_item(&source, directory)?, b"new");
    assert!(quirks.used_name(".strata-originals-*"));
    assert_eq!(quirks.count(Op::Exchange), 0);
    Ok(())
}

#[test]
fn replace_without_trash_or_exchange_sets_the_original_aside() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let plain: &[Item] = &[Item::File, Item::Folder];
    let mounts: [(FsWord, Preset, &[Item]); 3] = [
        (
            NFS_SUPER_MAGIC,
            Quirks::without_rename_flags,
            &[Item::File, Item::Folder, Item::EmptyReadOnlyFolder],
        ),
        (FUSE_SUPER_MAGIC, Quirks::sshfs, plain),
        (FUSE_SUPER_MAGIC, Quirks::vboxsf, plain),
    ];
    for (fs_type, quirks, items) in mounts {
        for &item in items {
            aside_without_trash(fs_type, quirks, item)?;
        }
    }
    Ok(())
}

#[test]
fn replace_falls_back_to_aside_when_trash_lies() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (trash, move_source) in [
        (Trash::FalseWithoutError, false),
        (Trash::SucceedsWithoutMoving, true),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"new")?;
        fs::write(&target, b"old")?;
        let quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash,
                fs_type: Some(NFS_SUPER_MAGIC),
                ..Quirks::default()
            },
        );

        let (result, replaced) = replace_reporting(&source, &target, move_source);

        assert!(result.is_ok(), "{trash:?}: {result:?}");
        assert_eq!(quirks.count(Op::Trash), 1);
        assert_eq!(replaced, [ReplacedOriginal::Deleted]);
        let listed: &[&str] = if move_source {
            &["target"]
        } else {
            &["source", "target"]
        };
        expect_left(&target, false, b"new", listed);
        assert!(quirks.used_name(".strata-originals-*"));
    }
    Ok(())
}

#[test]
fn replace_never_uses_exchange_on_fuse() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"new")?;
    fs::write(&target, b"old")?;
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(
            FUSE_SUPER_MAGIC,
            Quirks {
                exchange: Exchange::IgnoredOverwrites,
                ..Quirks::default()
            },
        ),
    );

    let (result, _) = replace_reporting(&source, &target, false);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(quirks.count(Op::Exchange), 0);
    expect_left(&target, false, b"new", &["source", "target"]);
    Ok(())
}

/// Which folder is read-only while a folder is replaced without Trash.
#[derive(Clone, Copy, Debug)]
enum Locked {
    Copy,
    EmptyOriginal,
}

fn locked_folder_falls_back_to_aside(
    locked: Locked,
    exchanges: usize,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    write_item(&source, true, b"new")?;
    match locked {
        Locked::Copy => {
            write_item(&target, true, b"old")?;
            set_mode(&source, 0o555)?;
        }
        Locked::EmptyOriginal => {
            fs::create_dir(&target)?;
            set_mode(&target, 0o555)?;
        }
    }
    let incoming = mode(&source)?;
    let quirks = FsQuirks::install(root.path(), without_trash(TMPFS_MAGIC, Quirks::default()));

    let (result, replaced) = replace_reporting(&source, &target, false);

    let replaced_mode = mode(&target);
    for path in [&source, &target] {
        set_mode(path, 0o755)?;
    }
    assert!(result.is_ok(), "{locked:?}: {result:?}");
    assert_eq!(replaced, [ReplacedOriginal::Deleted], "{locked:?}");
    assert_eq!(quirks.count(Op::Exchange), exchanges, "{locked:?}");
    expect_left(&target, true, b"new", &["source", "target"]);
    assert_eq!(replaced_mode?, incoming, "{locked:?}");
    assert_ne!(
        mount_capabilities(quirks.key()).exchange,
        Some(false),
        "{locked:?}"
    );
    Ok(())
}

#[test]
fn rejected_exchange_falls_back_to_aside_and_is_remembered() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (locked, exchanges) in [(Locked::Copy, 0), (Locked::EmptyOriginal, 1)] {
        locked_folder_falls_back_to_aside(locked, exchanges)?;
    }

    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"new")?;
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(
            TMPFS_MAGIC,
            Quirks {
                exchange: Exchange::Rejected,
                ..Quirks::default()
            },
        ),
    );

    for exchanges in [1, 0] {
        fs::write(&target, b"old")?;
        quirks.clear_calls();

        let (result, replaced) = replace_reporting(&source, &target, false);

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(replaced, [ReplacedOriginal::Deleted]);
        assert_eq!(quirks.count(Op::Exchange), exchanges);
        expect_left(&target, false, b"new", &["source", "target"]);
    }
    assert_eq!(mount_capabilities(quirks.key()).exchange, Some(false));
    Ok(())
}

/// Evicts the original set aside under `folder` from its mount's caches.
fn evicting_the_original(folder: &Path) -> Racer {
    let folder = folder.to_path_buf();
    Box::new(move || {
        let originals = originals_folder(&folder)
            .expect("list the folder")
            .expect("the original is aside");
        assert!(evict(&folder.join(originals).join("target")));
    })
}

/// A publication step that fails, after `skip` of its kind, once the original is evicted
/// if `evicted`.
struct Failing {
    op: Op,
    skip: usize,
    evicted: bool,
}

fn failed_publication(item: Item, quirks: Quirks, failing: Failing) -> Result<(), Box<dyn Error>> {
    let Failing { op, skip, evicted } = failing;
    let directory = item.is_directory();
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    write_item(&source, directory, b"new")?;
    item.write_target(&target)?;
    let original = (inode(&target)?, forced_mode(&target)?);
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(
            quirks.fs_type.unwrap_or(NFS_SUPER_MAGIC),
            Quirks {
                fail: vec![(op, "target", skip, Errno::IO)],
                before: if evicted {
                    vec![(op, "target", skip, evicting_the_original(root.path()))]
                } else {
                    Vec::new()
                },
                ..quirks
            },
        ),
    );

    let (result, replaced) = replace_reporting(&source, &target, false);

    let left = (inode(&target)?, fs::symlink_metadata(&target)?.mode());
    if let Item::ReadOnlyFolder = item {
        set_mode(&target, 0o755)?;
    }
    let error = result.expect_err("the publication fails");
    let injected = super::super::super::native_fs::os_error_detail(Errno::IO);
    assert!(error.message().contains(&injected), "{item:?}: {error}");
    assert!(replaced.is_empty());
    expect_set_aside(&quirks, root.path(), &["source", "target"])?;
    if item == Item::Symlink {
        assert_eq!(fs::read_link(&target)?, Path::new("old"));
    } else {
        assert_eq!(left, original, "{item:?}");
        assert_eq!(read_item(&target, directory)?, b"old");
    }
    Ok(())
}

#[test]
fn a_failed_publication_puts_the_original_back() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let vfat = Quirks {
        exchange: Exchange::Rejected,
        ..Quirks::vfat()
    };
    let failing = |op, skip, evicted| Failing { op, skip, evicted };
    let rows = [
        (Item::File, Quirks::default(), failing(Op::Rename, 1, false)),
        (
            Item::Folder,
            Quirks::default(),
            failing(Op::Rename, 1, false),
        ),
        (
            Item::ReadOnlyFolder,
            Quirks::default(),
            failing(Op::Rename, 2, false),
        ),
        (
            Item::File,
            Quirks::without_rename_flags(),
            failing(Op::Link, 0, false),
        ),
        (Item::File, vfat, failing(Op::Rename, 3, true)),
        (
            Item::Symlink,
            Quirks::sshfs(),
            failing(Op::Rename, 1, false),
        ),
    ];
    for (item, quirks, failing) in rows {
        failed_publication(item, quirks, failing)?;
    }
    Ok(())
}

#[test]
fn replace_fails_before_staging_when_no_strategy_is_safe() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"new")?;
    fs::write(&target, b"old")?;
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(FUSE_SUPER_MAGIC, Quirks::without_placement()),
    );

    let result = replace(&source, &target, false);

    let error = result.expect_err("no safe way to replace");
    assert!(error.message().contains("replaced safely"), "{error}");
    assert_eq!(quirks.count(Op::OpenSource), 0);
    expect_left(&target, false, b"old", &["source", "target"]);
    Ok(())
}

/// Records, at each flush, whether an original is aside under `watched`.
fn recording_aside(watched: &Path, record: &Arc<std::sync::Mutex<Vec<bool>>>) -> Racer {
    let (record, watched) = (record.clone(), watched.to_path_buf());
    Box::new(move || {
        let kept = originals_folder(&watched).is_ok_and(|folder| folder.is_some());
        record
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(kept);
    })
}

#[test]
fn the_original_is_deleted_only_once_its_replacement_is_flushed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for flushed in [true, false] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"new")?;
        fs::write(&target, b"old")?;
        let aside_during_flush = Arc::new(std::sync::Mutex::new(Vec::new()));
        let racer = recording_aside(root.path(), &aside_during_flush);
        let _quirks = FsQuirks::install(
            root.path(),
            without_trash(
                NFS_SUPER_MAGIC,
                Quirks {
                    before: vec![(Op::SyncFs, "*", 0, racer)],
                    fail: if flushed {
                        Vec::new()
                    } else {
                        vec![(Op::SyncFs, "*", 0, Errno::IO)]
                    },
                    ..Quirks::default()
                },
            ),
        );

        let (result, _) = replace_reporting(&source, &target, false);

        assert_eq!(
            *aside_during_flush
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            [true],
            "flushed: {flushed}"
        );
        assert_eq!(fs::read(&target)?, b"new");
        if flushed {
            assert!(result.is_ok(), "{result:?}");
            expect_names(root.path(), &["source", "target"], "flushed")?;
            continue;
        }
        let aside = originals_folder(root.path())?.ok_or("the original stays aside")?;
        assert_eq!(fs::read(root.path().join(&aside).join("target"))?, b"old");
        let error = result.expect_err("the original is kept");
        assert!(
            error.message().contains(&aside)
                && error.message().contains("writes may still be pending"),
            "{error}"
        );
    }
    Ok(())
}
