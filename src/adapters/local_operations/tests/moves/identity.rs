// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_source_is_deleted_once_its_copy_was_stamped_again_at_close() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // A FUSE writeback cache stamps the close time, within the 2 s of a
    // recent source's that the times check lets stand. An NFSv4.2 write
    // delegation stamps any copy, which gets the source's times again after
    // the close, by name where its owner may neither read nor write it.
    let fuse_writeback = || Quirks {
        times_reset_at_close: true,
        ..Quirks::sshfs()
    };
    let rows: [(&str, Preset, bool, Option<u32>, bool); 4] = [
        ("FUSE writeback cache", fuse_writeback, false, None, true),
        ("NFSv4.2", Quirks::nfs42, false, None, false),
        ("NFSv4.2, inside a folder", Quirks::nfs42, true, None, false),
        (
            "NFSv4.2, unreadable",
            Quirks::nfs42,
            false,
            Some(0o044),
            false,
        ),
    ];
    for (row, preset, in_folder, mode, recent) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(sources.join("folder"))?;
        fs::create_dir(&destination)?;
        let source = sources.join("folder/notes.txt");
        fs::write(&source, b"notes")?;
        if !recent {
            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
            set_times(&source, now.as_secs() - 3600, 0)?;
        }
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                reported_mode: mode.map(|mode| ("notes.txt", mode)),
                ..Quirks::default()
            },
        );
        let _destination = FsQuirks::install(&destination, preset());
        let (item, moved) = if in_folder {
            (sources.join("folder"), destination.join("folder/notes.txt"))
        } else {
            (source.clone(), destination.join("notes.txt"))
        };

        let (_operation, events) =
            paste_move(98, &destination, &item, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(!item.exists(), "{row}: {:?}", reported(&events).kept);
        fs::set_permissions(&moved, fs::Permissions::from_mode(0o644))?;
        assert_eq!(fs::read(moved)?, b"notes", "{row}");
    }
    Ok(())
}

