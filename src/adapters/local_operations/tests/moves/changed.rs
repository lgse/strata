// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_move_keeps_source_entries_changed_or_added_after_their_copy() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for conflict in MOVES {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(folder.join("sub"))?;
        fs::write(folder.join("a.txt"), b"a")?;
        fs::write(folder.join("sub/b.txt"), b"b")?;
        let changed = folder.join("sub/b.txt");
        let added = folder.join("added.txt");
        let (rewrite, add) = (changed.clone(), added.clone());
        // Once the copy is done, as the deletion opens the moved folder.
        let racer: super::super::fs_quirks::Racer = Box::new(move || {
            fs::write(&rewrite, b"rewritten").expect("rewrite a copied entry");
            fs::write(&add, b"added").expect("add an entry");
        });
        let _source = FsQuirks::install(
            &source_dir,
            Quirks {
                before: vec![(Op::OpenDir, "folder", 1, racer)],
                ..Quirks::default()
            },
        );
        let (destination, _destination) = destination_for(root.path(), conflict)?;

        let (_operation, events) = paste_move(92, &destination, &folder, conflict);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{conflict:?}: {:?}",
            events.borrow()
        );
        let moved = destination.join("folder");
        assert_eq!(fs::read(moved.join("a.txt"))?, b"a", "{conflict:?}");
        assert_eq!(fs::read(moved.join("sub/b.txt"))?, b"b", "{conflict:?}");
        assert_eq!(names(&folder)?, ["added.txt", "sub"], "{conflict:?}");
        assert_eq!(fs::read(&changed)?, b"rewritten", "{conflict:?}");
        assert_eq!(fs::read(&added)?, b"added", "{conflict:?}");
        assert_eq!(
            reported(&events).kept,
            [
                kept(changed, KeptCause::Changed),
                kept(folder.clone(), KeptCause::Changed)
            ],
            "{conflict:?}"
        );
    }
    Ok(())
}

#[test]
fn a_file_written_during_its_own_move_stays_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rewrite = |path: &Path, contents: &'static [u8]| -> super::super::fs_quirks::Racer {
        let path = path.to_path_buf();
        Box::new(move || fs::write(&path, contents).expect("rewrite the source"))
    };
    let cases: [(&str, usize, Preset); 4] = [
        ("in a folder", 1, Quirks::default),
        ("in a folder", 2, Quirks::default),
        ("linked", 2, Quirks::nfs3),
        ("selected", 2, Quirks::default),
    ];
    for (kind, rewrites, destination_quirks) in cases {
        let case = format!("{kind}, rewrites: {rewrites}");
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(&folder)?;
        let db = folder.join("db");
        fs::write(&db, b"first")?;
        let reads = if kind == "linked" {
            fs::hard_link(&db, folder.join("DB"))?;
            "*"
        } else {
            fs::write(folder.join("notes.txt"), b"notes")?;
            "db"
        };
        // Each pass reads the data, then the end of the file.
        let mut before = vec![(Op::Read, reads, 0, rewrite(&db, b"second write"))];
        if rewrites == 2 {
            before.push((Op::Read, reads, 1, rewrite(&db, b"third")));
        }
        let _source = FsQuirks::install(
            &source_dir,
            Quirks {
                before,
                ..Quirks::default()
            },
        );
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let _destination = FsQuirks::install(&destination, destination_quirks());
        let moved = if kind == "selected" { &db } else { &folder };

        let (_operation, events) =
            paste_move(93, &destination, moved, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        let terminal = terminal_transfer(&events.borrow()).cloned();
        if kind == "selected" {
            let Some(OperationEvent::TransferFailed { message, .. }) = terminal else {
                panic!("{case}: {:?}", events.borrow());
            };
            assert_eq!(
                message,
                rust_i18n::t!("%{name} changed while it was being copied", name = "db"),
                "{case}"
            );
            assert!(names(&destination)?.is_empty(), "{case}");
            assert_eq!(fs::read(&db)?, b"third", "{case}");
            continue;
        }
        assert!(
            matches!(terminal, Some(OperationEvent::Pasted { .. })),
            "{case}: {:?}",
            events.borrow()
        );
        let mut left = reported(&events).kept;
        left.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
        if rewrites == 1 {
            assert_eq!(
                fs::read(destination.join("folder/db"))?,
                b"second write",
                "{case}"
            );
            assert!(!folder.exists(), "{case}");
            assert!(left.is_empty(), "{case}");
        } else if kind == "linked" {
            assert!(names(&destination.join("folder"))?.is_empty(), "{case}");
            assert_eq!(names(&folder)?, ["DB", "db"], "{case}");
            assert_eq!(fs::read(&db)?, b"third", "{case}");
            let changed = |name| kept(folder.join(name), KeptCause::Changed);
            assert_eq!(left, [changed("DB"), changed("db")], "{case}");
        } else {
            assert_eq!(names(&destination.join("folder"))?, ["notes.txt"], "{case}");
            assert_eq!(names(&folder)?, ["db"], "{case}");
            assert_eq!(fs::read(&db)?, b"third", "{case}");
            assert_eq!(left, [kept(db.clone(), KeptCause::Changed)], "{case}");
        }
    }
    Ok(())
}

