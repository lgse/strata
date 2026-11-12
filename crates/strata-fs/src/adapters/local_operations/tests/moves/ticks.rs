// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn later_names_rewritten_within_the_tick_are_kept() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _kernel = KernelGuard::set(Some((6, 12, 0)));
    // Within one coarse tick only a digest tells a same-size rewrite. SMB
    // refuses to open a later name, so none of them can be compared there;
    // which of the unchanged pair is later depends on the listing order.
    for (row, quirks, later_kept) in [
        ("ext4", Quirks::ext4 as fn() -> Quirks, false),
        ("smb", smb_with_one_name_open, true),
    ] {
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = ClockGuard::set(tick);
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(folder.join("sub"))?;
        for (first, second) in [("x", "sub/y"), ("m", "sub/n")] {
            fs::write(folder.join(first), first)?;
            fs::hard_link(folder.join(first), folder.join(second))?;
        }
        let rewritten = folder.join("m");
        let racer: Racer = Box::new(move || fs::write(&rewritten, b"M").expect("rewrite m"));
        let quirks = Quirks {
            before: vec![(Op::OpenDir, "folder", 1, racer)],
            coarse_clock: Some(tick),
            ..quirks()
        };

        let (moved, report, _) = move_across_volumes(&folder, quirks)?;

        assert_eq!(fs::read(moved.join("sub/n"))?, b"m", "{row}");
        let mut left = report.kept;
        left.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
        let unchanged_pair = [folder.join("x"), folder.join("sub/y")];
        let later = unchanged_pair
            .iter()
            .filter(|name| {
                left.iter()
                    .any(|item| item.location == Location::local(name))
            })
            .collect::<Vec<_>>();
        assert_eq!(later.len(), usize::from(later_kept), "{row}: {left:?}");
        let mut expected = vec![
            kept(folder.join("m"), KeptCause::Changed),
            kept(folder.join("sub/n"), KeptCause::Changed),
        ];
        expected.extend(
            later
                .iter()
                .map(|name| kept((*name).clone(), KeptCause::Changed)),
        );
        expected.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
        assert_eq!(left, expected, "{row}");
        for name in &unchanged_pair {
            assert_eq!(name.exists(), later.contains(&name), "{row}: {name:?}");
        }
    }
    Ok(())
}

fn smb_with_one_name_open() -> Quirks {
    Quirks {
        mount_type: Some(("cifs", "rw")),
        name_opens: NameOpens::OneAtATime,
        ..Quirks::default()
    }
}

#[test]
fn linked_names_on_an_smb_source_are_never_open_at_once() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(folder.join("sub"))?;
    fs::write(folder.join("a"), b"linked")?;
    let hour_before = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        - 3600;
    set_times(&folder.join("a"), hour_before, 0)?;
    fs::hard_link(folder.join("a"), folder.join("sub/b"))?;
    fs::hard_link(folder.join("a"), folder.join("sub/c"))?;
    let smb = Quirks {
        mount_type: Some(("cifs", "rw")),
        name_opens: NameOpens::FirstOnly,
        ..Quirks::default()
    };

    let (moved, report, _) = move_across_volumes(&folder, smb)?;

    assert!(fs::symlink_metadata(&folder).is_err());
    assert_eq!(report, TransferReport::default());
    let [a, b, c] = ["a", "sub/b", "sub/c"].map(|name| fs::metadata(moved.join(name)));
    let (a, b, c) = (a?, b?, c?);
    assert_eq!((a.ino(), a.nlink()), (b.ino(), 3));
    assert_eq!(c.ino(), a.ino());
    assert_eq!(fs::read(moved.join("sub/c"))?, b"linked");
    Ok(())
}