#[test]
fn a_copy_replaced_before_its_times_are_settled_keeps_its_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Outside sshfs's 3 s window, so the copy of a recent source is not
    // compared by content.
    let fuse_daemon = || Quirks {
        mount_type: Some(("fuse.rclone", "rw,user_id=1000,group_id=1000")),
        ..Quirks::sshfs()
    };
    let rows: [(&str, Preset, u64, u32); 3] = [
        ("times set again", Quirks::sshfs, 3600, 0o644),
        ("times set again by name", Quirks::sshfs, 3600, 0o044),
        ("times let stand", fuse_daemon, 1, 0o644),
    ];
    for (row, preset, age, mode) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        let source = sources.join("folder");
        fs::create_dir_all(&source)?;
        fs::create_dir(&destination)?;
        let notes = source.join("notes.txt");
        fs::write(&notes, b"notes")?;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
        set_times(&notes, now.as_secs() - age, 500_000_000)?;
        let replaced = Arc::new(AtomicBool::new(false));
        let replacing = replaced.clone();
        let staged = destination.clone();
        let racer: super::super::fs_quirks::Racer = Box::new(move || {
            for entry in fs::read_dir(&staged).expect("list the destination") {
                let copy = entry.expect("an entry").path().join("notes.txt");
                if copy.exists() {
                    fs::write(&copy, b"another program's file").expect("replace the copy");
                    fs::set_permissions(&copy, fs::Permissions::from_mode(mode))
                        .expect("set the replacement's mode");
                    replacing.store(true, Ordering::Release);
                    return;
                }
            }
        });
        let _sources = FsQuirks::install(&sources, Quirks::ext4());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                after: vec![(Op::Close, "notes.txt", 0, racer)],
                ..preset()
            },
        );

        let (_operation, events) =
            paste_move(98, &destination, &source, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(replaced.load(Ordering::Acquire), "{row}");
        assert_eq!(fs::read(&notes).ok(), Some(b"notes".to_vec()), "{row}");
        assert_eq!(
            reported(&events).kept,
            [kept(source, KeptCause::CopyGone)],
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn a_special_file_move_keeps_its_source_without_proven_published_ownership()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (pipe, can_place, replace) in [
        (false, true, true),
        (true, true, true),
        (false, false, true),
        (true, false, true),
        (false, false, false),
        (true, false, false),
    ] {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join("special");
        if pipe {
            rustix::fs::mknodat(
                rustix::fs::CWD,
                &source,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_raw_mode(0o600),
                0,
            )?;
        } else {
            std::os::unix::fs::symlink("original-target", &source)?;
        }
        let arrival = root.path().join("arrival");
        fs::write(&arrival, b"another program's file")?;
        fs::set_permissions(&arrival, fs::Permissions::from_mode(0o640))?;
        let replaced = Arc::new(AtomicBool::new(false));
        let creation = if pipe { Op::CreateNode } else { Op::Symlink };
        let after = std::iter::once(creation)
            .chain(can_place.then_some(Op::Rename))
            .filter(|_| replace)
            .map(|op| {
                let arrival = arrival.clone();
                let target = destination.join("special");
                let replaced = replaced.clone();
                let racer: super::super::fs_quirks::Racer = Box::new(move || {
                    if !replaced.swap(true, Ordering::AcqRel) {
                        fs::rename(&arrival, &target).expect("replace the published special file");
                    }
                });
                (op, "special", 0, racer)
            })
            .collect();
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                after,
                ..if can_place {
                    Quirks::default()
                } else {
                    Quirks::without_placement()
                }
            },
        );
        let (_operation, events) =
            paste_move(99, &destination, &source, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        let row = format!("pipe: {pipe}, can_place: {can_place}, replace: {replace}");
        assert_eq!(replaced.load(Ordering::Acquire), replace, "{row}");
        // A link made again at its name holds the copy once it points where
        // the staged link did.
        let proven = !pipe && !replace;
        assert_eq!(
            fs::symlink_metadata(&source).is_ok(),
            !proven,
            "{row}: {:?}",
            events.borrow()
        );
        if replace {
            assert_eq!(
                fs::read(destination.join("special"))?,
                b"another program's file"
            );
            assert_eq!(
                fs::metadata(destination.join("special"))?.mode() & 0o777,
                0o640
            );
        } else {
            let expected = if proven {
                Vec::new()
            } else {
                vec![kept(source.clone(), KeptCause::CopyGone)]
            };
            assert_eq!(reported(&events).kept, expected, "{row}");
            if pipe {
                assert!(
                    fs::symlink_metadata(destination.join("special"))?
                        .file_type()
                        .is_fifo()
                );
            } else {
                assert_eq!(
                    fs::read_link(destination.join("special"))?,
                    PathBuf::from("original-target")
                );
            }
        }
        if pipe {
            assert!(fs::symlink_metadata(&source)?.file_type().is_fifo());
        } else if !proven {
            assert_eq!(fs::read_link(&source)?, PathBuf::from("original-target"));
        }
    }
    Ok(())
}

