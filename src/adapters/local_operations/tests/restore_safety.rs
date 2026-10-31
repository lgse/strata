// SPDX-License-Identifier: MIT

use super::super::move_restore_path;
use super::fs_quirks::{Op, OverEmptyDirectory, Racer, Support};
use super::*;
use crate::services::{KeptCause, KeptItem, SpecialKind, TransferReport};
use rustix::io::Errno;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};

fn restore_path(
    source: &Path,
    destination: &Path,
    allowed: &Path,
) -> Result<TransferReport, glib::Error> {
    glib::MainContext::default()
        .block_on(move_restore_path(
            source.to_path_buf(),
            destination.to_path_buf(),
            allowed.to_path_buf(),
            Arc::default(),
            gio::Cancellable::new(),
        ))
        .map(|(report, _)| report)
}

#[test]
fn restore_without_rename_flags_never_replaces_a_racing_destination() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // The name is taken just before the step that would take it: the
    // fallback rung, or the copy made where nothing can place the item,
    // whose barrier is opened before it writes.
    let rows: [(Preset, Option<(Op, &str)>); 4] = [
        (Quirks::without_rename_flags, None),
        (
            || Quirks {
                rename_flags: Support::Rejected(Errno::NOSYS),
                ..Quirks::default()
            },
            None,
        ),
        (
            || Quirks {
                rename_flags: Support::Rejected(Errno::OPNOTSUPP),
                ..Quirks::default()
            },
            None,
        ),
        (Quirks::without_placement, Some((Op::OpenDir, "."))),
    ];
    for (quirks, copying) in rows {
        for directory in [false, true] {
            let fixture = tempfile::tempdir()?;
            let source = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            if directory {
                fs::create_dir(&source)?;
                fs::write(source.join("contents"), b"original")?;
            } else {
                fs::write(&source, b"original")?;
            }
            let raced = destination.clone();
            let (op, pattern) = copying.unwrap_or(if directory {
                (Op::Mkdir, "destination")
            } else {
                (Op::Link, "destination")
            });
            let racer: Racer = Box::new(move || {
                fs::write(&raced, b"concurrent user data").expect("racing destination")
            });
            let _scope = FsQuirks::install(
                fixture.path(),
                Quirks {
                    before: vec![(op, pattern, 0, racer)],
                    ..quirks()
                },
            );

            let message = restore_path(&source, &destination, fixture.path())
                .expect_err("the destination was taken")
                .to_string();

            assert!(
                message.contains("something already exists at the destination"),
                "{message}"
            );
            assert_eq!(fs::read(&destination)?, b"concurrent user data");
            let original = if directory {
                source.join("contents")
            } else {
                source
            };
            assert_eq!(fs::read(original)?, b"original");
        }
    }
    Ok(())
}

#[test]
fn restoring_without_rename_flags_moves_the_item_without_copying() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let presets: [Preset; 5] = [
        Quirks::without_rename_flags,
        || Quirks {
            rename_flags: Support::Rejected(Errno::NOSYS),
            ..Quirks::default()
        },
        || Quirks {
            rename_flags: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::default()
        },
        Quirks::sshfs,
        Quirks::vboxsf,
    ];
    for quirks in presets {
        for directory in [false, true] {
            let fixture = tempfile::tempdir()?;
            let _scope = FsQuirks::install(fixture.path(), quirks());
            let source = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            if directory {
                fs::create_dir(&source)?;
                fs::write(source.join("contents"), b"original")?;
            } else {
                fs::write(&source, b"original")?;
            }
            let inode = fs::metadata(&source)?.ino();

            restore_path(&source, &destination, fixture.path())?;

            assert!(!source.exists(), "source should be removed from trash");
            assert_eq!(fs::metadata(&destination)?.ino(), inode);
            let restored = if directory {
                destination.join("contents")
            } else {
                destination
            };
            assert_eq!(fs::read(restored)?, b"original");
        }
    }
    Ok(())
}

#[test]
fn restore_without_rename_flags_keeps_both_parents_pinned() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let allowed = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let source_parent = allowed.path().join("trash");
    let target_parent = allowed.path().join("documents");
    fs::create_dir(&source_parent)?;
    fs::create_dir(&target_parent)?;
    let source = source_parent.join("report");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    fs::create_dir(outside.path().join("report"))?;
    fs::write(outside.path().join("report/contents"), b"unrelated")?;
    let retained_source = allowed.path().join("old-trash");
    let retained_target = allowed.path().join("old-documents");
    let old_source = retained_source.clone();
    let old_target = retained_target.clone();
    let escaped = outside.path().to_path_buf();
    let (moved_source, moved_target) = (source_parent.clone(), target_parent.clone());
    let _scope = FsQuirks::install(
        allowed.path(),
        Quirks {
            before: vec![(
                Op::Rename,
                "report",
                0,
                Box::new(move || {
                    fs::rename(&moved_source, &old_source).expect("move source parent");
                    fs::rename(&moved_target, &old_target).expect("move destination parent");
                    symlink(&escaped, &moved_source).expect("redirect source parent");
                    symlink(&escaped, &moved_target).expect("redirect destination parent");
                }),
            )],
            ..Quirks::without_rename_flags()
        },
    );

    restore_path(&source, &target_parent.join("report"), allowed.path())?;

    assert_eq!(
        fs::read(retained_target.join("report/contents"))?,
        b"original"
    );
    assert!(!retained_source.join("report").exists());
    assert_eq!(
        fs::read(outside.path().join("report/contents"))?,
        b"unrelated"
    );
    Ok(())
}