#[test]
fn moving_across_volumes_deletes_every_entry_it_copied() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for selected in ["folder", "link", "pipe"] {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(folder.join("sub"))?;
        fs::write(folder.join("a"), b"linked")?;
        fs::hard_link(folder.join("a"), folder.join("sub/b"))?;
        let outside = source_dir.join("outside");
        fs::write(&outside, b"outside")?;
        fs::hard_link(&outside, folder.join("c"))?;
        std::os::unix::fs::symlink("../a", folder.join("sub/link"))?;
        make_fifo(&folder.join("sub/pipe"))?;
        std::os::unix::fs::symlink("folder/a", source_dir.join("link"))?;
        make_fifo(&source_dir.join("pipe"))?;
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let _volumes = separate_volumes(&source_dir, &destination);
        let source = source_dir.join(selected);

        let (_operation, events) =
            paste_move(94, &destination, &source, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{selected}: {:?}",
            events.borrow()
        );
        assert!(fs::symlink_metadata(&source).is_err(), "{selected}");
        assert_eq!(reported(&events), TransferReport::default(), "{selected}");
        let moved = destination.join(selected);
        match selected {
            "folder" => {
                let (a, b) = (
                    fs::metadata(moved.join("a"))?,
                    fs::metadata(moved.join("sub/b"))?,
                );
                assert_eq!((a.ino(), a.nlink()), (b.ino(), 2));
                assert_eq!(fs::read(moved.join("c"))?, b"outside");
                assert_eq!(names(&moved.join("sub"))?, ["b", "link", "pipe"]);
                assert_eq!(fs::metadata(&outside)?.nlink(), 1);
            }
            "link" => assert_eq!(fs::read_link(&moved)?, Path::new("folder/a")),
            _ => assert!(fs::symlink_metadata(&moved)?.file_type().is_fifo()),
        }
    }
    Ok(())
}

