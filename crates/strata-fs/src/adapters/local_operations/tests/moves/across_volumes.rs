// SPDX-License-Identifier: MIT

use super::*;

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

/// A folder of linked names, a link and a pipe, a name linked from outside
/// it, and a link and pipe beside it, in `source_dir`. Returns the outside
/// file.
fn linked_tree(source_dir: &Path) -> io::Result<PathBuf> {
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
    Ok(outside)
}

fn assert_moved_entry(selected: &str, moved: &Path, outside: &Path) -> io::Result<()> {
    match selected {
        "folder" => {
            let (a, b) = (
                fs::metadata(moved.join("a"))?,
                fs::metadata(moved.join("sub/b"))?,
            );
            assert_eq!((a.ino(), a.nlink()), (b.ino(), 2));
            assert_eq!(fs::read(moved.join("c"))?, b"outside");
            assert_eq!(names(&moved.join("sub"))?, ["b", "link", "pipe"]);
            assert_eq!(fs::metadata(outside)?.nlink(), 1);
        }
        "link" => assert_eq!(fs::read_link(moved)?, Path::new("folder/a")),
        _ => assert!(fs::symlink_metadata(moved)?.file_type().is_fifo()),
    }
    Ok(())
}

#[test]
fn moving_across_volumes_deletes_every_entry_it_copied() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for selected in ["folder", "link", "pipe"] {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let outside = linked_tree(&source_dir)?;
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let _volumes = separate_volumes(&source_dir, &destination);
        let source = source_dir.join(selected);

        let (_operation, events) =
            paste_move(94, &destination, &source, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert_pasted(&events, selected);
        assert!(fs::symlink_metadata(&source).is_err(), "{selected}");
        assert_eq!(reported(&events), TransferReport::default(), "{selected}");
        assert_moved_entry(selected, &destination.join(selected), &outside)?;
    }
    Ok(())
}

/// The reads a rewrite comes before, and what the copy must hold.
type Rereads = (Vec<Before>, &'static [u8]);

/// The reads `row` rewrites `db` before, and what its copy must hold.
fn rereads(row: &str, db: &Path, tick: SystemTime) -> Result<Rereads, Box<dyn Error>> {
    if row == "same size" {
        let hour_before = tick.duration_since(SystemTime::UNIX_EPOCH)?.as_secs() - 3600;
        set_times(db, hour_before, 0)?;
        return Ok((vec![(Op::Read, "db", 1, rewrite(db, b"other"))], b"other"));
    }
    let before = vec![
        (Op::Read, "db", 0, rewrite(db, b"a much longer version")),
        (Op::Read, "db", 0, rewrite(db, b"short")),
    ];
    Ok((before, b"short"))
}

fn last_transferred(events: &[OperationEvent]) -> Option<u64> {
    events.iter().rev().find_map(|event| match event {
        OperationEvent::TransferProgress {
            transferred_bytes, ..
        } => Some(*transferred_bytes),
        _ => None,
    })
}

fn check_rewritten_as_read(row: &str, kernel: (u32, u32, u32)) -> Result<(), Box<dyn Error>> {
    let _kernel = KernelGuard::set(Some(kernel));
    // Before the kernel's coarse clock, which may run a tick behind.
    let tick = SystemTime::now() - Duration::from_secs(1);
    let _clock = ClockGuard::set(tick);
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(&folder)?;
    let db = folder.join("db");
    fs::write(&db, b"first")?;
    let (before, expected) = rereads(row, &db, tick)?;
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
        assert_eq!(last_transferred(&events), Some(5), "{row}");
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
        check_rewritten_as_read(row, kernel)?;
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
        set_times(&file, an_hour_ago()?, 0)?;
        let quirks = Quirks {
            mount_type: Some(("fuse.lxcfs", "rw")),
            virtual_sizes: true,
            ..Quirks::default()
        };

        let item = if in_folder { &folder } else { &file };
        let (moved, report, _) = move_across_volumes(item, quirks)?;

        let row = format!("in a folder: {in_folder}");
        assert_eq!(report, TransferReport::default(), "{row}");
        assert!(!file.exists(), "{row}");
        let moved = if in_folder {
            moved.join("cpuinfo")
        } else {
            moved
        };
        assert_eq!(fs::read(moved)?, b"processor : 0\n", "{row}");
    }
    Ok(())
}

/// A folder of files and of names linked in pairs, all with settled times.
fn linked_pairs_folder(folder: &Path) -> Result<(), Box<dyn Error>> {
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
    let hour_before = an_hour_ago()?;
    for file in ["a.txt", "sub/b.txt", "x", "m", "p"] {
        set_times(&folder.join(file), hour_before, 0)?;
    }
    Ok(())
}

/// The kernel evicts every entry between the copy and the deletion, while
/// files are rewritten. cifs counts links, which sshfs does not.
fn rewrite_and_renumber(changed: PathBuf) -> Racer {
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
    .map(|path| changed.join(path));
    Box::new(move || {
        fs::write(changed.join("sub/b.txt"), b"rewritten").expect("rewrite b");
        fs::write(changed.join("m"), b"M").expect("rewrite m");
        let resized = changed.join("p");
        let before = fs::metadata(&resized).expect("stat p");
        fs::write(&resized, b"longer").expect("rewrite p");
        put_times_back(&resized, &before).expect("put the times back");
        for path in &evicted {
            super::super::fs_quirks::renumbered(path);
        }
    })
}

#[test]
fn moves_off_mounts_without_stable_inodes_keep_only_what_changed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    linked_pairs_folder(&folder)?;
    let quirks = Quirks {
        mount_type: Some(("cifs", "rw,vers=3.1.1")),
        before: vec![(
            Op::OpenDir,
            "folder",
            1,
            rewrite_and_renumber(folder.clone()),
        )],
        ..Quirks::default()
    };

    let (moved, report, _) = move_across_volumes(&folder, quirks)?;

    assert_files(&moved, &[("sub/b.txt", "b"), ("sub/y", "x")], "")?;
    assert_eq!(names(&folder)?, ["m", "p", "sub"]);
    assert_eq!(names(&folder.join("sub"))?, ["b.txt", "n", "q"]);
    let expected = ["m", "p", "sub/b.txt", "sub/n", "sub/q"]
        .map(|path| kept(folder.join(path), KeptCause::Changed));
    assert_eq!(kept_in_order(report.kept), expected);
    Ok(())
}