#[test]
fn restore_without_rename_flags_preserves_a_replaced_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    let original = fixture.path().join("original");
    let destination = fixture.path().join("destination");
    fs::write(&source, b"original")?;
    let raced_source = source.clone();
    let retained = original.clone();
    let _scope = FsQuirks::install(
        fixture.path(),
        Quirks {
            before: vec![(
                Op::Rename,
                "source",
                0,
                Box::new(move || {
                    fs::rename(&raced_source, &retained).expect("retain original source");
                    fs::write(&raced_source, b"replacement").expect("replace source");
                }),
            )],
            ..Quirks::without_rename_flags()
        },
    );

    assert!(restore_path(&source, &destination, fixture.path()).is_err());

    assert_eq!(fs::read(source)?, b"replacement");
    assert_eq!(fs::read(original)?, b"original");
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn restore_copies_files_links_and_named_pipes_where_nothing_can_place_them()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    symlink("missing", source.join("link"))?;
    rustix::fs::mknodat(
        rustix::fs::CWD,
        source.join("fifo"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )?;
    fs::set_permissions(source.join("fifo"), fs::Permissions::from_mode(0o640))?;
    let destination = fixture.path().join("destination");
    let scope = FsQuirks::install(fixture.path(), Quirks::without_placement());

    let report = restore_path(&source, &destination, fixture.path())?;

    assert!(!scope.used_name(".strata-copy-*"), "nothing is staged");
    assert_eq!(report, TransferReport::default());
    assert_eq!(fs::read(destination.join("contents"))?, b"original");
    assert_eq!(
        fs::read_link(destination.join("link"))?,
        Path::new("missing")
    );
    let pipe = fs::symlink_metadata(destination.join("fifo"))?;
    assert!(pipe.file_type().is_fifo());
    assert_eq!(pipe.mode() & 0o777, 0o640);
    assert!(!source.exists());
    Ok(())
}

#[test]
fn restore_copy_refuses_a_trash_item_replaced_after_it_was_checked() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Replaced before the copy starts, and between the copy's check of the
    // name and its open of the file.
    for (op, pattern) in [(Op::OpenDir, "."), (Op::OpenSource, "source")] {
        let fixture = tempfile::tempdir()?;
        let source = fixture.path().join("source");
        let original = fixture.path().join("original");
        let destination = fixture.path().join("destination");
        fs::write(&source, b"original")?;
        let (raced, retained) = (source.clone(), original.clone());
        let racer: Racer = Box::new(move || {
            fs::rename(&raced, &retained).expect("retain original source");
            fs::write(&raced, b"replacement").expect("replace source");
        });
        let _scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                before: vec![(op, pattern, 0, racer)],
                ..Quirks::without_placement().on_ext4()
            },
        );

        let result = restore_path(&source, &destination, fixture.path());

        assert!(result.is_err(), "{op:?}");
        assert!(!destination.exists(), "{op:?}");
        assert_eq!(fs::read(&source)?, b"replacement");
        assert_eq!(fs::read(&original)?, b"original");
    }
    Ok(())
}

#[test]
fn a_failed_restore_copy_removes_only_what_it_made() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let exfat_in_one_tick: Preset = || Quirks {
        coarse_clock: Some(SystemTime::now() - Duration::from_secs(1)),
        links: Support::Rejected(Errno::MLINK),
        rename_flags: Support::Rejected(Errno::INVAL),
        rename_over_empty_dir: OverEmptyDirectory::NotEmpty,
        ..Quirks::exfat()
    };
    let rows: [(Preset, bool); 3] = [
        (Quirks::without_placement, false),
        (exfat_in_one_tick, false),
        (exfat_in_one_tick, true),
    ];
    for (preset, evicted) in rows {
        let row = format!("{:?}, evicted: {evicted}", preset().mount_type);
        let fixture = tempfile::tempdir()?;
        let source = fixture.path().join("source");
        fs::create_dir_all(source.join("sub"))?;
        fs::write(source.join("contents"), b"original")?;
        fs::write(source.join("sub/nested"), b"nested")?;
        let destination = fixture.path().join("destination");
        let (copy, aside) = (destination.clone(), fixture.path().join("old-copy"));
        let racer: Racer = Box::new(move || {
            fs::write(copy.join("foreign"), b"unrelated").expect("add a foreign entry");
            if evicted {
                assert!(super::fs_quirks::evict(&copy.join("contents")), "evict");
                return;
            }
            fs::rename(copy.join("contents"), &aside).expect("move a copied entry aside");
            fs::write(copy.join("contents"), b"replacement").expect("replace a copied entry");
        });
        let _scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                before: vec![(Op::SetTimes, "destination", 0, racer)],
                fail: vec![(Op::SetTimes, "destination", 0, Errno::IO)],
                ..preset()
            },
        );

        assert!(
            restore_path(&source, &destination, fixture.path()).is_err(),
            "{row}"
        );

        assert_eq!(
            fs::read(destination.join("foreign"))?,
            b"unrelated",
            "{row}"
        );
        assert_eq!(
            fs::read(destination.join("contents")).ok(),
            (!evicted).then(|| b"replacement".to_vec()),
            "{row}"
        );
        assert!(!destination.join("sub").exists(), "{row}");
        assert_eq!(fs::read(source.join("contents"))?, b"original", "{row}");
        assert_eq!(fs::read(source.join("sub/nested"))?, b"nested", "{row}");
    }
    Ok(())
}