#[test]
fn a_name_of_a_linked_file_changed_after_its_other_name_went_stays() -> Result<(), Box<dyn Error>> {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let _serial = serial()?;
    let cifs = || Quirks {
        mount_type: Quirks::cifs().mount_type,
        ..Quirks::default()
    };
    for (row, source, numbers_go, evicted) in [
        ("ext4", Quirks::ext4 as Preset, false, false),
        ("cifs, server inode numbers turned off", cifs, true, false),
        (
            "cifs, server inode numbers turned off, names numbered afresh after the copy",
            cifs,
            true,
            true,
        ),
    ] {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("a"), b"linked")?;
        if !hard_linked(&folder.join("a"), &folder.join("b"))? {
            eprintln!("skipped on a base without hard links: {row}");
            continue;
        }
        if !numbers_go && super::super::fs_quirks::smb_base(fs::File::open(&folder)?.as_fd()) {
            eprintln!(
                "skipped on an SMB base, which refuses to open a link's second name while the first is open: {row}"
            );
            continue;
        }
        let hour_before = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs()
            - 3600;
        set_times(&folder.join("a"), hour_before, 0)?;
        if numbers_go && !a_later_change_time(&folder.join("b"), &folder.join("a"), hour_before)? {
            eprintln!(
                "skipped on a base that reports the modification time as the change time: {row}"
            );
            continue;
        }
        let (deleting, checked) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let rewrite_second = |name: &'static str| -> super::super::fs_quirks::Racer {
            let (deleting, checked, path, probe) = (
                deleting.clone(),
                checked.clone(),
                folder.join(name),
                root.path().join("probe"),
            );
            Box::new(move || {
                if deleting.load(Ordering::Acquire) && checked.fetch_add(1, Ordering::SeqCst) == 1 {
                    let before = fs::metadata(&path).expect("stat the file");
                    wait_for_a_later_change_time(&probe, &before);
                    fs::write(&path, b"LINKED").expect("rewrite the file");
                    set_times(&path, hour_before, 0).expect("put the times back");
                }
            })
        };
        let went = Arc::new(AtomicBool::new(!numbers_go));
        let (opening, renumbering, linked) = (
            deleting.clone(),
            went.clone(),
            [folder.join("a"), folder.join("b")],
        );
        let deletion_opens: super::super::fs_quirks::Racer = Box::new(move || {
            if evicted {
                renumbering.fetch_and(
                    linked
                        .iter()
                        .all(|path| super::super::fs_quirks::evict(path)),
                    Ordering::AcqRel,
                );
            }
            opening.store(true, Ordering::Release);
        });
        let mut before = vec![(Op::OpenDir, "folder", 1, deletion_opens)];
        if numbers_go {
            let (at, going) = (source_dir.clone(), went.clone());
            let numbers_went: super::super::fs_quirks::Racer = Box::new(move || {
                going.store(
                    super::super::fs_quirks::autodisable_serverino(&at),
                    Ordering::Release,
                );
            });
            before.push((Op::Stat, "folder", 0, numbers_went));
        }
        for name in ["a", "b", "a", "b"] {
            before.push((Op::Stat, name, 0, rewrite_second(name)));
        }
        let quirks = Quirks { before, ..source() };

        let (moved, report, _) = move_across_volumes(&folder, quirks)?;

        assert!(went.load(Ordering::Acquire), "{row}");
        assert_eq!(fs::read(moved.join("a"))?, b"linked", "{row}");
        assert_eq!(fs::read(moved.join("b"))?, b"linked", "{row}");
        assert!(folder.exists(), "{row}");
        let left = names(&folder)?;
        assert_eq!(left.len(), 1, "{row}: {left:?}");
        let second = folder.join(&left[0]);
        assert_eq!(fs::read(&second)?, b"LINKED", "{row}");
        assert_eq!(report.kept, [kept(second, KeptCause::Changed)], "{row}");
    }
    Ok(())
}

fn make_fifo(path: &Path) -> io::Result<()> {
    rustix::fs::mknodat(
        rustix::fs::CWD,
        path,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )?;
    Ok(())
}

#[test]
fn a_rewrite_that_keeps_the_size_and_times_is_kept_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unstable = || Quirks {
        mount_type: Some(("cifs", "rw,vers=3.1.1")),
        ..Quirks::default()
    };
    for (row, quirks) in [
        ("stable", Quirks::default as Preset),
        ("unstable", unstable),
    ] {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(&folder)?;
        let changed = folder.join("b.txt");
        fs::write(&changed, b"b1")?;
        let path = changed.clone();
        let probe = root.path().join("probe");
        // After the copy, as the deletion opens the folder: only the change
        // time tells.
        let racer: super::super::fs_quirks::Racer = Box::new(move || {
            let before = fs::metadata(&path).expect("stat the source");
            wait_for_a_later_change_time(&probe, &before);
            fs::write(&path, b"B2").expect("rewrite the source");
            let modified = rustix::fs::Timespec {
                tv_sec: before.mtime(),
                tv_nsec: before.mtime_nsec(),
            };
            rustix::fs::utimensat(
                rustix::fs::CWD,
                &path,
                &rustix::fs::Timestamps {
                    last_access: modified,
                    last_modification: modified,
                },
                rustix::fs::AtFlags::empty(),
            )
            .expect("put the times back");
        });
        let quirks = Quirks {
            before: vec![(Op::OpenDir, "folder", 1, racer)],
            ..quirks()
        };

        let (moved, report, _) = move_across_volumes(&folder, quirks)?;

        assert_eq!(fs::read(moved.join("b.txt"))?, b"b1", "{row}");
        assert_eq!(fs::read(&changed)?, b"B2", "{row}");
        assert_eq!(report.kept, [kept(changed, KeptCause::Changed)], "{row}");
    }
    Ok(())
}