#[test]
fn copies_a_mount_numbers_afresh_are_known_by_birth_time_or_else_by_inode_number()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Change {
        None,
        /// Rewritten in place with as many bytes, a tick after the source.
        Rewritten,
        /// Replaced by another file of its size within the tick that stamped
        /// the source.
        ReplacedWithinTheTick,
        /// Replaced by another file of its size and modification time.
        ReplacedAlike,
        /// Renamed away, with what the merge placed in it, and another folder
        /// made at its name.
        FolderReplaced,
        FolderEvicted,
    }
    // exFAT and FUSE daemons without `use_ino` number an entry afresh once
    // the kernel evicts it, here right after the copy is published; on ext4,
    // which keeps inode numbers, another number is another file. Merging into
    // a folder on exFAT sets its birth time again. Whether the move deletes
    // its source.
    let (fail, merge) = (TransferConflict::FailIfExists, TransferConflict::Merge);
    let rows: [(Preset, &str, TransferConflict, Change, bool); 11] = [
        (Quirks::sshfs, "notes.txt", fail, Change::None, false),
        (Quirks::sshfs, "folder", fail, Change::FolderEvicted, false),
        (
            Quirks::exfat_fuse,
            "folder",
            fail,
            Change::FolderEvicted,
            false,
        ),
        (Quirks::vfat, "folder", fail, Change::None, true),
        (Quirks::exfat, "folder", fail, Change::None, true),
        (Quirks::exfat, "folder", fail, Change::Rewritten, false),
        (
            Quirks::exfat_fuse,
            "folder",
            fail,
            Change::ReplacedWithinTheTick,
            false,
        ),
        (Quirks::ext4, "notes.txt", fail, Change::None, false),
        (Quirks::exfat, "folder", merge, Change::None, true),
        (Quirks::exfat, "folder", merge, Change::ReplacedAlike, false),
        (Quirks::vfat, "folder", merge, Change::FolderReplaced, false),
    ];
    for (preset, item, conflict, change, deleted) in rows {
        let row = format!(
            "{:?}, {item}, {conflict:?}, {change:?}",
            preset().mount_type
        );
        let within_the_tick = change == Change::ReplacedWithinTheTick;
        // Before the kernel's coarse clock, which may run a tick behind.
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = within_the_tick.then(|| ClockGuard::set(tick));
        // Multigrain source times: only the destination can call for a
        // comparison of contents.
        let _kernel = KernelGuard::set(Some((6, 13, 0)));
        let coarse_clock = within_the_tick.then_some(tick);
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join(item);
        let copy = destination.join(item);
        let mut copied = vec![copy.clone()];
        let notes = if item == "folder" {
            fs::create_dir_all(source.join("sub"))?;
            fs::write(source.join("sub/b.txt"), b"b")?;
            copied.extend(["notes.txt", "sub", "sub/b.txt"].map(|path| copy.join(path)));
            source.join("notes.txt")
        } else {
            source.clone()
        };
        if conflict == merge {
            fs::create_dir(&copy)?;
            fs::write(copy.join("stays.txt"), b"stays")?;
        }
        fs::write(&notes, b"notes")?;
        let arrival = root.path().join("arrival");
        fs::write(&arrival, b"other")?;
        if matches!(change, Change::Rewritten | Change::ReplacedAlike) {
            let hour_before = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
            for file in [&notes, &arrival] {
                set_times(file, hour_before.as_secs() - 3600, 0)?;
            }
        }
        // What replaces the copy must be born in a later tick than what it stands in for.
        let probe = root.path().join("probe");
        match change {
            Change::ReplacedAlike => wait_for_a_later_birth_time(&probe, &fs::metadata(&arrival)?),
            Change::FolderReplaced => wait_for_a_later_birth_time(&probe, &fs::metadata(&copy)?),
            _ => {}
        }
        let copied_notes = if item == "folder" {
            copy.join("notes.txt")
        } else {
            copy.clone()
        };
        let replaced = copied_notes.clone();
        let merged = copy.clone();
        if change == Change::FolderEvicted {
            copied.truncate(1);
        }
        let evicted = Arc::new(AtomicBool::new(false));
        let evicting = evicted.clone();
        let racer: super::super::fs_quirks::Racer = Box::new(move || {
            match change {
                Change::None | Change::FolderEvicted => {}
                Change::Rewritten => fs::write(&replaced, b"NOTES").expect("rewrite the copy"),
                Change::ReplacedWithinTheTick | Change::ReplacedAlike => {
                    fs::rename(&arrival, &replaced).expect("replace the copy");
                }
                Change::FolderReplaced => {
                    fs::rename(&merged, merged.with_file_name("old")).expect("rename it away");
                    fs::create_dir(&merged).expect("make another folder");
                }
            }
            for path in &copied {
                super::super::fs_quirks::renumbered(path);
            }
            evicting.store(true, Ordering::Release);
        });
        let placed_last = if conflict == merge { "sub" } else { item };
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                coarse_clock,
                ..Quirks::ext4()
            },
        );
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                rename_flags: Support::Supported,
                tmpfile: Support::Rejected(Errno::OPNOTSUPP),
                coarse_clock,
                after: vec![(Op::Rename, placed_last, 0, racer)],
                ..preset()
            },
        );

        let (_operation, events) = paste_move(98, &destination, &source, conflict);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{row}: {:?}",
            events.borrow()
        );
        assert!(evicted.load(Ordering::Acquire), "{row}");
        if deleted {
            assert!(!source.exists(), "{row}: {:?}", reported(&events).kept);
            assert_eq!(fs::read(&copied_notes)?, b"notes", "{row}");
        } else {
            assert_eq!(fs::read(&notes).ok(), Some(b"notes".to_vec()), "{row}");
            assert_eq!(
                reported(&events).kept,
                [kept(source, KeptCause::CopyGone)],
                "{row}"
            );
        }
    }
    Ok(())
}