#[test]
fn a_failed_restore_copy_of_a_file_leaves_nothing_at_its_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::write(&source, b"original")?;
    let destination = fixture.path().join("destination");
    let _scope = FsQuirks::install(
        fixture.path(),
        Quirks {
            fail: vec![(Op::SetTimes, "destination", 0, Errno::IO)],
            ..Quirks::without_placement()
        },
    );

    assert!(restore_path(&source, &destination, fixture.path()).is_err());

    assert!(fs::symlink_metadata(&destination).is_err());
    assert_eq!(fs::read(&source)?, b"original");
    Ok(())
}

#[test]
fn a_cancelled_restore_copy_of_a_folder_is_a_cancel_only_once_the_folder_is_gone()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Cleanup opens the cut-off folder (its third open, after making and
    // filling it), checks it, empties it and checks its name before removing
    // it. Rows fail the open or a check, or renumber the folder as libfuse
    // without `use_ino` does, where only its type is compared.
    let host: Preset = Quirks::without_placement;
    let ext4 = || Quirks::without_placement().on_ext4();
    let sshfs = || Quirks {
        mount_type: Some(("fuse.sshfs", "rw")),
        ..Quirks::without_placement()
    };
    for (preset, fail, renumber, removed) in [
        (host, Some((Op::OpenDir, 2)), false, false),
        (host, Some((Op::StatFd, 2)), false, false),
        (ext4, None, true, false),
        (host, Some((Op::Stat, 2)), false, false),
        (sshfs, None, true, true),
    ] {
        let row = format!(
            "{:?}, {fail:?}, renumbered: {renumber}",
            preset().mount_type
        );
        let fixture = tempfile::tempdir()?;
        let source = fixture.path().join("source");
        fs::create_dir(&source)?;
        let contents = vec![0x5a; 4 * 1024 * 1024];
        fs::write(source.join("data"), &contents)?;
        let destination = fixture.path().join("destination");
        let cancellable = gio::Cancellable::new();
        let racer = |cancel: gio::Cancellable| -> Racer { Box::new(move || cancel.cancel()) };
        let path = destination.clone();
        let renumbering: Racer = Box::new(move || super::fs_quirks::renumbered(&path));
        let _scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                before: vec![
                    (Op::CopyRange, "*", 0, racer(cancellable.clone())),
                    (Op::Write, "*", 0, racer(cancellable.clone())),
                ],
                after: if renumber {
                    vec![(Op::OpenDir, "destination", 2, renumbering)]
                } else {
                    Vec::new()
                },
                fail: fail
                    .map(|(op, skip)| (op, "destination", skip, Errno::IO))
                    .into_iter()
                    .collect(),
                reflink: Support::Rejected(Errno::OPNOTSUPP),
                ..preset()
            },
        );

        let restored = glib::MainContext::default().block_on(move_restore_path(
            source.clone(),
            destination.clone(),
            fixture.path().to_path_buf(),
            Arc::default(),
            cancellable,
        ));

        let error = restored.expect_err("a cut-off copy");
        assert_eq!(
            error.matches(gio::IOErrorEnum::Cancelled),
            removed,
            "{row}: {error}"
        );
        assert_eq!(
            fs::symlink_metadata(&destination).is_err(),
            removed,
            "{row}"
        );
        assert_eq!(fs::read(source.join("data"))?, contents, "{row}");
    }
    Ok(())
}

#[test]
fn a_restore_cancelled_once_its_copy_is_complete_finishes() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    let destination = fixture.path().join("destination");
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let racer: Racer = Box::new(move || cancel.cancel());
    let _scope = FsQuirks::install(
        fixture.path(),
        Quirks {
            mount_type: Some(("ext4", "rw")),
            before: vec![(Op::SyncFs, "*", 0, racer)],
            ..Quirks::without_placement()
        },
    );

    glib::MainContext::default().block_on(move_restore_path(
        source.clone(),
        destination.clone(),
        fixture.path().to_path_buf(),
        Arc::default(),
        cancellable,
    ))?;

    assert_eq!(fs::read(destination.join("contents"))?, b"original");
    assert!(fs::symlink_metadata(&source).is_err());
    Ok(())
}

#[test]
fn restore_copies_keep_the_metadata_the_destination_takes() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let refusing = || Quirks {
        extra_xattrs: vec![("security.strata-test", b"label")],
        xattrs_refused: vec![("security.", Errno::PERM)],
        ..Quirks::without_placement()
    };
    // Read-only items keep the xattrs set before their mode, and an xattr
    // the destination refuses is left out.
    let rows: [(bool, u32, Preset); 3] = [
        (false, 0o444, Quirks::without_placement),
        (true, 0o555, Quirks::without_placement),
        (false, 0o640, refusing),
    ];
    for (directory, mode, quirks) in rows {
        let row = format!("directory: {directory}, mode: {mode:o}");
        let fixture = tempfile::tempdir()?;
        let source = fixture.path().join("source");
        if directory {
            fs::create_dir(&source)?;
        } else {
            fs::write(&source, b"original")?;
        }
        let opened = fs::File::open(&source)?;
        rustix::fs::fsetxattr(
            &opened,
            "user.strata-test",
            b"metadata",
            rustix::fs::XattrFlags::empty(),
        )?;
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        opened.set_times(fs::FileTimes::new().set_modified(modified))?;
        fs::set_permissions(&source, fs::Permissions::from_mode(mode))?;
        let destination = fixture.path().join("destination");
        let _scope = FsQuirks::install(fixture.path(), quirks());

        restore_path(&source, &destination, fixture.path())?;

        let restored = fs::symlink_metadata(&destination)?;
        assert_eq!(restored.mode() & 0o777, mode, "{row}");
        assert_eq!(restored.modified()?, modified, "{row}");
        let mut value = [0u8; 32];
        let count = rustix::fs::getxattr(&destination, "user.strata-test", &mut value[..])?;
        assert_eq!(&value[..count], b"metadata", "{row}");
        assert!(fs::symlink_metadata(&source).is_err(), "{row}");
    }
    Ok(())
}