#[test]
fn a_file_rewritten_as_it_is_read_is_copied_again_whole() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Each pass reads the data, then the end of the file. Every write lands in
    // one tick of a coarse clock, so only a file last written in an earlier
    // tick shows a same-size rewrite in its times. Before Linux 6.13, a file
    // modified within the tick is copied for a digest.
    for (row, kernel) in [
        ("same size", (6, 13, 0)),
        ("shrinks", (6, 13, 0)),
        ("shrinks, digested", (6, 12, 0)),
    ] {
        let _kernel = KernelGuard::set(Some(kernel));
        // Before the kernel's coarse clock, which may run a tick behind.
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = ClockGuard::set(tick);
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(&folder)?;
        let db = folder.join("db");
        fs::write(&db, b"first")?;
        let (before, expected): (Vec<_>, &[u8]) = if row == "same size" {
            let hour_before = tick.duration_since(SystemTime::UNIX_EPOCH)?.as_secs() - 3600;
            set_times(&db, hour_before, 0)?;
            (vec![(Op::Read, "db", 1, rewrite(&db, b"other"))], b"other")
        } else {
            (
                vec![
                    (Op::Read, "db", 0, rewrite(&db, b"a much longer version")),
                    (Op::Read, "db", 0, rewrite(&db, b"short")),
                ],
                b"short",
            )
        };
        let quirks = Quirks {
            before,
            coarse_clock: Some(tick),
            ..Quirks::ext4()
        };

        let (moved, report, events) = move_across_volumes(&folder, quirks)?;

        assert_eq!(fs::read(moved.join("db"))?, expected, "{row}");
        assert!(!folder.exists(), "{row}");
        assert_eq!(report, TransferReport::default(), "{row}");
        if row == "same size" {
            // The second read repeats bytes already counted.
            let transferred = events.iter().rev().find_map(|event| match event {
                OperationEvent::TransferProgress {
                    transferred_bytes, ..
                } => Some(*transferred_bytes),
                _ => None,
            });
            assert_eq!(transferred, Some(5), "{row}");
        }
    }
    Ok(())
}

#[test]
fn a_file_that_reports_no_size_is_moved_whole() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for in_folder in [false, true] {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(&folder)?;
        let file = folder.join("cpuinfo");
        fs::write(&file, b"processor : 0\n")?;
        let hour_before = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs()
            - 3600;
        set_times(&file, hour_before, 0)?;
        let quirks = Quirks {
            mount_type: Some(("fuse.lxcfs", "rw")),
            virtual_sizes: true,
            ..Quirks::default()
        };

        let item = if in_folder { &folder } else { &file };
        let (moved, report, _) = move_across_volumes(item, quirks)?;

        assert_eq!(
            report,
            TransferReport::default(),
            "in a folder: {in_folder}"
        );
        assert!(!file.exists(), "in a folder: {in_folder}");
        let moved = if in_folder {
            moved.join("cpuinfo")
        } else {
            moved
        };
        assert_eq!(
            fs::read(moved)?,
            b"processor : 0\n",
            "in a folder: {in_folder}"
        );
    }
    Ok(())
}