#[test]
fn a_file_put_in_a_moved_files_place_within_its_time_tick_is_kept() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(&folder)?;
    let moved_file = folder.join("a.txt");
    fs::write(&moved_file, b"original")?;
    let substituted = moved_file.clone();
    let racer: Racer = Box::new(move || {
        let before = fs::metadata(&substituted).expect("stat the original");
        let other = substituted.with_file_name("other");
        fs::write(&other, b"newfile!").expect("write the other file");
        let modified = rustix::fs::Timespec {
            tv_sec: before.mtime(),
            tv_nsec: before.mtime_nsec(),
        };
        rustix::fs::utimensat(
            rustix::fs::CWD,
            &other,
            &rustix::fs::Timestamps {
                last_access: modified,
                last_modification: modified,
            },
            rustix::fs::AtFlags::empty(),
        )
        .expect("give it the same times");
        fs::rename(&other, &substituted).expect("put it in the original's place");
        super::super::fs_quirks::recreated(&substituted);
    });
    let fat = Quirks {
        mount_type: Some(("vfat", "rw")),
        ctime_is_mtime: true,
        reused_inodes: Some("a.txt"),
        before: vec![(Op::OpenDir, "folder", 1, racer)],
        ..Quirks::default()
    };

    let (moved, report, _) = move_across_volumes(&folder, fat)?;

    assert_eq!(fs::read(moved.join("a.txt"))?, b"original");
    assert_eq!(fs::read(&moved_file)?, b"newfile!");
    assert_eq!(report.kept, [kept(moved_file, KeptCause::Changed)]);
    Ok(())
}