#[test]
fn restore_deletes_the_trash_copy_only_once_its_copy_is_flushed() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // The family, how its barrier is reached, whether each restored file is
    // synced before it is closed, and whether the flush fails.
    let rows = [
        ("ext4", Op::SyncFs, false, false),
        ("ext4", Op::SyncFs, false, true),
        ("fuseblk", Op::SyncDir, true, false),
    ];
    for (fs_type, flush, each_file, failing) in rows {
        let row = format!("{fs_type}, failing: {failing}");
        let fixture = tempfile::tempdir()?;
        let source = fixture.path().join("source");
        fs::create_dir(&source)?;
        fs::write(source.join("contents"), b"original")?;
        let destination = fixture.path().join("destination");
        let fail = if failing {
            vec![(flush, "*", 0, Errno::IO)]
        } else {
            Vec::new()
        };
        let scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                mount_type: Some((fs_type, "rw")),
                fail,
                ..Quirks::without_placement()
            },
        );

        let restored = restore_path(&source, &destination, fixture.path());

        let calls = scope.calls();
        let first = |op: Op| {
            calls
                .iter()
                .position(|call| call.op == op && call.names.iter().any(|n| n == "contents"))
        };
        let flushed = calls.iter().position(|call| call.op == flush);
        assert!(flushed.is_some(), "{row}");
        let synced = first(Op::SyncData);
        assert_eq!(synced.is_some(), each_file, "{row}");
        assert!(synced < first(Op::Close), "{row}");
        assert_eq!(fs::read(destination.join("contents"))?, b"original");
        let deleted = first(Op::Unlink);
        if failing {
            let message = restored.expect_err("the flush failed").to_string();
            assert!(message.contains("could not finish writing"), "{message}");
            assert_eq!(deleted, None, "{row}");
            assert_eq!(fs::read(source.join("contents"))?, b"original");
        } else {
            restored?;
            assert!(deleted > flushed, "{row}");
            assert!(!source.exists(), "{row}");
        }
    }
    Ok(())
}

#[test]
fn restore_leaves_in_the_trash_what_it_cannot_copy_or_finds_changed() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let uid = rustix::process::getuid().as_raw();
    // Whether the Trash copy changes, and whether a merge undo restores it.
    for (changed, merge_undo) in [(false, false), (true, false), (false, true)] {
        let fixture = tempfile::tempdir()?;
        let trash = fixture.path().join(format!(".Trash-{uid}"));
        let item = trash.join("files/report");
        fs::create_dir_all(&item)?;
        fs::create_dir_all(trash.join("info"))?;
        fs::create_dir(fixture.path().join("Documents"))?;
        fs::write(item.join("notes.txt"), b"notes")?;
        let info = trash.join("info/report.trashinfo");
        fs::write(
            &info,
            "[Trash Info]\nPath=Documents/report\nDeletionDate=2026-01-01T00:00:00\n",
        )?;
        let (name, cause) = if changed {
            ("notes.txt", KeptCause::Changed)
        } else {
            std::os::unix::net::UnixListener::bind(item.join("socket"))?;
            ("socket", KeptCause::Skipped(SpecialKind::Socket))
        };
        // Rewritten in the Trash once it is copied, before the copy is flushed.
        let rewritten = item.join("notes.txt");
        let racer: Racer = Box::new(move || {
            fs::write(&rewritten, b"rewritten notes").expect("rewrite the Trash copy")
        });
        let _scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                mount_type: Some(("ext4", "rw")),
                before: if changed {
                    vec![(Op::SyncFs, "*", 0, racer)]
                } else {
                    Vec::new()
                },
                ..Quirks::without_placement()
            },
        );
        let destination = fixture.path().canonicalize()?.join("Documents/report");
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let emit = Rc::new(move |event| emitted.borrow_mut().push(event));

        if merge_undo {
            let staged = RefCell::new(Some(RestoreEntry {
                source: Location::local(&item),
                display_name: "report".to_owned(),
                original_target: Some(Location::local(&destination)),
                trash_info: Some(info.clone()),
                confirmed_destination: None,
                physical_path: Some(item.clone()),
            }));
            let lookup: StagedOriginalLookup = Rc::new(move |_, _| {
                let staged = staged.borrow_mut().take();
                Box::pin(async move { staged.ok_or_else(|| io_error("staged once")) })
            });
            glib::MainContext::default().block_on(run_merge_undo(
                OperationRequestId(504),
                (
                    Vec::new(),
                    vec![Location::local(&destination)],
                    HashMap::new(),
                ),
                emit,
                gio::Cancellable::new(),
                lookup,
            ));
        } else {
            let _operation = LocalOperationProvider.restore(
                RestoreRequest {
                    id: OperationRequestId(503),
                    source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                        entry: directory_entry(&item),
                        destination: destination.clone(),
                    }]),
                },
                emit,
            );
            wait_for_restore(&events);
        }

        let events = events.borrow();
        assert!(
            matches!(events.last(), Some(OperationEvent::Restored { .. })),
            "{events:?}"
        );
        let kept = events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::ItemsReported { report, .. } => Some(report.kept_in_trash.clone()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(
            kept,
            vec![KeptItem {
                location: Location::local(item.join(name)),
                cause,
            }]
        );
        assert!(fs::symlink_metadata(item.join(name)).is_ok());
        assert!(info.exists(), "the Trash entry stays restorable");
        assert_eq!(fs::read(destination.join("notes.txt"))?, b"notes");
        assert!(fs::symlink_metadata(destination.join("socket")).is_err());
        assert_eq!(item.join("notes.txt").exists(), changed);
    }
    Ok(())
}