#[test]
fn moves_off_mounts_without_stable_inodes_keep_only_what_changed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(folder.join("sub"))?;
    fs::write(folder.join("a.txt"), b"a")?;
    fs::write(folder.join("sub/b.txt"), b"b")?;
    // Removing a name of a file changes the other's change time, so only size
    // and modification time tell.
    for (first, second, contents) in [
        ("x", "sub/y", "x"),
        ("m", "sub/n", "m"),
        ("p", "sub/q", "p"),
    ] {
        fs::write(folder.join(first), contents)?;
        fs::hard_link(folder.join(first), folder.join(second))?;
    }
    // Settled times, so a rewrite shows in them on any kernel.
    let hour_before = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        - 3600;
    for file in ["a.txt", "sub/b.txt", "x", "m", "p"] {
        set_times(&folder.join(file), hour_before, 0)?;
    }
    let evicted = [
        "",
        "a.txt",
        "sub",
        "sub/b.txt",
        "x",
        "sub/y",
        "m",
        "sub/n",
        "p",
        "sub/q",
    ]
    .map(|path| folder.join(path));
    let changed = folder.clone();
    // The kernel evicts every entry between the copy and the deletion, while
    // files are rewritten. cifs counts links, which sshfs does not.
    let racer: super::super::fs_quirks::Racer = Box::new(move || {
        fs::write(changed.join("sub/b.txt"), b"rewritten").expect("rewrite b");
        fs::write(changed.join("m"), b"M").expect("rewrite m");
        let resized = changed.join("p");
        let before = fs::metadata(&resized).expect("stat p");
        fs::write(&resized, b"longer").expect("rewrite p");
        let modified = rustix::fs::Timespec {
            tv_sec: before.mtime(),
            tv_nsec: before.mtime_nsec(),
        };
        rustix::fs::utimensat(
            rustix::fs::CWD,
            &resized,
            &rustix::fs::Timestamps {
                last_access: modified,
                last_modification: modified,
            },
            rustix::fs::AtFlags::empty(),
        )
        .expect("put the times back");
        for path in &evicted {
            super::super::fs_quirks::renumbered(path);
        }
    });
    let quirks = Quirks {
        mount_type: Some(("cifs", "rw,vers=3.1.1")),
        before: vec![(Op::OpenDir, "folder", 1, racer)],
        ..Quirks::default()
    };

    let (moved, report, _) = move_across_volumes(&folder, quirks)?;

    assert_eq!(fs::read(moved.join("sub/b.txt"))?, b"b");
    assert_eq!(fs::read(moved.join("sub/y"))?, b"x");
    assert_eq!(names(&folder)?, ["m", "p", "sub"]);
    assert_eq!(names(&folder.join("sub"))?, ["b.txt", "n", "q"]);
    let mut left = report.kept;
    left.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
    let expected = ["m", "p", "sub/b.txt", "sub/n", "sub/q"]
        .map(|path| kept(folder.join(path), KeptCause::Changed));
    assert_eq!(left, expected);
    Ok(())
}

#[test]
fn entries_swapped_or_removed_after_their_copy_are_never_deleted() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let vfat = || Quirks {
        mount_type: Some(("vfat", "rw")),
        ..Quirks::default()
    };
    let rows: [(&str, Preset); 4] = [
        ("swapped", Quirks::default),
        ("removed", Quirks::default),
        ("put back", Quirks::default),
        ("put back", vfat),
    ];
    for (row, source) in rows {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(folder.join("sub"))?;
        fs::write(folder.join("a.txt"), b"a")?;
        fs::write(folder.join("sub/b.txt"), b"b")?;
        let at = folder.clone();
        let (before, after) = match row {
            "swapped" => {
                let racer: super::super::fs_quirks::Racer = Box::new(move || {
                    fs::remove_dir_all(at.join("sub")).expect("remove the folder");
                    fs::write(at.join("sub"), b"a file now").expect("put a file there");
                });
                (vec![(Op::OpenDir, "sub", 1, racer)], Vec::new())
            }
            "removed" => {
                let racer: super::super::fs_quirks::Racer = Box::new(move || {
                    fs::remove_file(at.join("a.txt")).expect("remove the file");
                });
                (vec![(Op::OpenDir, "folder", 1, racer)], Vec::new())
            }
            _ => {
                let racer: super::super::fs_quirks::Racer = Box::new(move || {
                    fs::rename(&at, at.with_file_name("emptied")).expect("move it");
                    fs::create_dir(&at).expect("make another folder");
                });
                (Vec::new(), vec![(Op::Unlink, "a.txt", 0, racer)])
            }
        };
        let quirks = Quirks {
            before,
            after,
            ..source()
        };

        let (moved, report, _) = move_across_volumes(&folder, quirks)?;

        assert_eq!(fs::read(moved.join("sub/b.txt"))?, b"b", "{row}");
        match row {
            "swapped" => {
                assert_eq!(fs::read(folder.join("sub"))?, b"a file now", "{row}");
                assert_eq!(report.kept, [kept(folder.join("sub"), KeptCause::Changed)]);
            }
            "removed" => {
                assert!(!folder.exists(), "{row}");
                assert_eq!(report, TransferReport::default(), "{row}");
            }
            _ => {
                assert_eq!(names(&folder)?, Vec::<String>::new(), "{row}");
                assert_eq!(report.kept, [kept(folder.clone(), KeptCause::Changed)]);
            }
        }
    }
    Ok(())
}