#[test]
fn files_on_a_coarse_mount_inside_a_moved_folder_are_compared() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Multigrain times on the outer mount, so outer.txt, dated around the
    // frozen clock, is never compared and waited for.
    let _kernel = KernelGuard::set(Some((6, 13, 0)));
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let folder = source_dir.join("folder");
    let inner = folder.join("inner");
    fs::create_dir_all(&inner)?;
    fs::write(folder.join("outer.txt"), b"outer")?;
    let file = inner.join("a.txt");
    fs::write(&file, b"original")?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    // exFAT keeps hundredths of a second.
    let fraction = 500_000_000;
    set_times(&file, seconds, fraction)?;
    let _clock =
        ClockGuard::set(epoch + Duration::new(seconds, fraction) + Duration::from_millis(500));
    let path = file.clone();
    let rewrite: Racer = Box::new(move || {
        fs::write(&path, b"ORIGINAL").expect("rewrite the source");
        set_times(&path, seconds, fraction).expect("keep its times");
    });
    let outer = FsQuirks::install(
        &source_dir,
        Quirks {
            before: vec![(Op::OpenDir, "folder", 1, rewrite)],
            ..Quirks::ext4()
        },
    );
    // The outer file is read first, as a single worker does anyway.
    let calls = outer.reader();
    let outer_read: Racer = Box::new(move || {
        let start = Instant::now();
        while !calls()
            .iter()
            .any(|call| call.op == Op::Read && call.names == ["outer.txt"])
        {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "outer.txt was never read"
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
    let _inner = FsQuirks::install(
        &inner,
        Quirks {
            mount_type: Some(("fuseblk", "rw")),
            device_format: Some("exfat"),
            ctime_is_mtime: true,
            no_birth_time: true,
            before: vec![(Op::OpenSource, "a.txt", 0, outer_read)],
            ..Quirks::default()
        },
    );
    let destination = root.path().join("destination");
    fs::create_dir_all(&destination)?;
    let _destination = FsQuirks::install(&destination, Quirks::default());

    let (_operation, events) =
        paste_move(97, &destination, &folder, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    let report = reported(&events);
    assert_eq!(
        fs::read(destination.join("folder/inner/a.txt"))?,
        b"original"
    );
    assert_eq!(fs::read(&file)?, b"ORIGINAL");
    assert_eq!(report.kept, [kept(file, KeptCause::Changed)]);
    Ok(())
}

#[test]
fn files_modified_within_a_tick_of_the_clock_stamping_them_are_compared_before_deletion()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let ramfs = || Quirks {
        mount_type: Some(("ramfs", "rw")),
        ..Quirks::default()
    };
    // Since Linux 6.13, ext4, XFS, btrfs and tmpfs stamp a change right after
    // a stat with a finer time; elsewhere a same-size rewrite within the tick
    // keeps the times. How many milliseconds after the tick the copy reads the
    // file, whether it is rewritten as the deletion starts, and whether the
    // deletion reads it again.
    type Row = (&'static str, (u32, u32, u32), Preset, u64, bool, bool);
    let rows: [Row; 6] = [
        ("ext4 before 6.13", (6, 12, 0), Quirks::ext4, 5, true, true),
        (
            "ext4 before 6.13, untouched",
            (6, 12, 0),
            Quirks::ext4,
            5,
            false,
            true,
        ),
        (
            "ext4 before 6.13, a tick later",
            (6, 12, 0),
            Quirks::ext4,
            150,
            false,
            false,
        ),
        ("ext4 since 6.13", (6, 13, 0), Quirks::ext4, 5, false, false),
        ("ramfs since 6.13", (6, 13, 0), ramfs, 5, true, true),
        ("NFS since 6.13", (6, 13, 0), Quirks::nfs42, 5, true, true),
    ];
    for (row, kernel, preset, read_after, rewritten, read_again) in rows {
        let _kernel = KernelGuard::set(Some(kernel));
        // Before the kernel's coarse clock, which may run a tick behind.
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = ClockGuard::set(tick + Duration::from_millis(read_after));
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(&folder)?;
        let file = folder.join("a.txt");
        fs::write(&file, b"original")?;
        let before = if rewritten {
            vec![(Op::OpenDir, "folder", 1, rewrite(&file, b"ORIGINAL"))]
        } else {
            Vec::new()
        };
        let quirks = Quirks {
            before,
            coarse_clock: Some(tick),
            ..preset()
        };

        let (moved, report, _, calls) = move_across_volumes_logged(&folder, quirks)?;

        assert_eq!(fs::read(moved.join("a.txt"))?, b"original", "{row}");
        if rewritten {
            assert_eq!(fs::read(&file).ok(), Some(b"ORIGINAL".to_vec()), "{row}");
            assert_eq!(report.kept, [kept(file, KeptCause::Changed)], "{row}");
        } else {
            assert!(!folder.exists(), "{row}");
            assert_eq!(report.kept, [], "{row}");
        }
        let opened = calls
            .iter()
            .filter(|call| call.op == Op::OpenSource && call.names == ["a.txt"])
            .count();
        assert_eq!(opened > 1, read_again, "{row}");
    }
    Ok(())
}

#[test]
fn a_source_growing_as_it_is_compared_is_kept_without_reading_what_it_grew_by()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(&folder)?;
    let file = folder.join("a.txt");
    fs::write(&file, b"original")?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    set_times(&file, seconds, 0)?;
    let _clock = ClockGuard::set(epoch + Duration::from_secs(seconds) + Duration::from_millis(500));
    let grown = 64 << 20;
    let path = file.clone();
    let grow: Racer = Box::new(move || {
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_len(grown))
            .expect("grow the source");
    });
    let quirks = Quirks {
        ctime_is_mtime: true,
        after: vec![(Op::OpenSource, "a.txt", 1, grow)],
        ..Quirks::vfat()
    };

    let (moved, report, _, calls) = move_across_volumes_logged(&folder, quirks)?;

    assert_eq!(fs::read(moved.join("a.txt"))?, b"original");
    assert_eq!(fs::metadata(&file)?.len(), grown);
    assert_eq!(report.kept, [kept(file, KeptCause::Changed)]);
    let reopened = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.op == Op::OpenSource && call.names == ["a.txt"])
        .nth(1)
        .map(|(index, _)| index)
        .ok_or("the deletion opened the source to compare it")?;
    let reads = calls[reopened..]
        .iter()
        .filter(|call| call.op == Op::Read && call.names == ["a.txt"])
        .count();
    assert!(reads <= 2, "{reads} reads of a source that grew by 64 MiB");
    Ok(())
}