#[test]
fn restore_directory_copy_preserves_permissions_times_and_xattrs() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    fs::set_permissions(&source, fs::Permissions::from_mode(0o750))?;
    let source_file = fs::File::open(&source)?;
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let accessed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    source_file.set_times(
        fs::FileTimes::new()
            .set_accessed(accessed)
            .set_modified(modified),
    )?;
    rustix::fs::fsetxattr(
        &source_file,
        "user.strata-test",
        b"metadata",
        rustix::fs::XattrFlags::empty(),
    )?;
    let destination = fixture.path().join("destination");
    let source_inode = fs::metadata(&source)?.ino();
    let _scope = FsQuirks::install(fixture.path(), Quirks::without_placement());
    restore_path(&source, &destination, fixture.path())?;
    assert!(!source.exists());
    assert_ne!(fs::metadata(&destination)?.ino(), source_inode);
    assert_eq!(fs::metadata(&destination)?.mode() & 0o777, 0o750);
    assert_eq!(fs::metadata(&destination)?.modified()?, modified);
    assert_eq!(fs::metadata(&destination)?.accessed()?, accessed);
    let target = fs::File::open(destination)?;
    let mut value = [0u8; 32];
    let count = rustix::fs::fgetxattr(&target, "user.strata-test", &mut value[..])?;
    assert_eq!(&value[..count], b"metadata");
    Ok(())
}

#[test]
fn restore_refuses_a_destination_created_immediately_before_rename() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    fs::write(&source, b"original")?;
    let raced = destination.clone();
    let _scope = FsQuirks::install(
        fixture.path(),
        Quirks {
            before: vec![(
                Op::Rename,
                "destination",
                0,
                Box::new(move || {
                    fs::write(&raced, b"concurrent user data").expect("racing destination")
                }),
            )],
            ..Quirks::default()
        },
    );
    let result = restore_path(&source, &destination, fixture.path());
    assert!(
        result
            .expect_err("collision")
            .message()
            .contains("already exists")
    );
    assert_eq!(fs::read(&source)?, b"original");
    assert_eq!(fs::read(&destination)?, b"concurrent user data");
    Ok(())
}

#[test]
fn restore_keeps_payload_and_metadata_when_destination_is_occupied() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for directory in [false, true] {
        let fixture = tempfile::tempdir()?;
        let destination = fixture.path().join("report");
        if directory {
            fs::create_dir(&destination)?;
        } else {
            symlink(fixture.path().join("absent"), &destination)?;
        }
        let (source, entry) = volume_trash_entry(fixture.path(), "report", "report", b"original")?;
        let info = source
            .parent()
            .and_then(Path::parent)
            .expect("trash root")
            .join("info/report.trashinfo");
        let metadata = fs::read(&info)?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.restore(
            RestoreRequest {
                id: OperationRequestId(502),
                source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                    entry,
                    destination: destination.clone(),
                }]),
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        wait_for_restore(&events);
        assert!(
            matches!(events.borrow().last(), Some(OperationEvent::RestoreCompletedWithErrors { message, .. }) if message.contains("already exists")),
            "{:?}",
            events.borrow()
        );
        assert_eq!(fs::read(&source)?, b"original");
        assert_eq!(fs::read(&info)?, metadata);
        assert!(fs::symlink_metadata(&destination).is_ok());
    }
    Ok(())
}

#[test]
fn restore_execution_refuses_a_parent_symlink_escape() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let allowed = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let source = allowed.path().join("source");
    fs::write(&source, b"original")?;
    symlink(outside.path(), allowed.path().join("parent"))?;
    let result = glib::MainContext::default().block_on(move_restore_path(
        source.clone(),
        allowed.path().join("parent/report"),
        allowed.path().to_path_buf(),
        Arc::default(),
        gio::Cancellable::new(),
    ));
    assert!(result.is_err());
    assert_eq!(fs::read(&source)?, b"original");
    assert!(!outside.path().join("report").exists());
    Ok(())
}

#[test]
fn restore_shared_trash_moves_the_item_and_removes_its_metadata() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let trash = fixture
        .path()
        .join(".Trash")
        .join(rustix::process::getuid().as_raw().to_string());
    fs::create_dir_all(trash.join("files"))?;
    fs::create_dir_all(trash.join("info"))?;
    fs::create_dir(fixture.path().join("Documents"))?;
    let source = trash.join("files/report");
    let info = trash.join("info/report.trashinfo");
    fs::write(&source, b"original")?;
    fs::write(&info, "[Trash Info]\nPath=Documents/report\n")?;
    let destination = fixture.path().join("Documents/report");
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(502),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry: file_entry(&source),
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);
    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(destination)?, b"original");
    assert!(!source.exists());
    assert!(!info.exists());
    assert!(!fixture.path().join(".Trash/Documents/report").exists());
    Ok(())
}