#[test]
fn copied_links_a_mount_numbers_afresh_are_known_by_their_size_and_time()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let noserverino = || Quirks {
        mount_type: Some(("cifs", "rw,vers=3.1.1")),
        ..Quirks::default()
    };
    let sshfs_renaming = || Quirks {
        rename_flags: Support::Supported,
        ..Quirks::sshfs()
    };
    // The placed link is numbered afresh, and on sshfs, which leaves a
    // link's times as it was made, another link may take its place first.
    // Without `RENAME_NOREPLACE`, a merge and the publication of a selected
    // link make the link again at its name. Whether another link takes its
    // place.
    let merge = TransferConflict::Merge;
    let fail = TransferConflict::FailIfExists;
    let rows: [(Preset, &str, TransferConflict, bool); 10] = [
        (sshfs_renaming, "link", fail, true),
        (sshfs_renaming, "folder", fail, true),
        (sshfs_renaming, "folder", merge, true),
        (Quirks::sshfs, "link", fail, true),
        (Quirks::sshfs, "link", fail, false),
        (Quirks::sshfs, "folder", merge, true),
        (Quirks::sshfs, "folder", merge, false),
        (noserverino, "link", fail, false),
        (noserverino, "folder", fail, false),
        (noserverino, "folder", merge, false),
    ];
    for (preset, item, conflict, replaced) in rows {
        let made_again = matches!(preset().rename_flags, Support::Rejected(_));
        let row = format!(
            "{:?}, {item}, {conflict:?}, made again: {made_again}, replaced: {replaced}",
            preset().mount_type
        );
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join(item);
        let copy = destination.join(item);
        let (link, copied) = if item == "folder" {
            fs::create_dir(&source)?;
            (source.join("link"), copy.join("link"))
        } else {
            (source.clone(), copy.clone())
        };
        let placed = if conflict == merge {
            fs::create_dir(&copy)?;
            "link"
        } else {
            item
        };
        std::os::unix::fs::symlink("original-target", &link)?;
        let arrival = root.path().join("arrival");
        std::os::unix::fs::symlink("another-target", &arrival)?;
        let evicted = Arc::new(AtomicBool::new(false));
        let evicting = evicted.clone();
        let replacing = copied.clone();
        let racer: super::super::fs_quirks::Racer = Box::new(move || {
            if replaced {
                fs::rename(&arrival, &replacing).expect("replace the link");
            }
            super::super::fs_quirks::renumbered(&replacing);
            evicting.store(true, Ordering::Release);
        });
        let _sources = FsQuirks::install(&sources, Quirks::ext4());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                tmpfile: Support::Rejected(Errno::OPNOTSUPP),
                // A link in a staged folder is first made there under its name.
                after: vec![if made_again {
                    (Op::Symlink, placed, usize::from(item == "folder"), racer)
                } else {
                    (Op::Rename, placed, 0, racer)
                }],
                ..preset()
            },
        );

        let (_operation, events) = paste_move(98, &destination, &source, conflict);
        pump_until_transfer(&events);

        assert!(evicted.load(Ordering::Acquire), "{row}");
        let original = Some(PathBuf::from("original-target"));
        if replaced {
            assert_eq!(fs::read_link(&link).ok(), original, "{row}");
            assert_eq!(
                reported(&events).kept,
                [kept(source, KeptCause::CopyGone)],
                "{row}"
            );
        } else {
            assert!(
                fs::symlink_metadata(&source).is_err(),
                "{row}: {:?}",
                reported(&events).kept
            );
            assert_eq!(fs::read_link(&copied).ok(), original, "{row}");
        }
    }
    Ok(())
}

#[test]
fn a_replacing_move_keeps_the_actual_copy_out_identity() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    let source = sources.join("folder");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(destination.join("folder"))?;
    fs::write(source.join("notes.txt"), b"notes")?;
    fs::write(destination.join("folder/old.txt"), b"old")?;
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let destination_scope = FsQuirks::install(
        &destination,
        Quirks {
            exchange: super::super::fs_quirks::Exchange::Rejected,
            ..Quirks::vboxsf()
        },
    );
    let (_operation, events) =
        paste_move(98, &destination, &source, TransferConflict::ReplaceExisting);
    pump_until_transfer(&events);
    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(destination.join("folder/notes.txt"))?, b"notes");
    assert!(
        destination_scope
            .calls()
            .iter()
            .any(|call| call.op == Op::Mkdir && call.names.iter().any(|name| name == "folder")),
        "exclusive copy-out created the final folder"
    );
    assert!(!source.exists());
    assert!(reported(&events).kept.is_empty(), "{:?}", reported(&events));
    Ok(())
}