#[test]
fn home_trash_fallback_finds_broken_symlinks_the_virtual_backend_has_not_refreshed()
-> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!("strata-home-trash-fallback-{unique}"));
    let trash = fixture.join("Trash");
    let original = fixture.join("original report.txt");
    fs::create_dir_all(trash.join("files"))?;
    fs::create_dir_all(trash.join("info"))?;
    std::os::unix::fs::symlink("missing-target", trash.join("files/report.txt"))?;
    let encoded = original.display().to_string().replace(' ', "%20");
    fs::write(
        trash.join("info/report.txt.trashinfo"),
        format!("[Trash Info]\nPath={encoded}\nDeletionDate=2026-09-03T16:05:39\n"),
    )?;

    let entries = trash_entries_at(
        std::slice::from_ref(&trash),
        &HashSet::from([original.clone()]),
        &gio::Cancellable::new(),
        &HashMap::new(),
    );

    let entry = entries
        .get(&original)
        .and_then(|found| found.as_ref().ok())
        .expect("fallback entry");
    assert_eq!(
        entry.source,
        Location::local(trash.join("files/report.txt"))
    );
    assert_eq!(entry.original_target, Some(Location::local(&original)));
    assert_eq!(
        entry.trash_info.as_deref(),
        Some(trash.join("info/report.txt.trashinfo").as_path())
    );
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    assert!(
        trash_entries_at(
            std::slice::from_ref(&trash),
            &HashSet::from([original]),
            &cancellable,
            &HashMap::new()
        )
        .is_empty()
    );
    assert!(fs::symlink_metadata(trash.join("files/report.txt")).is_ok());
    assert!(trash.join("info/report.txt.trashinfo").exists());
    fs::remove_dir_all(fixture)?;
    Ok(())
}

#[test]
fn cancelling_restore_before_io_reports_every_item_as_unattempted() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let location = Location::local("/fixture/trashed.txt");
    for (source, expected) in [
        (
            RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry: file_entry(Path::new("/fixture/trashed.txt")),
                destination: PathBuf::from("/fixture/trashed.txt"),
            }]),
            vec![location.clone()],
        ),
        (
            RestoreSource::OriginalLocations {
                locations: vec![location.clone()],
                originals: HashMap::new(),
            },
            vec![location],
        ),
        (
            RestoreSource::OriginalLocations {
                locations: Vec::new(),
                originals: HashMap::new(),
            },
            Vec::new(),
        ),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let operation = LocalOperationProvider.restore(
            RestoreRequest {
                id: OperationRequestId(9),
                source,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );

        drop(operation);
        while events.borrow().is_empty() {
            glib::MainContext::default().iteration(true);
        }

        assert!(matches!(
            events.borrow().as_slice(),
            [OperationEvent::Cancelled { result, .. }]
                if result.completed.is_empty()
                    && result.failed.is_empty()
                    && result.not_attempted == expected
        ));
    }
    Ok(())
}

#[test]
fn a_restore_cut_off_before_it_places_the_item_counts_it_as_not_attempted()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let trash = fixture
        .path()
        .join(format!(".Trash-{}", rustix::process::getuid().as_raw()));
    fs::create_dir_all(trash.join("files"))?;
    fs::create_dir_all(trash.join("info"))?;
    fs::create_dir(fixture.path().join("Documents"))?;
    let source = trash.join("files/report");
    fs::write(&source, b"original")?;
    fs::write(
        trash.join("info/report.trashinfo"),
        "[Trash Info]\nPath=Documents/report\n",
    )?;
    let destination = fixture.path().join("Documents/report");
    let cancel = CancelFromMain::default();
    let _scope = FsQuirks::install(
        fixture.path(),
        Quirks {
            before: vec![(Op::Stat, "report", 0, cancel.racer())],
            ..Quirks::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let mut operation = Some(LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(1615),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry: file_entry(&source),
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    ));

    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Cancelled { .. }))
    {
        assert!(Instant::now() < deadline, "{:?}", events.borrow());
        cancel.serve(|| drop(operation.take()));
        context.iteration(false);
        thread::sleep(Duration::from_millis(1));
    }

    assert!(cancel.served());
    let events = events.borrow();
    let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
        panic!("expected a cancel, got {events:?}");
    };
    assert!(result.completed.is_empty());
    assert!(result.failed.is_empty());
    assert_eq!(result.not_attempted, [Location::local(&source)]);
    assert_eq!(fs::read(&source)?, b"original");
    assert!(fs::symlink_metadata(&destination).is_err());
    Ok(())
}

#[test]
fn a_restore_cancelled_while_it_copies_an_item_ends_as_a_cancel() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Items in restore order: "cut" is being copied when the cancel lands,
    // after the first item ended, "taken" fails as its name is in use, and a
    // stuck cut-off copy cannot be removed.
    for (names, stuck) in [
        (&["first", "cut"][..], false),
        (&["first", "cut", "last"], false),
        (&["taken", "cut"], false),
        (&["first", "cut"], true),
    ] {
        let fixture = tempfile::tempdir()?;
        let trash = fixture
            .path()
            .join(format!(".Trash-{}", rustix::process::getuid().as_raw()));
        fs::create_dir_all(trash.join("files"))?;
        fs::create_dir_all(trash.join("info"))?;
        let documents = fixture.path().join("Documents");
        fs::create_dir(&documents)?;
        if names.contains(&"taken") {
            fs::write(documents.join("taken"), b"occupant")?;
        }
        let contents = |name: &str| match name {
            "cut" => vec![0x5a; 4 * 1024 * 1024],
            _ => name.as_bytes().to_vec(),
        };
        for name in names {
            fs::write(trash.join("files").join(name), contents(name))?;
            fs::write(
                trash.join(format!("info/{name}.trashinfo")),
                format!("[Trash Info]\nPath=Documents/{name}\n"),
            )?;
        }
        let cancel = CancelFromMain::default();
        let _scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                before: vec![
                    (Op::CopyRange, "cut", 0, cancel.racer()),
                    (Op::Write, "cut", 0, cancel.racer()),
                    (Op::Stat, "last", 0, cancel.waiter()),
                ],
                fail: if stuck {
                    vec![(Op::Unlink, "cut", 0, Errno::IO)]
                } else {
                    Vec::new()
                },
                reflink: Support::Rejected(Errno::OPNOTSUPP),
                ..Quirks::without_placement()
            },
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let mut operation = Some(
            LocalOperationProvider.restore(
                RestoreRequest {
                    id: OperationRequestId(1615),
                    source: RestoreSource::TrashEntries(
                        names
                            .iter()
                            .map(|name| RestoreTrashItem {
                                entry: file_entry(&trash.join("files").join(name)),
                                destination: documents.join(name),
                            })
                            .collect(),
                    ),
                },
                Rc::new(move |event| emitted.borrow_mut().push(event)),
            ),
        );

        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !events.borrow().iter().any(|event| {
            matches!(
                event,
                OperationEvent::Cancelled { .. }
                    | OperationEvent::Restored { .. }
                    | OperationEvent::RestoreCompletedWithErrors { .. }
            )
        }) {
            assert!(
                Instant::now() < deadline,
                "{names:?}, {stuck}: {:?}",
                events.borrow()
            );
            if events
                .borrow()
                .iter()
                .any(|event| matches!(event, OperationEvent::RestoreProgress { completed: 1, .. }))
            {
                cancel.serve(|| drop(operation.take()));
            }
            context.iteration(false);
            thread::sleep(Duration::from_millis(1));
        }

        assert!(cancel.served(), "{names:?}, {stuck}");
        let events = events.borrow();
        let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
            panic!(
                "{names:?}, {stuck}: expected a cancel, got {:?}",
                events.last()
            );
        };
        let in_trash = |name: &str| Location::local(trash.join("files").join(name));
        let mut failed = Vec::new();
        let mut not_attempted = names[1..].iter().map(|name| in_trash(name)).collect();
        if names[0] == "taken" {
            assert!(result.completed.is_empty());
            failed.push(in_trash("taken"));
            assert_eq!(fs::read(documents.join("taken"))?, b"occupant");
        } else {
            assert_eq!(result.completed, [Location::local(documents.join("first"))]);
            assert_eq!(fs::read(documents.join("first"))?, b"first");
            assert!(!trash.join("info/first.trashinfo").exists());
        }
        if stuck {
            failed.append(&mut not_attempted);
        }
        assert_eq!(result.failed, failed, "{names:?}, {stuck}");
        assert_eq!(result.not_attempted, not_attempted, "{names:?}, {stuck}");
        for name in &names[1..] {
            assert_eq!(fs::read(trash.join("files").join(name))?, contents(name));
            assert!(trash.join(format!("info/{name}.trashinfo")).exists());
            let left = fs::symlink_metadata(documents.join(name)).is_ok();
            assert_eq!(left, stuck && *name == "cut", "{names:?}, {stuck}: {name}");
        }
    }
    Ok(())
}

#[test]
fn restore_rejects_a_volume_orig_path_on_another_device() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let Some((home, stick)) = crate::test_support::distinct_device_dirs(
        "restore_rejects_a_volume_orig_path_on_another_device",
    ) else {
        return Ok(());
    };
    let dest = home.path().join(".config/autostart/payload.desktop");
    let (source, entry) = volume_trash_entry(
        stick.path(),
        "payload",
        &dest.to_string_lossy(),
        b"ssh-ed25519 AAAA attacker",
    )?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(478),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: dest.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::RestoreCompletedWithErrors { message, .. })
                if message.contains("outside the trash volume")
        ),
        "{:?}",
        events.borrow()
    );
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"ssh-ed25519 AAAA attacker");
    assert!(!dest.exists());
    Ok(())
}

#[test]
fn restore_returns_a_volume_item_to_a_path_on_the_same_volume() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    let destination = fixture.path().canonicalize()?.join("Documents/report.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(479),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(&destination)?, b"notes");
    assert!(!source.exists());
    Ok(())
}

#[test]
fn restore_fails_when_the_confirmed_destination_no_longer_matches() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    let confirmed = fixture.path().canonicalize()?.join("Documents/other.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(480),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: confirmed.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::RestoreCompletedWithErrors { message, .. })
                if message.contains("no longer matches the confirmed destination")
        ),
        "{:?}",
        events.borrow()
    );
    assert!(source.exists());
    assert!(!confirmed.exists());
    assert!(!fixture.path().join("Documents/report.txt").exists());
    Ok(())
}

#[test]
fn restore_uses_the_trash_entry_target_path_as_the_physical_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, mut entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    entry.location = Location::uri("trash:///report.txt");
    entry.thumbnail_path = Some(source.clone());
    let destination = fixture.path().canonicalize()?.join("Documents/report.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(481),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(&destination)?, b"notes");
    assert!(!source.exists());
    Ok(())
}

#[test]
fn trash_items_whose_files_share_names_are_restored_whole() -> Result<(), Box<dyn Error>> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let uid = rustix::process::getuid().as_raw();
    let fixture = tempfile::tempdir()?;
    let trash = fixture.path().join(format!(".Trash-{uid}"));
    let names = ["daily.0", "daily.1"];
    fs::create_dir_all(trash.join("info"))?;
    fs::create_dir(fixture.path().join("Backups"))?;
    for name in names {
        fs::create_dir_all(trash.join("files").join(name))?;
        fs::write(
            trash.join("info").join(format!("{name}.trashinfo")),
            format!("[Trash Info]\nPath=Backups/{name}\nDeletionDate=2026-01-01T00:00:00\n"),
        )?;
    }
    let shared = trash.join("files/daily.0/notes.txt");
    fs::write(&shared, b"notes")?;
    fs::hard_link(&shared, trash.join("files/daily.1/notes.txt"))?;
    // Neither Trash copy is deleted before both items are copied.
    let copied = Arc::new(AtomicUsize::new(0));
    let after = (0..names.len())
        .map(|_| {
            let copied = copied.clone();
            let racer: Racer = Box::new(move || {
                copied.fetch_add(1, Ordering::SeqCst);
            });
            (Op::Close, "notes.txt", 0, racer)
        })
        .collect();
    let both = copied.clone();
    let gave_up = Arc::new(AtomicBool::new(false));
    let giving_up = gave_up.clone();
    let wait: Racer = Box::new(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        while both.load(Ordering::SeqCst) < 2 {
            if Instant::now() > deadline {
                giving_up.store(true, Ordering::SeqCst);
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
    });
    let _scope = FsQuirks::install(
        fixture.path(),
        Quirks {
            mount_type: Some(("ext4", "rw")),
            before: vec![(Op::SyncFs, "*", 0, wait)],
            after,
            ..Quirks::without_placement()
        },
    );
    let destination = fixture.path().canonicalize()?.join("Backups");
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(507),
            source: RestoreSource::TrashEntries(
                names
                    .iter()
                    .map(|name| RestoreTrashItem {
                        entry: directory_entry(&trash.join("files").join(name)),
                        destination: destination.join(name),
                    })
                    .collect(),
            ),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        !gave_up.load(Ordering::SeqCst),
        "a Trash copy was synced for deletion before both items were copied"
    );
    let events = events.borrow();
    assert!(
        matches!(events.last(), Some(OperationEvent::Restored { .. })),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, OperationEvent::ItemsReported { .. })),
        "{events:?}"
    );
    assert_eq!(fs::read_dir(trash.join("files"))?.count(), 0);
    assert_eq!(fs::read_dir(trash.join("info"))?.count(), 0);
    for name in names {
        assert_eq!(
            fs::read(destination.join(name).join("notes.txt"))?,
            b"notes"
        );
    }
    Ok(())
}

#[test]
fn a_trash_entry_stays_restorable_while_its_item_is_in_the_trash() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let uid = rustix::process::getuid().as_raw();
    // An entry added to the Trash item once the copy listed it, or an item
    // put at its name in the Trash once the restore removed it.
    for added in [true, false] {
        let fixture = tempfile::tempdir()?;
        let trash = fixture.path().join(format!(".Trash-{uid}"));
        let item = trash.join("files/report");
        fs::create_dir_all(&item)?;
        fs::create_dir_all(trash.join("info"))?;
        fs::create_dir(fixture.path().join("Documents"))?;
        fs::write(item.join("notes.txt"), b"notes")?;
        let info = trash.join("info/report.trashinfo");
        fs::write(
            &info,
            "[Trash Info]\nPath=Documents/report\nDeletionDate=2026-01-01T00:00:00\n",
        )?;
        let (before, after) = if added {
            let added = item.join("added.txt");
            let racer: Racer = Box::new(move || fs::write(&added, b"added").expect("add"));
            (vec![(Op::SyncFs, "*", 0, racer)], Vec::new())
        } else {
            let put = item.clone();
            let racer: Racer = Box::new(move || fs::write(&put, b"another").expect("put"));
            // The first removes the placeholder the placement ladder tried.
            (Vec::new(), vec![(Op::Unlink, "report", 1, racer)])
        };
        let _scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                mount_type: Some(("ext4", "rw")),
                before,
                after,
                ..Quirks::without_placement()
            },
        );
        let destination = fixture.path().canonicalize()?.join("Documents/report");
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.restore(
            RestoreRequest {
                id: OperationRequestId(508),
                source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                    entry: directory_entry(&item),
                    destination: destination.clone(),
                }]),
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        wait_for_restore(&events);

        let events = events.borrow();
        assert!(
            matches!(events.last(), Some(OperationEvent::Restored { .. })),
            "{added}: {events:?}"
        );
        let kept = events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::ItemsReported { report, .. } => Some(report.kept_in_trash.clone()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        let expected = if added {
            vec![KeptItem {
                location: Location::local(&item),
                cause: KeptCause::Changed,
            }]
        } else {
            Vec::new()
        };
        assert_eq!(kept, expected, "{added}");
        assert!(fs::symlink_metadata(&item).is_ok(), "{added}");
        assert!(info.exists(), "{added}: the Trash entry stays restorable");
        assert_eq!(fs::read(destination.join("notes.txt"))?, b"notes");
    }
    Ok(())
}
